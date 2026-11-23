//! `run` サブコマンドの引数解析 (AIエージェント向けの完全CLI起動)。
//! GUI・コンソール対話を一切使わず、引数だけでセッションを開始する。
//! 未指定の項目は sparta.json の値が既定になる。ここで作る上書きは
//! メモリ上だけに適用され、sparta.json へは書き戻されない。

use serde_json::{json, Map, Value};

pub const PICKS: [&str; 3] = ["priority", "noplay_failed", "random"];
pub const GAUGES: [&str; 6] = ["assist-easy", "easy", "normal", "hard", "ex-hard", "hazard"];
pub const GASES: [&str; 5] = [
    "off",
    "continue",
    "hard-to-groove",
    "best-clear",
    "select-to-under",
];

pub struct RunArgs {
    /// sparta.json の設定へ重ねる上書き (JSONオブジェクト)
    pub overrides: Value,
    pub dry: bool,
}

fn err_enum(name: &str, val: &str, valid: &[&str]) -> String {
    format!(
        "{name} の値が不正です: {val} (有効な値: {})",
        valid.join(" | ")
    )
}

pub fn parse(args: &[String]) -> Result<RunArgs, String> {
    let mut obj = Map::new();
    let mut dry = false;
    let mut i = 0;
    let mut value_of = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{name} には値が必要です"))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--dry-run" => dry = true,
            "--hud" => {
                obj.insert("hud".into(), json!(true));
            }
            "--no-hud" => {
                obj.insert("hud".into(), json!(false));
            }
            "--report" => {
                obj.insert("report".into(), json!(true));
            }
            "--no-report" => {
                obj.insert("report".into(), json!(false));
            }
            "--bmz-dir" => {
                let v = value_of(&mut i, a)?;
                obj.insert("bmz_dir".into(), json!(v));
            }
            "--level" => {
                let v = value_of(&mut i, a)?;
                obj.insert("start_level".into(), json!(v));
            }
            "--table" => {
                let v = value_of(&mut i, a)?;
                let n: i64 = v
                    .parse()
                    .map_err(|_| format!("--table は数値で指定してください: {v}"))?;
                obj.insert("table_id".into(), json!(n));
            }
            "--minutes" => {
                let v = value_of(&mut i, a)?;
                let n: i64 = v
                    .parse()
                    .map_err(|_| format!("--minutes は数値で指定してください: {v}"))?;
                if !(1..=10080).contains(&n) {
                    return Err("--minutes は1〜10080で指定してください".into());
                }
                obj.insert("minutes".into(), json!(n));
            }
            "--interval" => {
                let v = value_of(&mut i, a)?;
                let n: i64 = v
                    .parse()
                    .map_err(|_| format!("--interval は数値で指定してください: {v}"))?;
                if n < 0 {
                    return Err("--interval は0以上で指定してください".into());
                }
                obj.insert("result_seconds".into(), json!(n));
            }
            "--pick" => {
                let v = value_of(&mut i, a)?;
                if !PICKS.contains(&v.as_str()) {
                    return Err(err_enum("--pick", &v, &PICKS));
                }
                obj.insert("pick_mode".into(), json!(v));
            }
            "--gauge" => {
                let v = value_of(&mut i, a)?;
                if !GAUGES.contains(&v.as_str()) {
                    return Err(err_enum("--gauge", &v, &GAUGES));
                }
                obj.insert("gauge".into(), json!(v));
            }
            "--gas" => {
                let v = value_of(&mut i, a)?;
                if !GASES.contains(&v.as_str()) {
                    return Err(err_enum("--gas", &v, &GASES));
                }
                obj.insert("gas".into(), json!(v));
            }
            other => {
                return Err(format!("不明なオプションです: {other}"));
            }
        }
        i += 1;
    }
    Ok(RunArgs {
        overrides: Value::Object(obj),
        dry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<RunArgs, String> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_full_options() {
        let r = parse_strs(&[
            "--bmz-dir",
            "D:/BMZ Player",
            "--table",
            "1",
            "--level",
            "7",
            "--minutes",
            "15",
            "--interval",
            "10",
            "--pick",
            "priority",
            "--gas",
            "best-clear",
            "--gauge",
            "ex-hard",
            "--no-hud",
        ])
        .unwrap();
        assert_eq!(r.overrides["bmz_dir"], json!("D:/BMZ Player"));
        assert_eq!(r.overrides["table_id"], json!(1));
        assert_eq!(r.overrides["start_level"], json!("7"));
        assert_eq!(r.overrides["minutes"], json!(15));
        assert_eq!(r.overrides["result_seconds"], json!(10));
        assert_eq!(r.overrides["pick_mode"], json!("priority"));
        assert_eq!(r.overrides["gas"], json!("best-clear"));
        assert_eq!(r.overrides["gauge"], json!("ex-hard"));
        assert_eq!(r.overrides["hud"], json!(false));
        assert!(!r.dry);
    }

    #[test]
    fn rejects_bad_enum_and_unknown() {
        assert!(parse_strs(&["--pick", "bogus"]).is_err());
        assert!(parse_strs(&["--gauge", "bogus"]).is_err());
        assert!(parse_strs(&["--gas", "bogus"]).is_err());
        assert!(parse_strs(&["--unknown"]).is_err());
        assert!(parse_strs(&["--minutes"]).is_err());
        assert!(parse_strs(&["--minutes", "0"]).is_err());
    }

    #[test]
    fn empty_args_ok() {
        let r = parse_strs(&[]).unwrap();
        assert!(r.overrides.as_object().unwrap().is_empty());
    }
}
