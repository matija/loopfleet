//! The report renderer: a task's (or plan's) stored data rendered as one
//! complete, standalone HTML document.
//!
//! This module is pure — it takes already-loaded data (task text, runs with
//! their outcomes and timings, normalized events, diff summaries) and returns
//! an HTML string with all CSS inlined, so the result can be saved, mailed, or
//! opened anywhere with no app running and no external assets. The styles
//! mirror the app's visual language (see `frontend/src/tokens.css`): one
//! near-black field, Inter/system type, hairline rules instead of filled
//! boxes, and the same semantic status hues (ok green, warn amber, danger
//! red, accent indigo).
//!
//! Every string that originates from the user's plan or from an agent (task
//! text, event text, command lines, diff patches, file paths) is HTML-escaped
//! before it reaches the document — an agent that emits `<script>` must render
//! as text, never execute.

use std::fmt::Write as _;

use loopfleet_gitx::run_cumulative_diff_at;
use loopfleet_store::Connection;

use crate::event::NormalizedEvent;
use crate::task_status::{derive_status, TaskRun as TaskRunState, TaskStatus};
use crate::timeline::{to_diff_view, DiffView};
use crate::RunState;

/// One task's stored data, assembled by the caller for rendering.
#[derive(Debug)]
pub struct TaskReport {
    /// The stable anchor identity (normalized task text).
    pub anchor: String,
    /// The authored task text, verbatim from the plan.
    pub text: String,
    /// The authored `- [x]` state.
    pub checked: bool,
    /// The derived live status (see [`derive_status`](crate::derive_status)).
    pub status: TaskStatus,
    /// Every run bound to this task, in launch order.
    pub runs: Vec<RunReport>,
}

/// One run's stored data: outcome, timings, events, and diff summary.
#[derive(Debug)]
pub struct RunReport {
    pub run_id: String,
    pub agent: String,
    /// The run's lifecycle outcome (its persisted `runs.status`).
    pub outcome: RunState,
    /// Whether this run was accepted ("use this run").
    pub accepted: bool,
    /// Unix seconds when the run started, if recorded.
    pub started_ts: Option<i64>,
    /// Unix seconds when the run reached a terminal state, if recorded.
    pub ended_ts: Option<i64>,
    /// The run's normalized events in log order.
    pub events: Vec<ReportEvent>,
    /// What the run produced against its base (`None` if no snapshot).
    pub diff: Option<DiffView>,
}

/// One event with its timestamp (unix seconds).
#[derive(Debug)]
pub struct ReportEvent {
    pub ts: i64,
    pub event: NormalizedEvent,
}

/// A whole plan's stored data: its identity plus every task.
#[derive(Debug)]
pub struct PlanReport {
    pub title: Option<String>,
    pub file_path: Option<String>,
    pub tasks: Vec<TaskReport>,
}

/// Render one task as a complete standalone HTML document.
pub fn render_task_report(task: &TaskReport) -> String {
    let mut body = String::new();
    push_task_section(&mut body, task);
    document(&format!("Task report — {}", task.text), &body)
}

/// Render a whole plan — every task with its runs — as one standalone HTML
/// document.
pub fn render_plan_report(plan: &PlanReport) -> String {
    let title = plan.title.as_deref().unwrap_or("Plan report");
    let mut body = String::new();
    if let Some(path) = &plan.file_path {
        let _ = writeln!(body, "<p class=\"meta\">{}</p>", escape_html(path));
    }
    for task in &plan.tasks {
        push_task_section(&mut body, task);
    }
    if plan.tasks.is_empty() {
        body.push_str("<p class=\"meta\">No tasks in this plan.</p>\n");
    }
    document(title, &body)
}

// --- assembly ---
//
// The functions below gather a task's or plan's stored data (task/run rows,
// event log, cumulative diffs) into the structs above, for `render_*` to turn
// into HTML. Kept in this module (rather than `lib.rs`) because they are the
// direct data-source counterpart to the renderer — the same "what a report
// contains" decision governs both.

