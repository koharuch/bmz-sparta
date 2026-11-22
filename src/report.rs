//! セッション結果レポート (HTML) と X 投稿リンク。
//!
//! 文面は稼働中の BMS トラッカーの「成果報告」と同じ形式に揃えたローカル版:
//! 2026/11/22
//! スパルタ 12曲プレイ（EXH 2 / HARD 5 / NORMAL 2 / EASY 1 / FAILED 2）
//! レベル ★7 → ★9
//! #BMS
//!
//! セッション終了時に exe と同じフォルダへ result-YYYYMMDD-HHMMSS.html を
//! 書き出し、既定ブラウザで開く (sparta.json の "report": false で無効化)。

use std::path::PathBuf;

pub struct SongRow {
pub level: String,
pub title: String,
pub lamp: String,
pub score_fmt: Option<String>,
pub bp: Option<i64>,
pub gained: i64,
pub event: Option<String>,
}

pub struct SessionReport {
pub ended_unix: f64,
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
let era = if z >= 0 { z} else { z - 146_096} / 146_097;
let doe = z - era * 146_097;
let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
let y = yoe + era * 400;
let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
let mp = (5 * doy + 2) / 153;
let d = doy - (153 * mp + 2) / 5 + 1;
let m = if mp < 10 { mp + 3} else { mp - 9};
(if m <= 2 { y + 1} else { y}, m as u32, d as u32)
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
let (y, m, d, _, _, _) = jst_parts(r.ended_unix);
let mut lines = vec![
format!("{y}/{m:02}/{d:02}"),
format!(
"スパルタ {}曲プレイ（{}）",
r.played,
lamp_breakdown(r)
),
];
if r.start_level == r.final_level {
lines.push(format!("レベル {}", r.final_level));
} else {
lines.push(format!(
"レベル {} → {}",
r.start_level, r.final_level
));
}
if r.max_level!= r.final_level && r.max_level!= r.start_level {
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
let (y, m, d, hh, mm, _) = jst_parts(r.ended_unix);
let date = format!("{y}/{m:02}/{d:02}");
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
let max_line = if r.max_level!= r.final_level && r.max_level!= r.start_level {
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
body {{ font-family: "Meiryo", "Yu Gothic", sans-serif; background: #17181c; color: #eee; margin: 24px;}}
.card {{ max-width: 880px; margin: 0 auto; background: #22242b; border-radius: 12px; padding: 24px 28px;}}
h1 {{ font-size: 22px; margin: 0 0 4px;}}
.meta {{ color: #9aa0aa; font-size: 13px; margin-bottom: 16px;}}
.big {{ font-size: 17px; line-height: 1.7;}}
.sub {{ color: #9aa0aa; font-size: 14px;}}
.bar {{ display: flex; height: 18px; border-radius: 6px; overflow: hidden; margin: 14px 0 4px; background: #333;}}
table {{ width: 100%; border-collapse: collapse; margin-top: 18px; font-size: 14px;}}
th, td {{ padding: 6px 8px; border-bottom: 1px solid #34363f; text-align: left; white-space: nowrap;}}
td.title {{ white-space: normal;}}
th {{ color: #9aa0aa; font-weight: normal;}}
.fail {{ color: #ff5064;}}
.xpost {{ display: inline-block; margin-top: 20px; background: #000; color: #fff; border: 1px solid #555;
padding: 10px 22px; border-radius: 999px; text-decoration: none; font-weight: bold;}}
.foot {{ color: #666; font-size: 12px; margin-top: 16px;}}
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
<div class="foot">bmz-sparta ローカル結果レポート</div>
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

/// result-YYYYMMDD-HHMMSS.html を exe と同じフォルダへ書き出す
pub fn generate(r: &SessionReport) -> Option<PathBuf> {
let (y, m, d, hh, mm, ss) = jst_parts(r.ended_unix);
let name = format!("result-{y}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}.html");
let path = crate::config::base_dir().join(name);
std::fs::write(&path, render_html(r)).ok()?;
Some(path)
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
table: "発狂BMS難度表".into(),
start_level: "★7".into(),
final_level: "★9".into(),
max_level: "★9".into(),
played: 3,
cleared: 2,
lamp_counts: vec![("HARD".into(), 1), ("NORMAL".into(), 1), ("FAILED".into(), 1)],
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
}
