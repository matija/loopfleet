//! PATH repair for the packaged (GUI-launched) app.
//!
//! `npm run tauri dev` starts the app from a terminal, so it inherits the
//! shell's `PATH` and finds the agent CLIs. A `.app` launched from Finder or
//! the Dock inherits launchd's minimal `PATH` instead
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), so `claude`, `pi` and `cursor-agent` —
//! which live in Homebrew, `~/.local/bin`, `~/.bun/bin`, a node version
//! manager's shim dir, … — are invisible and discovery reports them as
//! "not found on PATH".
//!
//! [`repair`] rebuilds the process `PATH` once at startup from the user's login
//! shell plus a set of well-known install dirs. Every later spawn (discovery
//! and the agent runs themselves) inherits it.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Markers around the PATH the login shell prints, so rc-file chatter (banners,
/// version managers, `zsh` job notices) can't be mistaken for the value.
const BEGIN: &str = "__LOOPFLEET_PATH__";
const END: &str = "__LOOPFLEET_END__";

/// How long the login shell gets to print its `PATH` before we give up on it
/// and fall back to the well-known dirs alone. A slow rc file must not stall
/// app startup.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// Rebuild `PATH` as: the inherited `PATH`, then the login shell's `PATH`, then
/// the well-known install dirs — de-duplicated, order preserved. Purely
/// additive, so a `PATH` that was already correct (dev runs) is unchanged apart
/// from the appended fallbacks.
///
/// Call once from `run()` before any threads or child processes exist:
/// `set_var` mutates process-global state.
pub fn repair() {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |dir: PathBuf| {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };

    for dir in std::env::var_os("PATH").iter().flat_map(std::env::split_paths) {
        push(dir);
    }
    for dir in login_shell_path().iter().flat_map(std::env::split_paths) {
        push(dir);
    }
    for dir in well_known_dirs() {
        push(dir);
    }

    if let Ok(joined) = std::env::join_paths(&dirs) {
        std::env::set_var("PATH", joined);
    }
}

/// Ask the user's login shell for its `PATH`. Interactive + login (`-ilc`) so
/// both `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc` are sourced — version
/// managers commonly extend `PATH` from only one of the two.
///
/// Returns `None` when `$SHELL` is unset, the shell fails, or it doesn't answer
/// within [`SHELL_TIMEOUT`]; the well-known dirs then carry the lookup alone.
fn login_shell_path() -> Option<std::ffi::OsString> {
    let shell = std::env::var_os("SHELL")?;
    let script = format!(r#"printf '{BEGIN}%s{END}' "$PATH""#);
    let mut child = Command::new(&shell)
        .args(["-ilc", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // No timeout on `wait`, so poll and kill a shell whose rc files hang.
    let deadline = Instant::now() + SHELL_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }

    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find(BEGIN)? + BEGIN.len();
    let end = text[start..].find(END)? + start;
    let path = text[start..end].trim();
    (!path.is_empty()).then(|| path.into())
}

/// Dirs the v1 agent CLIs are commonly installed into, appended as a fallback
/// for when the login shell can't be asked (or exports `PATH` from a file we
/// don't source). Nonexistent dirs are filtered out so `PATH` stays honest.
fn well_known_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".local/bin",
            "bin",
            ".bun/bin",
            ".cargo/bin",
            ".deno/bin",
            ".volta/bin",
            ".npm-global/bin",
            ".yarn/bin",
            ".local/share/pnpm",
            ".claude/local",
        ] {
            dirs.push(home.join(rel));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repaired PATH keeps every entry it started with — repair is additive,
    /// so a correct dev PATH is never narrowed.
    #[test]
    fn repair_preserves_existing_entries() {
        let before: Vec<PathBuf> = std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths)
            .collect();
        repair();
        let after: Vec<PathBuf> = std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths)
            .collect();
        for dir in before {
            assert!(after.contains(&dir), "repair dropped {dir:?}");
        }
    }

    /// A shell that prints noise around the markers still yields just the PATH.
    #[test]
    fn marker_extraction_ignores_rc_chatter() {
        let text = format!("welcome to your shell\n{BEGIN}/a:/b{END}\n");
        let start = text.find(BEGIN).unwrap() + BEGIN.len();
        let end = text[start..].find(END).unwrap() + start;
        assert_eq!(&text[start..end], "/a:/b");
    }
}
