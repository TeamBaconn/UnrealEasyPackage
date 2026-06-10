//! Persistence for build records under `<project>/.uep/history/<buildId>/`
//! (`metadata.json` + `build.log`). Callers pass the `history` dir, so these stay
//! pure + unit-testable (mirrors `profiles::store`). The JSON records are the
//! source of truth; a derived SQLite index (`docs/data-storage.md`) is deferred.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use super::schema::BuildRecord;

fn build_dir(history_dir: &Path, build_id: &str) -> PathBuf {
    history_dir.join(build_id)
}

/// Write `metadata.json` + `build.log` + `build.idx` for one build (creating its
/// folder). `phase_idx` is one phase index per `build.log` line (parallel arrays),
/// so replay can re-attribute each line to its phase for the per-phase console.
pub fn write(history_dir: &Path, record: &BuildRecord, log: &str, phase_idx: &[u32]) -> io::Result<()> {
    let dir = build_dir(history_dir, &record.build_id);
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(record)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(dir.join("metadata.json"), json)?;
    fs::write(dir.join("build.log"), log)?;
    let idx = phase_idx.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
    fs::write(dir.join("build.idx"), idx)?;
    Ok(())
}

fn load_one_path(dir: &Path) -> Option<BuildRecord> {
    let text = fs::read_to_string(dir.join("metadata.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// All records under `history_dir`, newest first. Unparseable folders are skipped.
pub fn load_all(history_dir: &Path) -> Vec<BuildRecord> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(history_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Some(rec) = load_one_path(&entry.path()) {
                out.push(rec);
            }
        }
    }
    out.sort_by(|a, b| {
        b.started_at_ms
            .partial_cmp(&a.started_at_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

pub fn load_one(history_dir: &Path, build_id: &str) -> Option<BuildRecord> {
    load_one_path(&build_dir(history_dir, build_id))
}

pub fn load_log(history_dir: &Path, build_id: &str) -> Option<String> {
    fs::read_to_string(build_dir(history_dir, build_id).join("build.log")).ok()
}

/// Per-line phase indices saved alongside `build.log` (empty when absent - older
/// records, where replay attributes every line to phase 0).
pub fn load_phase_idx(history_dir: &Path, build_id: &str) -> Vec<u32> {
    fs::read_to_string(build_dir(history_dir, build_id).join("build.idx"))
        .ok()
        .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

pub fn delete(history_dir: &Path, build_id: &str) -> io::Result<()> {
    match fs::remove_dir_all(build_dir(history_dir, build_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()), // idempotent
        Err(e) => Err(e),
    }
}

/// Total bytes of a directory tree (0 if missing) - the archived build size.
pub fn dir_size(path: &Path) -> u64 {
    // WalkDir on a missing path yields an error that `.flatten()` drops → sum 0,
    // so no explicit existence pre-check is needed (and it would only race).
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// A path's modified time as epoch milliseconds (0 if unavailable) - the basis for
/// the "Open location" modified-date integrity check.
pub fn mtime_ms(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::schema::{PhaseTiming, SCHEMA_VERSION};
    use tempfile::tempdir;

    fn sample(id: &str, started: f64) -> BuildRecord {
        BuildRecord {
            schema_version: SCHEMA_VERSION,
            build_id: id.into(),
            started_at_ms: started,
            duration: 980.0,
            build_size: 4_831_838_208.0,
            warning_count: 5,
            error_count: 2,
            output_path: "C:/Builds/out".into(),
            output_mtime_ms: started + 1000.0,
            phases: vec![PhaseTiming {
                phase: "Build".into(),
                start_offset: 0.0,
                duration: 250.0,
                status: "Success".into(),
                kind: "external".into(),
                command: "Build.bat SampleProjectSteam Win64 Development".into(),
            }],
            tags: vec!["Win64".into(), "Development".into(), "SampleProjectSteam".into(), "Success".into()],
        }
    }

    #[test]
    fn write_load_delete_round_trip() {
        let d = tempdir().unwrap();
        let rec = sample("b1", 1000.0);
        write(d.path(), &rec, "line one\nline two\n", &[0, 1]).unwrap();

        assert_eq!(load_one(d.path(), "b1").unwrap(), rec);
        assert_eq!(load_log(d.path(), "b1").unwrap(), "line one\nline two\n");
        assert_eq!(load_phase_idx(d.path(), "b1"), vec![0, 1]);
        assert!(load_phase_idx(d.path(), "missing").is_empty());
        assert_eq!(load_all(d.path()).len(), 1);

        delete(d.path(), "b1").unwrap();
        assert!(load_one(d.path(), "b1").is_none());
        delete(d.path(), "b1").unwrap(); // idempotent
    }

    #[test]
    fn load_all_is_newest_first() {
        let d = tempdir().unwrap();
        write(d.path(), &sample("old", 1000.0), "", &[]).unwrap();
        write(d.path(), &sample("new", 5000.0), "", &[]).unwrap();
        write(d.path(), &sample("mid", 3000.0), "", &[]).unwrap();
        let ids: Vec<String> = load_all(d.path()).into_iter().map(|r| r.build_id).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[test]
    fn dir_size_sums_files() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("a.bin"), vec![0u8; 100]).unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub/b.bin"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size(d.path()), 150);
        assert_eq!(dir_size(&d.path().join("missing")), 0);
    }
}
