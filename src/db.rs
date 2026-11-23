//! bmz-player の DB 読み取り (library.db / score.db、読み取り専用)

use crate::rules::{clear_value, lamp_name};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn open_ro(db_path: &Path) -> rusqlite::Result<Connection> {
    let con = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let _ = con.busy_timeout(std::time::Duration::from_secs(10));
    Ok(con)
}

fn columns(con: &Connection, table: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = con.prepare(&format!("PRAGMA table_info({table})")) {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            for name in rows.flatten() {
                out.push(name);
            }
        }
    }
    out
}

fn tables(con: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(mut stmt) =
        con.prepare("SELECT name FROM sqlite_master WHERE type='table'")
    {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for name in rows.flatten() {
                out.push(name);
            }
        }
    }
    out
}

fn to_json(v: &SqlValue) -> JsonValue {
    match v {
        SqlValue::Null => JsonValue::Null,
        SqlValue::Integer(i) => JsonValue::from(*i),
        SqlValue::Real(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        SqlValue::Text(s) => JsonValue::String(s.clone()),
        SqlValue::Blob(_) => JsonValue::Null,
    }
}

#[derive(Debug, Clone)]
pub struct Chart {
    pub chart_id: Option<i64>,
    pub sha256: Option<String>,
    pub md5: Option<String>,
    pub title: String,
    pub path: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum ChartField {
    Sha256,
    Md5,
    ChartId,
}

impl ChartField {
    fn keyval(&self, chart: &Chart) -> Option<SqlValue> {
        match self {
            ChartField::Sha256 => chart.sha256.clone().map(SqlValue::Text),
            ChartField::Md5 => chart.md5.clone().map(SqlValue::Text),
            ChartField::ChartId => chart.chart_id.map(SqlValue::Integer),
        }
    }
}

// ---------- library.db ----------

pub struct Library {
    con: Connection,
    chart_cols: Vec<String>,
    entry_cols: Vec<String>,
    has_files: bool,
    has_links: bool,
    level_order: HashMap<i64, String>,
    symbols: HashMap<i64, String>,
}

impl Library {
    pub fn open(db_path: &Path) -> rusqlite::Result<Library> {
        Self::with_connection(open_ro(db_path)?)
    }

    pub fn with_connection(con: Connection) -> rusqlite::Result<Library> {
        let chart_cols = columns(&con, "charts");
        let entry_cols = columns(&con, "difficulty_table_entries");
        let t = tables(&con);
        Ok(Library {
            con,
            chart_cols,
            entry_cols,
            has_files: t.iter().any(|n| n == "chart_files"),
            has_links: t.iter().any(|n| n == "chart_file_links"),
            level_order: HashMap::new(),
            symbols: HashMap::new(),
        })
    }

    /// 登録済みの難易度表 [(id, 表示名)]。level_order / symbol も保持する
    pub fn list_tables(&mut self) -> Vec<(i64, String)> {
        let cols = columns(&self.con, "difficulty_tables");
        let name_col = ["table_name", "name", "title"]
            .iter()
            .find(|c| cols.iter().any(|x| x == *c))
            .copied();
        let has_url = cols.iter().any(|c| c == "source_url");
        let id_col = if cols.iter().any(|c| c == "id") {
            "id"
        } else {
            "rowid"
        };
        let sql = format!("SELECT {id_col} AS _tid, * FROM difficulty_tables");
        let mut stmt = match self.con.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let col_names: Vec<String> =
            stmt.column_names().iter().map(|s| s.to_string()).collect();
        let mut rows = Vec::new();
        if let Ok(mut q) = stmt.query([]) {
            while let Ok(Some(row)) = q.next() {
                let get = |name: &str| -> Option<SqlValue> {
                    col_names
                        .iter()
                        .position(|c| c == name)
                        .and_then(|i| row.get::<_, SqlValue>(i).ok())
                };
                let tid = match get("_tid") {
                    Some(SqlValue::Integer(i)) => i,
                    _ => continue,
                };
                let mut label = name_col.and_then(|c| match get(c) {
                    Some(SqlValue::Text(s)) if !s.is_empty() => Some(s),
                    _ => None,
                });
                if label.is_none() && has_url {
                    if let Some(SqlValue::Text(s)) = get("source_url") {
                        if !s.is_empty() {
                            label = Some(s);
                        }
                    }
                }
                rows.push((tid, label.unwrap_or_else(|| format!("table#{tid}"))));
                if let Some(SqlValue::Text(s)) = get("level_order") {
                    self.level_order.insert(tid, s);
                }
                if let Some(SqlValue::Text(s)) = get("symbol") {
                    self.symbols.insert(tid, s);
                }
            }
        }
        rows
    }

    pub fn symbol(&self, table_id: i64) -> String {
        self.symbols.get(&table_id).cloned().unwrap_or_default()
    }

    pub fn total_notes(&self, chart: &Chart) -> Option<i64> {
        for (key, val) in [
            ("sha256", chart.sha256.as_ref()),
            ("md5", chart.md5.as_ref()),
        ] {
            if let Some(v) = val {
                let sql = format!("SELECT total_notes FROM charts WHERE {key} = ?");
                if let Ok(mut stmt) = self.con.prepare(&sql) {
                    if let Ok(n) = stmt.query_row([v], |r| r.get::<_, Option<i64>>(0)) {
                        if n.is_some() {
                            return n;
                        }
                    }
                }
            }
        }
        None
    }

    pub fn count_entries(&self, table_id: i64, level: &str) -> i64 {
        self.con
            .query_row(
                "SELECT COUNT(*) FROM difficulty_table_entries \
                 WHERE table_id = ? AND level = ?",
                rusqlite::params![table_id, level],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }

    pub fn levels(&self, table_id: i64) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        if let Ok(mut stmt) = self.con.prepare(
            "SELECT DISTINCT level FROM difficulty_table_entries WHERE table_id = ?",
        ) {
            if let Ok(rows) =
                stmt.query_map([table_id], |r| r.get::<_, SqlValue>(0))
            {
                for v in rows.flatten() {
                    found.push(match v {
                        SqlValue::Text(s) => s,
                        SqlValue::Integer(i) => i.to_string(),
                        SqlValue::Real(f) => f.to_string(),
                        _ => String::new(),
                    });
                }
            }
        }
        let mut order: Vec<String> = Vec::new();
        if let Some(raw) = self.level_order.get(&table_id) {
            if let Ok(JsonValue::Array(a)) =
                serde_json::from_str::<JsonValue>(raw)
            {
                order = a
                    .iter()
                    .map(|x| match x {
                        JsonValue::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
            } else {
                order = raw
                    .split([',', '\n'])
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
        }
        let ranked: Vec<String> = order
            .iter()
            .filter(|lv| found.contains(lv))
            .cloned()
            .collect();
        let mut rest: Vec<String> = found
            .iter()
            .filter(|lv| !ranked.contains(lv))
            .cloned()
            .collect();
        rest.sort_by_key(|lv| crate::rules::natural_key(lv));
        ranked.into_iter().chain(rest).collect()
    }

    /// そのレベルの譜面一覧を返す
    pub fn charts_in_level(&self, table_id: i64, level: &str) -> Vec<Chart> {
        let e = &self.entry_cols;
        let on = if e.iter().any(|c| c == "chart_id")
            && self.chart_cols.iter().any(|c| c == "id")
        {
            "c.id = e.chart_id".to_string()
        } else {
            let keys: Vec<&str> = ["sha256", "md5"]
                .iter()
                .filter(|k| {
                    self.chart_cols.iter().any(|c| c == *k)
                        && e.iter().any(|c| c == *k)
                })
                .copied()
                .collect();
            if keys.is_empty() {
                return Vec::new();
            }
            let conds: Vec<String> =
                keys.iter().map(|k| format!("c.{k} = e.{k}")).collect();
            format!("({})", conds.join(" OR "))
        };
        // e.* と c.* は列名が衝突するため、必ず別名で SELECT する
        let sql = format!(
            "SELECT c.id AS cid, c.sha256 AS csha, c.md5 AS cmd5, \
             c.title AS ctitle, e.title AS etitle \
             FROM difficulty_table_entries e JOIN charts c ON {on} \
             WHERE e.table_id = ? AND e.level = ?"
        );
        let mut stmt = match self.con.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(rusqlite::params![table_id, level], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        });
        let mut out = Vec::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        if let Ok(rows) = rows {
            for (row_idx, row) in rows.flatten().enumerate() {
                let (cid, csha, cmd5, ctitle, etitle) = row;
                // 重複判定キー: SHA-256 → MD5 → chart_id → 行番号 の順。
                // 識別子が無い譜面同士を同一視して落とさないため
                let key = csha
                    .clone()
                    .or_else(|| cmd5.clone())
                    .or_else(|| cid.map(|i| format!("id:{i}")))
                    .unwrap_or_else(|| format!("row:{row_idx}"));
                if seen_keys.contains(&key) {
                    continue;
                }
                seen_keys.insert(key);
                let path = self.file_path(cid, csha.as_deref(), cmd5.as_deref());
                out.push(Chart {
                    chart_id: cid,
                    sha256: csha,
                    md5: cmd5,
                    title: ctitle
                        .filter(|s| !s.is_empty())
                        .or(etitle)
                        .unwrap_or_else(|| "(タイトル不明)".to_string()),
                    path,
                });
            }
        }
        out
    }

    /// 譜面ファイルの絶対パス。アーカイブ内 (!/) より展開済みを優先する
    fn file_path(
        &self,
        chart_id: Option<i64>,
        sha256: Option<&str>,
        md5: Option<&str>,
    ) -> Option<String> {
        if !self.has_files {
            return None;
        }
        let mut candidates: Vec<String> = Vec::new();
        if let (Some(cid), true) = (chart_id, self.has_links) {
            if let Ok(mut stmt) = self.con.prepare(
                "SELECT cf.path FROM chart_file_links l \
                 JOIN chart_files cf ON cf.id = l.chart_file_id \
                 WHERE l.chart_id = ?",
            ) {
                if let Ok(rows) = stmt.query_map([cid], |r| r.get::<_, String>(0)) {
                    candidates.extend(rows.flatten());
                }
            }
        }
        let file_cols = columns(&self.con, "chart_files");
        for (key, val) in [("sha256", sha256), ("md5", md5)] {
            if let Some(v) = val {
                if file_cols.iter().any(|c| c == key) {
                    let sql = format!("SELECT path FROM chart_files WHERE {key} = ?");
                    if let Ok(mut stmt) = self.con.prepare(&sql) {
                        if let Ok(rows) =
                            stmt.query_map([v], |r| r.get::<_, String>(0))
                        {
                            candidates.extend(rows.flatten());
                        }
                    }
                }
            }
        }
        let mut uniq: Vec<String> = Vec::new();
        for p in candidates {
            if !p.is_empty() && !uniq.contains(&p) {
                uniq.push(p);
            }
        }
        if uniq.is_empty() {
            return None;
        }
        let plain: Vec<&String> = uniq
            .iter()
            .filter(|p| !p.contains("!/") && !p.contains("!\\"))
            .collect();
        // 実在する展開済み候補を優先する (移動・削除済みの残骸パスを掴まない)
        if let Some(p) = plain.iter().find(|p| std::path::Path::new(p.as_str()).exists()) {
            return Some(p.to_string());
        }
        Some(plain.first().map(|s| s.to_string()).unwrap_or_else(|| uniq[0].clone()))
    }
}

// ---------- score.db ----------

pub struct BestDetail {
    pub lamp: String,
    pub ex_score: Option<i64>,
    pub bp: Option<i64>,
}

pub struct ResultDetail {
    pub clear_type: i64,
    pub ex_score: Option<i64>,
    pub bp: Option<i64>,
}

pub struct Scores {
    con: Connection,
    key: Option<(String, ChartField)>,
    id_col: String,
    has_best: bool,
}

impl Scores {
    pub fn open(db_path: &Path) -> rusqlite::Result<Scores> {
        Self::with_connection(open_ro(db_path)?)
    }

    pub fn with_connection(con: Connection) -> rusqlite::Result<Scores> {
        let best_cols = columns(&con, "score_best");
        let hist_cols = columns(&con, "score_history");
        let base = if !hist_cols.is_empty() {
            &hist_cols
        } else {
            &best_cols
        };
        let mut key = None;
        for (col, field) in [
            ("chart_sha256", ChartField::Sha256),
            ("sha256", ChartField::Sha256),
            ("md5", ChartField::Md5),
            ("chart_id", ChartField::ChartId),
        ] {
            if base.iter().any(|c| c == col) {
                key = Some((col.to_string(), field));
                break;
            }
        }
        let id_col = if hist_cols.iter().any(|c| c == "id") {
            "id"
        } else {
            "rowid"
        }
        .to_string();
        let has_best = tables(&con).iter().any(|t| t == "score_best");
        Ok(Scores {
            con,
            key,
            id_col,
            has_best,
        })
    }

    fn keyval(&self, chart: &Chart) -> Option<(String, SqlValue)> {
        let (col, field) = self.key.as_ref()?;
        field.keyval(chart).map(|v| (col.clone(), v))
    }

    pub fn best_clear(&self, chart: &Chart) -> i64 {
        let (col, kv) = match self.keyval(chart) {
            Some(x) => x,
            None => return 0,
        };
        let sql = format!("SELECT clear_type FROM score_best WHERE {col} = ?");
        let mut best = 0;
        if let Ok(mut stmt) = self.con.prepare(&sql) {
            if let Ok(rows) = stmt.query_map([kv], |r| r.get::<_, SqlValue>(0)) {
                for v in rows.flatten() {
                    best = best.max(clear_value(&to_json(&v)));
                }
            }
        }
        best
    }

    /// 自己ベスト。未プレイなら None
    pub fn best_detail(&self, chart: &Chart) -> Option<BestDetail> {
        if !self.has_best {
            return None;
        }
        let (col, kv) = self.keyval(chart)?;
        let sql = format!("SELECT clear_type, ex_score, bp FROM score_best WHERE {col} = ?");
        let mut stmt = self.con.prepare(&sql).ok()?;
        let rows = stmt
            .query_map([kv], |r| {
                Ok((
                    r.get::<_, SqlValue>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })
            .ok()?;
        let mut best: Option<(i64, Option<i64>, Option<i64>)> = None;
        for row in rows.flatten() {
            let (ctv, ex, bp) = row;
            let cand = (clear_value(&to_json(&ctv)), ex, bp);
            let better = match &best {
                None => true,
                Some(b) => (cand.0, cand.1.unwrap_or(0)) > (b.0, b.1.unwrap_or(0)),
            };
            if better {
                best = Some(cand);
            }
        }
        let (ct, ex, bp) = best?;
        Some(BestDetail {
            lamp: lamp_name(ct).unwrap_or("?").to_string(),
            ex_score: ex,
            bp,
        })
    }

    pub fn played_ever(&self, chart: &Chart) -> bool {
        let (col, kv) = match self.keyval(chart) {
            Some(x) => x,
            None => return false,
        };
        let sql = format!("SELECT COUNT(*) FROM score_history WHERE {col} = ?");
        self.con
            .query_row(&sql, [kv], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// 起動前の履歴最大ID (この後増えた分が今回の結果)。
    /// 読み取りに失敗した場合は None (0 と区別する。失敗を 0 として
    /// 扱うと、回復後に過去の結果を今回の結果と誤認する)
    pub fn history_mark(&self, chart: &Chart) -> Option<i64> {
        let (col, kv) = self.keyval(chart)?;
        let sql = format!(
            "SELECT MAX({}) FROM score_history WHERE {col} = ?",
            self.id_col
        );
        match self.con.query_row(&sql, [kv], |r| r.get::<_, Option<i64>>(0)) {
            Ok(v) => Some(v.unwrap_or(0)),
            Err(_) => None,
        }
    }

    /// 起動後に増えた最新の履歴の clear_type (数値)。無ければ None
    pub fn latest_result(&self, chart: &Chart, after_id: i64) -> Option<i64> {
        self.latest_detail(chart, after_id)
            .map(|d| d.clear_type)
    }

    /// 最新の履歴の詳細。無ければ None
    pub fn latest_detail(&self, chart: &Chart, after_id: i64) -> Option<ResultDetail> {
        let (col, kv) = self.keyval(chart)?;
        let sql = format!(
            "SELECT clear_type, ex_score, bp FROM score_history \
             WHERE {col} = ? AND {} > ? ORDER BY {} DESC LIMIT 1",
            self.id_col, self.id_col
        );
        let mut stmt = self.con.prepare(&sql).ok()?;
        stmt.query_row(rusqlite::params![kv, after_id], |r| {
            Ok((
                r.get::<_, SqlValue>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })
        .ok()
        .map(|(ctv, ex, bp)| ResultDetail {
            clear_type: clear_value(&to_json(&ctv)),
            ex_score: ex,
            bp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_library() -> Library {
        let con = Connection::open_in_memory().unwrap();
        con.execute_batch(
            "CREATE TABLE difficulty_tables
               (id INTEGER, table_name TEXT, level_order TEXT, symbol TEXT);
             CREATE TABLE difficulty_table_entries
               (table_id INTEGER, level TEXT, chart_id INTEGER, title TEXT);
             CREATE TABLE charts
               (id INTEGER, sha256 TEXT, md5 TEXT, title TEXT, total_notes INTEGER);
             CREATE TABLE chart_files (id INTEGER, sha256 TEXT, path TEXT);
             CREATE TABLE chart_file_links (chart_id INTEGER, chart_file_id INTEGER);
             INSERT INTO difficulty_tables VALUES (1, '発狂BMS', '[\"2\",\"1\"]', '★');
             INSERT INTO charts VALUES (10, 'sha-a', 'md5-a', 'Song A', 1000);
             INSERT INTO charts VALUES (11, 'sha-b', 'md5-b', 'Song B', 2000);
             INSERT INTO difficulty_table_entries VALUES (1, '1', 10, NULL);
             INSERT INTO difficulty_table_entries VALUES (1, '1', 11, NULL);
             INSERT INTO difficulty_table_entries VALUES (1, '2', 10, NULL);
             INSERT INTO chart_files VALUES (100, 'sha-a', 'D:/bms/a.bms');
             INSERT INTO chart_file_links VALUES (10, 100);",
        )
        .unwrap();
        Library::with_connection(con).unwrap()
    }

    #[test]
    fn tables_levels_and_charts() {
        let mut lib = test_library();
        let ts = lib.list_tables();
        assert_eq!(ts, vec![(1, "発狂BMS".to_string())]);
        assert_eq!(lib.symbol(1), "★");
        // level_order の指定順が優先される
        assert_eq!(lib.levels(1), vec!["2".to_string(), "1".to_string()]);
        let charts = lib.charts_in_level(1, "1");
        assert_eq!(charts.len(), 2);
        let a = charts.iter().find(|c| c.title == "Song A").unwrap();
        assert_eq!(a.path.as_deref(), Some("D:/bms/a.bms"));
        assert_eq!(lib.total_notes(a), Some(1000));
        assert_eq!(lib.count_entries(1, "1"), 2);
    }

    fn test_scores() -> Scores {
        let con = Connection::open_in_memory().unwrap();
        con.execute_batch(
            "CREATE TABLE score_best
               (chart_sha256 TEXT, clear_type TEXT, ex_score INTEGER, bp INTEGER);
             CREATE TABLE score_history
               (id INTEGER, chart_sha256 TEXT, clear_type TEXT,
                ex_score INTEGER, bp INTEGER);
             INSERT INTO score_best VALUES ('sha-a', 'Hard', 1800, 12);
             INSERT INTO score_history VALUES (1, 'sha-a', 'Failed', 900, 300);
             INSERT INTO score_history VALUES (2, 'sha-a', 'Hard', 1800, 12);",
        )
        .unwrap();
        Scores::with_connection(con).unwrap()
    }

    fn chart_a() -> Chart {
        Chart {
            chart_id: Some(10),
            sha256: Some("sha-a".to_string()),
            md5: Some("md5-a".to_string()),
            title: "Song A".to_string(),
            path: None,
        }
    }

    #[test]
    fn score_queries() {
        let sc = test_scores();
        let a = chart_a();
        assert_eq!(sc.best_clear(&a), 6);
        assert!(sc.played_ever(&a));
        assert_eq!(sc.history_mark(&a), Some(2));
        assert_eq!(sc.latest_result(&a, 1), Some(6));
        assert_eq!(sc.latest_result(&a, 2), None);
        let best = sc.best_detail(&a).unwrap();
        assert_eq!(best.lamp, "HARD CLEAR");
        assert_eq!(best.ex_score, Some(1800));
    }
}
