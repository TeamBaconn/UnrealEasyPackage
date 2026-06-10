//! Derived **SQLite index** over the JSON build records (`docs/data-storage.md`
//! §"History index"). The `.uep/history/<id>/metadata.json` files are the source of
//! truth; this index at `.uep/cache/history.db` is a disposable, fully-rebuildable
//! cache that gives the Build tab fast **paged + filtered** queries without reading
//! every record into memory.
//!
//! Sync (spec §"Keeping the index in sync"): the runner **upserts** a row on each
//! build finish; reads call [`open_synced`], which rebuilds on a missing/stale-schema
//! DB (`PRAGMA user_version` mismatch) and reconciles a **folder-count vs row-count**
//! drift by reindexing from the JSON. Deleting `history.db` is a safe manual rebuild.
//!
//! Deviations from the doc's illustrative DDL: numeric columns are `REAL`/`INTEGER`
//! matching the record's `f64`/`u32` fields (so a row round-trips a [`BuildRecord`]
//! losslessly) rather than ISO-text; `build_phases` also stores `kind`/`command` and
//! an `ord` for that lossless reconstruction; drift reconcile is automatic (no prompt).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension};

use super::schema::{BuildRecord, PhaseTiming};
use super::store;

/// Bumped when the table shapes below change ⇒ the DB is rebuilt from JSON.
/// (v2: `warning_count` / `error_count` columns on `builds`.)
const USER_VERSION: i64 = 2;

/// Filter dimensions (each a tag value the build must carry); `None` ⇒ unconstrained.
#[derive(Default)]
pub struct Filter {
    pub platform: Option<String>,
    pub config: Option<String>,
    pub target: Option<String>,
    pub status: Option<String>,
}

/// `<project>/.uep/cache/history.db` for a `<project>/.uep/history` dir.
fn db_path(history_dir: &Path) -> PathBuf {
    history_dir
        .parent()
        .unwrap_or(history_dir)
        .join("cache")
        .join("history.db")
}

/// Open the index, ensuring the schema (rebuilding on a `user_version` mismatch or a
/// missing `builds` table). Does **not** reconcile drift - use this on the write path
/// (finish upsert / delete), where the caller keeps rows in step with the folders.
pub fn open(history_dir: &Path) -> rusqlite::Result<Connection> {
    let path = db_path(history_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != USER_VERSION || !schema_present(&conn) {
        rebuild_schema(&conn)?;
    }
    Ok(conn)
}

/// [`open`] plus the spec's boot drift-check: compare the `history/` subfolder count
/// with `SELECT COUNT(*) FROM builds` and reindex from the JSON on any mismatch (this
/// also fills a freshly-rebuilt empty schema). Use this on the read path.
pub fn open_synced(history_dir: &Path) -> rusqlite::Result<Connection> {
    let conn = open(history_dir)?;
    let folders = count_dirs(history_dir) as i64;
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM builds", [], |r| r.get(0))?;
    if folders != rows {
        reindex_all(&conn, &store::load_all(history_dir))?;
    }
    Ok(conn)
}

fn schema_present(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'builds'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
    .unwrap_or(false)
}

fn count_dirs(history_dir: &Path) -> usize {
    // Count only folders that actually hold a `metadata.json` - the same population
    // `store::load_all` (and thus `reindex_all`) indexes. Counting *every* subdir
    // would let a stray/partial folder make folders != rows forever, re-triggering
    // the heavy reconcile on every read.
    std::fs::read_dir(history_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("metadata.json").is_file())
                .count()
        })
        .unwrap_or(0)
}

