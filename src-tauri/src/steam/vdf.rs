//! A minimal Valve **KeyValues (VDF)** tree + the Steam `app_build`/`depot_build`
//! generators.
//!
//! Steam build scripts are simple nested maps of quoted strings, so a hand-rolled
//! order-preserving tree (no external crate) is enough - and it lets us do the one thing
//! the feature needs: **regenerate the managed keys while preserving any other keys the
//! user hand-added** (`docs/build-commands.md` §11). Order is preserved (a `Vec`, not a
//! map) so regeneration produces stable diffs and duplicate keys (e.g. repeated
//! `FileExclusion`) survive a round-trip. Comments are **not** preserved (the requirement
//! is custom *fields*, i.e. key/value pairs; `//` comments are dropped on rewrite).

use std::io;
use std::path::{Path, PathBuf};

use crate::profiles::schema::{BuildConfig, DepotItem, SteamUploadCfg};

/// A VDF value: a quoted string leaf, or a nested block of key/value entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Str(String),
    Obj(Entries),
}

/// An ordered list of key/value entries (a VDF block body). Duplicate keys are allowed.
pub type Entries = Vec<(String, Vdf)>;

// ── tree helpers (order-preserving, preserve-unknown) ──────────────────────────

/// First value for `key`, if present.
fn get<'a>(entries: &'a Entries, key: &str) -> Option<&'a Vdf> {
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Set (or append) a string entry. Preserves the key's existing position when present,
/// overwriting only its value; appends at the end otherwise.
fn set_str(entries: &mut Entries, key: &str, val: &str) {
    if let Some(slot) = entries.iter_mut().find(|(k, _)| k == key) {
        slot.1 = Vdf::Str(val.to_string());
    } else {
        entries.push((key.to_string(), Vdf::Str(val.to_string())));
    }
}

/// Get a mutable ref to `key`'s nested block, inserting an empty one (in place if the key
/// exists as a string, else appended) when needed.
fn get_or_insert_obj<'a>(entries: &'a mut Entries, key: &str) -> &'a mut Entries {
    let pos = entries.iter().position(|(k, _)| k == key);
    let idx = match pos {
        Some(i) => {
            if !matches!(entries[i].1, Vdf::Obj(_)) {
                entries[i].1 = Vdf::Obj(Vec::new());
            }
            i
        }
        None => {
            entries.push((key.to_string(), Vdf::Obj(Vec::new())));
            entries.len() - 1
        }
    };
    match &mut entries[idx].1 {
        Vdf::Obj(o) => o,
        _ => unreachable!("just ensured Obj"),
    }
}

// ── parser ─────────────────────────────────────────────────────────────────────

/// Parse a VDF document into its top-level entries. Accepts quoted (`"key"`) and bare
/// (`key`) tokens; skips `//` line comments and whitespace.
pub fn parse(src: &str) -> Result<Entries, String> {
    let tokens = tokenize(src)?;
    let mut pos = 0;
    let entries = parse_entries(&tokens, &mut pos, false)?;
    if pos != tokens.len() {
        return Err("unexpected '}' at top level".to_string());
    }
    Ok(entries)
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Str(String),
    LBrace,
    RBrace,
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            // line comment → skip to end of line
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '{' {
            out.push(Token::LBrace);
            i += 1;
        } else if c == '}' {
            out.push(Token::RBrace);
            i += 1;
        } else if c == '"' {
            // quoted string with \" and \\ escapes
            let mut s = String::new();
            i += 1;
            loop {
                if i >= chars.len() {
                    return Err("unterminated quoted string".to_string());
                }
                match chars[i] {
                    '"' => {
                        i += 1;
                        break;
                    }
                    '\\' if i + 1 < chars.len() => {
                        // Recognized escapes (\\, \", \n, \t) decode; any other `\X` is kept
                        // verbatim (backslash preserved) so the tokenizer stays the exact
                        // inverse of `escape()`. Otherwise a hand-authored Windows path like
                        // C:\Games\Mine would lose its backslashes (become C:GamesMine) on a
                        // parse → rewrite cycle, silently corrupting user-preserved keys.
                        match chars[i + 1] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            other => {
                                s.push('\\');
                                s.push(other);
                            }
                        }
                        i += 2;
                    }
                    ch => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Token::Str(s));
        } else {
            // bare token: run until whitespace/brace/quote
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '{' | '}' | '"') {
                i += 1;
            }
            out.push(Token::Str(chars[start..i].iter().collect()));
        }
    }
    Ok(out)
}