/// Why a report could not be assembled.
#[derive(Debug)]
pub enum ReportError {
    /// No such task or plan is synced.
    NotFound(String),
    /// Reading the plan file failed.
    Io(std::io::Error),
    /// Reading the stored task/run/event data failed.
    Store(rusqlite::Error),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::NotFound(id) => write!(f, "not found: {id}"),
            ReportError::Io(e) => write!(f, "reading plan: {e}"),
            ReportError::Store(e) => write!(f, "reading report data: {e}"),
        }
    }
}

impl std::error::Error for ReportError {}

impl From<rusqlite::Error> for ReportError {
    fn from(e: rusqlite::Error) -> Self {
        ReportError::Store(e)
    }
}

/// Assemble the report for one task: its authored fields, derived status, and
/// every run bound to it (events plus cumulative diff), in launch order.
pub fn task_report(conn: &Connection, plan_id: &str, task_anchor: &str) -> Result<TaskReport, ReportError> {
    let task_row = loopfleet_store::load_task(conn, plan_id, task_anchor)?
        .ok_or_else(|| ReportError::NotFound(task_anchor.to_string()))?;
    let summaries: Vec<_> = loopfleet_store::list_runs_for_plan(conn, plan_id)?
        .into_iter()
        .filter(|s| s.task_anchor == task_anchor)
        .collect();
    let status = derive_status(
        &summaries
            .iter()
            .filter_map(|s| {
                RunState::from_token(&s.status).map(|state| TaskRunState {
                    state,
                    accepted: s.accepted,
                })
            })
            .collect::<Vec<_>>(),
        task_row.checked,
    );
    let runs = summaries
        .iter()
        .map(|s| build_run_report(conn, s))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TaskReport {
        anchor: task_anchor.to_string(),
        text: task_row.text,
        checked: task_row.checked,
        status,
        runs,
    })
}

/// Assemble the report for a whole plan: its identity plus every task's
/// report, in document order.
pub fn plan_report(conn: &Connection, plan_id: &str) -> Result<PlanReport, ReportError> {
    let file_path = loopfleet_store::plan_file_path(conn, plan_id)?
        .ok_or_else(|| ReportError::NotFound(plan_id.to_string()))?;
    let markdown = std::fs::read_to_string(&file_path).map_err(ReportError::Io)?;
    let parsed = crate::plan::parse_plan(&markdown);
    let summaries = loopfleet_store::list_runs_for_plan(conn, plan_id)?;

    let mut tasks = Vec::with_capacity(parsed.tasks.len());
    for t in &parsed.tasks {
        let task_summaries: Vec<_> = summaries
            .iter()
            .filter(|s| s.task_anchor == t.anchor.normalized_text)
            .collect();
        let status = derive_status(
            &task_summaries
                .iter()
                .filter_map(|s| {
                    RunState::from_token(&s.status).map(|state| TaskRunState {
                        state,
                        accepted: s.accepted,
                    })
                })
                .collect::<Vec<_>>(),
            t.checked,
        );
        let runs = task_summaries
            .iter()
            .map(|s| build_run_report(conn, s))
            .collect::<Result<Vec<_>, _>>()?;
        tasks.push(TaskReport {
            anchor: t.anchor.normalized_text.clone(),
            text: t.text.clone(),
            checked: t.checked,
            status,
            runs,
        });
    }

    Ok(PlanReport {
        title: parsed.title,
        file_path: Some(file_path),
        tasks,
    })
}

