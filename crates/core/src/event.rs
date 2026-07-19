//! The normalized event enum: the single vocabulary every downstream consumer
//! (run timeline, diff capture, plan chat) speaks. Adapters map each agent's
//! native stream into these variants and nothing downstream ever learns which
//! agent produced an event.
//!
//! Events arrive in two lanes (see [`Lane`]):
//! - **adapter-sourced** — mapped from the agent's stream by an `AgentAdapter`.
//! - **app-sourced** — emitted by the app itself, never parsed from the agent
//!   stream. Currently just [`FileChanged`], observed from worktree watching so
//!   it stays reliable across agents and catches shell-command edits too.
//!
//! `ToolCall` / `ToolResult` are correlated by `call_id`. `CommandRun` is a
//! deliberate normalization of every agent's shell-exec tool (each names it
//! differently); all non-shell tools go through the generic `ToolCall` /
//! `ToolResult` pair.
//!
//! [`FileChanged`]: NormalizedEvent::FileChanged

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Token usage an agent reports when a turn completes. Agents report different
/// subsets; unreported counts default to zero.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Which lane produced a [`NormalizedEvent`]. The app owns the app-sourced lane;
/// adapters own everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Adapter,
    App,
}

/// The normalized event. Serialized to the event log as JSON tagged by `kind`
/// (e.g. `{"kind":"tool_call","call_id":"…",…}`), which is what the store's
/// `normalized_event_json` column holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NormalizedEvent {
    // --- adapter-sourced lane ---
    /// A new agent turn began.
    TurnStarted,
    /// Assistant-visible text emitted during the turn.
    AssistantText { text: String },
    /// Model reasoning / thinking text, when the agent exposes it.
    Reasoning { text: String },
    /// The agent invoked a (non-shell) tool. Correlates to a `ToolResult` by
    /// `call_id`.
    ToolCall {
        call_id: String,
        name: String,
        input_excerpt: String,
    },
    /// The result of a prior `ToolCall`, correlated by `call_id`.
    ToolResult {
        call_id: String,
        ok: bool,
        output_excerpt: String,
    },
    /// A shell command ran. `exit` is the process exit code, absent if the
    /// command was killed or never produced one.
    CommandRun { cmd: String, exit: Option<i32> },
    /// The turn finished; carries the usage the agent reported for it.
    TurnCompleted { usage: Usage },
    /// The agent is asking for approval. Only fires in interactive sessions
    /// (M5); headless runs never surface it — the sandbox is the boundary.
    NeedsApproval,
    /// The agent failed; `reason` is a human-readable description.
    Failed { reason: String },
    /// The agent hit a rate limit. `reset_at` is an ISO-8601 timestamp (when
    /// the limit resets) when the agent provides one; `message` carries details
    /// from the agent (e.g. which limit was exceeded).
    RateLimited {
        reset_at: Option<String>,
        message: Option<String>,
    },
    /// The agent stream ended.
    Ended,

    // --- app-sourced lane ---
    /// A worktree file changed, observed by the app (git status / fs events),
    /// never parsed from the agent stream.
    FileChanged { path: PathBuf },
}

impl NormalizedEvent {
    /// Which lane this event belongs to.
    pub fn lane(&self) -> Lane {
        match self {
            NormalizedEvent::FileChanged { .. } => Lane::App,
            _ => Lane::Adapter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant round-trips through JSON unchanged.
    #[test]
    fn round_trips_every_variant() {
        let events = vec![
            NormalizedEvent::TurnStarted,
            NormalizedEvent::AssistantText {
                text: "hello".into(),
            },
            NormalizedEvent::Reasoning {
                text: "let me think".into(),
            },
            NormalizedEvent::ToolCall {
                call_id: "c1".into(),
                name: "read_file".into(),
                input_excerpt: "{\"path\":\"a.rs\"}".into(),
            },
            NormalizedEvent::ToolResult {
                call_id: "c1".into(),
                ok: true,
                output_excerpt: "fn main() {}".into(),
            },
            NormalizedEvent::CommandRun {
                cmd: "cargo build".into(),
                exit: Some(0),
            },
            NormalizedEvent::CommandRun {
                cmd: "sleep 999".into(),
                exit: None,
            },
            NormalizedEvent::TurnCompleted {
                usage: Usage {
                    input_tokens: 12,
                    output_tokens: 34,
                },
            },
            NormalizedEvent::NeedsApproval,
            NormalizedEvent::RateLimited {
                reset_at: Some("2025-01-15T10:30:00Z".into()),
                message: Some("rate limit hit".into()),
            },
            NormalizedEvent::RateLimited {
                reset_at: None,
                message: None,
            },
            NormalizedEvent::Failed {
                reason: "boom".into(),
            },
            NormalizedEvent::Ended,
            NormalizedEvent::FileChanged {
                path: PathBuf::from("src/lib.rs"),
            },
        ];

        for ev in events {
            let json = serde_json::to_string(&ev).unwrap();
            let back: NormalizedEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(ev, back, "round-trip mismatch for {json}");
        }
    }

    /// The `kind` tag is the stable on-disk discriminator (snake_case).
    #[test]
    fn kind_tag_shape_is_stable() {
        let v = serde_json::to_value(&NormalizedEvent::ToolCall {
            call_id: "c1".into(),
            name: "grep".into(),
            input_excerpt: "x".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], "tool_call");
        assert_eq!(v["call_id"], "c1");

        let unit = serde_json::to_value(&NormalizedEvent::Ended).unwrap();
        assert_eq!(unit["kind"], "ended");
    }

    /// A `ToolResult` correlates to its `ToolCall` by `call_id` across a
    /// serialize / deserialize boundary.
    #[test]
    fn tool_call_and_result_correlate_by_call_id() {
        let call = NormalizedEvent::ToolCall {
            call_id: "abc".into(),
            name: "read_file".into(),
            input_excerpt: "in".into(),
        };
        let result = NormalizedEvent::ToolResult {
            call_id: "abc".into(),
            ok: true,
            output_excerpt: "out".into(),
        };

        let call: NormalizedEvent =
            serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
        let result: NormalizedEvent =
            serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();

        match (call, result) {
            (
                NormalizedEvent::ToolCall { call_id: a, .. },
                NormalizedEvent::ToolResult { call_id: b, .. },
            ) => assert_eq!(a, b),
            _ => panic!("unexpected variants"),
        }
    }

    /// Only `FileChanged` is app-sourced; everything else is adapter-sourced.
    #[test]
    fn lanes_classify_correctly() {
        assert_eq!(
            NormalizedEvent::FileChanged {
                path: PathBuf::from("a")
            }
            .lane(),
            Lane::App
        );
        assert_eq!(NormalizedEvent::TurnStarted.lane(), Lane::Adapter);
        assert_eq!(
            NormalizedEvent::Failed { reason: "x".into() }.lane(),
            Lane::Adapter
        );
        assert_eq!(
            NormalizedEvent::RateLimited {
                reset_at: None,
                message: None,
            }
            .lane(),
            Lane::Adapter
        );
    }
}