fn parse_entries(tokens: &[Token], pos: &mut usize, nested: bool) -> Result<Entries, String> {
    let mut entries = Entries::new();
    loop {
        match tokens.get(*pos) {
            None => {
                if nested {
                    return Err("unexpected end of input inside a block".to_string());
                }
                return Ok(entries);
            }
            Some(Token::RBrace) => {
                if !nested {
                    return Ok(entries); // caller (parse) checks for the stray brace
                }
                *pos += 1; // consume the closing brace
                return Ok(entries);
            }
            Some(Token::LBrace) => return Err("expected a key, found '{'".to_string()),
            Some(Token::Str(key)) => {
                let key = key.clone();
                *pos += 1;
                match tokens.get(*pos) {
                    Some(Token::LBrace) => {
                        *pos += 1;
                        let inner = parse_entries(tokens, pos, true)?;
                        entries.push((key, Vdf::Obj(inner)));
                    }
                    Some(Token::Str(val)) => {
                        let val = val.clone();
                        *pos += 1;
                        entries.push((key, Vdf::Str(val)));
                    }
                    _ => return Err(format!("key \"{key}\" has no value")),
                }
            }
        }
    }
}

// ── serializer ─────────────────────────────────────────────────────────────────

/// Serialize entries into canonical tab-indented VDF text.
pub fn to_string(entries: &Entries) -> String {
    let mut out = String::new();
    write_entries(&mut out, entries, 0);
    out
}

