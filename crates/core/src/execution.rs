use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;

pub const RUNNER_CONTRACT: &str = "Loopfleet execution contract:\nLoopfleet owns all Git mutations, snapshots, commits, and merges. Do not stage, commit, push, merge, or write Git metadata. Repository or personal instructions to commit are fulfilled by Loopfleet, not by this agent. A Git write denial is not a task blocker. Keep useful worktree changes for the runner. Never bypass the sandbox, start a host VM, or stop the hosting Loopfleet application. Implement only the selected task. The full plan is reference context, not permission to implement other tasks. Do not weaken requirements or report skipped or failed checks as passing.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub preflight: Vec<Check>,
    #[serde(default)]
    pub verify: Vec<Check>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            version: 1,
            preflight: Vec::new(),
            verify: Vec::new(),
        }
    }
}

fn version() -> u32 {
    1
}

fn timeout_seconds() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default = "timeout_seconds")]
    pub timeout_seconds: u64,
}

impl Check {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.command.is_empty()
            || self.command[0].trim().is_empty()
            || self.command.iter().any(|s| s.contains('\0'))
            || !(1..=3600).contains(&self.timeout_seconds)
        {
            return Err("Each check needs a name, a nonempty command argv, and a timeout_seconds from 1 to 3600".into());
        }
        Ok(())
    }
}

impl ExecutionConfig {
    pub fn load(repo: &Path) -> Result<Self, String> {
        let path = repo.join(".loopfleet.json");
        let text = match crate::safe_file::read_bounded(repo, &path, 64 * 1024) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
        };
        let config: Self = serde_json::from_str(&text)
            .map_err(|error| format!("Invalid .loopfleet.json: {error}"))?;
        if config.version != 1 || config.preflight.len() + config.verify.len() > 32 {
            return Err(".loopfleet.json requires version 1 and at most 32 checks".into());
        }
        for check in config.preflight.iter().chain(&config.verify) {
            check.validate()?;
        }
        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AgentResult {
    Ready { summary: String, checks: Vec<Check> },
    Blocked { reason: String },
}

pub fn write_record(progress: &Path, name: &str, text: &str) -> std::io::Result<()> {
    let root = progress
        .parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new("/"));
    let path = progress.with_file_name(name);
    crate::safe_file::replace(root, &path, None, text)
}

pub fn append_progress(progress: &Path, text: &str) -> std::io::Result<()> {
    let root = progress
        .parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new("/"));
    let original = match crate::safe_file::read_bounded(root, progress, 256 * 1024) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let proposed = format!("{}\n{text}\n", original.as_deref().unwrap_or_default());
    crate::safe_file::replace(root, progress, original.as_deref(), &proposed)
}

pub fn result_path(progress: &Path, pass: u32) -> PathBuf {
    progress.with_file_name(format!("result-{pass}.json"))
}

pub fn read_result(progress: &Path, pass: u32) -> Result<AgentResult, String> {
    let path = result_path(progress, pass);
    let text = crate::safe_file::read_bounded(
        progress.parent().unwrap_or(Path::new("/")),
        &path,
        64 * 1024,
    )
    .map_err(|error| {
        format!(
            "Missing or unreadable task result {}: {error}",
            path.display()
        )
    })?;
    let result: AgentResult =
        serde_json::from_str(&text).map_err(|error| format!("Invalid task result: {error}"))?;
    match &result {
        AgentResult::Ready { summary, checks } => {
            if summary.trim().is_empty()
                || summary.contains(['\n', '\r'])
                || summary.chars().count() >= 72
                || checks.len() > 32
            {
                return Err("Ready results require a one-line summary under 72 characters and at most 32 checks".into());
            }
            for check in checks {
                check.validate()?;
            }
        }
        AgentResult::Blocked { reason } if reason.trim().is_empty() => {
            return Err("Blocked results require a reason and recovery action".into());
        }
        _ => {}
    }
    Ok(result)
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub command: Vec<String>,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

struct ProcessGroup(u32);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::killpg(self.0 as libc::pid_t, libc::SIGKILL);
        }
    }
}

async fn bounded_output(mut stream: impl AsyncRead + Unpin) -> String {
    let mut kept = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(n) = stream.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        let remaining = (32 * 1024usize).saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..n.min(remaining)]);
    }
    String::from_utf8_lossy(&kept).into_owned()
}

pub async fn run_check(
    check: &Check,
    cwd: &Path,
    wrapper: &[OsString],
    cancel: &mut watch::Receiver<bool>,
) -> CheckResult {
    let mut result = CheckResult {
        name: check.name.clone(),
        command: check.command.clone(),
        passed: false,
        exit_code: None,
        output: String::new(),
    };
    if let Err(error) = check.validate() {
        result.output = error;
        return result;
    }
    if *cancel.borrow() || cancel.has_changed().is_err() {
        result.output = "Stopped".into();
        return result;
    }
    let mut command = match wrapper.split_first() {
        Some((program, args)) => {
            let mut command = Command::new(program);
            command.args(args).arg(&check.command[0]);
            command
        }
        None => Command::new(&check.command[0]),
    };
    if !wrapper.is_empty() {
        if let Some(home) = std::env::var_os("HOME") {
            command.env(
                "CARGO_HOME",
                PathBuf::from(home).join(".cache/loopfleet/cargo"),
            );
        }
    }
    command
        .args(&check.command[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            result.output = error.to_string();
            return result;
        }
    };
    let group = ProcessGroup(child.id().unwrap());
    let stdout = bounded_output(child.stdout.take().unwrap());
    let stderr = bounded_output(child.stderr.take().unwrap());
    let outcome = tokio::select! {
        outcome = tokio::time::timeout(Duration::from_secs(check.timeout_seconds), async {
            tokio::join!(child.wait(), stdout, stderr)
        }) => Some(outcome),
        _ = cancel.changed() => None,
    };
    drop(group);
    match outcome {
        Some(Ok((Ok(status), stdout, stderr))) => {
            result.passed = status.success();
            result.exit_code = status.code();
            result.output = format!("{stdout}{stderr}");
        }
        Some(Ok((Err(error), _, _))) => result.output = error.to_string(),
        Some(Err(_)) => {
            result.output = format!("Timed out after {} seconds", check.timeout_seconds)
        }
        None => result.output = "Stopped".into(),
    }
    let _ = child.wait().await;
    result
}
