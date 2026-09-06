//! Reaping agent processes a previous app instance left behind.
//!
//! A run's agent leads its own process group (see `adapters::base_command`), so
//! a clean stop SIGTERMs the whole group. A crash, a force-quit, or a `kill -9`
//! of the app never gets to send that signal, and the agent — plus every shell,
//! build, and test process it forked — keeps running against a worktree nobody
//! is watching any more. The run row is marked failed on the next start
//! (`fail_interrupted_runs`), but the processes survive, burn CPU, and hold the
//! worktree open so `sweep_worktrees` refuses to reap it.
//!
//! [`reap`] closes that gap at startup: everything still working inside the
//! app-managed worktrees directory is, by definition, from an instance that is
//! no longer around to own it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Name of the pidfile holding the pid of the instance that owns the worktrees
/// directory, written on every start.
const PIDFILE: &str = "instance.pid";

/// How long a stale process gets to exit on SIGTERM before it is SIGKILLed.
const GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Kill every process still running inside `<data_dir>/worktrees` and claim
/// ownership of that directory for this instance. Returns the pids signalled.
///
/// Skips reaping entirely when the pidfile names a live process: another
/// Loopfleet (a dev build alongside the packaged app, say) owns those worktrees
/// and its agents are not stale. Processes attached to a terminal are never
/// touched — those are shells and commands a user started themselves, the same
/// case `worktree_in_use` protects a worktree from being deleted for. Agents the
/// app spawned are detached from any tty, whether the app was launched from
/// Finder or from a dev terminal.
pub async fn reap(data_dir: &Path) -> Vec<u32> {
    let pidfile = data_dir.join(PIDFILE);
    if let Some(owner) = live_owner(&pidfile) {
        eprintln!("orphan reaping: skipped, instance {owner} still owns the worktrees");
        return Vec::new();
    }
    let _ = std::fs::write(&pidfile, std::process::id().to_string());

    // Canonicalized because lsof reports resolved paths (`/private/tmp/…` for
    // `/tmp/…`), which a prefix test against the unresolved dir would miss.
    let Ok(worktrees) = data_dir.join("worktrees").canonicalize() else {
        return Vec::new(); // no worktrees dir yet: nothing has ever run
    };

    let stale = stale_pids(&lsof_cwd(), &ps_snapshot(), &worktrees, std::process::id());
    if stale.is_empty() {
        return stale;
    }
    eprintln!(
        "orphan reaping: signalling {} stale process(es) from a previous instance",
        stale.len()
    );

    for pid in &stale {
        signal(*pid, libc::SIGTERM);
    }
    tokio::time::sleep(GRACE).await;
    for pid in &stale {
        if alive(*pid) {
            signal(*pid, libc::SIGKILL);
        }
    }
    stale
}

/// The pid in `pidfile`, if it names a process that is still alive. A pid that
/// has since been recycled by an unrelated process reads as live, which errs
/// toward leaving stale processes alone rather than killing a stranger's.
fn live_owner(pidfile: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()?;
    (pid != std::process::id() && alive(pid)).then_some(pid)
}