fn write_entries(out: &mut String, entries: &Entries, depth: usize) {
    let indent = "\t".repeat(depth);
    for (key, val) in entries {
        match val {
            Vdf::Str(s) => {
                out.push_str(&format!("{indent}\"{}\"\t\"{}\"\n", escape(key), escape(s)));
            }
            Vdf::Obj(inner) => {
                out.push_str(&format!("{indent}\"{}\"\n{indent}{{\n", escape(key)));
                write_entries(out, inner, depth + 1);
                out.push_str(&format!("{indent}}}\n"));
            }
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ── path helpers ───────────────────────────────────────────────────────────────

/// `<project>/.uep/steam-config` - the committed (shared) VDF root.
pub fn steam_config_dir(project_root: &Path) -> PathBuf {
    project_root.join(".uep").join("steam-config")
}

/// `<project>/.uep/steam-config/<profile-id>` - a profile's committed VDFs.
pub fn committed_dir(project_root: &Path, profile_id: &str) -> PathBuf {
    steam_config_dir(project_root).join(profile_id)
}

/// `<project>/.uep/steam-build-output` - the git-ignored scratch root (steamcmd cache/logs
/// + resolved run VDFs); the `SteamBuildOutput` cleanup category maps here.
pub fn scratch_root(project_root: &Path) -> PathBuf {
    project_root.join(".uep").join("steam-build-output")
}

/// `<project>/.uep/steam-build-output/<profile-id>` - a run's resolved VDFs + steamcmd output.
pub fn scratch_dir(project_root: &Path, profile_id: &str) -> PathBuf {
    scratch_root(project_root).join(profile_id)
}

/// The resolved `app_build.vdf` steamcmd runs against (written by [`resolve_run_vdf`]).
pub fn run_app_build_vdf_path(project_root: &Path, profile_id: &str) -> PathBuf {
    scratch_dir(project_root, profile_id).join("app_build.vdf")
}

fn depot_file_name(depot_id: &str) -> String {
    format!("depot_{depot_id}.vdf")
}

// ── generators ─────────────────────────────────────────────────────────────────

/// Set the app-level managed keys (owned by the program) on an `AppBuild` block, leaving
/// every other key untouched. `Depots` is rebuilt from the profile (it's fully managed).
fn set_managed_app_keys(app: &mut Entries, cfg: &SteamUploadCfg) {
    set_str(app, "AppID", cfg.app_id.trim());
    set_str(app, "Desc", &cfg.description);
    set_str(app, "Preview", if cfg.preview { "1" } else { "0" });
    set_str(app, "SetLive", cfg.branch.trim());
    let depots = get_or_insert_obj(app, "Depots");
    depots.clear();
    for d in &cfg.depots {
        let id = d.depot_id.trim();
        depots.push((id.to_string(), Vdf::Str(depot_file_name(id))));
    }
}

/// The default `FileMapping` for a freshly-templated depot: map everything under the
/// depot's `path` (content-root-relative) into the depot root, recursively.
fn default_file_mapping(depot: &DepotItem) -> Vdf {
    let p = depot.path.trim().trim_end_matches(['/', '\\']);
    let local = if p.is_empty() || p == "." { "*".to_string() } else { format!("{p}/*") };
    Vdf::Obj(vec![
        ("LocalPath".to_string(), Vdf::Str(local)),
        ("DepotPath".to_string(), Vdf::Str(".".to_string())),
        ("Recursive".to_string(), Vdf::Str("1".to_string())),
    ])
}

/// Merge a depot's managed keys (just `DepotID`) into an existing/absent depot tree,
/// seeding the default `FileMapping` only when the depot is new (no mapping yet), so a
/// user's hand-tuned mappings/exclusions are preserved on regeneration.
fn build_depot_entries(existing: Option<Entries>, depot: &DepotItem) -> Entries {
    let mut entries = existing.unwrap_or_default();
    let build = get_or_insert_obj(&mut entries, "DepotBuild");
    set_str(build, "DepotID", depot.depot_id.trim());
    if get(build, "FileMapping").is_none() {
        build.push(("FileMapping".to_string(), default_file_mapping(depot)));
    }
    entries
}

/// Read + parse a VDF file. `Ok(None)` when the file is absent; `Err` when it exists but
/// can't be parsed (callers decide whether to skip or fail rather than clobber it).
fn read_vdf(path: &Path) -> Result<Option<Entries>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn write_text(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

fn io_err(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Regenerate the **committed** VDFs from a profile's managed Steam fields, preserving any
/// user-added keys. Writes `app_build.vdf` + one `depot_<id>.vdf` per depot into
/// `.uep/steam-config/<profile-id>/`. `ContentRoot`/`BuildOutput` are **not** written here
/// (they're machine paths, injected at run). Called on profile save when the phase is
/// enabled; if a file exists but can't be parsed it is left untouched (don't clobber a
/// file the user is mid-edit on).
pub fn write_committed_vdf(project_root: &Path, profile: &BuildConfig) -> io::Result<()> {
    let cfg = &profile.phases.steam_upload;
    let dir = committed_dir(project_root, &profile.id);
    std::fs::create_dir_all(&dir)?;

    // app_build.vdf
    let app_path = dir.join("app_build.vdf");
    match read_vdf(&app_path) {
        Ok(existing) => {
            let mut root = existing.unwrap_or_default();
            let app = get_or_insert_obj(&mut root, "AppBuild");
            set_managed_app_keys(app, cfg);
            write_text(&app_path, &to_string(&root))?;
        }
        Err(_) => { /* unparsable committed file → leave it for the user to fix */ }
    }

    // depot_<id>.vdf (one per depot)
    for d in &cfg.depots {
        let path = dir.join(depot_file_name(d.depot_id.trim()));
        match read_vdf(&path) {
            Ok(existing) => {
                let entries = build_depot_entries(existing, d);
                write_text(&path, &to_string(&entries))?;
            }
            Err(_) => { /* leave unparsable depot file alone */ }
        }
    }
    Ok(())
}

/// Materialize the **resolved** run VDFs into the scratch dir and return the
/// `app_build.vdf` path steamcmd runs against. Reads the committed VDFs (regenerating
/// managed keys defensively so the run works even if the profile was never saved through
/// the committed-write hook), injects `ContentRoot` (the archive dir) + `BuildOutput` (the
/// scratch output subdir), and copies each depot VDF beside the app VDF (referenced by bare
/// filename). Errors if a committed VDF exists but can't be parsed - the upload can't run
/// against a broken script.
pub fn resolve_run_vdf(project_root: &Path, profile: &BuildConfig, content_root: &str) -> io::Result<PathBuf> {
    let cfg = &profile.phases.steam_upload;
    let committed = committed_dir(project_root, &profile.id);
    let run_dir = scratch_dir(project_root, &profile.id);
    let build_output = run_dir.join("output");
    std::fs::create_dir_all(&build_output)?;

    // app_build.vdf: committed content (or fresh) + managed keys + injected paths.
    let existing = read_vdf(&committed.join("app_build.vdf")).map_err(io_err)?;
    let mut root = existing.unwrap_or_default();
    let app = get_or_insert_obj(&mut root, "AppBuild");
    set_managed_app_keys(app, cfg);
    set_str(app, "ContentRoot", content_root);
    set_str(app, "BuildOutput", &build_output.display().to_string());
    let app_run = run_app_build_vdf_path(project_root, &profile.id);
    write_text(&app_run, &to_string(&root))?;

    // depot VDFs beside it.
    for d in &cfg.depots {
        let name = depot_file_name(d.depot_id.trim());
        let existing = read_vdf(&committed.join(&name)).map_err(io_err)?;
        let entries = build_depot_entries(existing, d);
        write_text(&run_dir.join(&name), &to_string(&entries))?;
    }
    Ok(app_run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::schema::BuildConfig;
    use tempfile::tempdir;

    fn cfg_profile() -> BuildConfig {
        let mut p = BuildConfig::default();
        p.id = "dev".into();
        p.name = "Development".into();
        let s = &mut p.phases.steam_upload;
        s.enabled = true;
        s.app_id = "480".into();
        s.description = "nightly".into();
        s.preview = true;
        s.branch = "beta".into();
        s.depots = vec![DepotItem { depot_id: "481".into(), path: ".".into() }];
        p
    }

    #[test]
    fn parse_round_trips_nested_blocks() {
        let src = "\"AppBuild\"\n{\n\t\"AppID\"\t\"480\"\n\t\"Depots\"\n\t{\n\t\t\"481\"\t\"depot_481.vdf\"\n\t}\n}\n";
        let tree = parse(src).unwrap();
        // AppBuild → { AppID, Depots → { 481 } }
        let app = match get(&tree, "AppBuild").unwrap() {
            Vdf::Obj(o) => o,
            _ => panic!("AppBuild is a block"),
        };
        assert_eq!(get(app, "AppID"), Some(&Vdf::Str("480".into())));
        // serialize + reparse is stable
        let s = to_string(&tree);
        assert_eq!(parse(&s).unwrap(), tree);
    }

    #[test]
    fn round_trip_preserves_backslashes_and_quotes() {
        // A user-authored key whose value is a Windows path with an embedded quote, a
        // tab and a newline: serialize → parse must be lossless (escape/tokenizer are
        // exact inverses), so no backslash is dropped on a save cycle.
        let tree: Entries = vec![(
            "AppBuild".to_string(),
            Vdf::Obj(vec![
                ("InstallDir".to_string(), Vdf::Str("C:\\Games\\Mine".to_string())),
                ("Note".to_string(), Vdf::Str("say \"hi\"\tand\nbye".to_string())),
            ]),
        )];
        assert_eq!(parse(&to_string(&tree)).unwrap(), tree);

        // Hand-authored single backslashes (never produced by escape) must survive too:
        // "C:\Games\Mine" parses to the literal path, not the corrupted "C:GamesMine".
        let hand = "\"InstallDir\"\t\"C:\\Games\\Mine\"\n";
        let parsed = parse(hand).unwrap();
        assert_eq!(get(&parsed, "InstallDir"), Some(&Vdf::Str("C:\\Games\\Mine".into())));
    }

    #[test]
    fn merge_preserves_unknown_user_keys() {
        // A user-authored app_build.vdf with a custom "Local" key + a custom key inside Depots.
        let src = "\"AppBuild\"\n{\n\t\"AppID\"\t\"1\"\n\t\"Local\"\t\"C:/mirror\"\n}\n";
        let mut root = parse(src).unwrap();
        let app = get_or_insert_obj(&mut root, "AppBuild");
        let mut cfg = SteamUploadCfg::default();
        cfg.app_id = "480".into();
        cfg.description = "d".into();
        cfg.depots = vec![DepotItem { depot_id: "481".into(), path: ".".into() }];
        set_managed_app_keys(app, &cfg);
        // managed key overwritten in place, custom key preserved
        assert_eq!(get(app, "AppID"), Some(&Vdf::Str("480".into())));
        assert_eq!(get(app, "Local"), Some(&Vdf::Str("C:/mirror".into())));
        assert!(matches!(get(app, "Depots"), Some(Vdf::Obj(_))));
    }

    #[test]
    fn write_committed_has_no_paths_and_preview_flag() {
        let d = tempdir().unwrap();
        let root = d.path();
        let p = cfg_profile();
        write_committed_vdf(root, &p).unwrap();

        let app = std::fs::read_to_string(committed_dir(root, "dev").join("app_build.vdf")).unwrap();
        assert!(app.contains("\"AppID\"\t\"480\""));
        assert!(app.contains("\"Preview\"\t\"1\""));
        assert!(app.contains("\"SetLive\"\t\"beta\""));
        assert!(app.contains("\"481\"\t\"depot_481.vdf\""));
        // committed VDF must never carry machine paths
        assert!(!app.contains("ContentRoot"));
        assert!(!app.contains("BuildOutput"));

        let depot = std::fs::read_to_string(committed_dir(root, "dev").join("depot_481.vdf")).unwrap();
        assert!(depot.contains("\"DepotID\"\t\"481\""));
        assert!(depot.contains("\"LocalPath\"\t\"*\""));
    }

    #[test]
    fn write_committed_preserves_user_custom_depot_mapping() {
        let d = tempdir().unwrap();
        let root = d.path();
        let dir = committed_dir(root, "dev");
        std::fs::create_dir_all(&dir).unwrap();
        // Pre-existing hand-tuned depot with an exclusion + a non-default mapping.
        let custom = "\"DepotBuild\"\n{\n\t\"DepotID\"\t\"999\"\n\t\"FileMapping\"\n\t{\n\t\t\"LocalPath\"\t\"bin/*\"\n\t\t\"DepotPath\"\t\".\"\n\t\t\"Recursive\"\t\"1\"\n\t}\n\t\"FileExclusion\"\t\"*.pdb\"\n}\n";
        std::fs::write(dir.join("depot_481.vdf"), custom).unwrap();

        write_committed_vdf(root, &cfg_profile()).unwrap();
        let depot = std::fs::read_to_string(dir.join("depot_481.vdf")).unwrap();
        // DepotID is managed (corrected to the profile's id), but the custom mapping +
        // exclusion are preserved (no default map-all injected over them).
        assert!(depot.contains("\"DepotID\"\t\"481\""));
        assert!(depot.contains("\"LocalPath\"\t\"bin/*\""));
        assert!(depot.contains("\"FileExclusion\"\t\"*.pdb\""));
    }

    #[test]
    fn resolve_run_injects_content_root_and_build_output() {
        let d = tempdir().unwrap();
        let root = d.path();
        let p = cfg_profile();
        write_committed_vdf(root, &p).unwrap();

        let app_run = resolve_run_vdf(root, &p, "C:/Builds/out").unwrap();
        assert_eq!(app_run, run_app_build_vdf_path(root, "dev"));
        let text = std::fs::read_to_string(&app_run).unwrap();
        assert!(text.contains("\"ContentRoot\"\t\"C:/Builds/out\""));
        assert!(text.contains("BuildOutput"));
        assert!(text.contains("\"AppID\"\t\"480\""));
        // the depot VDF is copied beside the app VDF (bare-filename reference works)
        assert!(scratch_dir(root, "dev").join("depot_481.vdf").exists());
    }

    #[test]
    fn resolve_run_works_without_committed_files() {
        // Never-saved profile: resolve regenerates defensively from cfg.
        let d = tempdir().unwrap();
        let root = d.path();
        let p = cfg_profile();
        let app_run = resolve_run_vdf(root, &p, "C:/Builds/out").unwrap();
        let text = std::fs::read_to_string(&app_run).unwrap();
        assert!(text.contains("\"AppID\"\t\"480\""));
        assert!(text.contains("\"481\"\t\"depot_481.vdf\""));
    }

    #[test]
    fn resolve_run_errors_on_unparsable_committed_vdf() {
        let d = tempdir().unwrap();
        let root = d.path();
        let dir = committed_dir(root, "dev");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("app_build.vdf"), "\"AppBuild\" {{{ broken").unwrap();
        assert!(resolve_run_vdf(root, &cfg_profile(), "C:/out").is_err());
    }

    #[test]
    fn depot_path_becomes_local_path_glob() {
        let d = DepotItem { depot_id: "7".into(), path: "SubDir".into() };
        match default_file_mapping(&d) {
            Vdf::Obj(o) => assert_eq!(get(&o, "LocalPath"), Some(&Vdf::Str("SubDir/*".into()))),
            _ => panic!(),
        }
    }
}
