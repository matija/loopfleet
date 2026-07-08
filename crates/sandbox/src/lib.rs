//! loopfleet sandbox: the `Sandbox` trait and the macOS `SeatbeltSandbox` impl
//! that renders a per-run `.sb` profile and wraps command construction.
//! Implemented in M2.
//!
//! This module ports the `ralph.sb` Seatbelt profile to a per-run template plus
//! a renderer. The macOS `sandbox-exec` profile is the single security boundary
//! for a headless run: writes are confined to the run's worktree, the per-run
//! progress dir, agent config/cache dirs, and temp dirs; reads and network stay
//! open. See PRD "Sandbox".

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

/// The macOS Seatbelt driver. `sandbox-exec -f <profile> <program> <args…>`
/// runs `program` confined by the SBPL profile.
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

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
    /// Per-project write overrides (PRD M6 settings). Additional absolute paths
    /// the run may write. Never include the parent repo's `.git`.
    pub extra_writes: Vec<PathBuf>,
}

impl RenderParams {
    /// A params set with the standard macOS temp dirs and no agent dirs yet.
    pub fn new(worktree: impl Into<PathBuf>, progress_dir: impl Into<PathBuf>) -> Self {
        RenderParams {
            worktree: worktree.into(),
            progress_dir: progress_dir.into(),
            agent_dirs: Vec::new(),
            temp_dirs: default_temp_dirs(),
            extra_writes: Vec::new(),
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
        .chain(params.temp_dirs.iter())
        .chain(params.extra_writes.iter());

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

/// Render `params`, persist the profile to `profile_path`, and return the opaque
/// argv prefix that runs a program confined: `[sandbox-exec, -f, <profile_path>]`.
///
/// This is the adapter-facing counterpart to [`Sandbox::wrap`]. An
/// [`AgentAdapter`](loopfleet_core) owns its own `program args…` and must stay
/// ignorant of the backend, so rather than handing the sandbox a whole command
/// it receives this prefix (via `RunSpec::wrapper`) and prepends it. The Seatbelt
/// argv shape stays inside this crate — the adapter never learns it is
/// `sandbox-exec`, keeping the boundary detail from leaking upward (PRD:
/// Sandbox).
pub fn confine_prefix(
    params: &RenderParams,
    profile_path: &Path,
) -> Result<Vec<OsString>, SandboxError> {
    persist_profile(params, profile_path)?;
    Ok(vec![
        OsString::from(SANDBOX_EXEC),
        OsString::from("-f"),
        profile_path.to_path_buf().into_os_string(),
    ])
}

/// Render the profile and write it to `profile_path` (creating its parent dir).
/// Shared by [`confine_prefix`] and [`SeatbeltSandbox::wrap`].
fn persist_profile(params: &RenderParams, profile_path: &Path) -> Result<(), SandboxError> {
    let profile = render(params)?;
    if let Some(parent) = profile_path.parent() {
        std::fs::create_dir_all(parent).map_err(SandboxError::Io)?;
    }
    std::fs::write(profile_path, profile).map_err(SandboxError::Io)?;
    Ok(())
}

/// Confines a child process to a per-run write boundary.
///
/// Kept behind a trait so non-macOS backends (Landlock/bubblewrap, containers)
/// can slot in later without leaking Seatbelt specifics upward — callers spawn
/// the returned [`WrappedCommand`] opaquely. See PRD "Sandbox".
pub trait Sandbox {
    /// Wrap `command` so it runs confined. Renders the run's profile, persists
    /// it to `command.profile_path`, and returns the argv to spawn.
    fn wrap(&self, command: &SandboxCommand) -> Result<WrappedCommand, SandboxError>;
}

/// The agent command to confine, its write boundary, and where to persist the
/// rendered profile (app-owned, keyed by run-id — set by the supervisor).
pub struct SandboxCommand {
    /// Program to run confined (e.g. `claude`).
    pub program: OsString,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The run's writable boundary.
    pub params: RenderParams,
    /// Where to write the rendered `.sb` profile. Also surfaced back on the
    /// [`WrappedCommand`] for the run UI's profile panel.
    pub profile_path: PathBuf,
}

/// A confined command: an opaque program + argv the caller spawns as-is. The
/// caller does not need to know it runs under `sandbox-exec`.
#[derive(Debug)]
pub struct WrappedCommand {
    program: OsString,
    args: Vec<OsString>,
    /// The rendered profile on disk — surfaced so the run UI can show the
    /// active boundary (trust is a feature, not a footnote).
    pub profile_path: PathBuf,
}

impl WrappedCommand {
    /// The program to spawn.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// The arguments to spawn it with.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }
}

/// Why a command could not be confined.
#[derive(Debug)]
pub enum SandboxError {
    /// The profile could not be rendered (bad write path).
    Render(RenderError),
    /// The rendered profile could not be persisted to disk.
    Io(std::io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::Render(e) => write!(f, "{e}"),
            SandboxError::Io(e) => write!(f, "writing sandbox profile: {e}"),
        }
    }
}

impl std::error::Error for SandboxError {}

impl From<RenderError> for SandboxError {
    fn from(e: RenderError) -> Self {
        SandboxError::Render(e)
    }
}

/// The macOS `sandbox-exec` (Seatbelt) sandbox. Stateless: each [`wrap`] renders
/// and writes a fresh profile for the run it confines.
///
/// [`wrap`]: Sandbox::wrap
pub struct SeatbeltSandbox;

impl Sandbox for SeatbeltSandbox {
    fn wrap(&self, command: &SandboxCommand) -> Result<WrappedCommand, SandboxError> {
        persist_profile(&command.params, &command.profile_path)?;

        // sandbox-exec -f <profile> <program> <args…>
        let mut args = Vec::with_capacity(command.args.len() + 3);
        args.push(OsString::from("-f"));
        args.push(command.profile_path.clone().into_os_string());
        args.push(command.program.clone());
        args.extend(command.args.iter().cloned());

        Ok(WrappedCommand {
            program: OsString::from(SANDBOX_EXEC),
            args,
            profile_path: command.profile_path.clone(),
        })
    }
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
    fn renders_per_project_extra_writes() {
        let mut p = params();
        p.extra_writes.push(PathBuf::from("/opt/shared-cache"));
        let profile = render(&p).unwrap();
        assert!(profile.contains("(subpath \"/opt/shared-cache\")"));
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

    fn sandbox_command(dir: &Path) -> SandboxCommand {
        let worktree = dir.join("wt");
        let progress = dir.join("progress");
        SandboxCommand {
            program: OsString::from("claude"),
            args: vec![OsString::from("-p"), OsString::from("do the thing")],
            params: RenderParams::new(&worktree, &progress),
            profile_path: dir.join("run.sb"),
        }
    }

    #[test]
    fn wrap_builds_sandbox_exec_argv_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = sandbox_command(dir.path());
        let profile_path = cmd.profile_path.clone();

        let wrapped = SeatbeltSandbox.wrap(&cmd).unwrap();

        assert_eq!(wrapped.program(), OsStr::new(SANDBOX_EXEC));
        let expected: Vec<OsString> = vec![
            OsString::from("-f"),
            profile_path.clone().into_os_string(),
            OsString::from("claude"),
            OsString::from("-p"),
            OsString::from("do the thing"),
        ];
        assert_eq!(wrapped.args(), expected.as_slice());
        assert_eq!(wrapped.profile_path, profile_path);
    }

    #[test]
    fn wrap_writes_the_rendered_profile_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = sandbox_command(dir.path());
        let worktree = cmd.params.worktree.to_str().unwrap().to_string();

        let wrapped = SeatbeltSandbox.wrap(&cmd).unwrap();

        let written = std::fs::read_to_string(&wrapped.profile_path).unwrap();
        assert!(written.contains("(deny default)"));
        assert!(written.contains(&format!("(subpath \"{worktree}\")")));
    }

    #[test]
    fn wrap_creates_the_profile_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = sandbox_command(dir.path());
        cmd.profile_path = dir.path().join("nested/deeper/run.sb");

        let wrapped = SeatbeltSandbox.wrap(&cmd).unwrap();

        assert!(wrapped.profile_path.exists());
    }

