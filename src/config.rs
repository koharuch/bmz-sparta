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

/// 設定を保存する。一時ファイルへ書いてから置換するので、
/// 途中終了でファイルが壊れることはない。成功したら true
pub fn save_config(cfg: &Value) -> bool {
    let text = serde_json::to_string_pretty(cfg).unwrap_or_default();
    let path = config_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_err() {
        return false;
    }
    std::fs::rename(&tmp, &path).is_ok()
}

/// セッション実行中の実効設定を HUD へ渡すためのファイル
pub fn session_cfg_path() -> PathBuf {
    base_dir().join("sparta-session.json")
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
/// 自動検出の対象。bmz-player 公式インストーラ (Inno Setup) の
/// 標準インストール先 `{localappdata}\Programs\BMZ Player` のみ。
/// それ以外に置いた人は手動指定 (cfg の bmz_dir) を使う前提
pub fn bmz_candidates(cfg: &Value) -> Vec<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(d) = cfg_str(cfg, "bmz_dir") {
        cands.push(PathBuf::from(d));
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join("AppData").join("Local")));
    if let Some(lad) = local_app_data {
        cands.push(lad.join("Programs").join("BMZ Player"));
    }
    cands
}

pub fn find_bmz_dir(cfg: &Value) -> Option<PathBuf> {
    bmz_candidates(cfg)
        .into_iter()
        .find(|d| d.join("bmz-player.exe").exists())
}

/// bmz_dir の解決ルール:
/// 手動指定 (cfg の bmz_dir) がある場合はそれだけを信じ、見つからなくても
/// 標準インストール先へ勝手に切り替えない (再スキャンで指定を潰さない)。
/// 手動指定が無い場合のみ自動検出する
pub fn resolve_bmz_dir(cfg: &Value) -> Option<PathBuf> {
    if let Some(d) = cfg_str(cfg, "bmz_dir") {
        let p = PathBuf::from(d);
        return if p.join("bmz-player.exe").exists() {
            Some(p)
        } else {
            None
        };
    }
    find_bmz_dir(cfg)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// score.db の場所を決める。
/// profile が指定されていればそのプロファイルだけを見る (別人のDBへ
/// フォールバックしない)。未指定でプロファイルが複数ある場合は
/// 曖昧なのでエラーにし、指定を促す
pub fn find_score_db(bmz_dir: &Path, cfg: &Value) -> Result<PathBuf, String> {
    let profiles_dir = bmz_dir.join("data").join("profiles");
    if let Some(prof) = cfg_str(cfg, "profile") {
        let p = profiles_dir.join(&prof).join("score.db");
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!(
                "指定されたプロファイル {prof} の score.db が見つかりません: {}",
                p.display()
            ))
        };
    }
    let mut subs: Vec<PathBuf> = Vec::new();
    if profiles_dir.exists() {
        if let Ok(rd) = std::fs::read_dir(&profiles_dir) {
            for e in rd.flatten() {
                let p = e.path().join("score.db");
                if p.exists() {
                    subs.push(p);
                }
            }
        }
        subs.sort();
    }
    match subs.len() {
        1 => Ok(subs.into_iter().next().unwrap()),
        0 => {
            let p = bmz_dir.join("data").join("score.db");
            if p.exists() {
                Ok(p)
            } else {
                Err("score.db が見つかりませんでした。".to_string())
            }
        }
        _ => {
            let names: Vec<String> = subs
                .iter()
                .filter_map(|p| {
                    p.parent()
                        .and_then(|d| d.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .collect();
            Err(format!(
                "プロファイルが複数あります ({} )。sparta.json の profile で使うプロファイルを指定してください。",
                names.join(", ")
            ))
        }
    }
}
