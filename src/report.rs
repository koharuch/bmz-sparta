//! セッション結果レポート (HTML) と X 投稿リンク。
//!
//! 文面は稼働中の BMS トラッカーの「成果報告」と同じ形式に揃えたローカル版:
//! 2026/11/22
//! スパルタ 12曲プレイ（EXH 2 / HARD 5 / NORMAL 2 / EASY 1 / FAILED 2）
//! レベル ★7 → ★9
//! #BMS
//!
//! 結果は「日」単位でマージされる: セッション終了時にその日の蓄積
//! (sparta-day.json) と合算し、result-YYYYMMDD.html を書き直して
//! 既定ブラウザで開く。日付の切り替わりは sparta.json の
//! "day_start_hour" (既定 0 = 0:00) で変更できる。
//! レポート自体は "report": false で無効化できる。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct SongRow {
    pub level: String,
    pub title: String,
    pub lamp: String,
    pub score_fmt: Option<String>,
    pub bp: Option<i64>,
    pub gained: i64,
    pub event: Option<String>,
}

#[derive(Clone)]
pub struct SessionReport {
    pub ended_unix: f64,
    /// 論理日付の表示形 "2026/11/22"
    pub date_label: String,
    /// 論理日付のキー "20261122" (ファイル名・マージ判定用)
    pub day_key: String,
    /// 難易度表ID (同日でも表が違えば別集計にするためのキー)
    pub table_id: i64,
    /// 最高到達レベルの、その表における順位 (文字列比較は使わない)
    pub max_level_idx: i64,
    /// 結果HTMLのファイル名の幹 ("" なら day_key。同日に2つ目の表は "-t<id>" 付き)
    pub stem: String,
    pub table: String,
    pub start_level: String,
    pub final_level: String,
    pub max_level: String,
    pub played: i64,
    pub cleared: i64,
    /// (短縮ラベル, 件数)。強い順、0件は含めない
    pub lamp_counts: Vec<(String, i64)>,
    pub songs: Vec<SongRow>,
}

/// clear_type をトラッカー式の短縮ラベルへ寄せるバケット名
pub fn lamp_bucket(clear_type: i64) -> &'static str {
    match clear_type {
        7.. => "EXH",
        6 => "HARD",
        5 => "NORMAL",
        4 => "EASY",
        2 | 3 => "ASSIST",
        _ => "FAILED",
    }
}

pub const LAMP_ORDER: [&str; 6] = ["EXH", "HARD", "NORMAL", "EASY", "ASSIST", "FAILED"];

fn lamp_color(label: &str) -> &'static str {
    match label {
        "EXH" => "#9b5de5",
        "HARD" => "#ef476f",
        "NORMAL" => "#118ab2",
        "EASY" => "#06d6a0",
        "ASSIST" => "#8d99ae",
        _ => "#6c757d",
    }
}

// ---------- 日時 (JST固定、chronoなしで civil 変換) ----------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// (年, 月, 日, 時, 分, 秒) JST
pub fn jst_parts(unix: f64) -> (i64, u32, u32, u32, u32, u32) {
    let t = unix as i64 + 9 * 3600;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (secs / 3600) as u32,
        ((secs % 3600) / 60) as u32,
        (secs % 60) as u32,
    )
}

/// 論理日付: 時刻から day_start_hour を引いた日付を「その日」とする。
/// (day_key "20261122", date_label "2026/11/22") を返す
pub fn logical_day(unix: f64, day_start_hour: i64) -> (String, String) {
    let h = day_start_hour.clamp(0, 23) as f64;
    let (y, m, d, _, _, _) = jst_parts(unix - h * 3600.0);
    (format!("{y}{m:02}{d:02}"), format!("{y}/{m:02}/{d:02}"))
}

// ---------- テキスト組み立て ----------

