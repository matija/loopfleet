//! Claude Code adapter, headless. Spawns
//! `claude -p <prompt> --output-format stream-json --verbose
//! --dangerously-skip-permissions` in the run's worktree and maps its
//! newline-delimited stream-json into [`NormalizedEvent`]s.
//!
//! The transport is one JSON object per line. The shapes this adapter maps
//! (captured from Claude Code 2.1.x):
//! - `{"type":"system","subtype":"init",…}` — session start → `TurnStarted`.
//! - `{"type":"assistant","message":{"content":[…blocks…]}}` — each content
//!   block maps: `text` → `AssistantText`, `thinking` → `Reasoning`,
//!   `tool_use` → `CommandRun` when the tool is `Bash`, else `ToolCall`.
//! - `{"type":"user","message":{"content":[{"type":"tool_result",…}]}}` —
//!   `tool_result` → `ToolResult`, correlated to its `tool_use` by id. Results
//!   of `Bash` calls are dropped: they were already normalized to `CommandRun`,
//!   which the enum carries as a single event with no result pairing.
//! - `{"type":"result","subtype":"success"|…,"usage":{…}}` — terminal line →
//!   `TurnCompleted` (success) or `Failed` (error), then `Ended`.
//!
//! Other line types Claude emits (hook lifecycle, rate-limit notices, partial
//! deltas) carry nothing the enum represents and are ignored.
//!
//! Separately from runs, the adapter answers [`AgentAdapter::usage_snapshot`]
//! by probing the CLI's `/usage` slash command headlessly. That command is
//! answered locally (no model turn, no tokens, no cost) and its only
//! machine-readable surface is the `result` string of `--output-format json`,
//! which holds a block of prose:
//!
//! ```text
//! You are currently using your subscription to power your Claude Code usage
//!
//! Current session: 17% used · resets Aug 25 at 12:20pm (Europe/Zagreb)
//! Current week (all models): 56% used · resets Aug 25 at 1pm (Europe/Zagreb)
//! ```
//!
//! [`map_usage`] reads the `<label>: <n>% used` rows out of that prose, and the
//! fullest row's `resets …` clause through [`parse_reset_at`]. Because prose is
//! not a contract, every way it can disappoint us — the CLI missing, the probe
//! failing, JSON we cannot parse, wording we do not recognize — maps to
//! [`UsageSnapshot::unknown`] rather than an error: the snapshot's
//! [`UsageSource::Unknown`](loopfleet_core::UsageSource::Unknown) already says "nothing is known", and a limit query
//! is not worth failing a caller over.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use loopfleet_core::{NormalizedEvent, Usage, UsageSnapshot};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::{AdapterError, AgentAdapter, RunHandle, RunSpec, SessionHandle, SessionSeed};

/// Longest excerpt (in bytes, on a char boundary) kept for tool inputs and
/// results — the event log stores excerpts, not full payloads.
const EXCERPT_LIMIT: usize = 2000;

/// The agent key every snapshot this adapter produces is stamped with — the
/// same key `discovery` registers the CLI under.
const AGENT_KEY: &str = "claude";

/// How long the out-of-band `/usage` probe may take before we give up and call
/// the answer unknown. Generous enough for a cold CLI start, short enough that
/// a wedged binary cannot stall a scheduling decision.
const USAGE_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The Claude Code headless adapter. Stateless; each `start_run` spawns its own
/// process and mapper.
pub struct ClaudeAdapter;

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    async fn start_run(&self, spec: &RunSpec) -> Result<RunHandle, AdapterError> {
        let mut cmd = crate::base_command(&spec.wrapper, "claude");
        cmd.arg("-p")
            .arg(&spec.prompt)
            .args(["--output-format", "stream-json", "--verbose"])
            .arg("--dangerously-skip-permissions");
        if let Some(model) = &spec.model {
            cmd.arg("--model").arg(model);
        }
        let mut child = cmd
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(AdapterError::Spawn)?;

        let stdout = child
            .stdout
            .take()
            .expect("stdout was piped so it is present");
        let stderr = child
            .stderr
            .take()
            .expect("stderr was piped so it is present");

        // Bounded channel: the backpressure contract (a slow consumer stalls the
        // reader) matching the stub and the M1 event-log writer.
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(drive(child, stdout, stderr, tx));
        Ok(RunHandle { events: rx })
    }

    async fn open_session(
        &self,
        _cwd: &Path,
        _seed: SessionSeed,
    ) -> Result<SessionHandle, AdapterError> {
        Err(AdapterError::SessionsUnsupported)
    }

    /// Probes `/usage` and maps its prose into a snapshot stamped `now_ms`.
    ///
    /// Never `Err`: a failed or unrecognized probe answers
    /// [`UsageSnapshot::unknown`]. That is a truthful "the agent can report,
    /// but told us nothing this time", and is deliberately distinct from the
    /// trait default's [`AdapterError::UsageUnsupported`], which claims the
    /// agent has no way to report at all.
    async fn usage_snapshot(&self, now_ms: i64) -> Result<UsageSnapshot, AdapterError> {
        Ok(map_usage(probe_usage().await.as_deref(), now_ms))
    }
}

