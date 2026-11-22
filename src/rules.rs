//! レベル昇降ルールと、ランプ・グレードの換算 (純粋ロジック)

use serde_json::Value;

/// ClearType: 4=Easy, 5=Normal, 6=Hard, 7=ExHard, 8=FC, 9=Perfect, 10=Max
pub const CLEAR_MIN: i64 = 4;

/// レベル増減ルール。sparta.json の同名キーで上書きできる
#[derive(Debug, Clone, Copy)]
pub struct Rules {
    /// この点数でレベルUP
    pub up_points: i64,
    /// EX HARD CLEAR 以上の得点
    pub pts_exhard: i64,
    /// HARD CLEAR の得点
    pub pts_hard: i64,
    /// CLEAR / EASY CLEAR の得点
    pub pts_clear: i64,
    /// この連敗数でレベルDOWN
    pub fail_streak: i64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            up_points: 3,
            pts_exhard: 3,
            pts_hard: 2,
            pts_clear: 1,
            fail_streak: 3,
        }
    }
}

impl Rules {
    pub fn from_config(cfg: &Value) -> Rules {
        let mut r = Rules::default();
        let get = |k: &str| cfg.get(k).and_then(Value::as_i64);
        if let Some(v) = get("up_points") {
            r.up_points = v;
        }
        if let Some(v) = get("pts_exhard") {
            r.pts_exhard = v;
        }
        if let Some(v) = get("pts_hard") {
            r.pts_hard = v;
        }
        if let Some(v) = get("pts_clear") {
            r.pts_clear = v;
        }
        if let Some(v) = get("fail_streak") {
            r.fail_streak = v;
        }
        r
    }
}

/// クリアの強さ -> レベルUPまでの点数換算
pub fn clear_points(rules: &Rules, clear_type: i64) -> i64 {
    if clear_type >= 7 {
        rules.pts_exhard
    } else if clear_type == 6 {
        rules.pts_hard
    } else if clear_type >= CLEAR_MIN {
        rules.pts_clear
    } else {
        0
    }
}

/// clear_type は DB 内で文字列 ('Normal' 等) のことがあるので数値化する
pub fn clear_value(v: &Value) -> i64 {
    match v {
        Value::Null => 0,
        Value::Number(n) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                return s.parse().unwrap_or(0);
            }
            match s.replace('_', "").replace('-', "").to_lowercase().as_str() {
                "noplay" => 0,
                "failed" => 1,
                "assisteasy" => 2,
                "lightassisteasy" => 3,
                "easy" => 4,
                "normal" => 5,
                "hard" => 6,
                "exhard" => 7,
                "fullcombo" => 8,
                "perfect" => 9,
                "max" => 10,
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// bmz / beatoraja 公式表記 (IR 変換テーブル準拠)
pub fn lamp_name(clear_type: i64) -> Option<&'static str> {
    Some(match clear_type {
        0 => "NO PLAY",
        1 => "FAILED",
        2 | 3 => "ASSIST CLEAR",
        4 => "EASY CLEAR",
        5 => "CLEAR",
        6 => "HARD CLEAR",
        7 => "EX HARD CLEAR",
        8 => "FULL COMBO",
        9 => "PERFECT",
        10 => "MAX",
        _ => return None,
    })
}

/// IIDX式グレード表記: 'AAA +132 [ 2354 ]' (+はそのグレードの下限超過分)
pub fn grade_fmt(ex_score: Option<i64>, total_notes: Option<i64>) -> Option<String> {
    let ex = ex_score?;
    let notes = match total_notes {
        Some(n) if n > 0 => n,
        _ => return Some(format!("[ {ex} ]")),
    };
    let max_ex = 2 * notes;
    if ex >= max_ex {
        return Some(format!("MAX -{} [ {} ]", max_ex - ex, ex));
    }
    for (name, k) in [
        ("AAA", 8),
        ("AA", 7),
        ("A", 6),
        ("B", 5),
        ("C", 4),
        ("D", 3),
        ("E", 2),
    ] {
        let border = (max_ex * k + 8) / 9; // ceil(max_ex * k / 9)
        if ex >= border {
            return Some(format!("{} +{} [ {} ]", name, ex - border, ex));
        }
    }
    Some(format!("F [ {ex} ]"))
}

/// "★10" のような文字列を数値部で比較するためのキー
pub fn natural_key(s: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut num = String::new();
    let mut text = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                parts.push(NaturalPart::Text(std::mem::take(&mut text)));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                parts.push(NaturalPart::Num(num.parse().unwrap_or(0)));
                num.clear();
            }
            text.push(c);
        }
    }
    if !num.is_empty() {
        parts.push(NaturalPart::Num(num.parse().unwrap_or(0)));
    }
    if !text.is_empty() {
        parts.push(NaturalPart::Text(text));
    }
    parts
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NaturalPart {
    Num(i64),
    Text(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn points_by_lamp_strength() {
        let r = Rules::default();
        assert_eq!(clear_points(&r, 10), 3);
        assert_eq!(clear_points(&r, 7), 3);
        assert_eq!(clear_points(&r, 6), 2);
        assert_eq!(clear_points(&r, 5), 1);
        assert_eq!(clear_points(&r, 4), 1);
        assert_eq!(clear_points(&r, 3), 0);
        assert_eq!(clear_points(&r, 1), 0);
    }

    #[test]
    fn rules_overridable_from_config() {
        let r = Rules::from_config(&json!({"up_points": 5, "fail_streak": 2}));
        assert_eq!(r.up_points, 5);
        assert_eq!(r.fail_streak, 2);
        assert_eq!(r.pts_hard, 2);
    }

    #[test]
    fn clear_value_parses_numbers_and_names() {
        assert_eq!(clear_value(&json!(6)), 6);
        assert_eq!(clear_value(&json!("Hard")), 6);
        assert_eq!(clear_value(&json!("EX-HARD")), 7);
        assert_eq!(clear_value(&json!("Full_Combo")), 8);
        assert_eq!(clear_value(&json!("9")), 9);
        assert_eq!(clear_value(&Value::Null), 0);
        assert_eq!(clear_value(&json!("unknown")), 0);
    }

    #[test]
    fn grade_format_borders() {
        // 1000ノーツ -> MAX 2000, AAA境界 = ceil(2000*8/9) = 1778
        assert_eq!(grade_fmt(Some(2000), Some(1000)).unwrap(), "MAX -0 [ 2000 ]");
        assert_eq!(grade_fmt(Some(1778), Some(1000)).unwrap(), "AAA +0 [ 1778 ]");
        assert_eq!(grade_fmt(Some(1777), Some(1000)).unwrap(), "AA +221 [ 1777 ]");
        assert_eq!(grade_fmt(Some(100), Some(1000)).unwrap(), "F [ 100 ]");
        assert_eq!(grade_fmt(Some(500), None).unwrap(), "[ 500 ]");
        assert!(grade_fmt(None, Some(1000)).is_none());
    }

    #[test]
    fn natural_sort_orders_numbers() {
        let mut v = vec!["10", "2", "1", "☆12", "☆3"];
        v.sort_by_key(|s| natural_key(s));
        assert_eq!(v, vec!["1", "2", "10", "☆3", "☆12"]);
    }
}
