//! 設定 (sparta.json) と配置場所の解決。
//! 設定・状態ファイルは「exe と同じフォルダ」に置くポータブル形式。

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// exe と同じフォルダ
pub fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    base_dir().join("sparta.json")
}

pub fn status_path() -> PathBuf {
    base_dir().join("sparta-status.json")
}

pub fn hud_pos_path() -> PathBuf {
    base_dir().join("hud-pos.json")
}

pub fn load_config() -> Value {
    let p = config_path();
    if p.exists() {
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                return v;
            }
        }
    }
    Value::Object(Map::new())
}

pub fn save_config(cfg: &Value) {
    let text = serde_json::to_string_pretty(cfg).unwrap_or_default();
    let _ = std::fs::write(config_path(), text);
}

pub fn cfg_str(cfg: &Value, key: &str) -> Option<String> {
    cfg.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

pub fn cfg_i64(cfg: &Value, key: &str) -> Option<i64> {
    cfg.get(key).and_then(|v| match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

pub fn cfg_bool(cfg: &Value, key: &str, default: bool) -> bool {
    cfg.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub fn cfg_set(cfg: &mut Value, key: &str, val: Value) {
    if !cfg.is_object() {
        *cfg = Value::Object(Map::new());
    }
    cfg.as_object_mut()
        .unwrap()
        .insert(key.to_string(), val);
}

/// bmz-player.exe のあるフォルダを探す
/// 自動検出の候補フォルダ (優先順)。手動指定 (cfg の bmz_dir) が先頭
pub fn bmz_candidates(cfg: &Value) -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(d) = cfg_str(cfg, "bmz_dir") {
        cands.push(PathBuf::from(d));
    }
    let home = home_dir();
    cands.push(base_dir());
    if let Some(h) = &home {
        cands.push(h.join("bmz-player"));
    }
    for d in [
        "C:/bmz-player",
        "D:/bmz-player",
        "C:/BMZ Player",
        "D:/BMZ Player",
        "C:/Program Files/bmz-player",
    ] {
        cands.push(PathBuf::from(d));
    }
    if let Some(h) = &home {
        cands.push(h.join("AppData/Local/Programs/bmz-player"));
    }
    cands
}

pub fn find_bmz_dir(cfg: &Value) -> Option<PathBuf> {
    bmz_candidates(cfg)
        .into_iter()
        .find(|d| d.join("bmz-player.exe").exists())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// score.db の場所を決める。プロファイル別があればそちらを優先
pub fn find_score_db(bmz_dir: &Path, cfg: &Value) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    let profiles_dir = bmz_dir.join("data").join("profiles");
    if let Some(prof) = cfg_str(cfg, "profile") {
        cands.push(profiles_dir.join(prof).join("score.db"));
    }
    if profiles_dir.exists() {
        let mut subs: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&profiles_dir) {
            for e in rd.flatten() {
                let p = e.path().join("score.db");
                if p.exists() {
                    subs.push(p);
                }
            }
        }
        subs.sort();
        cands.extend(subs);
    }
    cands.push(bmz_dir.join("data").join("score.db"));
    cands.into_iter().find(|p| p.exists())
}