/// Runs `claude -p /usage --output-format json` and returns the envelope's
/// `result` string, or `None` if anything at all went wrong — the binary is
/// missing, the probe timed out or exited non-zero, stdout was not the JSON
/// envelope, or the envelope reported an error or carried no `result`.
///
/// Callers turn `None` into an unknown snapshot. Which failure it was is not a
/// distinction [`UsageSnapshot`] can express, so it is not one worth carrying
/// up.
///
/// The probe needs no worktree, no sandbox wrapper and no permissions: `/usage`
/// is answered by the CLI itself, without a model turn.
async fn probe_usage() -> Option<String> {
    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p")
        .arg("/usage")
        .args(["--output-format", "json"])
        // A limit query has no business in the user's resumable history.
        .arg("--no-session-persistence")
        .stdin(std::process::Stdio::null());
    // Own process group, matching every other agent spawn here, so a probe that
    // outlives the timeout is signallable on its own.
    #[cfg(unix)]
    cmd.process_group(0);

    let output = tokio::time::timeout(USAGE_PROBE_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&output.stdout).ok()?;
    if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    v.get("result").and_then(Value::as_str).map(String::from)
}

/// One `<label>: <n>% used` row of the `/usage` prose, split into the parts a
/// [`UsageSnapshot`] has fields for.
#[derive(Debug, PartialEq)]
struct UsageWindow {
    /// The window's label, normalized where we recognize the wording.
    window: String,
    /// The model the row scopes its window to, when it names one.
    model: Option<String>,
    used_fraction: f64,
}

/// Maps the `/usage` prose (or its absence) into a snapshot stamped `now_ms`.
///
/// When several windows are reported — session and week, or a per-model week
/// alongside the all-model one — the snapshot describes the *fullest* of them.
/// A snapshot carries one fraction, and the fullest window is the one that will
/// stop the next run, so it is the one a scheduler has to see.
///
/// The fullest row's own `resets …` clause is resolved through
/// [`parse_reset_at`] into [`UsageSnapshot::reset_at_ms`]. A row with no clause,
/// or one [`parse_reset_at`] cannot resolve unambiguously, leaves it `None`, and
/// staleness falls back to the snapshot's age.
fn map_usage(text: Option<&str>, now_ms: i64) -> UsageSnapshot {
    let unknown = UsageSnapshot::unknown(AGENT_KEY, now_ms);
    let Some(text) = text else {
        return unknown;
    };
    let fullest = text
        .lines()
        .filter_map(|line| parse_usage_row(line).map(|row| (line, row)))
        .max_by(|(_, a), (_, b)| a.used_fraction.total_cmp(&b.used_fraction));
    // Prose we recognize nothing in tells us exactly as much as no prose.
    let Some((line, fullest)) = fullest else {
        return unknown;
    };

    let mut snapshot = UsageSnapshot::reported(AGENT_KEY, fullest.used_fraction, now_ms)
        .with_limit_window(fullest.window);
    if let Some(model) = fullest.model {
        snapshot = snapshot.with_model(model);
    }
    if let Some(reset_at_ms) = parse_reset_at(line, now_ms) {
        snapshot = snapshot.with_reset_at(reset_at_ms);
    }
    snapshot
}

/// Parses one line of the prose as a usage row, or `None` if it is not one.
///
/// The shape required is `<label>: <n>% used`, optionally trailed by the
/// `· resets …` clause we ignore. Insisting on the `used` keyword is what keeps
/// the prose's other colon-and-percent lines (`Top skills: /to-prd 4%`) from
/// reading as windows.
fn parse_usage_row(line: &str) -> Option<UsageWindow> {
    let (label, rest) = line.trim().split_once(':')?;
    let (percent, tail) = rest.split_once('%')?;
    if !tail.trim_start().starts_with("used") {
        return None;
    }
    let used_fraction = percent.trim().parse::<f64>().ok()? / 100.0;
    if !used_fraction.is_finite() {
        return None;
    }
    let (window, model) = split_label(label.trim());
    Some(UsageWindow {
        window,
        model,
        used_fraction,
    })
}

/// Splits a row's label into a window name and, where the label scopes the
/// window to a model, that model.
///
/// The labels the CLI is known to print become the short window names the rest
/// of the app already speaks (`"session"`, `"weekly"`), and a parenthesized
/// qualifier that is not the all-models marker is read as a model name
/// (`"Current week (Opus)"`). An unfamiliar label is passed through verbatim
/// rather than guessed at — it is still worth showing.
fn split_label(label: &str) -> (String, Option<String>) {
    let (head, qualifier) = match label.split_once('(') {
        Some((head, rest)) => (head.trim(), rest.strip_suffix(')').map(str::trim)),
        None => (label, None),
    };
    let window = match head.to_ascii_lowercase().as_str() {
        "current session" => "session".to_string(),
        "current week" => "weekly".to_string(),
        _ => label.to_string(),
    };
    // "all models" scopes nothing: it is how the CLI spells *no* model
    // qualifier.
    let model = qualifier
        .filter(|q| !q.eq_ignore_ascii_case("all models"))
        .map(str::to_string);
    (window, model)
}

