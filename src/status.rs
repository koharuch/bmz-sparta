//! HUD が読む状態ファイル (sparta-status.json)。
//! 失敗してもセッションは止めない。

use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct StatusWriter {
    path: std::path::PathBuf,
}

impl StatusWriter {
    pub fn new() -> StatusWriter {
        StatusWriter {
            path: crate::config::status_path(),
        }
    }

    pub fn write(&self, merge: bool, fields: BTreeMap<&str, Value>) {
        let mut obj: Map<String, Value> = if merge {
            std::fs::read_to_string(&self.path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| match v {
                    Value::Object(m) => Some(m),
                    _ => None,
                })
                .unwrap_or_default()
        } else {
            Map::new()
        };
        for (k, v) in fields {
            obj.insert(k.to_string(), v);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, Value::Object(obj).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

/// 書き込み用フィールドの組み立てヘルパー
#[macro_export]
macro_rules! status_fields {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m = ::std::collections::BTreeMap::new();
        $( m.insert($k, $v); )*
        m
    }};
}
