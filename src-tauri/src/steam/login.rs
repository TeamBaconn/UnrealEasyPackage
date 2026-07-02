//! Steam session check + login-output classifier.
//!
//! The Steam upload phase never asks for a password in the app. When a build reaches the
//! upload step the runner calls [`verify`] (`steamcmd +login <account> +quit`, no password)
//! to see whether steamcmd already has a cached session; if not, the runner opens steamcmd in
//! its own console so the user signs in there (`runner::exec`). This module only performs that
//! non-interactive check and classifies steamcmd's output. Those strings are version-dependent,
//! so classification is best-effort, verified manually against the installed steamcmd
//! (`docs/build-commands.md` §11).

use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

/// steamcmd's first run self-updates (can download ~hundreds of MB), so allow a generous
/// ceiling before we give up and kill it. Public so the runner's cancellable preflight
/// session check applies the same ceiling as the standalone [`verify`].
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SteamLoginStatus {
    /// Logged in; steamcmd cached the session.
    Success,
    /// Steam emailed a Steam Guard code - re-run with it.
    NeedGuardCode,
    /// Login failed (bad password, rate-limited, expired, etc.).
    Failed,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SteamLoginResult {
    pub status: SteamLoginStatus,
    /// A short human message (the tail of steamcmd's output) for the status line.
    pub message: String,
}

/// Verify steamcmd already has a **cached session** for `account`, without a password: runs
/// `+login <account> +quit` and classifies the result. Powers the "Check login" button, so
/// the user can authenticate outside the app (a terminal / mobile-app confirmation) and we
/// just confirm the session works. A cached session logs in silently (Success); no session
/// falls through to a password prompt it can't answer (stdin is null) → non-Success.
pub async fn verify(steamcmd_path: &str, account: &str) -> SteamLoginResult {
    let mut cmd = build_verify_command(steamcmd_path, account);
    let output = match tokio::time::timeout(LOGIN_TIMEOUT, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return SteamLoginResult {
                status: SteamLoginStatus::Failed,
                message: format!("could not launch steamcmd: {e}"),
            }
        }
        Err(_) => {
            return SteamLoginResult {
                status: SteamLoginStatus::Failed,
                message: "steamcmd timed out".to_string(),
            }
        }
    };
    classify_output(&output.stdout, &output.stderr)
}

/// Build the non-interactive **session-check** command: `steamcmd +login <account> +quit`
/// with no password (stdin null, stdout/stderr piped, no console window). Shared by the
/// standalone [`verify`] and the runner's cancellable preflight so both spawn steamcmd
/// identically; the runner adopts the returned child into its process group, races it against
/// Cancel, and classifies the captured output with [`classify_output`].
pub fn build_verify_command(steamcmd_path: &str, account: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(steamcmd_path);
    cmd.arg("+login").arg(account).arg("+quit");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Open steamcmd in its **own visible console** for an interactive sign-in (password / Steam
/// Guard code, or mobile-app confirmation) - used by the Setup modal's "Sign in" button.
/// Detached: the app never waits on or kills it (std's `Child` doesn't kill on drop), and
/// steamcmd caches the session on success. Pre-fills `+login <account>` when known; otherwise
/// opens steamcmd bare. Windows opens a new console; other platforms spawn it directly.
pub fn open_login_terminal(steamcmd_path: &str, account: &str) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(steamcmd_path);
    let account = account.trim();
    if !account.is_empty() {
        cmd.arg("+login").arg(account);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE - its own interactive window
    }
    cmd.spawn()?; // detached; drop the Child so we neither wait nor kill it
    Ok(())
}

/// Classify steamcmd's captured stdout+stderr into a login outcome. Shared by the standalone
/// [`verify`] and the runner's cancellable preflight (which spawns via [`build_verify_command`]
/// and hands the captured streams here). See [`classify`] for the fail-safe rule.
pub fn classify_output(stdout: &[u8], stderr: &[u8]) -> SteamLoginResult {
    let mut combined = String::from_utf8_lossy(stdout).into_owned();
    if !stderr.is_empty() {
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(stderr));
    }
    classify(&combined)
}

/// Classify steamcmd's combined output into a login outcome (best-effort; strings are
/// version-dependent - verified manually). **Fail-safe:** only a positive "signed in" marker
/// yields `Success`. steamcmd exits 0 even when no session was established, so a bare exit-0
/// with no marker is treated as *not signed in* (`Failed`) - the preflight then opens the
/// interactive console instead of letting a build run against a session that was never cached.
fn classify(output: &str) -> SteamLoginResult {
    let lower = output.to_lowercase();
    let message = tail(output);

    let status = if lower.contains("logged in ok") || lower.contains("waiting for user info...ok") {
        SteamLoginStatus::Success
    } else if lower.contains("steam guard")
        || lower.contains("account logon denied")
        || lower.contains("two-factor")
        || lower.contains("two factor")
        || lower.contains("set_steam_guard_code")
    {
        SteamLoginStatus::NeedGuardCode
    } else {
        // No positive sign-in marker (covers explicit "failed"/"invalid password"/"error"
        // AND the bare exit-0-with-no-marker case): do not assume a session exists.
        SteamLoginStatus::Failed
    };
    SteamLoginResult { status, message }
}

/// The last few non-empty output lines, for the status line.
fn tail(output: &str) -> String {
    let lines: Vec<&str> = output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(4);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_guard_prompt_as_need_code() {
        let out = "Logging in user 'bot' to Steam Public...\nFAILED (Account Logon Denied)\n";
        assert_eq!(classify(out).status, SteamLoginStatus::NeedGuardCode);
    }

    #[test]
    fn classifies_success() {
        let out = "Logging in user 'bot' to Steam Public...\nWaiting for user info...OK\n";
        assert_eq!(classify(out).status, SteamLoginStatus::Success);
    }

    #[test]
    fn classifies_bad_password_as_failed() {
        let out = "Logging in user 'bot'...\nFAILED (Invalid Password)\n";
        assert_eq!(classify(out).status, SteamLoginStatus::Failed);
    }

    #[test]
    fn exit_zero_without_marker_is_not_signed_in() {
        // steamcmd can exit 0 having done nothing useful; with no positive sign-in marker we
        // must NOT report success (else the preflight skips sign-in and the build later fails).
        let out = "Redirecting stderr to '...'\nLoading Steam API...OK\nSteam>quit\n";
        assert_ne!(classify(out).status, SteamLoginStatus::Success);
    }

    #[test]
    fn classify_output_combines_streams_and_succeeds_on_marker() {
        let r = classify_output(b"Logging in user 'bot'...\nWaiting for user info...OK\n", b"");
        assert_eq!(r.status, SteamLoginStatus::Success);
    }

    #[test]
    fn message_is_the_output_tail() {
        let out = "line1\n\nline2\nlast line\n";
        assert!(classify(out).message.contains("last line"));
    }
}