/// Assemble one run's report: its metadata, first/last event timestamps, every
/// normalized event in log order, and its cumulative diff against the final
/// iteration's shadow ref (`None` if it produced no snapshot).
fn build_run_report(
    conn: &Connection,
    summary: &loopfleet_store::RunSummary,
) -> Result<RunReport, ReportError> {
    let run_id = &summary.id;
    let detail = loopfleet_store::load_run(conn, run_id)?
        .ok_or_else(|| ReportError::NotFound(run_id.clone()))?;
    let iterations = loopfleet_store::load_iterations(conn, run_id)?;
    let stored_events = loopfleet_store::load_events(conn, run_id)?;

    let started_ts = stored_events.first().map(|e| e.ts / 1000);
    let ended_ts = stored_events.last().map(|e| e.ts / 1000);
    let events = stored_events
        .iter()
        .filter_map(|e| {
            serde_json::from_str::<NormalizedEvent>(&e.event_json)
                .ok()
                .map(|event| ReportEvent { ts: e.ts / 1000, event })
        })
        .collect();

    let final_iter = iterations.last();
    let diff = final_iter.and_then(|it| {
        run_cumulative_diff_at(std::path::Path::new(&detail.repo_path), run_id, it.n)
            .ok()
            .map(to_diff_view)
    });

    Ok(RunReport {
        run_id: run_id.clone(),
        agent: detail.agent,
        outcome: RunState::from_token(&detail.status)
            .ok_or_else(|| ReportError::NotFound(format!("unknown run status: {}", detail.status)))?,
        accepted: summary.accepted,
        started_ts,
        ended_ts,
        events,
        diff,
    })
}

// --- sections ---

fn push_task_section(out: &mut String, task: &TaskReport) {
    let (class, label) = task_status_style(task.status);
    let _ = write!(
        out,
        "<section class=\"task\">\n\
         <header class=\"task-head\">\n\
         <h2>{text}</h2>\n\
         <span class=\"pill {class}\">{label}</span>\n\
         </header>\n\
         <p class=\"meta\">anchor: <code>{anchor}</code>{checked}</p>\n",
        text = escape_html(&task.text),
        class = class,
        label = label,
        anchor = escape_html(&task.anchor),
        checked = if task.checked {
            " · authored checked"
        } else {
            ""
        },
    );

    if task.runs.is_empty() {
        out.push_str("<p class=\"meta\">No runs.</p>\n");
    }
    for run in &task.runs {
        push_run_section(out, run);
    }
    out.push_str("</section>\n");
}

fn push_run_section(out: &mut String, run: &RunReport) {
    let (class, label) = run_outcome_style(run.outcome);
    let _ = write!(
        out,
        "<section class=\"run\">\n\
         <header class=\"run-head\">\n\
         <h3>run <code>{id}</code> <span class=\"meta\">({agent})</span></h3>\n\
         <span class=\"pill {class}\">{label}</span>{accepted}\n\
         </header>\n",
        id = escape_html(&run.run_id),
        agent = escape_html(&run.agent),
        class = class,
        label = label,
        accepted = if run.accepted {
            "\n<span class=\"pill st-ok\">accepted</span>"
        } else {
            ""
        },
    );

    // Timings: absolute UTC start/end when recorded, plus the duration when
    // both ends are known.
    let mut timing = Vec::new();
    if let Some(ts) = run.started_ts {
        timing.push(format!("started {}", format_utc(ts)));
    }
    if let Some(ts) = run.ended_ts {
        timing.push(format!("ended {}", format_utc(ts)));
    }
    if let (Some(a), Some(b)) = (run.started_ts, run.ended_ts) {
        timing.push(format!("took {}", format_duration(b.saturating_sub(a))));
    }
    if !timing.is_empty() {
        let _ = writeln!(out, "<p class=\"meta\">{}</p>", timing.join(" · "));
    }

    if !run.events.is_empty() {
        out.push_str("<table class=\"events\">\n");
        for ev in &run.events {
            push_event_row(out, ev);
        }
        out.push_str("</table>\n");
    }

    if let Some(diff) = &run.diff {
        push_diff_section(out, diff);
    }
    out.push_str("</section>\n");
}