fn rebuild_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS build_phases;
         DROP TABLE IF EXISTS build_tags;
         DROP TABLE IF EXISTS tags;
         DROP TABLE IF EXISTS builds;
         CREATE TABLE builds (
            build_id        TEXT PRIMARY KEY,
            schema_version  INTEGER NOT NULL,
            started_at_ms   REAL    NOT NULL,
            duration        REAL    NOT NULL,
            build_size      REAL    NOT NULL,
            warning_count   INTEGER NOT NULL DEFAULT 0,
            error_count     INTEGER NOT NULL DEFAULT 0,
            output_path     TEXT    NOT NULL,
            output_mtime_ms REAL    NOT NULL
         );
         CREATE TABLE tags (
            tag_id INTEGER PRIMARY KEY,
            value  TEXT NOT NULL UNIQUE
         );
         CREATE TABLE build_tags (
            build_id TEXT    NOT NULL REFERENCES builds(build_id) ON DELETE CASCADE,
            tag_id   INTEGER NOT NULL REFERENCES tags(tag_id),
            ord      INTEGER NOT NULL,
            PRIMARY KEY (build_id, tag_id)
         );
         CREATE TABLE build_phases (
            build_id     TEXT    NOT NULL REFERENCES builds(build_id) ON DELETE CASCADE,
            ord          INTEGER NOT NULL,
            phase        TEXT    NOT NULL,
            start_offset REAL    NOT NULL,
            duration     REAL    NOT NULL,
            status       TEXT    NOT NULL,
            kind         TEXT    NOT NULL,
            command      TEXT    NOT NULL,
            PRIMARY KEY (build_id, ord)
         );
         CREATE INDEX ix_build_tags_tag    ON build_tags(tag_id);
         CREATE INDEX ix_builds_started    ON builds(started_at_ms);
         CREATE INDEX ix_build_phases_phase ON build_phases(phase);",
    )?;
    conn.pragma_update(None, "user_version", USER_VERSION)?;
    Ok(())
}

/// Replace the whole index with `records` (the heavy reconcile / rebuild).
pub fn reindex_all(conn: &Connection, records: &[BuildRecord]) -> rusqlite::Result<()> {
    conn.execute_batch(
        "BEGIN;
         DELETE FROM build_phases;
         DELETE FROM build_tags;
         DELETE FROM tags;
         DELETE FROM builds;",
    )?;
    for rec in records {
        insert_record(conn, rec)?;
    }
    conn.execute_batch("COMMIT;")?;
    Ok(())
}

/// Insert-or-replace one record's rows across all three tables.
pub fn upsert(conn: &Connection, rec: &BuildRecord) -> rusqlite::Result<()> {
    insert_record(conn, rec)
}

fn insert_record(conn: &Connection, rec: &BuildRecord) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO builds
            (build_id, schema_version, started_at_ms, duration, build_size, warning_count, error_count, output_path, output_mtime_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            rec.build_id,
            rec.schema_version,
            rec.started_at_ms,
            rec.duration,
            rec.build_size,
            rec.warning_count,
            rec.error_count,
            rec.output_path,
            rec.output_mtime_ms,
        ],
    )?;
    // Clear children first so a replace doesn't leave stale tag/phase rows.
    conn.execute("DELETE FROM build_tags WHERE build_id = ?1", params![rec.build_id])?;
    conn.execute("DELETE FROM build_phases WHERE build_id = ?1", params![rec.build_id])?;
    for (i, tag) in rec.tags.iter().enumerate() {
        conn.execute("INSERT OR IGNORE INTO tags (value) VALUES (?1)", params![tag])?;
        let tag_id: i64 = conn.query_row("SELECT tag_id FROM tags WHERE value = ?1", params![tag], |r| r.get(0))?;
        conn.execute(
            "INSERT OR REPLACE INTO build_tags (build_id, tag_id, ord) VALUES (?1, ?2, ?3)",
            params![rec.build_id, tag_id, i as i64],
        )?;
    }
    for (i, p) in rec.phases.iter().enumerate() {
        conn.execute(
            "INSERT OR REPLACE INTO build_phases
                (build_id, ord, phase, start_offset, duration, status, kind, command)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![rec.build_id, i as i64, p.phase, p.start_offset, p.duration, p.status, p.kind, p.command],
        )?;
    }
    Ok(())
}