/// Whether `pid` names a live process. Signal `0` delivers nothing and only
/// reports whether the process could be signalled, so a process we are not
/// allowed to signal (`EPERM` — a root-owned pid) still counts as alive; only
/// `ESRCH` means gone.
fn alive(pid: u32) -> bool {
    signal(pid, 0) || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Send `sig` to `pid`, reporting whether it landed. Signal `0` sends nothing
/// and just tests that the process exists.
fn signal(pid: u32, sig: libc::c_int) -> bool {
    // SAFETY: `kill` is a thin libc syscall wrapper with no memory effects.
    unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
}

/// Every process's working directory, as `lsof -F` field output. `-d cwd` keeps
/// this to one descriptor per process instead of walking every open file.
fn lsof_cwd() -> String {
    std::process::Command::new("lsof")
        .args(["-w", "-n", "-P", "-d", "cwd", "-F", "pn"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Every process as `pid ppid tty`, used to skip terminal-attached processes
/// and this app's own descendants.
fn ps_snapshot() -> String {
    std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,tty="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Pids working inside `worktrees` that belong to no live owner: not attached to
/// a terminal, and not `self_pid` or one of its descendants (the git actor runs
/// `git` with a worktree as its cwd).
fn stale_pids(lsof_output: &str, ps_output: &str, worktrees: &Path, self_pid: u32) -> Vec<u32> {
    let procs = parse_ps(ps_output);
    let ours = descendants(&procs, self_pid);

    let mut stale = Vec::new();
    for (pid, cwd) in parse_lsof(lsof_output) {
        if !cwd.starts_with(worktrees) || ours.contains(&pid) {
            continue;
        }
        match procs.get(&pid) {
            Some(proc) if proc.tty => continue,
            // A process lsof saw but ps did not (it exited in between) is not
            // worth signalling.
            None => continue,
            Some(_) => stale.push(pid),
        }
    }
    stale
}

/// One row of [`ps_snapshot`].
struct ProcInfo {
    ppid: u32,
    /// Whether the process has a controlling terminal (`ps` prints `??` when
    /// it has none).
    tty: bool,
}

fn parse_ps(output: &str) -> HashMap<u32, ProcInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let tty = fields.next()? != "??";
            Some((pid, ProcInfo { ppid, tty }))
        })
        .collect()
}

/// `pid` and every process below it, so this instance can never signal itself
/// or a child it just spawned.
fn descendants(procs: &HashMap<u32, ProcInfo>, pid: u32) -> HashSet<u32> {
    let mut ours = HashSet::from([pid]);
    // A pid's ancestors are always numerically unrelated, so walk up from each
    // process rather than trying to build the tree downward.
    for &candidate in procs.keys() {
        let mut walk = candidate;
        let mut hops = 0;
        while let Some(proc) = procs.get(&walk) {
            if ours.contains(&walk) {
                ours.insert(candidate);
                break;
            }
            // Guard against a ppid cycle in a torn snapshot.
            hops += 1;
            if walk <= 1 || hops > 64 {
                break;
            }
            walk = proc.ppid;
        }
    }
    ours
}

/// `(pid, cwd)` pairs from `lsof -F pn` output: a `p<pid>` line opens a process
/// section and the `n<path>` line that follows names its cwd.
fn parse_lsof(output: &str) -> Vec<(u32, PathBuf)> {
    let mut pairs = Vec::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.parse::<u32>().ok();
        } else if let Some(rest) = line.strip_prefix('n') {
            if let Some(pid) = pid {
                pairs.push((pid, PathBuf::from(rest)));
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = "\
  1     0 ??
100     1 ??
200   100 ??
300     1 s001
400     1 ??
";

    fn lsof(entries: &[(u32, &str)]) -> String {
        entries
            .iter()
            .map(|(pid, cwd)| format!("p{pid}\nfcwd\nn{cwd}\n"))
            .collect()
    }

    /// An agent left behind in a worktree, with no terminal and no live parent,
    /// is exactly what startup is looking for.
    #[test]
    fn stale_agent_in_worktree_is_reaped() {
        let out = lsof(&[(400, "/data/worktrees/abc")]);
        assert_eq!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100), [400]);
    }

    /// Work outside the app-managed worktrees is none of our business.
    #[test]
    fn process_outside_worktrees_is_left_alone() {
        let out = lsof(&[(400, "/Users/me/dev/loopfleet")]);
        assert!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100).is_empty());
    }

    /// A sibling directory that merely shares a prefix is not inside it.
    #[test]
    fn prefix_match_respects_path_boundaries() {
        let out = lsof(&[(400, "/data/worktrees-backup/abc")]);
        assert!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100).is_empty());
    }

    /// A shell the user opened in a worktree has a controlling terminal; killing
    /// it would close a prompt they are looking at.
    #[test]
    fn terminal_attached_process_is_left_alone() {
        let out = lsof(&[(300, "/data/worktrees/abc")]);
        assert!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100).is_empty());
    }

    /// The running app and its children (the git actor works in worktrees) must
    /// never be signalled, however deep.
    #[test]
    fn own_process_tree_is_never_reaped() {
        let out = lsof(&[(100, "/data/worktrees/abc"), (200, "/data/worktrees/abc")]);
        assert!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100).is_empty());
    }

    /// lsof and ps are two snapshots; a pid only the first one saw has already
    /// exited.
    #[test]
    fn process_missing_from_ps_is_skipped() {
        let out = lsof(&[(999, "/data/worktrees/abc")]);
        assert!(stale_pids(&out, PS, Path::new("/data/worktrees"), 100).is_empty());
    }

    /// Field output repeats the pid only when it changes, so each cwd belongs to
    /// the section it appears under.
    #[test]
    fn lsof_fields_attach_each_cwd_to_its_process() {
        let out = "p400\nfcwd\nn/data/worktrees/a\np401\nfcwd\nn/tmp\n";
        assert_eq!(
            parse_lsof(out),
            [
                (400, PathBuf::from("/data/worktrees/a")),
                (401, PathBuf::from("/tmp")),
            ]
        );
    }

    /// A pidfile naming a live process means another instance owns the
    /// worktrees; one naming a dead process is just leftover.
    #[test]
    fn owner_is_live_only_while_its_process_is() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join(PIDFILE);

        // pid 1 is launchd: alive, and not ours to signal.
        std::fs::write(&pidfile, "1").unwrap();
        assert_eq!(live_owner(&pidfile), Some(1));

        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        std::fs::write(&pidfile, child.id().to_string()).unwrap();
        assert_eq!(live_owner(&pidfile), None);

        // Our own pid is not a competing owner: this is our pidfile from a
        // previous run of the same process in tests, and in production it means
        // reap already claimed the directory.
        std::fs::write(&pidfile, std::process::id().to_string()).unwrap();
        assert_eq!(live_owner(&pidfile), None);

        assert_eq!(live_owner(&dir.path().join("missing.pid")), None);
    }
}