    #[test]
    fn confine_prefix_writes_profile_and_returns_argv_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        let progress = dir.path().join("progress");
        let params = RenderParams::new(&worktree, &progress);
        let profile_path = dir.path().join("nested/run.sb");

        let prefix = confine_prefix(&params, &profile_path).unwrap();

        assert_eq!(
            prefix,
            vec![
                OsString::from(SANDBOX_EXEC),
                OsString::from("-f"),
                profile_path.clone().into_os_string(),
            ]
        );
        // The profile was rendered and persisted (parent dir created).
        let written = std::fs::read_to_string(&profile_path).unwrap();
        assert!(written.contains("(deny default)"));
        assert!(written.contains(&format!("(subpath \"{}\")", worktree.to_str().unwrap())));
    }

    #[test]
    fn confine_prefix_propagates_render_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut params = RenderParams::new(dir.path().join("wt"), dir.path().join("progress"));
        params.agent_dirs.push(PathBuf::from("relative/dir"));

        match confine_prefix(&params, &dir.path().join("run.sb")) {
            Err(SandboxError::Render(RenderError::RelativePath(bad))) => {
                assert_eq!(bad, PathBuf::from("relative/dir"));
            }
            other => panic!("expected Render(RelativePath), got {other:?}"),
        }
    }

    #[test]
    fn wrap_propagates_render_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = sandbox_command(dir.path());
        cmd.params.agent_dirs.push(PathBuf::from("relative/dir"));

        match SeatbeltSandbox.wrap(&cmd) {
            Err(SandboxError::Render(RenderError::RelativePath(bad))) => {
                assert_eq!(bad, PathBuf::from("relative/dir"));
            }
            other => panic!("expected Render(RelativePath), got {other:?}"),
        }
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

    /// End-to-end: `wrap` a real command and spawn the returned argv, proving
    /// the `sandbox-exec -f <profile> <program> <args…>` shape is spawnable.
    /// Ignored (nested sandboxing / platform), runnable manually on macOS:
    ///   cargo test -p loopfleet-sandbox -- --ignored wrapped_command_spawns
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn wrapped_command_spawns() {
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        let progress = dir.path().join("progress");
        std::fs::create_dir_all(&progress).unwrap();

        let cmd = SandboxCommand {
            program: OsString::from("/usr/bin/true"),
            args: vec![],
            params: RenderParams::new(&worktree, &progress),
            profile_path: dir.path().join("run.sb"),
        };
        let wrapped = SeatbeltSandbox.wrap(&cmd).unwrap();

        let out = Command::new(wrapped.program())
            .args(wrapped.args())
            .output()
            .expect("wrapped command should spawn");

        // Tolerate nested sandbox_apply denial (test runs inside a sandbox),
        // same as parses_under_sandbox_exec.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() || stderr.contains("sandbox_apply"),
            "wrapped command failed: {stderr}"
        );
    }
}