fn push_event_row(out: &mut String, ev: &ReportEvent) {
    let (class, label, detail) = event_style(&ev.event);
    let _ = writeln!(
        out,
        "<tr class=\"event\">\
         <td class=\"ev-ts\">{ts}</td>\
         <td class=\"ev-kind {class}\">{label}</td>\
         <td class=\"ev-detail\">{detail}</td>\
         </tr>",
        ts = format_utc_time(ev.ts),
        class = class,
        label = label,
        detail = detail,
    );
}

fn push_diff_section(out: &mut String, diff: &DiffView) {
    let (ins, del): (usize, usize) = diff
        .files
        .iter()
        .fold((0, 0), |(i, d), f| (i + f.insertions, d + f.deletions));
    let _ = write!(
        out,
        "<div class=\"diff\">\n\
         <p class=\"meta\">{n} file{s} changed, \
         <span class=\"ins\">+{ins}</span> \
         <span class=\"del\">\u{2212}{del}</span></p>\n\
         <table class=\"files\">\n",
        n = diff.files.len(),
        s = if diff.files.len() == 1 { "" } else { "s" },
        ins = ins,
        del = del,
    );
    for f in &diff.files {
        let path = match &f.old_path {
            Some(old) => format!("{} \u{2192} {}", escape_html(old), escape_html(&f.path)),
            None => escape_html(&f.path),
        };
        let _ = writeln!(
            out,
            "<tr><td class=\"f-status\">{status}</td>\
             <td class=\"f-path\"><code>{path}</code></td>\
             <td class=\"f-counts\"><span class=\"ins\">+{ins}</span> \
             <span class=\"del\">\u{2212}{del}</span></td></tr>",
            status = escape_html(&f.status),
            path = path,
            ins = f.insertions,
            del = f.deletions,
        );
    }
    out.push_str("</table>\n");
    if !diff.patch.is_empty() {
        let _ = writeln!(
            out,
            "<details><summary>patch</summary>\
             <pre class=\"patch\">{}</pre></details>",
            escape_html(&diff.patch)
        );
    }
    out.push_str("</div>\n");
}

// --- styling maps ---

/// The pill class + label for a derived task status. The classes carry the
/// app's semantic hues: ok green for accepted, warn amber for the review
/// queue, accent indigo for live work, muted grey for untouched.
fn task_status_style(status: TaskStatus) -> (&'static str, &'static str) {
    match status {
        TaskStatus::NotStarted => ("st-muted", "not started"),
        TaskStatus::InProgress => ("st-accent", "in progress"),
        TaskStatus::CompletedUnaccepted => ("st-warn", "completed \u{b7} unaccepted"),
        TaskStatus::Accepted => ("st-ok", "accepted"),
    }
}

/// The pill class + label for a run outcome.
fn run_outcome_style(outcome: RunState) -> (&'static str, &'static str) {
    match outcome {
        RunState::Queued => ("st-accent", "queued"),
        RunState::Running => ("st-accent", "running"),
        RunState::Completed => ("st-ok", "completed"),
        RunState::Failed => ("st-danger", "failed"),
        RunState::Stopped => ("st-warn", "stopped"),
        RunState::LimitReached => ("st-warn", "limit reached"),
    }
}

