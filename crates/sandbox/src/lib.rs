//! loopfleet sandbox: the `Sandbox` trait and the macOS `SeatbeltSandbox` impl
//! that renders a per-run `.sb` profile and wraps command construction.
//! Implemented in M2.
//!
//! This module ports the `ralph.sb` Seatbelt profile to a per-run template plus
//! a renderer. The macOS `sandbox-exec` profile is the single security boundary
//! for a headless run: writes are confined to the run's worktree, the per-run
//! progress dir, agent config/cache dirs, and temp dirs; reads and network stay
//! open. See PRD "Sandbox".

use std::fmt;
use std::path::{Path, PathBuf};

/// The `.sb` profile template. Rendered per run by [`render`], which fills the
/// `{{WRITE_SUBPATHS}}` marker with the run's writable subpaths.
const TEMPLATE: &str = include_str!("profile.sb.tmpl");

const WRITE_SUBPATHS_MARKER: &str = "{{WRITE_SUBPATHS}}";

/// The writable paths granted to one run's sandbox. Everything outside these
/// (and the fixed device nodes in the template) is read-only.
///
/// The parent repo's `.git` is deliberately absent: commits are app-owned, so
/// the agent never needs `.git` write. Do not add it — it is a real escape.
pub struct RenderParams {
    /// The run's git worktree. The agent edits code here.
    pub worktree: PathBuf,
    /// The per-run progress dir (outside the repo, keyed by run-id). The agent
    /// reads and writes its progress file here.
    pub progress_dir: PathBuf,
    /// Agent config/cache dirs the CLI needs to write (e.g. `~/.claude`).
    pub agent_dirs: Vec<PathBuf>,
    /// Temp dirs. Use [`default_temp_dirs`] for the standard macOS set.
    pub temp_dirs: Vec<PathBuf>,
}

impl RenderParams {
    /// A params set with the standard macOS temp dirs and no agent dirs yet.
    pub fn new(worktree: impl Into<PathBuf>, progress_dir: impl Into<PathBuf>) -> Self {
        RenderParams {
            worktree: worktree.into(),
            progress_dir: progress_dir.into(),
            agent_dirs: Vec::new(),
            temp_dirs: default_temp_dirs(),
        }
    }
}

