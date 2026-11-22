//! スパルタのメインセッション (選曲 -> 起動 -> 結果待ち -> レベル昇降)

use crate::config::*;
use crate::db::{Chart, Library, Scores};
use crate::rules::*;
use crate::status::StatusWriter;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn ask(prompt: &str, default: Option<&str>) -> Option<String> {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return default.map(|s| s.to_string());
    }
    let s = line.trim().to_string();
    if s.is_empty() {
        default.map(|s| s.to_string())
    } else {
        Some(s)
    }
}

/// 選曲。mode: "priority" (未プレイ/未クリア優先) / "noplay_failed" / "random"
pub fn choose_chart(
    lib: &Library,
    scores: &Scores,
    table_id: i64,
    level: &str,
    used: &HashSet<Option<i64>>,
    mode: &str,
) -> Option<Chart> {
    let pool: Vec<Chart> = lib
        .charts_in_level(table_id, level)
        .into_iter()
        .filter(|c| c.path.is_some() && !used.contains(&c.chart_id))
        .collect();
    if pool.is_empty() {
        return None;
    }
    let mut rng = rand::thread_rng();
    let mut pick = |group: &[&Chart]| -> Option<Chart> {
        use rand::seq::SliceRandom;
        group.choose(&mut rng).map(|c| (*c).clone())
    };
    if mode == "random" {
        let refs: Vec<&Chart> = pool.iter().collect();
        return pick(&refs);
    }
    if mode == "noplay_failed" {
        let first: Vec<&Chart> = pool
            .iter()
            .filter(|c| !scores.played_ever(c) || scores.best_clear(c) <= 1)
            .collect();
        let uncleared: Vec<&Chart> = pool
            .iter()
            .filter(|c| scores.best_clear(c) < CLEAR_MIN)
            .collect();
        let all: Vec<&Chart> = pool.iter().collect();
        for group in [first, uncleared, all] {
            if !group.is_empty() {
                return pick(&group);
            }
        }
        return None;
    }
    let unplayed: Vec<&Chart> = pool.iter().filter(|c| !scores.played_ever(c)).collect();
    let uncleared: Vec<&Chart> = pool
        .iter()
        .filter(|c| scores.best_clear(c) < CLEAR_MIN)
        .collect();
    let all: Vec<&Chart> = pool.iter().collect();
    for group in [unplayed, uncleared, all] {
        if !group.is_empty() {
            return pick(&group);
        }
    }
    None
}