fn lamp_breakdown(r: &SessionReport) -> String {
    r.lamp_counts
        .iter()
        .map(|(label, n)| format!("{label} {n}"))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// X 投稿文面 (トラッカーの成果報告と同じ骨格)
pub fn post_text(r: &SessionReport) -> String {
    let mut lines = vec![
        r.date_label.clone(),
        format!("スパルタ {}曲プレイ（{}）", r.played, lamp_breakdown(r)),
    ];
    if r.start_level == r.final_level {
        lines.push(format!("レベル {}", r.final_level));
    } else {
        lines.push(format!("レベル {} → {}", r.start_level, r.final_level));
    }
    if r.max_level != r.final_level && r.max_level != r.start_level {
        lines.push(format!("最高到達 {}", r.max_level));
    }
    lines.push("#BMS".to_string());
    lines.join("\n")
}

/// RFC 3986 の unreserved 以外を %XX 化する
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn x_intent_url(r: &SessionReport) -> String {
    format!(
        "https://twitter.com/intent/tweet?text={}",
        percent_encode(&post_text(r))
    )
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

// ---------- HTML 生成 ----------

pub fn render_html(r: &SessionReport) -> String {
    let (_, _, _, hh, mm, _) = jst_parts(r.ended_unix);
    let date = r.date_label.clone();
    let time = format!("{hh:02}:{mm:02}");
    let breakdown = lamp_breakdown(r);
    // ランプバー (件数比の積み上げ横バー)
    let total: i64 = r.lamp_counts.iter().map(|(_, n)| *n).sum();
    let mut bar = String::new();
    if total > 0 {
        for (label, n) in &r.lamp_counts {
            let pct = *n as f64 * 100.0 / total as f64;
            bar.push_str(&format!(
                "<div style=\"width:{pct:.1}%;background:{}\" title=\"{}\"></div>",
                lamp_color(label),
                esc(&format!("{label} {n}"))
            ));
        }
    }
    let level_line = if r.start_level == r.final_level {
        format!("レベル {}", esc(&r.final_level))
    } else {
        format!(
            "レベル {} → {}",
            esc(&r.start_level),
            esc(&r.final_level)
        )
    };
    let max_line = if r.max_level != r.final_level && r.max_level != r.start_level {
        format!("<div class=\"sub\">最高到達 {}</div>", esc(&r.max_level))
    } else {
        String::new()
    };
    let mut rows = String::new();
    for (i, s) in r.songs.iter().enumerate() {
        let lamp_html = if s.lamp == "FAILED" {
            format!("<span class=\"fail\">{}</span>", esc(&s.lamp))
        } else {
            esc(&s.lamp)
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"title\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            i + 1,
            esc(&s.level),
            esc(&s.title),
            lamp_html,
            esc(s.score_fmt.as_deref().unwrap_or("-")),
            s.bp.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            if s.gained > 0 {
                format!("+{}", s.gained)
            } else {
                "0".to_string()
            },
            esc(s.event.as_deref().unwrap_or("")),
        ));
    }
    format!(
        r#"<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="utf-8">
<title>BMS成果報告 {date}</title>
<style>
body {{ font-family: "Meiryo", "Yu Gothic", sans-serif; background: #17181c; color: #eee; margin: 24px; }}
.card {{ max-width: 880px; margin: 0 auto; background: #22242b; border-radius: 12px; padding: 24px 28px; }}
h1 {{ font-size: 22px; margin: 0 0 4px; }}
.meta {{ color: #9aa0aa; font-size: 13px; margin-bottom: 16px; }}
.big {{ font-size: 17px; line-height: 1.7; }}
.sub {{ color: #9aa0aa; font-size: 14px; }}
.bar {{ display: flex; height: 18px; border-radius: 6px; overflow: hidden; margin: 14px 0 4px; background: #333; }}
table {{ width: 100%; border-collapse: collapse; margin-top: 18px; font-size: 14px; }}
th, td {{ padding: 6px 8px; border-bottom: 1px solid #34363f; text-align: left; white-space: nowrap; }}
td.title {{ white-space: normal; }}
th {{ color: #9aa0aa; font-weight: normal; }}
.fail {{ color: #ff5064; }}
.xpost {{ display: inline-block; margin-top: 20px; background: #000; color: #fff; border: 1px solid #555;
  padding: 10px 22px; border-radius: 999px; text-decoration: none; font-weight: bold; }}
.foot {{ color: #666; font-size: 12px; margin-top: 16px; }}
</style>
</head>
<body>
<div class="card">
<h1>{date}</h1>
<div class="meta">{time} 終了 ／ 対象表: {table}</div>
<div class="big">スパルタ {played}曲プレイ ／ {cleared}クリア<br>{breakdown}</div>
<div class="bar">{bar}</div>
<div class="big">{level_line}</div>
{max_line}
<table>
<tr><th>#</th><th>レベル</th><th>曲名</th><th>ランプ</th><th>スコア</th><th>BP</th><th>pt</th><th></th></tr>
{rows}
</table>
<a class="xpost" href="{xurl}" target="_blank" rel="noopener">Xにポストする</a>
<div class="foot">bmz-sparta ローカル結果レポート (同日分はマージされます)</div>
</div>
</body>
</html>
"#,
        date = date,
        time = time,
        table = esc(&r.table),
        played = r.played,
        cleared = r.cleared,
        breakdown = esc(&breakdown),
        bar = bar,
        level_line = level_line,
        max_line = max_line,
        rows = rows,
        xurl = esc(&x_intent_url(r)),
    )
}

/// result-YYYYMMDD.html (論理日付) を exe と同じフォルダへ書き出す
pub fn generate(r: &SessionReport) -> Option<PathBuf> {
    let stem = if r.stem.is_empty() { &r.day_key } else { &r.stem };
    let name = format!("result-{stem}.html");
    let path = crate::config::base_dir().join(name);
    std::fs::write(&path, render_html(r)).ok()?;
    Some(path)
}

// ---------- 同日マージ (sparta-day.json に蓄積) ----------

fn day_state_path() -> PathBuf {
    crate::config::base_dir().join("sparta-day.json")
}

fn song_to_json(s: &SongRow) -> Value {
    json!({
        "level": s.level,
        "title": s.title,
        "lamp": s.lamp,
        "score_fmt": s.score_fmt,
        "bp": s.bp,
        "gained": s.gained,
        "event": s.event,
    })
}

fn song_from_json(v: &Value) -> Option<SongRow> {
    Some(SongRow {
        level: v.get("level")?.as_str()?.to_string(),
        title: v.get("title")?.as_str()?.to_string(),
        lamp: v.get("lamp")?.as_str()?.to_string(),
        score_fmt: v.get("score_fmt").and_then(|x| x.as_str()).map(|x| x.to_string()),
        bp: v.get("bp").and_then(|x| x.as_i64()),
        gained: v.get("gained").and_then(|x| x.as_i64()).unwrap_or(0),
        event: v.get("event").and_then(|x| x.as_str()).map(|x| x.to_string()),
    })
}

fn report_to_json(r: &SessionReport) -> Value {
    let mut lamps = serde_json::Map::new();
    for (label, n) in &r.lamp_counts {
        lamps.insert(label.clone(), json!(n));
    }
    json!({
        "day_key": r.day_key,
        "table_id": r.table_id,
        "max_level_idx": r.max_level_idx,
        "stem": r.stem,
        "date_label": r.date_label,
        "table": r.table,
        "start_level": r.start_level,
        "final_level": r.final_level,
        "max_level": r.max_level,
        "played": r.played,
        "cleared": r.cleared,
        "lamp_counts": lamps,
        "songs": r.songs.iter().map(song_to_json).collect::<Vec<_>>(),
    })
}

fn report_from_json(v: &Value) -> Option<SessionReport> {
    let mut lamp_counts = Vec::new();
    if let Some(obj) = v.get("lamp_counts").and_then(|x| x.as_object()) {
        for label in LAMP_ORDER {
            if let Some(n) = obj.get(label).and_then(|x| x.as_i64()) {
                lamp_counts.push((label.to_string(), n));
            }
        }
    }
    let songs = v
        .get("songs")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(song_from_json).collect())
        .unwrap_or_default();
    Some(SessionReport {
        ended_unix: 0.0,
        date_label: v.get("date_label")?.as_str()?.to_string(),
        day_key: v.get("day_key")?.as_str()?.to_string(),
        table_id: v.get("table_id").and_then(|x| x.as_i64()).unwrap_or(0),
        max_level_idx: v
            .get("max_level_idx")
            .and_then(|x| x.as_i64())
            .unwrap_or(-1),
        stem: v
            .get("stem")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        table: v.get("table")?.as_str()?.to_string(),
        start_level: v.get("start_level")?.as_str()?.to_string(),
        final_level: v.get("final_level")?.as_str()?.to_string(),
        max_level: v.get("max_level")?.as_str()?.to_string(),
        played: v.get("played")?.as_i64()?,
        cleared: v.get("cleared")?.as_i64()?,
        lamp_counts,
        songs,
    })
}

/// 前回までの同日分と今回分をマージする (純粋関数)
pub fn merge_reports(prev: &SessionReport, cur: &SessionReport) -> SessionReport {
    let mut counts: HashMap<String, i64> = HashMap::new();
    for (label, n) in prev.lamp_counts.iter().chain(cur.lamp_counts.iter()) {
        *counts.entry(label.clone()).or_insert(0) += n;
    }
    let lamp_counts: Vec<(String, i64)> = LAMP_ORDER
        .iter()
        .filter_map(|l| counts.get(*l).map(|n| (l.to_string(), *n)))
        .collect();
    let mut songs = prev.songs.clone();
    songs.extend(cur.songs.iter().cloned());
    let (max_level, max_level_idx) = if cur.max_level_idx > prev.max_level_idx {
        (cur.max_level.clone(), cur.max_level_idx)
    } else {
        (prev.max_level.clone(), prev.max_level_idx)
    };
    SessionReport {
        ended_unix: cur.ended_unix,
        date_label: cur.date_label.clone(),
        day_key: cur.day_key.clone(),
        table_id: prev.table_id,
        max_level_idx,
        stem: prev.stem.clone(),
        table: prev.table.clone(),
        start_level: prev.start_level.clone(),
        final_level: cur.final_level.clone(),
        max_level,
        played: prev.played + cur.played,
        cleared: prev.cleared + cur.cleared,
        lamp_counts,
        songs,
    }
}

/// セッション結果を同日分へマージし、状態保存＋HTML生成まで行う。
/// 日次状態は (論理日付, 表ID) ごとに分けて保持する (別の表を混ぜない)
pub fn merge_and_generate(cur: &SessionReport) -> Option<PathBuf> {
    // 既存の状態を読む。新形式 {"reports": {key: report}} と
    // 旧形式 (単一レポート) の両方に対応する
    let mut reports: serde_json::Map<String, Value> = Default::default();
    if let Ok(t) = std::fs::read_to_string(day_state_path()) {
        if let Ok(v) = serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')) {
            if let Some(m) = v.get("reports").and_then(|x| x.as_object()) {
                reports = m.clone();
            } else if let Some(prev) = report_from_json(&v) {
                let k = format!("{}:{}", prev.day_key, prev.table_id);
                reports.insert(k, report_to_json(&prev));
            }
        }
    }
    let mut cur = cur.clone();
    if cur.stem.is_empty() {
        // その日の最初の表は result-YYYYMMDD.html、2つ目以降は表IDを付ける
        let prefix = format!("{}:", cur.day_key);
        let has_other = reports.keys().any(|k| k.starts_with(&prefix));
        cur.stem = if has_other {
            format!("{}-t{}", cur.day_key, cur.table_id)
        } else {
            cur.day_key.clone()
        };
    }
    let key = format!("{}:{}", cur.day_key, cur.table_id);
    let combined = match reports.get(&key).and_then(report_from_json) {
        Some(prev) => merge_reports(&prev, &cur),
        None => cur,
    };
    reports.insert(key, report_to_json(&combined));
    let state = serde_json::json!({ "reports": reports });
    let text = serde_json::to_string_pretty(&state).unwrap_or_default();
    let path = day_state_path();
    let tmp = path.with_extension("json.tmp");
    let saved = std::fs::write(&tmp, text)
        .ok()
        .and_then(|_| std::fs::rename(&tmp, &path).ok())
        .is_some();
    if !saved {
        eprintln!(
            "警告: 日次集計 (sparta-day.json) の保存に失敗しました。今回の結果は次回のレポートに引き継がれません。"
        );
    }
    let out = generate(&combined);
    if out.is_none() {
        eprintln!("警告: 結果レポートHTMLの保存に失敗しました。");
    }
    out
}

/// 既定ブラウザでファイルを開く (Windows)
#[cfg(windows)]
pub fn open_in_browser(path: &std::path::Path) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let op: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(not(windows))]
pub fn open_in_browser(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionReport {
        SessionReport {
            ended_unix: 0.0, // 1970-01-01 09:00 JST
            date_label: "1970/01/01".into(),
            day_key: "19700101".into(),
            table_id: 1,
            max_level_idx: 9,
            stem: String::new(),
            table: "発狂BMS難度表".into(),
            start_level: "★7".into(),
            final_level: "★9".into(),
            max_level: "★9".into(),
            played: 3,
            cleared: 2,
            lamp_counts: vec![
                ("HARD".into(), 1),
                ("NORMAL".into(), 1),
                ("FAILED".into(), 1),
            ],
            songs: vec![],
        }
    }

    #[test]
    fn jst_epoch() {
        assert_eq!(jst_parts(0.0), (1970, 1, 1, 9, 0, 0));
    }

    #[test]
    fn post_text_matches_tracker_style() {
        let t = post_text(&sample());
        assert_eq!(
            t,
            "1970/01/01\nスパルタ 3曲プレイ（HARD 1 / NORMAL 1 / FAILED 1）\nレベル ★7 → ★9\n#BMS"
        );
    }

    #[test]
    fn percent_encode_keeps_unreserved() {
        assert_eq!(percent_encode("a B#★"), "a%20B%23%E2%98%85");
    }

    #[test]
    fn buckets() {
        assert_eq!(lamp_bucket(10), "EXH");
        assert_eq!(lamp_bucket(7), "EXH");
        assert_eq!(lamp_bucket(6), "HARD");
        assert_eq!(lamp_bucket(1), "FAILED");
        assert_eq!(lamp_bucket(0), "FAILED");
    }

    #[test]
    fn logical_day_boundary() {
        // 2026-11-23 02:00 JST = 1795366800
        assert_eq!(logical_day(1795366800.0, 0).0, "20261123");
        assert_eq!(logical_day(1795366800.0, 4).0, "20261122");
        assert_eq!(logical_day(1795366800.0, 2).0, "20261123");
        assert_eq!(logical_day(1795366800.0, 4).1, "2026/11/22");
    }

    #[test]
    fn merge_two_sessions() {
        let mut a = sample();
        a.start_level = "★7".into();
        a.final_level = "★8".into();
        a.max_level = "★8".into();
        a.max_level_idx = 8;
        let mut b = sample();
        b.start_level = "★8".into();
        b.final_level = "★7".into();
        b.max_level = "★10".into();
        b.max_level_idx = 10;
        b.songs = vec![SongRow {
            level: "★8".into(),
            title: "T".into(),
            lamp: "HARD CLEAR".into(),
            score_fmt: None,
            bp: None,
            gained: 2,
            event: None,
        }];
        let m = merge_reports(&a, &b);
        assert_eq!(m.played, 6);
        assert_eq!(m.cleared, 4);
        assert_eq!(m.start_level, "★7"); // その日の最初
        assert_eq!(m.final_level, "★7"); // 最新
        assert_eq!(m.max_level, "★10"); // 自然順で大きい方
        assert_eq!(m.songs.len(), 1);
        assert_eq!(
            m.lamp_counts,
            vec![
                ("HARD".to_string(), 2),
                ("NORMAL".to_string(), 2),
                ("FAILED".to_string(), 2)
            ]
        );
    }

    #[test]
    fn state_json_roundtrip() {
        let r = sample();
        let back = report_from_json(&report_to_json(&r)).unwrap();
        assert_eq!(back.day_key, r.day_key);
        assert_eq!(back.played, r.played);
        assert_eq!(back.lamp_counts, r.lamp_counts);
    }
}