/// One event's row: kind class, kind label, and an escaped detail cell.
fn event_style(ev: &NormalizedEvent) -> (&'static str, &'static str, String) {
    match ev {
        NormalizedEvent::TurnStarted => ("st-muted", "turn started", String::new()),
        NormalizedEvent::AssistantText { text } => ("st-text", "assistant", escape_html(text)),
        NormalizedEvent::Reasoning { text } => ("st-muted", "reasoning", escape_html(text)),
        NormalizedEvent::ToolCall {
            name,
            input_excerpt,
            ..
        } => (
            "st-accent",
            "tool call",
            format!(
                "<code>{}</code> {}",
                escape_html(name),
                escape_html(input_excerpt)
            ),
        ),
        NormalizedEvent::ToolResult {
            ok, output_excerpt, ..
        } => (
            if *ok { "st-muted" } else { "st-danger" },
            "tool result",
            escape_html(output_excerpt),
        ),
        NormalizedEvent::CommandRun { cmd, exit } => (
            match exit {
                Some(0) => "st-muted",
                Some(_) => "st-danger",
                None => "st-warn",
            },
            "command",
            format!(
                "<code>{}</code>{}",
                escape_html(cmd),
                match exit {
                    Some(code) => format!(" (exit {code})"),
                    None => " (no exit code)".into(),
                }
            ),
        ),
        NormalizedEvent::TurnCompleted { usage } => (
            "st-muted",
            "turn completed",
            format!(
                "{} in / {} out tokens",
                usage.input_tokens, usage.output_tokens
            ),
        ),
        NormalizedEvent::NeedsApproval => ("st-warn", "needs approval", String::new()),
        NormalizedEvent::Failed { reason } => ("st-danger", "failed", escape_html(reason)),
        NormalizedEvent::RateLimited { reset_at, message } => (
            "st-warn",
            "rate limited",
            [
                message.as_deref().map(escape_html),
                reset_at
                    .as_deref()
                    .map(|t| format!("resets {}", escape_html(t))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{b7} "),
        ),
        NormalizedEvent::Ended => ("st-muted", "ended", String::new()),
        NormalizedEvent::FileChanged { path } => (
            "st-accent",
            "file changed",
            format!("<code>{}</code>", escape_html(&path.to_string_lossy())),
        ),
    }
}

// --- document shell ---

/// Wrap rendered body sections in the full standalone document: doctype, the
/// inlined stylesheet, and the page header. `title` is unescaped caller text.
fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<style>{css}</style>\n</head>\n<body>\n\
         <main>\n<h1>{title}</h1>\n{body}</main>\n</body>\n</html>\n",
        title = escape_html(title),
        css = STYLE,
        body = body,
    )
}

/// The inlined stylesheet: the app's design tokens flattened into one sheet.
/// Values mirror `frontend/src/tokens.css` (single dark theme; a report is a
/// snapshot of the same cockpit, so it reads the same).
const STYLE: &str = "\
:root{color-scheme:dark}\
*{box-sizing:border-box}\
body{margin:0;background:#0a0a0b;color:#ececee;\
font-family:'Inter Variable','Inter',-apple-system,BlinkMacSystemFont,\
'SF Pro Text',system-ui,sans-serif;\
font-size:13px;line-height:1.5;letter-spacing:-0.006em;\
-webkit-font-smoothing:antialiased}\
main{max-width:820px;margin:0 auto;padding:32px 24px 48px}\
h1,h2,h3{margin:0;line-height:1.3;letter-spacing:-0.02em;font-weight:600}\
h1{font-size:22px;margin-bottom:24px}\
h2{font-size:15px}\
h3{font-size:13px;font-weight:600}\
code,pre{font-family:'SF Mono',ui-monospace,'JetBrains Mono',Menlo,monospace;\
font-size:12px}\
.meta{color:#a0a0a8;font-size:12px;margin:4px 0}\
section.task{border-top:1px solid rgba(255,255,255,0.07);\
padding:16px 0;margin-top:16px}\
section.run{border:1px solid rgba(255,255,255,0.07);border-radius:10px;\
padding:12px 16px;margin:12px 0;background:#141416}\
.task-head,.run-head{display:flex;align-items:center;gap:8px;flex-wrap:wrap}\
.pill{display:inline-block;padding:1px 8px;border-radius:8px;font-size:11px;\
border:1px solid rgba(255,255,255,0.12)}\
.st-ok{color:#3fb950;border-color:rgba(63,185,80,0.14);\
background:rgba(63,185,80,0.08)}\
.st-warn{color:#d29922;border-color:rgba(210,153,34,0.14);\
background:rgba(210,153,34,0.08)}\
.st-danger{color:#f85149;border-color:rgba(248,81,73,0.15);\
background:rgba(248,81,73,0.08)}\
.st-accent{color:#98a5ff;border-color:rgba(98,117,242,0.32);\
background:#23294d}\
.st-muted{color:#85858c}\
.st-text{color:#ececee}\
table{border-collapse:collapse;width:100%;margin:8px 0}\
td{padding:3px 8px 3px 0;vertical-align:top;\
border-top:1px solid rgba(255,255,255,0.07)}\
.ev-ts{color:#85858c;font-size:11px;white-space:nowrap;width:1%}\
.ev-kind{white-space:nowrap;font-size:12px;width:1%}\
.ev-detail{color:#a0a0a8;font-size:12px;overflow-wrap:anywhere}\
.f-status{color:#85858c;font-size:11px;text-transform:uppercase;\
letter-spacing:0.06em;white-space:nowrap;width:1%}\
.f-counts{white-space:nowrap;width:1%;font-size:12px}\
.ins{color:#3fb950}\
.del{color:#f85149}\
details{margin:8px 0}\
summary{color:#a0a0a8;font-size:12px;cursor:pointer}\
pre.patch{background:#0a0a0b;border:1px solid rgba(255,255,255,0.07);\
border-radius:8px;padding:12px;overflow-x:auto;white-space:pre-wrap;\
overflow-wrap:anywhere;color:#a0a0a8}\
";