/// bmz の終了を待つ。結果が保存されたら result_seconds 秒後に強制終了し、
/// リザルト画面を自動で抜ける (スコア保存と IR 送信は保存処理側で完了する)。
#[allow(clippy::too_many_arguments)]
fn wait_with_result_timeout(
    child: &mut Child,
    scores: &Scores,
    chart: &Chart,
    mark: i64,
    result_seconds: i64,
    total_notes: Option<i64>,
    rules: &Rules,
    status: &StatusWriter,
    stop: &AtomicBool,
) {
    let mut seen_at: Option<f64> = None;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => break,
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if scores.latest_result(chart, mark).is_some() {
            if seen_at.is_none() {
                let t = now_secs();
                seen_at = Some(t);
                // HUD 用の付加情報。失敗してもセッションは止めない
                let det = scores.latest_detail(chart, mark);
                let (lamp, score_fmt, bp, gain) = match &det {
                    Some(d) => (
                        lamp_name(d.clear_type).map(|s| json!(s)).unwrap_or(Value::Null),
                        grade_fmt(d.ex_score, total_notes)
                            .map(|s| json!(s))
                            .unwrap_or(Value::Null),
                        d.bp.map(|v| json!(v)).unwrap_or(Value::Null),
                        clear_points(rules, d.clear_type),
                    ),
                    None => (Value::Null, Value::Null, Value::Null, 0),
                };
                status.write(
                    true,
                    crate::status_fields!(
                        "state" => json!("result"),
                        "skip_deadline" => json!((t as i64) + result_seconds),
                        "skip_total" => json!(result_seconds),
                        "last_lamp" => lamp,
                        "last_score_fmt" => score_fmt,
                        "last_bp" => bp,
                        "last_gain" => json!(gain),
                    ),
                );
            } else if now_secs() - seen_at.unwrap() >= result_seconds as f64 {
                let _ = child.kill();
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    // 終了待ち (最大10秒 -> kill)
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn run(dry: bool, setup: bool, console: bool) -> i32 {
    let mut cfg = if setup { json!({}) } else { load_config() };
    let rules = Rules::from_config(&cfg);

    let bmz_dir = match find_bmz_dir(&cfg) {
        Some(d) => d,
        None => {
            let p = ask("bmz-player.exe のあるフォルダをフルパスで入力: ", None);
            match p {
                Some(p) if std::path::Path::new(&p).join("bmz-player.exe").exists() => {
                    std::path::PathBuf::from(p)
                }
                _ => {
                    println!("bmz-player.exe が見つかりませんでした。終了します。");
                    return 1;
                }
            }
        }
    };
    cfg_set(&mut cfg, "bmz_dir", json!(bmz_dir.to_string_lossy()));
    let exe = bmz_dir.join("bmz-player.exe");
    let lib_db = bmz_dir.join("data").join("library.db");
    if !lib_db.exists() {
        println!("library.db が見つかりません: {}", lib_db.display());
        return 1;
    }
    let score_db = match find_score_db(&bmz_dir, &cfg) {
        Some(p) => p,
        None => {
            println!("score.db が見つかりませんでした。");
            return 1;
        }
    };
    println!("bmz_dir : {}", bmz_dir.display());
    println!("library : {}", lib_db.display());
    println!("score   : {}", score_db.display());

    let mut lib = match Library::open(&lib_db) {
        Ok(l) => l,
        Err(e) => {
            println!("library.db を開けませんでした: {e}");
            return 1;
        }
    };
    let scores = match Scores::open(&score_db) {
        Ok(s) => s,
        Err(e) => {
            println!("score.db を開けませんでした: {e}");
            return 1;
        }
    };

    let table_list = lib.list_tables();
    if table_list.is_empty() {
        println!("難易度表が登録されていません。bmz-player で表を追加してください。");
        return 1;
    }
    // 設定GUI (Windows)。キャンセルされたら終了、使えなければ対話へ落とす
    let mut gui_done = false;
    if !dry && !console {
        let levels_map: std::collections::HashMap<i64, Vec<String>> = table_list
            .iter()
            .map(|(id, _)| (*id, lib.levels(*id)))
            .collect();
        match crate::settings::show(
            &mut cfg,
            &bmz_dir.to_string_lossy(),
            &table_list,
            &levels_map,
        ) {
            Some(true) => {
                gui_done = true;
                save_config(&cfg);
            }
            Some(false) => {
                println!("キャンセルしました。");
                return 0;
            }
            None => {}
        }
    }
    if !gui_done && (cfg.get("table_id").is_none() || setup) {
        println!("\n登録済みの難易度表:");
        for (i, (_, label)) in table_list.iter().enumerate() {
            println!("  {i}: {label}");
        }
        let idx: usize = ask("使う表の番号: ", Some("0"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let idx = idx.min(table_list.len() - 1);
        cfg_set(&mut cfg, "table_id", json!(table_list[idx].0));
    }
    let table_id = cfg_i64(&cfg, "table_id").unwrap_or(table_list[0].0);
    let table_label = table_list
        .iter()
        .find(|(id, _)| *id == table_id)
        .map(|(_, l)| l.clone())
        .unwrap_or_default();

    let levels = lib.levels(table_id);
    if levels.is_empty() {
        println!("選んだ表にレベルがありません。");
        return 1;
    }
    println!("レベル一覧: {}", levels.join(", "));
    if !gui_done && (cfg.get("start_level").is_none() || setup) {
        let v = ask("開始レベル: ", Some(&levels[0])).unwrap_or_else(|| levels[0].clone());
        cfg_set(&mut cfg, "start_level", json!(v));
    }
    if !gui_done && (cfg.get("minutes").is_none() || setup) {
        let v: i64 = ask("セッション時間 (分): ", Some("60"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        cfg_set(&mut cfg, "minutes", json!(v));
    }
    save_config(&cfg);

    let level = cfg_str(&cfg, "start_level").unwrap_or_else(|| levels[0].clone());
    let mut level_idx = match levels.iter().position(|l| *l == level) {
        Some(i) => i,
        None => {
            println!("開始レベル {level} が表にありません。");
            return 1;
        }
    };
    let symbol = lib.symbol(table_id);
    let level_label = |lv: &str| -> String {
        if symbol.is_empty() {
            format!("Lv.{lv}")
        } else {
            format!("{symbol}{lv}")
        }
    };

    let start_level_idx = level_idx;
    let minutes = cfg_i64(&cfg, "minutes").unwrap_or(60);
    let result_seconds = cfg_i64(&cfg, "result_seconds").unwrap_or(20);
    let deadline = now_secs() + (minutes * 60) as f64;
    let status = StatusWriter::new();

    if dry {
        println!("\n[dry-run] 起動はしません。最初の選曲だけ表示します。");
        let lv = &levels[level_idx];
        let n_entries = lib.count_entries(table_id, lv);
        let charts = lib.charts_in_level(table_id, lv);
        let n_path = charts.iter().filter(|c| c.path.is_some()).count();
        println!(
            "診断: 表エントリ {n_entries}件 / ライブラリ照合 {}件 / パス解決 {n_path}件",
            charts.len()
        );
        if let Some(first) = charts.first() {
            println!(
                "先頭の譜面: chart_id={} title={} path={}",
                first.chart_id.map(|v| v.to_string()).unwrap_or_default(),
                first.title,
                first.path.as_deref().unwrap_or("")
            );
        }
        match choose_chart(&lib, &scores, table_id, lv, &HashSet::new(), "priority") {
            Some(c) => println!(
                "選曲: chart_id={} title={} path={}",
                c.chart_id.map(|v| v.to_string()).unwrap_or_default(),
                c.title,
                c.path.as_deref().unwrap_or("")
            ),
            None => println!("選曲: なし"),
        }
        return 0;
    }

    // Ctrl+C で安全に終了する
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop2 = stop.clone();
        let _ = ctrlc::set_handler(move || {
            stop2.store(true, Ordering::SeqCst);
        });
    }

    let mut points: i64 = 0;
    let mut lose: i64 = 0;
    let mut used: HashSet<Option<i64>> = HashSet::new();
    let mut played: i64 = 0;
    let mut cleared: i64 = 0;
    let mut max_idx = level_idx;
    let mut history: Vec<crate::report::SongRow> = Vec::new();
    let mut bucket_counts: std::collections::HashMap<&'static str, i64> =
        std::collections::HashMap::new();

    // HUD は同じ exe を子プロセスとして起動する (状態ファイル経由で分離)
    let mut hud_proc: Option<Child> = None;
    if cfg_bool(&cfg, "hud", true) {
        if let Ok(exe_path) = std::env::current_exe() {
            hud_proc = Command::new(exe_path).arg("hud").spawn().ok();
        }
    }

    println!(
        "\n=== スパルタ開始: レベル {} / 上限 {minutes}分 ===",
        levels[level_idx]
    );
    println!("レベルUP: EXHARD以上 1回 / HARD 2回 / NORMAL・EASY 3回 (点数制)");
    println!("レベルDOWN: 3連続 fail / ゲージはプロファイル設定のまま");
    println!("終了: Ctrl+C\n");

    while now_secs() < deadline && !stop.load(Ordering::SeqCst) {
        let lv = levels[level_idx].clone();
        let mode = cfg_str(&cfg, "pick_mode").unwrap_or_else(|| "priority".to_string());
        let chart = match choose_chart(&lib, &scores, table_id, &lv, &used, &mode) {
            Some(c) => c,
            None => {
                println!("レベル {lv} に未使用の譜面がありません。終了します。");
                break;
            }
        };
        used.insert(chart.chart_id);
        let remain = ((deadline - now_secs()) / 60.0) as i64;
        println!(
            "[{}曲目] レベル {lv} | UPまで {points}/{}点 / 連敗 {lose} | 残り約{remain}分",
            played + 1,
            rules.up_points
        );
        println!("  -> {}", chart.title);
        let best = scores.best_detail(&chart);
        let total_notes = lib.total_notes(&chart);
        status.write(
            false,
            crate::status_fields!(
                "state" => json!("playing"),
                "level" => json!(level_label(&lv)),
                "table" => json!(table_label),
                "points" => json!(points),
                "up_points" => json!(rules.up_points),
                "lose" => json!(lose),
                "lose_max" => json!(rules.fail_streak),
                "song" => json!(played + 1),
                "cleared" => json!(cleared),
                "deadline" => json!(deadline as i64),
                "title" => json!(chart.title),
                "prev_lamp" => best.as_ref().map(|b| json!(b.lamp)).unwrap_or(Value::Null),
                "prev_score" => best.as_ref().and_then(|b| b.ex_score).map(|v| json!(v)).unwrap_or(Value::Null),
                "prev_score_fmt" => best.as_ref().and_then(|b| grade_fmt(b.ex_score, total_notes)).map(|s| json!(s)).unwrap_or(Value::Null),
                "prev_bp" => best.as_ref().and_then(|b| b.bp).map(|v| json!(v)).unwrap_or(Value::Null),
            ),
        );
        let mark = scores.history_mark(&chart);
        // Decide画面は通さず直接Playへ入る (Resultは表示したまま。
        // 終了は wait_with_result_timeout が結果保存後に行う)
        let mut cmd = Command::new(&exe);
        cmd.arg("--skip-decide").arg(chart.path.as_deref().unwrap_or(""));
        if let Some(g) = cfg_str(&cfg, "gauge") {
            cmd.arg("--gauge").arg(g);
        }
        if let Some(g) = cfg_str(&cfg, "gas") {
            cmd.arg("--gas").arg(g);
        }
        let mut proc = match cmd.spawn() {
            Ok(p) => p,
            Err(e) => {
                println!("bmz-player の起動に失敗しました: {e}");
                break;
            }
        };
        wait_with_result_timeout(
            &mut proc,
            &scores,
            &chart,
            mark,
            result_seconds,
            total_notes,
            &rules,
            &status,
            &stop,
        );
        if stop.load(Ordering::SeqCst) {
            let _ = proc.kill();
            break;
        }
        let result = scores.latest_result(&chart, mark);
        let result = match result {
            Some(r) => r,
            None => {
                println!("  結果なし (中断としてノーカウント)");
                continue;
            }
        };
        played += 1;
        let gained = clear_points(&rules, result);
        if gained > 0 {
            cleared += 1;
            points += gained;
            lose = 0;
            println!("  CLEAR (lamp={result}) +{gained}点 -> {points}点");
        } else {
            points = 0;
            lose += 1;
            println!("  FAIL (lamp={result}) 連敗 {lose}");
        }
        let mut event: Option<String> = None;
        if points >= rules.up_points && level_idx < levels.len() - 1 {
            level_idx += 1;
            points = 0;
            lose = 0;
            max_idx = max_idx.max(level_idx);
            println!("  *** レベルUP -> {} ***", levels[level_idx]);
            event = Some(format!("LEVEL UP → {}", level_label(&levels[level_idx])));
        } else if lose >= rules.fail_streak && level_idx > 0 {
            level_idx -= 1;
            points = 0;
            lose = 0;
            println!("  *** レベルDOWN -> {} ***", levels[level_idx]);
            event = Some(format!("LEVEL DOWN → {}", level_label(&levels[level_idx])));
        }
        *bucket_counts.entry(crate::report::lamp_bucket(result)).or_insert(0) += 1;
        let det = scores.latest_detail(&chart, mark);
        history.push(crate::report::SongRow {
            level: level_label(&lv),
            title: chart.title.clone(),
            lamp: lamp_name(result).unwrap_or("?").to_string(),
            score_fmt: det
                .as_ref()
                .and_then(|d| grade_fmt(d.ex_score, total_notes)),
            bp: det.as_ref().and_then(|d| d.bp),
            gained,
            event,
        });
    }
    if stop.load(Ordering::SeqCst) {
        println!("\n中断しました。");
    }
    status.write(
        false,
        crate::status_fields!(
            "state" => json!("ended"),
            "level" => json!(level_label(&levels[level_idx])),
            "song" => json!(played),
            "cleared" => json!(cleared),
        ),
    );
    if let Some(mut hud) = hud_proc {
        // HUD は ended を見て自動で閉じる。最大8秒待って残れば終了する
        for _ in 0..80 {
            match hud.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = hud.kill();
        let _ = hud.wait();
    }
    if played > 0 && cfg_bool(&cfg, "report", true) {
        let lamp_counts: Vec<(String, i64)> = crate::report::LAMP_ORDER
            .iter()
            .filter_map(|l| bucket_counts.get(l).map(|n| (l.to_string(), *n)))
            .collect();
        let end_now = now_secs();
        let day_start_hour = cfg_i64(&cfg, "day_start_hour").unwrap_or(0);
        let (day_key, date_label) = crate::report::logical_day(end_now, day_start_hour);
        let rep = crate::report::SessionReport {
            ended_unix: end_now,
            date_label,
            day_key,
            table: table_label.clone(),
            start_level: level_label(&levels[start_level_idx]),
            final_level: level_label(&levels[level_idx]),
            max_level: level_label(&levels[max_idx]),
            played,
            cleared,
            lamp_counts,
            songs: history,
        };
        if let Some(p) = crate::report::merge_and_generate(&rep) {
            println!("結果レポート: {}", p.display());
            crate::report::open_in_browser(&p);
        }
    }
    println!(
        "\n=== 終了: {played}曲プレイ / {cleared}クリア / 最終レベル {} ===",
        levels[level_idx]
    );
    0
}