/// Resolves the `resets <Mon> <D> at <h:mm><am|pm> (<IANA zone>)` clause found
/// in `text` (the full `/usage` prose or a single row of it) into epoch
/// milliseconds, relative to `now_ms`.
///
/// The clause names a month, day and time of day but never a year, so the
/// year is chosen as whichever of *this* year or the next lands the resulting
/// instant nearest in the future relative to `now_ms` — the CLI always means
/// the next upcoming reset, never one already past. The zone is read from the
/// trailing `(...)` when present; when the clause names none, the host's
/// local zone is used instead, matching what a user reading the prose without
/// a zone annotation would assume.
///
/// Answers `None` for anything the clause does not resolve unambiguously:
/// no `resets` clause, an unrecognized month, an out-of-range day (e.g. Feb
/// 29 landing on a non-leap year in both candidate years), an unparseable
/// time, a zone name `chrono-tz` does not know, or a local time that a DST
/// transition makes ambiguous or nonexistent in the resolved zone.
pub fn parse_reset_at(text: &str, now_ms: i64) -> Option<i64> {
    let now = Utc.timestamp_millis_opt(now_ms).single()?;
    let clause = text.split("resets ").nth(1)?;

    let mut rest = clause.trim_start();
    let (month_word, r) = take_token(rest)?;
    let month = month_number(month_word)?;
    rest = r.trim_start();

    let (day_word, r) = take_token(rest)?;
    let day: u32 = day_word.parse().ok()?;
    rest = r.trim_start();

    rest = rest.strip_prefix("at ")?.trim_start();
    let (time_word, r) = take_token(rest)?;
    let time = parse_time(time_word)?;
    rest = r;

    let zone_name = rest
        .trim_start()
        .strip_prefix('(')
        .and_then(|r| r.split(')').next());

    // Only the same or the following year are ever candidates: the clause
    // names a month/day/time that repeats annually, so the nearest future
    // occurrence is either still to come this year or is next year's.
    let this_year = now.year();
    let candidates = [this_year, this_year + 1]
        .into_iter()
        .filter_map(|year| resolve_instant(year, month, day, time, zone_name));

    candidates
        .filter(|instant| *instant >= now)
        .min()
        .map(|instant| instant.timestamp_millis())
}

/// Builds the UTC instant for `year`-`month`-`day` `time` in `zone_name`
/// (falling back to the host's local zone when `zone_name` is `None`),
/// answering `None` if the date is invalid or the local time is ambiguous or
/// nonexistent in that zone.
fn resolve_instant(
    year: i32,
    month: u32,
    day: u32,
    time: NaiveTime,
    zone_name: Option<&str>,
) -> Option<chrono::DateTime<Utc>> {
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let naive = date.and_time(time);
    match zone_name {
        Some(name) => {
            let tz = Tz::from_str(name).ok()?;
            match tz.from_local_datetime(&naive) {
                LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
                _ => None,
            }
        }
        None => match chrono::Local.from_local_datetime(&naive) {
            LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
            _ => None,
        },
    }
}

/// Splits the next whitespace-delimited token off `s`, returning it along
/// with what follows.
fn take_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    match s.split_once(char::is_whitespace) {
        Some((word, rest)) => Some((word, rest)),
        None => Some((s, "")),
    }
}

/// Maps a three-letter English month abbreviation to its 1-based number.
fn month_number(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(word))
        .map(|i| i as u32 + 1)
}

/// Parses a clock reading of the form `<h>[:<mm>]<am|pm>` (the CLI omits the
/// minutes when they are `:00`, e.g. `1pm`) into a [`NaiveTime`].
fn parse_time(word: &str) -> Option<NaiveTime> {
    let lower = word.to_ascii_lowercase();
    let (digits, is_pm) = if let Some(d) = lower.strip_suffix("am") {
        (d, false)
    } else if let Some(d) = lower.strip_suffix("pm") {
        (d, true)
    } else {
        return None;
    };
    let (hour_str, minute_str) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None => (digits, "0"),
    };
    let hour12: u32 = hour_str.parse().ok()?;
    let minute: u32 = minute_str.parse().ok()?;
    if !(1..=12).contains(&hour12) {
        return None;
    }
    let hour24 = match (hour12, is_pm) {
        (12, false) => 0,  // 12am is midnight.
        (12, true) => 12,  // 12pm is noon.
        (h, false) => h,
        (h, true) => h + 12,
    };
    NaiveTime::from_hms_opt(hour24, minute, 0)
}