// --- helpers ---

/// Escape a string for safe interpolation into HTML text or attribute values.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Unix seconds → `YYYY-MM-DD HH:MM:SS UTC`. Pure civil-calendar arithmetic
/// (Howard Hinnant's `civil_from_days`), so no time crate is pulled in for a
/// report timestamp.
fn format_utc(ts: i64) -> String {
    let (date, time) = split_utc(ts);
    format!("{date} {time} UTC")
}

/// Unix seconds → just the `HH:MM:SS` part, for compact event rows.
fn format_utc_time(ts: i64) -> String {
    split_utc(ts).1
}

fn split_utc(ts: i64) -> (String, String) {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);

    // civil_from_days: days since 1970-01-01 → (y, m, d).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };

    (
        format!("{y:04}-{mo:02}-{d:02}"),
        format!("{h:02}:{m:02}:{s:02}"),
    )
}

/// Seconds → a compact human duration (`45s`, `3m 07s`, `1h 02m`).
fn format_duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs / 60) % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Usage;
    use crate::timeline::FileChangeView;

    fn run(id: &str, outcome: RunState) -> RunReport {
        RunReport {
            run_id: id.into(),
            agent: "claude".into(),
            outcome,
            accepted: false,
            started_ts: None,
            ended_ts: None,
            events: Vec::new(),
            diff: None,
        }
    }

    fn task(text: &str, status: TaskStatus, runs: Vec<RunReport>) -> TaskReport {
        TaskReport {
            anchor: text.to_lowercase(),
            text: text.into(),
            checked: false,
            status,
            runs,
        }
    }

    #[test]
    fn escapes_user_and_agent_content() {
        let mut r = run("r1", RunState::Completed);
        r.events = vec![
            ReportEvent {
                ts: 0,
                event: NormalizedEvent::AssistantText {
                    text: "<script>alert('xss')</script>".into(),
                },
            },
            ReportEvent {
                ts: 1,
                event: NormalizedEvent::CommandRun {
                    cmd: "echo \"<b>\" && true".into(),
                    exit: Some(0),
                },
            },
        ];
        r.diff = Some(DiffView {
            files: vec![FileChangeView {
                path: "src/<odd>.rs".into(),
                old_path: None,
                status: "added".into(),
                insertions: 1,
                deletions: 0,
            }],
            patch: "+let x = a < b && c > d;".into(),
        });
        let t = task(
            "Add <em>tags</em> & \"quotes\"",
            TaskStatus::Accepted,
            vec![r],
        );

        let html = render_task_report(&t);
        // No unescaped payloads anywhere in the document.
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<em>tags</em>"));
        assert!(!html.contains("<b>"));
        assert!(!html.contains("src/<odd>"));
        // The escaped forms are present.
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
        assert!(html.contains("Add &lt;em&gt;tags&lt;/em&gt; &amp; &quot;quotes&quot;"));
        assert!(html.contains("echo &quot;&lt;b&gt;&quot; &amp;&amp; true"));
        assert!(html.contains("src/&lt;odd&gt;.rs"));
        assert!(html.contains("+let x = a &lt; b &amp;&amp; c &gt; d;"));
    }

    #[test]
    fn is_a_complete_standalone_document_in_the_app_language() {
        let html = render_task_report(&task("alpha", TaskStatus::NotStarted, vec![]));
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<style>"));
        assert!(html.trim_end().ends_with("</html>"));
        // Black canvas + Inter/system stack + semantic hues from tokens.css.
        assert!(html.contains("background:#0a0a0b"));
        assert!(html.contains("'Inter Variable','Inter'"));
        assert!(html.contains("#3fb950")); // ok
        assert!(html.contains("#f85149")); // danger
                                           // No external assets: a standalone report never fetches anything.
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
        assert!(!html.contains("<link"));
        assert!(!html.contains("src="));
    }

    #[test]
    fn renders_one_section_per_run() {
        let t = task(
            "alpha",
            TaskStatus::CompletedUnaccepted,
            vec![run("r1", RunState::Completed), run("r2", RunState::Failed)],
        );
        let html = render_task_report(&t);
        assert_eq!(html.matches("<section class=\"run\">").count(), 2);
        assert!(html.contains("<code>r1</code>"));
        assert!(html.contains("<code>r2</code>"));
        // Runs are rendered in the order given.
        assert!(html.find("r1").unwrap() < html.find("r2").unwrap());
    }

    #[test]
    fn outcomes_get_their_semantic_classes() {
        let cases = [
            (RunState::Completed, "st-ok", "completed"),
            (RunState::Failed, "st-danger", "failed"),
            (RunState::Stopped, "st-warn", "stopped"),
            (RunState::LimitReached, "st-warn", "limit reached"),
            (RunState::Running, "st-accent", "running"),
            (RunState::Queued, "st-accent", "queued"),
        ];
        for (outcome, class, label) in cases {
            let html = render_task_report(&task(
                "alpha",
                TaskStatus::InProgress,
                vec![run("r", outcome)],
            ));
            let pill = format!("<span class=\"pill {class}\">{label}</span>");
            assert!(html.contains(&pill), "missing {pill}");
        }
    }

    #[test]
    fn task_status_pill_reflects_derived_status() {
        for (status, class, label) in [
            (TaskStatus::NotStarted, "st-muted", "not started"),
            (TaskStatus::InProgress, "st-accent", "in progress"),
            (
                TaskStatus::CompletedUnaccepted,
                "st-warn",
                "completed \u{b7} unaccepted",
            ),
            (TaskStatus::Accepted, "st-ok", "accepted"),
        ] {
            let html = render_task_report(&task("alpha", status, vec![]));
            assert!(html.contains(&format!("<span class=\"pill {class}\">{label}</span>")));
        }
    }

    #[test]
    fn accepted_run_shows_its_own_ok_pill() {
        let mut r = run("r1", RunState::Completed);
        r.accepted = true;
        let html = render_task_report(&task("alpha", TaskStatus::Accepted, vec![r]));
        assert!(html.contains("<span class=\"pill st-ok\">accepted</span>"));
    }

    #[test]
    fn renders_timings_and_duration() {
        let mut r = run("r1", RunState::Completed);
        r.started_ts = Some(1_700_000_000); // 2023-11-14 22:13:20 UTC
        r.ended_ts = Some(1_700_000_000 + 187);
        let html = render_task_report(&task("alpha", TaskStatus::Accepted, vec![r]));
        assert!(html.contains("started 2023-11-14 22:13:20 UTC"));
        assert!(html.contains("ended 2023-11-14 22:16:27 UTC"));
        assert!(html.contains("took 3m 07s"));
    }

    #[test]
    fn renders_events_with_kinds_and_details() {
        let mut r = run("r1", RunState::Failed);
        r.events = vec![
            ReportEvent {
                ts: 60,
                event: NormalizedEvent::TurnCompleted {
                    usage: Usage {
                        input_tokens: 12,
                        output_tokens: 34,
                    },
                },
            },
            ReportEvent {
                ts: 61,
                event: NormalizedEvent::CommandRun {
                    cmd: "cargo test".into(),
                    exit: Some(101),
                },
            },
            ReportEvent {
                ts: 62,
                event: NormalizedEvent::Failed {
                    reason: "boom".into(),
                },
            },
        ];
        let html = render_task_report(&task("alpha", TaskStatus::NotStarted, vec![r]));
        assert!(html.contains("12 in / 34 out tokens"));
        // A non-zero exit and a failure event both take the danger hue.
        assert!(html.contains("class=\"ev-kind st-danger\">command</td>"));
        assert!(html.contains("(exit 101)"));
        assert!(html.contains("class=\"ev-kind st-danger\">failed</td>"));
        assert!(html.contains("boom"));
        // Event rows carry a wall-clock time.
        assert!(html.contains("<td class=\"ev-ts\">00:01:00</td>"));
    }

    #[test]
    fn renders_diff_summary_with_totals() {
        let mut r = run("r1", RunState::Completed);
        r.diff = Some(DiffView {
            files: vec![
                FileChangeView {
                    path: "src/lib.rs".into(),
                    old_path: None,
                    status: "modified".into(),
                    insertions: 10,
                    deletions: 2,
                },
                FileChangeView {
                    path: "src/new.rs".into(),
                    old_path: Some("src/old.rs".into()),
                    status: "renamed".into(),
                    insertions: 1,
                    deletions: 1,
                },
            ],
            patch: String::new(),
        });
        let html = render_task_report(&task("alpha", TaskStatus::Accepted, vec![r]));
        assert!(html.contains("2 files changed"));
        assert!(html.contains("<span class=\"ins\">+11</span>"));
        assert!(html.contains("<span class=\"del\">\u{2212}3</span>"));
        assert!(html.contains("src/lib.rs"));
        assert!(html.contains("src/old.rs \u{2192} src/new.rs"));
        // An empty patch renders no <details> block.
        assert!(!html.contains("<details>"));
    }

    #[test]
    fn plan_report_covers_every_task() {
        let plan = PlanReport {
            title: Some("My Plan".into()),
            file_path: Some("/repo/PRD.md".into()),
            tasks: vec![
                task(
                    "alpha",
                    TaskStatus::Accepted,
                    vec![run("r1", RunState::Completed)],
                ),
                task("beta", TaskStatus::NotStarted, vec![]),
            ],
        };
        let html = render_plan_report(&plan);
        assert!(html.contains("<title>My Plan</title>"));
        assert!(html.contains("/repo/PRD.md"));
        assert_eq!(html.matches("<section class=\"task\">").count(), 2);
        assert!(html.contains("<h2>alpha</h2>"));
        assert!(html.contains("<h2>beta</h2>"));
        assert!(html.contains("No runs.")); // beta has none
    }

    #[test]
    fn empty_plan_says_so() {
        let html = render_plan_report(&PlanReport {
            title: None,
            file_path: None,
            tasks: vec![],
        });
        assert!(html.contains("<h1>Plan report</h1>"));
        assert!(html.contains("No tasks in this plan."));
    }
}