/// The standard macOS temp dirs a toolchain writes to. `/tmp` and `/var` are
/// symlinks into `/private`, but Seatbelt matches on resolved paths, so grant
/// both the symlink and the resolved location.
pub fn default_temp_dirs() -> Vec<PathBuf> {
    ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// Why a profile could not be rendered.
#[derive(Debug)]
pub enum RenderError {
    /// A writable path was relative; Seatbelt `subpath` needs absolute paths.
    RelativePath(PathBuf),
    /// A writable path was not valid UTF-8; it cannot be written into the SBPL
    /// string literal.
    NonUtf8Path(PathBuf),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::RelativePath(p) => {
                write!(f, "sandbox write path must be absolute: {}", p.display())
            }
            RenderError::NonUtf8Path(p) => {
                write!(f, "sandbox write path is not valid UTF-8: {}", p.display())
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Render a per-run Seatbelt profile from `params`.
///
/// Writable subpaths are validated (absolute + UTF-8) and escaped before being
/// spliced into the template — the profile is the security boundary, so a path
/// containing `"` or `\` must not be able to break out of its string literal.
pub fn render(params: &RenderParams) -> Result<String, RenderError> {
    // worktree and progress dir are always granted; then agent dirs and temp.
    let paths = std::iter::once(&params.worktree)
        .chain(std::iter::once(&params.progress_dir))
        .chain(params.agent_dirs.iter())
        .chain(params.temp_dirs.iter());

    let mut subpaths = String::new();
    for p in paths {
        subpaths.push_str(&render_subpath(p)?);
        subpaths.push('\n');
    }

    Ok(TEMPLATE.replace(WRITE_SUBPATHS_MARKER, subpaths.trim_end()))
}

/// One `(subpath "…")` line for a validated, escaped path.
fn render_subpath(p: &Path) -> Result<String, RenderError> {
    if !p.is_absolute() {
        return Err(RenderError::RelativePath(p.to_path_buf()));
    }
    let s = p.to_str().ok_or_else(|| RenderError::NonUtf8Path(p.to_path_buf()))?;
    Ok(format!("    (subpath \"{}\")", escape_sbpl(s)))
}

/// Escape a string for an SBPL double-quoted literal: backslash first (so we
/// don't double-escape the escapes we add), then the quote.
fn escape_sbpl(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> RenderParams {
        let mut p = RenderParams::new("/repo/.worktrees/run-1", "/app/progress/run-1");
        p.agent_dirs.push(PathBuf::from("/home/u/.claude"));
        p.temp_dirs = vec![PathBuf::from("/tmp")];
        p
    }

    #[test]
    fn renders_all_writable_subpaths() {
        let profile = render(&params()).unwrap();
        assert!(profile.contains("(subpath \"/repo/.worktrees/run-1\")"));
        assert!(profile.contains("(subpath \"/app/progress/run-1\")"));
        assert!(profile.contains("(subpath \"/home/u/.claude\")"));
        assert!(profile.contains("(subpath \"/tmp\")"));
    }

    #[test]
    fn no_marker_remains_after_render() {
        let profile = render(&params()).unwrap();
        assert!(!profile.contains(WRITE_SUBPATHS_MARKER));
    }

    #[test]
    fn keeps_the_boundary_shape() {
        let profile = render(&params()).unwrap();
        // deny-by-default, open reads/network, and the write grant are the
        // load-bearing lines; assert they survive rendering.
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow file-read*)"));
        assert!(profile.contains("(allow network*)"));
        assert!(profile.contains("(allow file-write*"));
    }

    #[test]
    fn never_grants_git_write() {
        // The whole point: no parent-.git write grant. Commits are app-owned.
        // Check the generated (subpath …) grants, not the prose, which mentions
        // .git to explain why it is excluded.
        let profile = render(&params()).unwrap();
        for line in profile.lines() {
            if line.trim_start().starts_with("(subpath") {
                assert!(!line.contains(".git"), "unexpected .git write grant: {line}");
            }
        }
    }

    #[test]
    fn relative_path_is_rejected() {
        let mut p = params();
        p.agent_dirs.push(PathBuf::from("relative/dir"));
        match render(&p) {
            Err(RenderError::RelativePath(bad)) => {
                assert_eq!(bad, PathBuf::from("relative/dir"));
            }
            other => panic!("expected RelativePath, got {other:?}"),
        }
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        let mut p = RenderParams::new("/repo/a\"b\\c", "/app/progress/run-1");
        p.temp_dirs.clear();
        p.agent_dirs.clear();
        let profile = render(&p).unwrap();
        // The raw path must not appear unescaped (it would break the literal).
        assert!(profile.contains("(subpath \"/repo/a\\\"b\\\\c\")"));
        assert!(!profile.contains("/repo/a\"b\\c\")"));
    }

    #[test]
    fn default_temp_dirs_cover_macos_private_paths() {
        let dirs = default_temp_dirs();
        assert!(dirs.contains(&PathBuf::from("/private/var/folders")));
        assert!(dirs.contains(&PathBuf::from("/tmp")));
    }

    /// Validates the rendered profile actually PARSES under real `sandbox-exec`
    /// — the true test that the port is valid SBPL. Ignored by default (nested
    /// sandboxing / platform), runnable manually on macOS:
    ///   cargo test -p loopfleet-sandbox -- --ignored parses_under_sandbox_exec
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn parses_under_sandbox_exec() {
        use std::io::Write;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        let progress = dir.path().join("progress");
        std::fs::create_dir_all(&progress).unwrap();

        let mut p = RenderParams::new(&worktree, &progress);
        p.temp_dirs = default_temp_dirs();
        let profile = render(&p).unwrap();

        let profile_path = dir.path().join("run.sb");
        std::fs::File::create(&profile_path)
            .unwrap()
            .write_all(profile.as_bytes())
            .unwrap();

        // sandbox-exec runs /usr/bin/true under the profile. A malformed profile
        // fails to PARSE (before applying); a valid one parses, applies, and runs
        // true (exit 0). When this test itself runs inside another Seatbelt
        // sandbox, the profile still parses but `sandbox_apply` is denied
        // ("Operation not permitted") — an environmental limit, not a bad
        // profile — so tolerate that while still catching real parse errors.
        let out = Command::new("/usr/bin/sandbox-exec")
            .arg("-f")
            .arg(&profile_path)
            .arg("/usr/bin/true")
            .output()
            .expect("sandbox-exec should be present on macOS");

        let stderr = String::from_utf8_lossy(&out.stderr);
        let nested_apply_denied = stderr.contains("sandbox_apply");
        assert!(
            out.status.success() || nested_apply_denied,
            "profile failed to parse under sandbox-exec: {stderr}"
        );
    }
}
