//! Pure severity classifier for streamed build-log lines (`docs/requirement.md`
//! R4).
//!
//! Tags each line **Error / Warning / Info by parsing the text, not color**:
//!
//! 1. anchor on the **first** Unreal `Category: Verbosity:` token, so an `Error`
//!    word *inside* a `Warning:` message isn't misread (leftmost match wins);
//! 2. else a bare line-leading `Error:` / `Warning:` (UAT prefixes + the
//!    cooker's category-less lines), after an optional `HH:MM:SS` stamp;
//! 3. else compiler diagnostics - MSVC `error C2065:` / `warning C4002:` /
//!    linker `LNK2019`, and clang/gcc `: error:` / `: warning:`.
//!
//! **Summary tally lines** ("3 Error(s), 5 Warning(s)", "Warning/Error Summary")
//! are ignored so the warning/error filter counts don't double-count. Pure +
//! unit-tested; the executor calls it on every streamed line. The final
//! success/failed status comes from the **process exit code**, never from these
//! tags (R4).

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Per-line severity. `lowercase` on the wire to match the other camelCase/lower
/// enums the editor consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// `Category: Verbosity:` - the canonical Unreal log shape. Leftmost match wins,
/// which is exactly the "first verbosity token" anchor R4 asks for.
fn re_unreal() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)\b[a-z_][a-z0-9_]*\s*:\s*(fatal|error|warning|display|verbose|veryverbose|log)\s*:")
            .unwrap()
    })
}

/// Bare leading `Error:` / `Warning:` after an optional `HH:MM:SS(.ms)` stamp and
/// optional `[..]` bracket - covers UAT `ERROR:`/`WARNING:` and the cooker's
/// category-less diagnostics.
fn re_leading() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:\d{1,2}:\d{2}:\d{2}(?:[.,]\d+)?\s+)?(?:\[[^\]]*\]\s*)?(error|warning)\s*:")
            .unwrap()
    })
}

/// MSVC/linker/clang error diagnostics anywhere in the line.
fn re_compiler_error() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)(\berror\s+c\d{2,5}\b|\b(?:error\s+)?lnk\d{3,5}\b|:\s*error\s*:)").unwrap())
}

/// MSVC/clang warning diagnostics anywhere in the line.
fn re_compiler_warn() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)(\bwarning\s+c\d{2,5}\b|:\s*warning\s*:)").unwrap())
}

/// Count-tally / summary lines that must not be (re)classified as warn/error.
fn re_summary() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)(warning/error summary|\b\d+\s+error\(s\)|\b\d+\s+warning\(s\))").unwrap())
}

fn verbosity_severity(v: &str) -> Severity {
    match v.to_ascii_lowercase().as_str() {
        "fatal" | "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => Severity::Info, // display / log / verbose / veryverbose
    }
}

/// Classify one raw log line. Order encodes the R4 precedence: skip summaries,
/// then the first Unreal verbosity token, then a bare leading prefix, then
/// compiler diagnostics; default Info.
pub fn classify_line(line: &str) -> Severity {
    let l = line.trim_end();
    if l.trim().is_empty() || re_summary().is_match(l) {
        return Severity::Info;
    }
    if let Some(c) = re_unreal().captures(l) {
        return verbosity_severity(&c[1]);
    }
    if let Some(c) = re_leading().captures(l) {
        return verbosity_severity(&c[1]);
    }
    if re_compiler_error().is_match(l) {
        return Severity::Error;
    }
    if re_compiler_warn().is_match(l) {
        return Severity::Warning;
    }
    Severity::Info
}

#[cfg(test)]
mod tests {
    use super::*;
    use Severity::*;

    fn c(s: &str) -> Severity {
        classify_line(s)
    }

    #[test]
    fn unreal_category_verbosity_anchors_on_first_token() {
        assert_eq!(c("LogCook: Warning: Texture T_Sky has no compression"), Warning);
        assert_eq!(c("LogMaterial: Error: MI_Broken failed to compile"), Error);
        assert_eq!(c("LogCook: Display: Cooking /Game/Maps/L_Arena_01"), Info);
        // the key case: an "Error" word inside a Warning line must stay Warning
        assert_eq!(c("LogCook: Warning: shader had an Error earlier, recovered"), Warning);
        // Display verbosity wins over a later "error" word (first token anchor)
        assert_eq!(c("LogShader: Display: 0 error budget remaining"), Info);
        assert_eq!(c("LogInit: Fatal: assertion failed"), Error);
    }

    #[test]
    fn bare_leading_prefix_with_timestamp() {
        assert_eq!(c("14:41:36  Warning: Asset references a missing material slot"), Warning);
        assert_eq!(c("14:45:22  Error: MaterialInstance MI_Broken failed (1 shader error)"), Error);
        assert_eq!(c("ERROR: AutomationTool exiting with code 1"), Error);
        assert_eq!(c("WARNING: stale cooked data detected"), Warning);
    }

    #[test]
    fn compiler_and_linker_diagnostics() {
        assert_eq!(c("SampleProject.cpp(42): error C2065: 'Foo': undeclared identifier"), Error);
        assert_eq!(c("Header.h(8): warning C4996: 'x' was deprecated"), Warning);
        assert_eq!(c("Module.SampleProject.cpp.obj : error LNK2019: unresolved external symbol"), Error);
        assert_eq!(c("main.cpp:12:5: error: expected ';'"), Error); // clang/gcc form
    }

    #[test]
    fn plain_and_summary_lines_are_info() {
        assert_eq!(c("14:35:04  [   1/842] Compile Module.CoreUObject.cpp"), Info);
        assert_eq!(c("14:47:13  LogDerivedDataCache: 62% - building shaders (8 workers)"), Info);
        assert_eq!(c("RunUAT BuildCookRun -project=SampleProject.uproject"), Info);
        // summary tallies must not re-count
        assert_eq!(c("Warning/Error Summary (Unique only)"), Info);
        assert_eq!(c("   3 error(s), 5 warning(s)"), Info);
        assert_eq!(c(""), Info);
    }

    #[test]
    fn windows_paths_do_not_false_trigger() {
        // "C:\..." must not read as a category:verbosity token
        assert_eq!(c("Copying C:\\Builds\\out to staging"), Info);
        assert_eq!(c("14:35:02  -archivedirectory=C:\\Builds\\sampleproject"), Info);
    }
}