/// Drop records by id (child rows cascade via `ON DELETE CASCADE`).
pub fn remove(conn: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    for id in ids {
        conn.execute("DELETE FROM builds WHERE build_id = ?1", params![id])?;
    }
    Ok(())
}

pub fn grand_total(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM builds", [], |r| r.get(0))
}

/// Distinct tag values still referenced by a build (drives the filter menus).
pub fn distinct_tags(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT t.value FROM tags t JOIN build_tags bt ON bt.tag_id = t.tag_id ORDER BY t.value",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.flatten().collect())
}

/// One page of records (newest first) for the given filter, plus the filtered total.
pub fn query_page(
    conn: &Connection,
    offset: u32,
    limit: u32,
    filter: &Filter,
) -> rusqlite::Result<(Vec<BuildRecord>, i64)> {
    let (where_sql, binds) = filter_clause(filter);

    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM builds{where_sql}"),
        rusqlite::params_from_iter(binds.iter()),
        |r| r.get(0),
    )?;

    let ids: Vec<String> = {
        let sql = format!("SELECT build_id FROM builds{where_sql} ORDER BY started_at_ms DESC LIMIT ? OFFSET ?");
        let mut values: Vec<Value> = binds.iter().map(|s| Value::Text(s.clone())).collect();
        values.push(Value::Integer(limit as i64));
        values.push(Value::Integer(offset as i64));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };

    let mut records = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(rec) = load_record(conn, id)? {
            records.push(rec);
        }
    }
    Ok((records, total))
}

/// `(" WHERE …", binds)` - one `build_id IN (…)` membership test per set dimension.
fn filter_clause(filter: &Filter) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut binds = Vec::new();
    for dim in [&filter.platform, &filter.config, &filter.target, &filter.status] {
        if let Some(value) = dim {
            clauses.push(
                "build_id IN (SELECT bt.build_id FROM build_tags bt JOIN tags t ON t.tag_id = bt.tag_id WHERE t.value = ?)"
                    .to_string(),
            );
            binds.push(value.clone());
        }
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    (where_sql, binds)
}

