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
/// 使用済み管理のキー。chart_id が無い譜面でもパスで区別できる
fn chart_key(c: &Chart) -> String {
    format!("{:?}|{}", c.chart_id, c.path.as_deref().unwrap_or(""))
}

pub fn choose_chart(
    lib: &Library,
    scores: &Scores,
    table_id: i64,
    level: &str,
    used: &HashSet<String>,
    mode: &str,
) -> Option<Chart> {
    let pool: Vec<Chart> = lib
        .charts_in_level(table_id, level)
        .into_iter()
        .filter(|c| c.path.is_some() && !used.contains(&chart_key(c)))
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
    deadline: f64,
) -> (Option<crate::db::ResultDetail>, bool) {
    // 検出した結果は保持し、それ以降の待機は DB 読み取りの成否に
    // 依存させない (読み取りが失敗し続けても期限で必ず抜ける)
    let mut seen: Option<(f64, crate::db::ResultDetail)> = None;
    let mut killed = false;
    let mut abnormal = false;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                if !killed && !st.success() {
                    abnormal = true;
                }
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        match &seen {
            None => {
                if stop.load(Ordering::SeqCst) && scores.latest_result(chart, mark).is_none() {
                    // 停止要求があり結果も未保存なら即抜ける。結果が保存済みなら
                    // 集計に回す (Ctrl+C で完了した曲を取りこぼさない)
                    break;
                }
                if now_secs() >= deadline {
                    // セッション時間の上限: 結果が出ないまま無期限に待たない
                    let _ = child.kill();
                    killed = true;
                    break;
                }
                if let Some(det) = scores.latest_detail(chart, mark) {
                    let t = now_secs();
                    status.write(
                        true,
                        crate::status_fields!(
                            "state" => json!("result"),
                            "skip_deadline" => json!((t as i64) + result_seconds),
                            "skip_total" => json!(result_seconds),
                            "last_lamp" => lamp_name(det.clear_type)
                                .map(|s| json!(s))
                                .unwrap_or(Value::Null),
                            "last_score_fmt" => grade_fmt(det.ex_score, total_notes)
                                .map(|s| json!(s))
                                .unwrap_or(Value::Null),
                            "last_bp" => det.bp.map(|v| json!(v)).unwrap_or(Value::Null),
                            "last_gain" => json!(clear_points(rules, det.clear_type)),
                        ),
                    );
                    seen = Some((t, det));
                }
            }
            Some((t0, _)) => {
                // 停止要求時は短い猶予で切り上げる
                let grace = if stop.load(Ordering::SeqCst) {
                    2.0
                } else {
                    result_seconds as f64
                };
                if now_secs() - t0 >= grace || now_secs() >= deadline {
                    // 猶予かセッション期限の早い方で打ち切る
                    let _ = child.kill();
                    killed = true;
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    // 終了待ち (最大10秒 -> kill)
    let mut exited = false;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(st)) => {
                if !killed && !st.success() {
                    abnormal = true;
                }
                exited = true;
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => {
                exited = true;
                break;
            }
        }
    }
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    // 検出済みの結果を優先し、無ければ最後にもう一度だけ読みに行く
    let det = seen
        .map(|(_, d)| d)
        .or_else(|| scores.latest_detail(chart, mark));
    (det, abnormal)
}

/// bmz_dir から library / score / 難易度表を開く。
/// Err はそのまま表示できる日本語メッセージ
fn open_env(
    bmz_dir: &std::path::Path,
    cfg: &Value,
) -> Result<(Library, Scores, Vec<(i64, String)>), String> {
    let lib_db = bmz_dir.join("data").join("library.db");
    if !lib_db.exists() {
        return Err(format!(
            "library.db が見つかりません: {}",
            lib_db.display()
        ));
    }
    let score_db = find_score_db(bmz_dir, cfg)?;
    println!("score   : {}", score_db.display());
    let mut lib = Library::open(&lib_db)
        .map_err(|e| format!("library.db を開けませんでした: {e}"))?;
    let scores = Scores::open(&score_db)
        .map_err(|e| format!("score.db を開けませんでした: {e}"))?;
    let tables = lib.list_tables();
    if tables.is_empty() {
        return Err(
            "難易度表が登録されていません。bmz-player で表を追加してください。".into(),
        );
    }
    Ok((lib, scores, tables))
}

pub fn run(dry: bool, setup: bool, console: bool, cli: Option<serde_json::Value>) -> i32 {
    let (loaded_cfg, load_err) = if setup {
        (json!({}), None)
    } else {
        crate::config::load_config_checked()
    };
    if let Some(e) = load_err {
        // 壊れた設定を空設定として扱い、上書きで元を失わないよう止める
        eprintln!("エラー: {e} ファイルを修正するか削除してから再実行してください。");
        return if cli.is_some() { 2 } else { 1 };
    }
    let mut cfg = loaded_cfg;
    // CLI (run サブコマンド) の上書きはメモリ上だけ。sparta.json へは書き戻さない
    let cli_mode = cli.is_some();
    if let Some(ov) = &cli {
        if let Some(obj) = ov.as_object() {
            for (k, v) in obj {
                cfg_set(&mut cfg, k, v.clone());
            }
        }
    }

    let mut bmz_dir = match crate::config::resolve_bmz_dir(&cfg) {
        Some(d) => d,
        None => {
            if cli_mode {
                eprintln!(
                    "エラー: bmz-player が見つかりません。--bmz-dir で指定するか、標準インストール先に置くか、sparta.json の bmz_dir を設定してください。"
                );
                return 2;
            } else if !dry && !console {
                // GUI起動時: コンソールで聞かず、場所指定ダイアログで選んでもらう
                match crate::settings::locate_bmz(&cfg) {
                    Some(d) => d,
                    None => {
                        println!("キャンセルしました。");
                        return 0;
                    }
                }
            } else {
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
        }
    };
    cfg_set(&mut cfg, "bmz_dir", json!(bmz_dir.to_string_lossy()));
    let mut exe = bmz_dir.join("bmz-player.exe");
    println!("bmz_dir : {}", bmz_dir.display());
    println!("library : {}", bmz_dir.join("data").join("library.db").display());
    let (mut lib, mut scores, mut table_list) = match open_env(&bmz_dir, &cfg) {
        Ok(v) => v,
        Err(m) => {
            println!("{m}");
            return 1;
        }
    };
    // 設定GUI (Windows)。キャンセルされたら終了、使えなければ対話へ落とす
    let mut gui_done = false;
    let mut gui_persist = true;
    if !dry && !console && !cli_mode {
        let levels_map: std::collections::HashMap<i64, Vec<String>> = table_list
            .iter()
            .map(|(id, _)| (*id, lib.levels(*id)))
            .collect();
        let symbols_map: std::collections::HashMap<i64, String> = table_list
            .iter()
            .map(|(id, _)| (*id, lib.symbol(*id)))
            .collect();
        match crate::settings::show(
            &mut cfg,
            &bmz_dir.to_string_lossy(),
            &table_list,
            &levels_map,
            &symbols_map,
        ) {
            Some((true, persist)) => {
                gui_done = true;
                gui_persist = persist;
            }
            Some((false, _)) => {
                println!("キャンセルしました。");
                return 0;
            }
            None => {}
        }
    }
    // GUIで別のインストールが選ばれた場合は読み直す
    if gui_done {
        if let Some(nd) = cfg_str(&cfg, "bmz_dir") {
            let nd = std::path::PathBuf::from(nd);
            if nd != bmz_dir {
                match open_env(&nd, &cfg) {
                    Ok((l, s, t)) => {
                        println!("bmz_dir : {}", nd.display());
                        bmz_dir = nd;
                        exe = bmz_dir.join("bmz-player.exe");
                        lib = l;
                        scores = s;
                        table_list = t;
                    }
                    Err(m) => {
                        println!("{m}");
                        return 1;
                    }
                }
            }
        }
    }
    // ルールは GUI での変更後の cfg から作る (開始直後のセッションにも反映する)
    let rules = Rules::from_config(&cfg);
    // 列挙値の共通検証 (GUI/JSON/CLI どの入口でも不正値を子プロセスへ渡さない)
    for (key, valid) in [
        ("gauge", &crate::cli::GAUGES[..]),
        ("gas", &crate::cli::GASES[..]),
        ("pick_mode", &crate::cli::PICKS[..]),
    ] {
        if let Some(v) = cfg_str(&cfg, key) {
            if !valid.contains(&v.as_str()) {
                eprintln!(
                    "エラー: {key} の値が不正です: {v} (有効な値: {})",
                    valid.join(" | ")
                );
                return if cli_mode { 2 } else { 1 };
            }
        }
    }
    if dry {
        // 表の一覧は選択内容の検証より前に出す (壊れた保存済み設定でも
        // 一覧だけは確認できるようにする復旧用途)
        println!("登録済みの難易度表:");
        for (id, label) in &table_list {
            println!(
                "  ID {id}: {label} (レベル: {})",
                lib.levels(*id).join(", ")
            );
        }
    }
    if cli_mode && !dry {
        // CLI では対話に落とさない。必須項目が欠けたら設定エラーで終了する
        // (dry-run では必須チェックを外し、表の一覧確認に使えるようにする)
        let mut missing: Vec<&str> = Vec::new();
        if cfg_i64(&cfg, "table_id").is_none() {
            missing.push("table_id (--table)");
        }
        if cfg_str(&cfg, "start_level").is_none() {
            missing.push("start_level (--level)");
        }
        if cfg_i64(&cfg, "minutes").is_none() {
            missing.push("minutes (--minutes)");
        }
        if !missing.is_empty() {
            eprintln!(
                "エラー: 設定が不足しています: {}。引数で指定するか sparta.json に設定してください。",
                missing.join(", ")
            );
            return 2;
        }
    }
    if !gui_done && !cli_mode && (cfg.get("table_id").is_none() || setup) {
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
    if !gui_done && !cli_mode && (cfg.get("start_level").is_none() || setup) {
        let v = ask("開始レベル: ", Some(&levels[0])).unwrap_or_else(|| levels[0].clone());
        cfg_set(&mut cfg, "start_level", json!(v));
    }
    if !gui_done && !cli_mode && (cfg.get("minutes").is_none() || setup) {
        let v: i64 = ask("セッション時間 (分): ", Some("60"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        cfg_set(&mut cfg, "minutes", json!(v));
    }
    if (!gui_done || gui_persist) && !cli_mode {
        if !save_config(&cfg) {
            println!("警告: sparta.json への保存に失敗しました。");
        }
    }

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
    let minutes = cfg_i64(&cfg, "minutes").unwrap_or(60).clamp(1, 10080);
    let result_seconds = cfg_i64(&cfg, "result_seconds")
        .unwrap_or(20)
        .clamp(0, 3600);
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
        let mode = cfg_str(&cfg, "pick_mode").unwrap_or_else(|| "priority".to_string());
        match choose_chart(&lib, &scores, table_id, lv, &HashSet::new(), &mode) {
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

    let mut exit_code = 0;
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
    let mut used: HashSet<String> = HashSet::new();
    let mut played: i64 = 0;
    let mut cleared: i64 = 0;
    let mut max_idx = level_idx;
    let mut history: Vec<crate::report::SongRow> = Vec::new();
    let mut bucket_counts: std::collections::HashMap<&'static str, i64> =
        std::collections::HashMap::new();

    // HUD が前回の ended 状態を読んで即終了しないよう、先に初期状態を書く
    status.write(
        false,
        crate::status_fields!(
            "state" => json!("playing"),
            "level" => json!(level_label(&levels[level_idx])),
            "table" => json!(table_label),
            "points" => json!(0),
            "up_points" => json!(rules.up_points),
            "pts_exhard" => json!(rules.pts_exhard),
            "pts_hard" => json!(rules.pts_hard),
            "pts_clear" => json!(rules.pts_clear),
            "lose" => json!(0),
            "lose_max" => json!(rules.fail_streak),
            "song" => json!(0),
            "cleared" => json!(0),
            "deadline" => json!(deadline as i64),
            "title" => json!(""),
        ),
    );
    // 実効設定を HUD へ渡す (「保存せず開始」でも今回の設定が反映される)
    let _ = std::fs::write(
        crate::config::session_cfg_path(),
        serde_json::to_string(&cfg).unwrap_or_default(),
    );

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
    println!(
        "レベルUP: {}点制 (EX HARD以上 {}点 / HARD {}点 / CLEAR・EASY {}点)",
        rules.up_points, rules.pts_exhard, rules.pts_hard, rules.pts_clear
    );
    println!(
        "レベルDOWN: {}連続 fail / ゲージ {} / GAS {}",
        rules.fail_streak,
        cfg_str(&cfg, "gauge").unwrap_or_else(|| "プロファイル設定".to_string()),
        cfg_str(&cfg, "gas").unwrap_or_else(|| "プロファイル設定".to_string())
    );
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
        used.insert(chart_key(&chart));
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
                "pts_exhard" => json!(rules.pts_exhard),
                "pts_hard" => json!(rules.pts_hard),
                "pts_clear" => json!(rules.pts_clear),
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
        let mark = match scores.history_mark(&chart) {
            Some(m) => m,
            None => {
                println!("  結果履歴の基準を取得できませんでした (score.db 読み取りエラー)。この曲はスキップします。");
                continue;
            }
        };
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
                exit_code = 1;
                break;
            }
        };
        let (det, abnormal) = wait_with_result_timeout(
            &mut proc,
            &scores,
            &chart,
            mark,
            result_seconds,
            total_notes,
            &rules,
            &status,
            &stop,
            deadline,
        );
        if abnormal {
            println!("  bmz-player が異常終了しました。この曲はノーカウントとし、セッションを終了します。");
            exit_code = 1;
            break;
        }
        if stop.load(Ordering::SeqCst) && det.is_none() {
            let _ = proc.kill();
            break;
        }
        let det = match det {
            Some(d) => d,
            None => {
                println!("  結果なし (中断としてノーカウント)");
                continue;
            }
        };
        let result = det.clear_type;
        played += 1;
        let gained = clear_points(&rules, result);
        if result >= CLEAR_MIN {
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
        history.push(crate::report::SongRow {
            level: level_label(&lv),
            title: chart.title.clone(),
            lamp: lamp_name(result).unwrap_or("?").to_string(),
            score_fmt: grade_fmt(det.ex_score, total_notes),
            bp: det.bp,
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
    let _ = std::fs::remove_file(crate::config::session_cfg_path());
    if played > 0 {
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
            table_id,
            max_level_idx: max_idx as i64,
            stem: String::new(),
            table: table_label.clone(),
            start_level: level_label(&levels[start_level_idx]),
            final_level: level_label(&levels[level_idx]),
            max_level: level_label(&levels[max_idx]),
            played,
            cleared,
            lamp_counts,
            songs: history,
        };
        // 日次集計と HTML 生成は常に行い、report 設定は
        // 「ブラウザで自動的に開くか」だけを制御する
        if let Some(p) = crate::report::merge_and_generate(&rep) {
            println!("結果レポート: {}", p.display());
            if cfg_bool(&cfg, "report", true) {
                crate::report::open_in_browser(&p);
            }
        }
    }
    println!(
        "\n=== 終了: {played}曲プレイ / {cleared}クリア / 最終レベル {} ===",
        levels[level_idx]
    );
    exit_code
}