/// Reads the process's stdout line by line, maps each into normalized events,
/// and forwards them. When the stream ends without a terminal `result` line,
/// synthesizes a `Failed`/`Ended` pair from stderr so consumers always see a
/// termination.
async fn drive(
    mut child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    tx: mpsc::Sender<NormalizedEvent>,
) {
    let mut mapper = ClaudeMapper::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_terminal = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let events = match mapper.map_line(&line) {
                    Ok(events) => events,
                    // A single unparseable line shouldn't kill the run; surface
                    // it and keep reading.
                    Err(e) => vec![NormalizedEvent::Failed {
                        reason: e.to_string(),
                    }],
                };
                for ev in events {
                    if matches!(ev, NormalizedEvent::Ended) {
                        saw_terminal = true;
                    }
                    if tx.send(ev).await.is_err() {
                        // Consumer dropped: SIGTERM the agent's group and stop.
                        crate::stop_agent(&mut child);
                        return;
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx
                    .send(NormalizedEvent::Failed {
                        reason: format!("reading agent stdout: {e}"),
                    })
                    .await;
                break;
            }
        }
    }

    if !saw_terminal {
        let reason = read_stderr(stderr)
            .await
            .filter(|s| !s.is_empty())
            .map(|s| format!("agent exited without a result: {s}"))
            .unwrap_or_else(|| "agent exited without a result".to_string());
        let _ = tx.send(NormalizedEvent::Failed { reason }).await;
        let _ = tx.send(NormalizedEvent::Ended).await;
    }

    let _ = child.wait().await;
}

/// Drains stderr into a string for a failure reason. Best-effort.
async fn read_stderr(stderr: tokio::process::ChildStderr) -> Option<String> {
    let mut lines = BufReader::new(stderr).lines();
    let mut collected = Vec::new();
    while let Ok(Some(line)) = lines.next_line().await {
        collected.push(line);
    }
    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n"))
    }
}

/// Stateful mapper from Claude stream-json lines to [`NormalizedEvent`]s. Holds
/// the set of `Bash` tool-call ids so their `tool_result` lines can be dropped
/// (already normalized to `CommandRun`).
struct ClaudeMapper {
    bash_calls: HashSet<String>,
}

impl ClaudeMapper {
    fn new() -> Self {
        Self {
            bash_calls: HashSet::new(),
        }
    }

    /// Maps one line into zero or more normalized events. Blank lines and line
    /// types the enum does not represent yield an empty vec.
    fn map_line(&mut self, line: &str) -> Result<Vec<NormalizedEvent>, AdapterError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(line)
            .map_err(|e| AdapterError::Protocol(format!("invalid stream-json line: {e}")))?;

        match v.get("type").and_then(Value::as_str) {
            Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                Ok(vec![NormalizedEvent::TurnStarted])
            }
            Some("assistant") => Ok(self.map_content(&v, Self::map_assistant_block())),
            Some("user") => Ok(self.map_content(&v, Self::map_user_block())),
            Some("rate_limit_event") => Ok(self.map_rate_limit(&v)),
            Some("result") => Ok(self.map_result(&v)),
            _ => Ok(vec![]),
        }
    }

    /// Applies a per-block mapper to `message.content[]`, in stream order.
    fn map_content(
        &mut self,
        v: &Value,
        mapper: impl Fn(&mut Self, &Value) -> Option<NormalizedEvent>,
    ) -> Vec<NormalizedEvent> {
        v.get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| mapper(self, b))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Block mapper for `type:"assistant"` messages.
    fn map_assistant_block() -> impl Fn(&mut Self, &Value) -> Option<NormalizedEvent> {
        |this: &mut Self, b: &Value| match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = b.get("text").and_then(Value::as_str).unwrap_or_default();
                (!text.is_empty()).then(|| NormalizedEvent::AssistantText { text: text.into() })
            }
            Some("thinking") => {
                let text = b.get("thinking").and_then(Value::as_str).unwrap_or_default();
                (!text.is_empty()).then(|| NormalizedEvent::Reasoning { text: text.into() })
            }
            Some("tool_use") => {
                let name = b.get("name").and_then(Value::as_str).unwrap_or_default();
                let id = b.get("id").and_then(Value::as_str).unwrap_or_default();
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                if name == "Bash" {
                    // Shell-exec is normalized to CommandRun (no result pairing);
                    // remember the id so we drop its tool_result.
                    this.bash_calls.insert(id.to_string());
                    let cmd = input
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    // Claude's bash tool_result carries no numeric exit code, so
                    // exit is unknown at invocation and stays absent.
                    Some(NormalizedEvent::CommandRun {
                        cmd: cmd.into(),
                        exit: None,
                    })
                } else {
                    Some(NormalizedEvent::ToolCall {
                        call_id: id.into(),
                        name: name.into(),
                        input_excerpt: excerpt(&compact(&input)),
                    })
                }
            }
            _ => None,
        }
    }

    /// Block mapper for `type:"user"` messages (tool results).
    fn map_user_block() -> impl Fn(&mut Self, &Value) -> Option<NormalizedEvent> {
        |this: &mut Self, b: &Value| {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                return None;
            }
            let id = b
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Result of a Bash call: already emitted as CommandRun, and the enum
            // has no way to correlate a result to it. Drop it.
            if this.bash_calls.contains(id) {
                return None;
            }
            // `is_error` absent means success.
            let ok = !b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            let output = stringify_content(b.get("content"));
            Some(NormalizedEvent::ToolResult {
                call_id: id.into(),
                ok,
                output_excerpt: excerpt(&output),
            })
        }
    }

    /// Maps a `rate_limit_event` line to `RateLimited`, extracting an optional
    /// ISO-8601 `reset_time` and an optional error `message`.
    fn map_rate_limit(&self, v: &Value) -> Vec<NormalizedEvent> {
        let reset_at = v
            .get("reset_time")
            .and_then(Value::as_str)
            .map(String::from);
        let message = v
            .get("error")
            .or_else(|| v.get("message"))
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| {
                // If all we have is a session_id, synthesize a basic message.
                v.get("session_id")
                    .and_then(Value::as_str)
                    .map(|_| "rate limit hit".to_string())
            });
        vec![NormalizedEvent::RateLimited { reset_at, message }]
    }

    /// Maps the terminal `result` line: `TurnCompleted` on success or `Failed`
    /// on error, always followed by `Ended`.
    fn map_result(&self, v: &Value) -> Vec<NormalizedEvent> {
        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false)
            || v.get("subtype").and_then(Value::as_str) != Some("success");
        let terminal = if is_error {
            let reason = v
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("subtype")
                        .and_then(Value::as_str)
                        .map(|s| format!("agent reported {s}"))
                })
                .unwrap_or_else(|| "agent failed".to_string());
            NormalizedEvent::Failed { reason }
        } else {
            NormalizedEvent::TurnCompleted {
                usage: parse_usage(v.get("usage")),
            }
        };
        vec![terminal, NormalizedEvent::Ended]
    }
}