/// Reconstruct a full [`BuildRecord`] from its `builds` row + ordered tags + phases.
fn load_record(conn: &Connection, id: &str) -> rusqlite::Result<Option<BuildRecord>> {
    let base = conn
        .query_row(
            "SELECT schema_version, started_at_ms, duration, build_size, warning_count, error_count, output_path, output_mtime_ms
             FROM builds WHERE build_id = ?1",
            params![id],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, u32>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, f64>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((schema_version, started_at_ms, duration, build_size, warning_count, error_count, output_path, output_mtime_ms)) =
        base
    else {
        return Ok(None);
    };

    let tags: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT t.value FROM build_tags bt JOIN tags t ON t.tag_id = bt.tag_id WHERE bt.build_id = ?1 ORDER BY bt.ord",
        )?;
        let rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
        rows.flatten().collect()
    };
    let phases: Vec<PhaseTiming> = {
        let mut stmt = conn.prepare(
            "SELECT phase, start_offset, duration, status, kind, command FROM build_phases WHERE build_id = ?1 ORDER BY ord",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            Ok(PhaseTiming {
                phase: r.get(0)?,
                start_offset: r.get(1)?,
                duration: r.get(2)?,
                status: r.get(3)?,
                kind: r.get(4)?,
                command: r.get(5)?,
            })
        })?;
        rows.flatten().collect()
    };

    Ok(Some(BuildRecord {
        schema_version,
        build_id: id.to_string(),
        started_at_ms,
        duration,
        build_size,
        warning_count,
        error_count,
        output_path,
        output_mtime_ms,
        phases,
        tags,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::schema::SCHEMA_VERSION;
    use tempfile::tempdir;

    fn rec(id: &str, started: f64, tags: Vec<&str>) -> BuildRecord {
        BuildRecord {
            schema_version: SCHEMA_VERSION,
            build_id: id.into(),
            started_at_ms: started,
            duration: 100.0,
            build_size: 1000.0,
            warning_count: 3,
            error_count: 1,
            output_path: "C:/out".into(),
            output_mtime_ms: started + 1.0,
            phases: vec![PhaseTiming {
                phase: "Build".into(),
                start_offset: 0.0,
                duration: 50.0,
                status: "Success".into(),
                kind: "external".into(),
                command: "Build.bat".into(),
            }],
            tags: tags.into_iter().map(String::from).collect(),
        }
    }

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        rebuild_schema(&c).unwrap();
        c
    }

    #[test]
    fn page_orders_newest_first_and_round_trips() {
        let c = conn();
        reindex_all(
            &c,
            &[
                rec("a", 1000.0, vec!["Win64", "Development", "Proj", "Success"]),
                rec("b", 3000.0, vec!["Win64", "Shipping", "Proj", "Failed"]),
                rec("c", 2000.0, vec!["Linux", "Development", "Proj", "Success"]),
            ],
        )
        .unwrap();
        let (page, total) = query_page(&c, 0, 2, &Filter::default()).unwrap();
        assert_eq!(total, 3);
        assert_eq!(page.iter().map(|r| r.build_id.clone()).collect::<Vec<_>>(), vec!["b", "c"]);
        // full round-trip of the first record
        assert_eq!(page[0], rec("b", 3000.0, vec!["Win64", "Shipping", "Proj", "Failed"]));
    }

    #[test]
    fn filter_requires_all_dimensions_and_offsets() {
        let c = conn();
        reindex_all(
            &c,
            &[
                rec("a", 1000.0, vec!["Win64", "Development", "Proj", "Success"]),
                rec("b", 3000.0, vec!["Win64", "Shipping", "Proj", "Failed"]),
                rec("c", 2000.0, vec!["Linux", "Development", "Proj", "Success"]),
            ],
        )
        .unwrap();
        let f = Filter { platform: Some("Win64".into()), status: Some("Success".into()), ..Default::default() };
        let (page, total) = query_page(&c, 0, 10, &f).unwrap();
        assert_eq!(total, 1);
        assert_eq!(page[0].build_id, "a");

        // offset past the end yields nothing but the total still reflects the filter
        let f2 = Filter { config: Some("Development".into()), ..Default::default() };
        let (page2, total2) = query_page(&c, 5, 10, &f2).unwrap();
        assert_eq!(total2, 2);
        assert!(page2.is_empty());
    }

    #[test]
    fn upsert_remove_and_distinct_tags() {
        let c = conn();
        upsert(&c, &rec("a", 1000.0, vec!["Win64", "Development", "Proj", "Success"])).unwrap();
        upsert(&c, &rec("a", 1500.0, vec!["Win64", "Shipping", "Proj", "Failed"])).unwrap(); // replace
        assert_eq!(grand_total(&c).unwrap(), 1);
        let (page, _) = query_page(&c, 0, 10, &Filter::default()).unwrap();
        assert_eq!(page[0].started_at_ms, 1500.0);
        assert!(page[0].tags.contains(&"Shipping".to_string()));

        remove(&c, &["a".into()]).unwrap();
        assert_eq!(grand_total(&c).unwrap(), 0);
        assert!(distinct_tags(&c).unwrap().is_empty()); // no referenced tags after delete
    }

    #[test]
    fn open_synced_rebuilds_from_json_on_count_drift() {
        let dir = tempdir().unwrap();
        let history = dir.path().join(".uep").join("history");
        std::fs::create_dir_all(&history).unwrap();
        store::write(&history, &rec("a", 1000.0, vec!["Win64", "Development", "Proj", "Success"]), "log", &[0]).unwrap();
        store::write(&history, &rec("b", 2000.0, vec!["Linux", "Shipping", "Proj", "Failed"]), "log", &[0]).unwrap();

        // first open sees an empty DB (0) vs two folders → reindex
        let c = open_synced(&history).unwrap();
        assert_eq!(grand_total(&c).unwrap(), 2);
    }
}