/// Reads `input_tokens` / `output_tokens` from a result's `usage` object.
/// Claude's cache-token fields are not part of the normalized `Usage`.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name: &str| {
        usage
            .and_then(|u| u.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: field("input_tokens"),
        output_tokens: field("output_tokens"),
    }
}

/// A `tool_result`'s `content` is either a plain string or an array of
/// `{type:"text",text:…}` blocks; flatten either into text.
fn stringify_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Compact (no-whitespace) JSON, for tool-input excerpts.
fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Truncates to [`EXCERPT_LIMIT`] on a char boundary, marking elision.
fn excerpt(s: &str) -> String {
    if s.len() <= EXCERPT_LIMIT {
        return s.to_string();
    }
    let mut end = EXCERPT_LIMIT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use loopfleet_core::UsageSource;

    fn map_all(text: &str) -> Vec<NormalizedEvent> {
        let mut mapper = ClaudeMapper::new();
        text.lines()
            .flat_map(|line| mapper.map_line(line).unwrap())
            .collect()
    }

    /// The captured real-world stream maps to the expected event sequence:
    /// TurnStarted, the Read call/result pair, the RateLimited notice, the
    /// Bash CommandRun (its result dropped), the final text, then
    /// TurnCompleted + Ended.
    #[test]
    fn maps_captured_stream() {
        let fixture = include_str!("../fixtures/claude_stream.jsonl");
        let events = map_all(fixture);

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));

        // Read tool → ToolCall/ToolResult correlated by id; Bash → CommandRun.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::ToolCall { name, .. } if name == "Read"
        )));
        let call_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolCall { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        let result_id = events.iter().find_map(|e| match e {
            NormalizedEvent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        });
        assert!(call_id.is_some() && call_id == result_id);

        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::CommandRun { cmd, exit: None } if cmd == "echo hi"
        )));
        // Exactly one ToolResult: the Read result. The Bash result is dropped.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, NormalizedEvent::ToolResult { .. }))
                .count(),
            1
        );

        // The fixture has a rate_limit_event — mapped with message but no
        // reset_time.
        assert!(events.iter().any(|e| matches!(
            e,
            NormalizedEvent::RateLimited {
                reset_at: None,
                message: Some(msg),
            } if msg == "rate limit hit"
        )));

        assert_eq!(
            events.iter().find(|e| matches!(e, NormalizedEvent::TurnCompleted { .. })),
            Some(&NormalizedEvent::TurnCompleted {
                usage: Usage {
                    input_tokens: 4363,
                    output_tokens: 180,
                },
            })
        );
    }

    #[test]
    fn thinking_maps_to_reasoning_and_text_to_assistant() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"answer"}]}}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::Reasoning { text: "hmm".into() },
                NormalizedEvent::AssistantText {
                    text: "answer".into()
                },
            ]
        );
    }

    #[test]
    fn empty_text_and_thinking_blocks_are_skipped() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"","signature":"s"},{"type":"text","text":""}]}}"#;
        assert!(map_all(line).is_empty());
    }

    #[test]
    fn non_bash_tool_use_becomes_tool_call_with_excerpt() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Grep","input":{"pattern":"fn main"}}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolCall {
                call_id: "t1".into(),
                name: "Grep".into(),
                input_excerpt: r#"{"pattern":"fn main"}"#.into(),
            }]
        );
    }

    #[test]
    fn tool_result_error_flag_sets_ok_false() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"boom","is_error":true}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolResult {
                call_id: "t1".into(),
                ok: false,
                output_excerpt: "boom".into(),
            }]
        );
    }

    #[test]
    fn error_result_maps_to_failed_then_ended() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"something broke"}"#;
        assert_eq!(
            map_all(line),
            vec![
                NormalizedEvent::Failed {
                    reason: "something broke".into()
                },
                NormalizedEvent::Ended,
            ]
        );
    }

    #[test]
    fn unknown_line_types_and_blanks_are_ignored() {
        let text = "\n{\"type\":\"system\",\"subtype\":\"hook_started\"}\n";
        assert!(map_all(text).is_empty());
    }

    #[test]
    fn rate_limit_event_maps_to_rate_limited() {
        let line = r#"{"type":"rate_limit_event","session_id":"sess-1"}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::RateLimited {
                reset_at: None,
                message: Some("rate limit hit".into()),
            }]
        );
    }

    #[test]
    fn rate_limit_event_with_reset_time_and_error() {
        let line = r#"{"type":"rate_limit_event","reset_time":"2025-01-15T10:30:00Z","error":"request limit exceeded","session_id":"sess-1"}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::RateLimited {
                reset_at: Some("2025-01-15T10:30:00Z".into()),
                message: Some("request limit exceeded".into()),
            }]
        );
    }

    #[test]
    fn rate_limit_event_with_message_field() {
        // Some Claude versions may use "message" instead of "error".
        let line = r#"{"type":"rate_limit_event","reset_time":"2025-01-15T10:30:00Z","message":"token rate limit"}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::RateLimited {
                reset_at: Some("2025-01-15T10:30:00Z".into()),
                message: Some("token rate limit".into()),
            }]
        );
    }

    #[test]
    fn invalid_json_is_a_protocol_error() {
        let mut mapper = ClaudeMapper::new();
        assert!(matches!(
            mapper.map_line("{not json}"),
            Err(AdapterError::Protocol(_))
        ));
    }

    #[test]
    fn array_content_tool_result_is_flattened() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t9","content":[{"type":"text","text":"line one"},{"type":"text","text":"line two"}]}]}}"#;
        assert_eq!(
            map_all(line),
            vec![NormalizedEvent::ToolResult {
                call_id: "t9".into(),
                ok: true,
                output_excerpt: "line one\nline two".into(),
            }]
        );
    }

    #[test]
    fn excerpt_truncates_on_char_boundary() {
        let long = "x".repeat(EXCERPT_LIMIT + 50);
        let out = excerpt(&long);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().filter(|c| *c == 'x').count(), EXCERPT_LIMIT);
    }

    // --- usage_snapshot mapping -------------------------------------------

    /// A fixed "now" (epoch millis) for the usage tests, so nothing reads a
    /// clock.
    const NOW: i64 = 1_760_000_000_000;

    /// Pulls the `/usage` prose out of a captured `--output-format json`
    /// envelope, the way [`probe_usage`] does.
    fn usage_text(envelope: &str) -> String {
        serde_json::from_str::<Value>(envelope)
            .unwrap()
            .get("result")
            .and_then(Value::as_str)
            .unwrap()
            .to_string()
    }

    /// The captured `/usage` payload maps to the fullest of its two windows:
    /// the 56% week, not the 20% session. No model (the row says "all models"),
    /// and the reset instant is the week row's own `resets …` clause resolved
    /// through [`parse_reset_at`], not the session row's.
    #[test]
    fn maps_captured_usage_payload() {
        let text = usage_text(include_str!("../fixtures/claude_usage.json"));
        assert_eq!(
            map_usage(Some(&text), NOW),
            UsageSnapshot {
                agent_key: "claude".into(),
                model: None,
                limit_window: Some("weekly".into()),
                used_fraction: 0.56,
                reset_at_ms: Some(1_787_655_540_000),
                observed_at_ms: NOW,
                source: UsageSource::Reported,
            }
        );
    }

    /// The session window wins when it is the fuller one — the snapshot tracks
    /// whichever limit will actually stop the next run.
    #[test]
    fn fullest_window_wins_when_it_is_the_session() {
        let text = "Current session: 91% used · resets Aug 25 at 12:20pm (Europe/Zagreb)\n\
                    Current week (all models): 12% used · resets Aug 27 at 1pm (Europe/Zagreb)";
        let snap = map_usage(Some(text), NOW);
        assert_eq!(snap.limit_window.as_deref(), Some("session"));
        assert!((snap.used_fraction - 0.91).abs() < f64::EPSILON);
        assert_eq!(snap.source, UsageSource::Reported);
    }

    /// A week row scoped to one model carries that model through; "all models"
    /// is the CLI's way of writing *no* model, and must not become one.
    #[test]
    fn per_model_week_row_carries_the_model() {
        let text = "Current session: 4% used · resets Aug 25 at 12:20pm (Europe/Zagreb)\n\
                    Current week (all models): 30% used · resets Aug 27 at 1pm (Europe/Zagreb)\n\
                    Current week (Opus): 72% used · resets Aug 27 at 1pm (Europe/Zagreb)";
        let snap = map_usage(Some(text), NOW);
        assert_eq!(snap.limit_window.as_deref(), Some("weekly"));
        assert_eq!(snap.model.as_deref(), Some("Opus"));
        assert!((snap.used_fraction - 0.72).abs() < f64::EPSILON);
    }

    /// Wording we do not recognize is still a window worth showing: the label
    /// passes through verbatim rather than being guessed at.
    #[test]
    fn unfamiliar_label_passes_through_as_the_window() {
        let snap = map_usage(Some("Current 5h block: 33% used"), NOW);
        assert_eq!(snap.limit_window.as_deref(), Some("Current 5h block"));
        assert_eq!(snap.model, None);
    }

    /// The prose's other colon-and-percent lines are not usage rows. A payload
    /// of nothing but those is as uninformative as no payload.
    #[test]
    fn non_window_rows_do_not_read_as_usage() {
        let text = "What's contributing to your limits usage?\n\
                    Last 24h · 706 requests · 31 sessions\n\
                    \x20 Top skills: /to-prd 4%";
        let snap = map_usage(Some(text), NOW);
        assert_eq!(snap, UsageSnapshot::unknown("claude", NOW));
        assert_eq!(snap.source, UsageSource::Unknown);
    }

    /// An API-key user gets prose with no percentages at all. Unknown, not an
    /// error, and not a zero that would read as headroom.
    #[test]
    fn payload_without_percentages_is_unknown() {
        let text = "You are currently using a Claude API key to power your Claude Code usage";
        assert_eq!(
            map_usage(Some(text), NOW),
            UsageSnapshot::unknown("claude", NOW)
        );
    }

    /// A failed probe — CLI missing, timed out, non-zero exit, unparseable
    /// JSON — reaches the mapper as `None` and is likewise unknown.
    #[test]
    fn absent_payload_is_unknown() {
        let snap = map_usage(None, NOW);
        assert_eq!(snap, UsageSnapshot::unknown("claude", NOW));
        assert_eq!(snap.used_fraction, 0.0);
        assert_eq!(snap.source, UsageSource::Unknown);
    }

    /// A percentage above 100 is clamped by the constructor, so consumers can
    /// compare against `EXHAUSTED_FRACTION` without re-validating.
    #[test]
    fn out_of_range_percentage_is_clamped() {
        assert_eq!(
            map_usage(Some("Current session: 140% used"), NOW).used_fraction,
            1.0
        );
    }

    /// Fractional percentages survive the trip.
    #[test]
    fn fractional_percentage_is_kept() {
        let snap = map_usage(Some("Current session: 7.5% used"), NOW);
        assert!((snap.used_fraction - 0.075).abs() < f64::EPSILON);
    }

    /// Rows the mapper accepts, split the way the snapshot's fields want.
    #[test]
    fn parses_a_row_into_window_model_and_fraction() {
        assert_eq!(
            parse_usage_row("Current week (Opus): 56% used · resets Aug 25 at 1pm"),
            Some(UsageWindow {
                window: "weekly".into(),
                model: Some("Opus".into()),
                used_fraction: 0.56,
            })
        );
        assert_eq!(
            parse_usage_row("Last 7d · 2238 requests · 104 sessions"),
            None
        );
        assert_eq!(parse_usage_row("  Top skills: /to-prd 4%"), None);
        assert_eq!(parse_usage_row("Current session: lots% used"), None);
    }

    // --- parse_reset_at -----------------------------------------------------

    /// Builds epoch millis for a UTC date/time, for expected values below.
    fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
            .timestamp_millis()
    }

    /// A clause whose date has not yet happened this year resolves in the
    /// zone it names, converted to UTC.
    #[test]
    fn resolves_named_zone_within_the_same_year() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        let got = parse_reset_at("resets Aug 25 at 12:20pm (Europe/Zagreb)", now);
        // Europe/Zagreb is UTC+2 (CEST) in August.
        assert_eq!(got, Some(utc_ms(2025, 8, 25, 10, 20)));
    }

    /// A clause naming no minutes (`1pm`, not `1:00pm`) is still parsed.
    #[test]
    fn resolves_bare_hour_with_no_minutes() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        let got = parse_reset_at("resets Aug 25 at 1pm (Europe/Zagreb)", now);
        assert_eq!(got, Some(utc_ms(2025, 8, 25, 11, 0)));
    }

    /// When this year's occurrence of the date has already passed relative to
    /// `now`, the next year's is chosen instead — the nearest instance still
    /// in the future.
    #[test]
    fn rolls_over_to_next_year_once_the_date_has_passed() {
        let now = utc_ms(2025, 9, 1, 0, 0);
        let got = parse_reset_at("resets Aug 25 at 12:20pm (Europe/Zagreb)", now);
        assert_eq!(got, Some(utc_ms(2026, 8, 25, 10, 20)));
    }

    /// A clause with no `(<zone>)` falls back to the host's local zone rather
    /// than assuming UTC.
    #[test]
    fn falls_back_to_local_zone_when_none_named() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        let naive = NaiveDate::from_ymd_opt(2025, 8, 25)
            .unwrap()
            .and_hms_opt(13, 0, 0)
            .unwrap();
        let expected = match chrono::Local.from_local_datetime(&naive) {
            LocalResult::Single(dt) => dt.with_timezone(&Utc).timestamp_millis(),
            other => panic!("host local zone made a plain date ambiguous: {other:?}"),
        };
        assert_eq!(parse_reset_at("resets Aug 25 at 1pm", now), Some(expected));
    }

    /// A zone name `chrono-tz` does not recognize is unresolvable, not a
    /// silent UTC or local guess.
    #[test]
    fn unknown_zone_name_is_none() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        assert_eq!(
            parse_reset_at("resets Aug 25 at 1pm (Nowhere/Nowhere)", now),
            None
        );
    }

    /// Prose with no `resets` clause at all resolves nothing.
    #[test]
    fn no_resets_clause_is_none() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        assert_eq!(
            parse_reset_at("Current session: 17% used", now),
            None
        );
    }

    /// A day the named month never has (Feb 30) is invalid in every
    /// candidate year, so it never resolves.
    #[test]
    fn impossible_date_is_none() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        assert_eq!(
            parse_reset_at("resets Feb 30 at 1pm (Europe/Zagreb)", now),
            None
        );
    }

    /// A whole `/usage`-style prose blob is scanned the same as a single row:
    /// the first `resets` clause found is the one resolved.
    #[test]
    fn resolves_the_clause_out_of_full_prose() {
        let now = utc_ms(2025, 8, 20, 0, 0);
        let text = "Current session: 17% used · resets Aug 25 at 12:20pm (Europe/Zagreb)\n\
                    Current week (all models): 56% used · resets Aug 25 at 1pm (Europe/Zagreb)";
        assert_eq!(parse_reset_at(text, now), Some(utc_ms(2025, 8, 25, 10, 20)));
    }

    /// A local time a DST transition skips over (spring-forward) or repeats
    /// (fall-back) is ambiguous in the named zone, so it does not resolve
    /// even though the date and time are individually well formed.
    #[test]
    fn dst_transition_local_times_are_none() {
        let zone = Tz::from_str("America/New_York").unwrap();
        // 2023-03-12: clocks sprang forward from 2:00am to 3:00am; 2:30am
        // never happened.
        let gap = NaiveDate::from_ymd_opt(2023, 3, 12)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(matches!(
            zone.from_local_datetime(&gap),
            LocalResult::None
        ));
        assert_eq!(resolve_instant(2023, 3, 12, gap.time(), Some("America/New_York")), None);

        // 2023-11-05: clocks fell back from 2:00am to 1:00am; 1:30am
        // happened twice.
        let doubled = NaiveDate::from_ymd_opt(2023, 11, 5)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert!(matches!(
            zone.from_local_datetime(&doubled),
            LocalResult::Ambiguous(_, _)
        ));
        assert_eq!(resolve_instant(2023, 11, 5, doubled.time(), Some("America/New_York")), None);
    }

    /// The probe against the real `claude` binary. Ignored by default: it needs
    /// the CLI installed and logged in. `/usage` is answered locally, so unlike
    /// `live_run` below it costs no tokens.
    #[tokio::test]
    #[ignore = "spawns the real claude CLI; needs it installed and logged in"]
    async fn live_usage_snapshot() {
        use crate::AgentAdapter;

        let snap = ClaudeAdapter.usage_snapshot(NOW).await.unwrap();
        assert_eq!(snap.agent_key, "claude");
        assert_eq!(snap.observed_at_ms, NOW);
        // Whatever the account's state, a logged-in CLI reports a figure.
        assert_eq!(snap.source, UsageSource::Reported);
        assert!((0.0..=1.0).contains(&snap.used_fraction));
    }

    /// End-to-end against the real `claude` binary in a throwaway git repo.
    /// Ignored by default: it needs the CLI installed, network, and credits.
    /// Run with `cargo test -p loopfleet-adapters -- --ignored live_run`.
    #[tokio::test]
    #[ignore = "spawns the real claude CLI; needs network + credits"]
    async fn live_run_against_fixture_repo() {
        use crate::AgentAdapter;
        use std::process::Command as StdCommand;

        let dir = tempfile::tempdir().unwrap();
        StdCommand::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("README.md"), "hello\n").unwrap();

        let spec = RunSpec {
            cwd: dir.path().to_path_buf(),
            prompt: "Read README.md and then say the single word done.".into(),
            wrapper: Vec::new(),
            model: None,
        };
        let mut handle = ClaudeAdapter.start_run(&spec).await.unwrap();

        let mut events = Vec::new();
        while let Some(ev) = handle.events.recv().await {
            events.push(ev);
        }

        assert_eq!(events.first(), Some(&NormalizedEvent::TurnStarted));
        assert_eq!(events.last(), Some(&NormalizedEvent::Ended));
        assert!(events
            .iter()
            .any(|e| matches!(e, NormalizedEvent::TurnCompleted { .. })));
    }
}
