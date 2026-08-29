//! Normalized per-agent usage/limit state: one snapshot type every agent's
//! limit reporting collapses into, plus the pure functions that fold, age, and
//! resolve it for display.
//!
//! Agents disagree about limits. Some report a percentage against a named
//! window ("weekly"), some only ever say "you are rate limited, try again at
//! T", and some say nothing at all until they fail. Rather than teach the UI
//! three dialects, adapters produce a [`UsageSnapshot`]: an agent key, an
//! optional model and limit-window label, a used fraction in `0.0..=1.0`, an
//! optional reset instant, the instant the snapshot was observed, and a
//! [`UsageSource`] recording how much of that is the agent's word
//! ([`Reported`]) versus our reading of a rate-limit notice ([`Inferred`])
//! versus nothing ([`Unknown`]).
//!
//! Instants are Unix-epoch milliseconds, matching the store's timestamps. Every
//! function here takes `now` as an argument instead of reading the clock, so
//! the whole module is pure and directly testable.
//!
//! [`Reported`]: UsageSource::Reported
//! [`Inferred`]: UsageSource::Inferred
//! [`Unknown`]: UsageSource::Unknown

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::event::{NormalizedEvent, Usage};

/// A run's total token usage: the sum of every `TurnCompleted` usage its event
/// log recorded, across all its iterations. `None` if the run has completed no
/// turns yet (a fresh run, or one that failed before its first `TurnCompleted`).
pub fn usage_for_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<Usage>> {
    let events = loopfleet_store::load_events(conn, run_id)?;
    let total = events
        .iter()
        .filter_map(|e| match serde_json::from_str(&e.event_json) {
            Ok(NormalizedEvent::TurnCompleted { usage }) => Some(usage),
            _ => None,
        })
        .fold(None, |acc: Option<Usage>, usage| {
            Some(match acc {
                Some(acc) => Usage {
                    input_tokens: acc.input_tokens + usage.input_tokens,
                    output_tokens: acc.output_tokens + usage.output_tokens,
                },
                None => usage,
            })
        });
    Ok(total)
}

/// A used fraction at or above this reads as exhausted.
pub const EXHAUSTED_FRACTION: f64 = 1.0;

/// Default fraction at which usage starts reading as [`UsageDisplay::Low`].
pub const DEFAULT_LOW_FRACTION: f64 = 0.8;

/// Default age (15 minutes) after which a snapshot no longer describes reality.
pub const DEFAULT_STALE_AFTER_MS: i64 = 15 * 60 * 1_000;

/// How a snapshot's numbers were come by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    /// The agent reported the figure itself (a percentage, a quota readout).
    Reported,
    /// We derived the figure from an observed rate-limit notice — the agent
    /// told us it was blocked, not how much of the window it had used.
    Inferred,
    /// Nothing is known: the agent has never said anything about limits.
    Unknown,
}

/// A normalized view of one agent's limit consumption at a point in time.
///
/// `used_fraction` is always clamped to `0.0..=1.0` by the constructors, so
/// consumers may compare it against [`DEFAULT_LOW_FRACTION`] /
/// [`EXHAUSTED_FRACTION`] without re-validating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    /// Which agent this describes (e.g. `"claude"`), the identity adapters key
    /// their configuration by.
    pub agent_key: String,
    /// The model the limit applies to, when the agent scopes limits per model.
    pub model: Option<String>,
    /// The limit window's label as the agent names it (e.g. `"5h"`,
    /// `"weekly"`), when it names one.
    pub limit_window: Option<String>,
    /// Fraction of the window consumed, in `0.0..=1.0`.
    pub used_fraction: f64,
    /// When the window resets, in epoch millis, when known.
    pub reset_at_ms: Option<i64>,
    /// When this snapshot was observed, in epoch millis.
    pub observed_at_ms: i64,
    /// How much of the above is the agent's word.
    pub source: UsageSource,
}

impl UsageSnapshot {
    /// A snapshot carrying a figure the agent reported itself.
    pub fn reported(agent_key: impl Into<String>, used_fraction: f64, observed_at_ms: i64) -> Self {
        UsageSnapshot {
            agent_key: agent_key.into(),
            model: None,
            limit_window: None,
            used_fraction: clamp_fraction(used_fraction),
            reset_at_ms: None,
            observed_at_ms,
            source: UsageSource::Reported,
        }
    }

    /// A snapshot for an agent that has told us nothing. Zero used, but the
    /// [`Unknown`] source keeps that zero from reading as "plenty left".
    ///
    /// [`Unknown`]: UsageSource::Unknown
    pub fn unknown(agent_key: impl Into<String>, observed_at_ms: i64) -> Self {
        UsageSnapshot {
            agent_key: agent_key.into(),
            model: None,
            limit_window: None,
            used_fraction: 0.0,
            reset_at_ms: None,
            observed_at_ms,
            source: UsageSource::Unknown,
        }
    }

    /// Builder-style: attach the model the limit is scoped to.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Builder-style: attach the limit window's label.
    pub fn with_limit_window(mut self, window: impl Into<String>) -> Self {
        self.limit_window = Some(window.into());
        self
    }

    /// Builder-style: attach the reset instant (epoch millis).
    pub fn with_reset_at(mut self, reset_at_ms: i64) -> Self {
        self.reset_at_ms = Some(reset_at_ms);
        self
    }
}

/// A rate-limit notice as observed from an agent's stream, already parsed out
/// of whatever dialect the agent speaks.
///
/// `used_fraction` is `Some` only when the agent volunteered a number; the
/// common case is `None`, meaning "blocked, no figure given".
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitNotice {
    pub agent_key: String,
    pub model: Option<String>,
    pub limit_window: Option<String>,
    pub used_fraction: Option<f64>,
    pub reset_at_ms: Option<i64>,
    pub observed_at_ms: i64,
}

impl RateLimitNotice {
    /// The bare notice: this agent is limited, nothing else known.
    pub fn new(agent_key: impl Into<String>, observed_at_ms: i64) -> Self {
        RateLimitNotice {
            agent_key: agent_key.into(),
            model: None,
            limit_window: None,
            used_fraction: None,
            reset_at_ms: None,
            observed_at_ms,
        }
    }

    /// Builder-style: the fraction the agent reported alongside the notice.
    pub fn with_used_fraction(mut self, used_fraction: f64) -> Self {
        self.used_fraction = Some(used_fraction);
        self
    }

    /// Builder-style: attach the reset instant (epoch millis).
    pub fn with_reset_at(mut self, reset_at_ms: i64) -> Self {
        self.reset_at_ms = Some(reset_at_ms);
        self
    }

    /// Builder-style: attach the model the notice names.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Builder-style: attach the limit window the notice names.
    pub fn with_limit_window(mut self, window: impl Into<String>) -> Self {
        self.limit_window = Some(window.into());
        self
    }
}

/// Fold an observed rate-limit notice into the agent's snapshot.
///
/// `prior` is the snapshot we already held for this agent, if any. Rules:
/// - A notice for a different agent is not ours to apply: `prior` is returned
///   untouched (or, with no prior, an [`UsageSource::Unknown`] snapshot for the
///   notice's agent — never a claim about the wrong agent).
/// - A notice older than the prior observation is ignored; out-of-order
///   delivery must not walk state backwards.
/// - A notice carrying a fraction is the agent's word: [`Reported`].
///   A bare notice means blocked without a figure, which we read as exhausted:
///   `1.0`, [`Inferred`].
/// - Labels (model, limit window) and the reset instant come from the notice
///   when it names them, otherwise the prior's are carried forward — a terse
///   notice should not erase what a richer earlier one told us.
///
/// [`Reported`]: UsageSource::Reported
/// [`Inferred`]: UsageSource::Inferred
pub fn fold_rate_limit(prior: Option<&UsageSnapshot>, notice: &RateLimitNotice) -> UsageSnapshot {
    let prior = prior.filter(|p| p.agent_key == notice.agent_key);
    if let Some(prior) = prior {
        if notice.observed_at_ms < prior.observed_at_ms {
            return prior.clone();
        }
    }

    let (used_fraction, source) = match notice.used_fraction {
        Some(fraction) => (clamp_fraction(fraction), UsageSource::Reported),
        None => (EXHAUSTED_FRACTION, UsageSource::Inferred),
    };

    UsageSnapshot {
        agent_key: notice.agent_key.clone(),
        model: notice
            .model
            .clone()
            .or_else(|| prior.and_then(|p| p.model.clone())),
        limit_window: notice
            .limit_window
            .clone()
            .or_else(|| prior.and_then(|p| p.limit_window.clone())),
        used_fraction,
        reset_at_ms: notice
            .reset_at_ms
            .or_else(|| prior.and_then(|p| p.reset_at_ms)),
        observed_at_ms: notice.observed_at_ms,
        source,
    }
}

/// Whether a snapshot has stopped describing reality as of `now`.
///
/// Two ways to go stale: the observation aged past `stale_after_ms`, or the
/// window it measured has since reset (a "97% used" from before the reset says
/// nothing about the fresh window). A clock that runs backwards — `now` before
/// the observation — is not treated as staleness; the snapshot is simply the
/// newest thing we have.
pub fn is_stale(snapshot: &UsageSnapshot, now_ms: i64, stale_after_ms: i64) -> bool {
    if let Some(reset_at_ms) = snapshot.reset_at_ms {
        if now_ms >= reset_at_ms && reset_at_ms >= snapshot.observed_at_ms {
            return true;
        }
    }
    now_ms.saturating_sub(snapshot.observed_at_ms) > stale_after_ms
}

/// What the UI should show for an agent's limit state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageDisplay {
    /// Room to spare; launching is unremarkable.
    Available,
    /// Close enough to the limit to warn before launching.
    Low,
    /// The window is spent; launches will hit the limit.
    Exhausted,
    /// We do not know — never reported, or what we knew went stale. Never
    /// dressed up as `Available`.
    Unknown,
}

/// Where the boundaries between display states sit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsageThresholds {
    /// Used fraction at or above which usage reads as [`UsageDisplay::Low`].
    pub low_fraction: f64,
    /// Age at which a snapshot stops being trusted, in millis.
    pub stale_after_ms: i64,
}

impl Default for UsageThresholds {
    fn default() -> Self {
        UsageThresholds {
            low_fraction: DEFAULT_LOW_FRACTION,
            stale_after_ms: DEFAULT_STALE_AFTER_MS,
        }
    }
}

/// Resolve a snapshot to the state the UI shows, as of `now`.
///
/// An [`UsageSource::Unknown`] snapshot and a stale one both resolve to
/// [`UsageDisplay::Unknown`]: the meter says "no idea" rather than inventing
/// headroom.
pub fn resolve_display(
    snapshot: &UsageSnapshot,
    now_ms: i64,
    thresholds: UsageThresholds,
) -> UsageDisplay {
    if snapshot.source == UsageSource::Unknown
        || is_stale(snapshot, now_ms, thresholds.stale_after_ms)
    {
        return UsageDisplay::Unknown;
    }
    if snapshot.used_fraction >= EXHAUSTED_FRACTION {
        UsageDisplay::Exhausted
    } else if snapshot.used_fraction >= thresholds.low_fraction {
        UsageDisplay::Low
    } else {
        UsageDisplay::Available
    }
}

/// Resolve an agent we may hold no snapshot for at all.
pub fn resolve_display_opt(
    snapshot: Option<&UsageSnapshot>,
    now_ms: i64,
    thresholds: UsageThresholds,
) -> UsageDisplay {
    match snapshot {
        Some(snapshot) => resolve_display(snapshot, now_ms, thresholds),
        None => UsageDisplay::Unknown,
    }
}

/// Whether a launch should proceed, given an agent's usage state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchDecision {
    /// Nothing stands in the way of launching.
    Proceed,
    /// The window is exhausted; launching would hit the limit. Carries the
    /// reset instant when known, so callers can say when it'll clear.
    Blocked { reset_at_ms: Option<i64> },
}

/// Resolve a snapshot we may hold none of into a launch decision, as of `now`.
///
/// Only [`UsageDisplay::Exhausted`] blocks a launch. [`UsageDisplay::Unknown`]
/// (never reported, gone stale, or no snapshot at all) and
/// [`UsageDisplay::Low`] both proceed — a warning belongs in the meter, not in
/// the gate.
pub fn launch_decision(
    snapshot: Option<&UsageSnapshot>,
    now_ms: i64,
    thresholds: UsageThresholds,
) -> LaunchDecision {
    match resolve_display_opt(snapshot, now_ms, thresholds) {
        UsageDisplay::Exhausted => LaunchDecision::Blocked {
            reset_at_ms: snapshot.and_then(|s| s.reset_at_ms),
        },
        UsageDisplay::Unknown | UsageDisplay::Low | UsageDisplay::Available => {
            LaunchDecision::Proceed
        }
    }
}

/// Clamp a reported fraction into `0.0..=1.0`; `NaN` reads as zero.
fn clamp_fraction(fraction: f64) -> f64 {
    if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, EXHAUSTED_FRACTION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- usage_for_run ---

    #[test]
    fn no_events_is_none() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        assert_eq!(usage_for_run(&conn, "ghost").unwrap(), None);
    }

    #[test]
    fn no_turn_completed_events_is_none() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        loopfleet_store::insert_event(&conn, "r1", r#"{"kind":"turn_started"}"#).unwrap();
        assert_eq!(usage_for_run(&conn, "r1").unwrap(), None);
    }

    #[test]
    fn sums_usage_across_turns() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        loopfleet_store::insert_event(
            &conn,
            "r1",
            r#"{"kind":"turn_completed","usage":{"input_tokens":3,"output_tokens":4}}"#,
        )
        .unwrap();
        loopfleet_store::insert_event(
            &conn,
            "r1",
            r#"{"kind":"turn_completed","usage":{"input_tokens":5,"output_tokens":1}}"#,
        )
        .unwrap();

        assert_eq!(
            usage_for_run(&conn, "r1").unwrap(),
            Some(Usage {
                input_tokens: 8,
                output_tokens: 5,
            })
        );
    }

    #[test]
    fn only_counts_the_named_run() {
        let conn = loopfleet_store::open(":memory:").unwrap();
        loopfleet_store::insert_event(
            &conn,
            "r1",
            r#"{"kind":"turn_completed","usage":{"input_tokens":3,"output_tokens":4}}"#,
        )
        .unwrap();
        loopfleet_store::insert_event(
            &conn,
            "r2",
            r#"{"kind":"turn_completed","usage":{"input_tokens":100,"output_tokens":100}}"#,
        )
        .unwrap();

        assert_eq!(
            usage_for_run(&conn, "r1").unwrap(),
            Some(Usage {
                input_tokens: 3,
                output_tokens: 4,
            })
        );
    }

    const MINUTE: i64 = 60 * 1_000;
    const NOW: i64 = 1_700_000_000_000;

    fn snapshot(used_fraction: f64, source: UsageSource) -> UsageSnapshot {
        UsageSnapshot {
            agent_key: "claude".into(),
            model: None,
            limit_window: None,
            used_fraction,
            reset_at_ms: None,
            observed_at_ms: NOW,
            source,
        }
    }

    // --- constructors ---

    #[test]
    fn constructors_clamp_out_of_range_fractions() {
        assert_eq!(
            UsageSnapshot::reported("claude", 1.7, NOW).used_fraction,
            1.0
        );
        assert_eq!(
            UsageSnapshot::reported("claude", -0.5, NOW).used_fraction,
            0.0
        );
        assert_eq!(
            UsageSnapshot::reported("claude", f64::NAN, NOW).used_fraction,
            0.0
        );
    }

    #[test]
    fn unknown_snapshot_is_zero_used_but_sourced_unknown() {
        let snap = UsageSnapshot::unknown("codex", NOW);
        assert_eq!(snap.used_fraction, 0.0);
        assert_eq!(snap.source, UsageSource::Unknown);
        assert_eq!(snap.agent_key, "codex");
    }

    // --- fold_rate_limit ---

    /// A bare notice (no figure) means blocked, which reads as exhausted and
    /// inferred rather than reported.
    #[test]
    fn bare_notice_infers_exhausted() {
        let folded = fold_rate_limit(None, &RateLimitNotice::new("claude", NOW));
        assert_eq!(folded.used_fraction, EXHAUSTED_FRACTION);
        assert_eq!(folded.source, UsageSource::Inferred);
        assert_eq!(folded.observed_at_ms, NOW);
    }

    #[test]
    fn notice_with_a_figure_is_reported_and_clamped() {
        let notice = RateLimitNotice::new("claude", NOW).with_used_fraction(0.42);
        let folded = fold_rate_limit(None, &notice);
        assert_eq!(folded.used_fraction, 0.42);
        assert_eq!(folded.source, UsageSource::Reported);

        let notice = RateLimitNotice::new("claude", NOW).with_used_fraction(9.0);
        assert_eq!(fold_rate_limit(None, &notice).used_fraction, 1.0);
    }

    #[test]
    fn notice_labels_and_reset_win_over_prior() {
        let prior = UsageSnapshot::reported("claude", 0.1, NOW - MINUTE)
            .with_model("opus")
            .with_limit_window("5h")
            .with_reset_at(NOW + MINUTE);
        let notice = RateLimitNotice::new("claude", NOW)
            .with_model("sonnet")
            .with_limit_window("weekly")
            .with_reset_at(NOW + 10 * MINUTE);

        let folded = fold_rate_limit(Some(&prior), &notice);
        assert_eq!(folded.model.as_deref(), Some("sonnet"));
        assert_eq!(folded.limit_window.as_deref(), Some("weekly"));
        assert_eq!(folded.reset_at_ms, Some(NOW + 10 * MINUTE));
    }

    /// A terse notice must not erase what a richer earlier observation told us.
    #[test]
    fn terse_notice_carries_prior_labels_forward() {
        let prior = UsageSnapshot::reported("claude", 0.1, NOW - MINUTE)
            .with_model("opus")
            .with_limit_window("5h")
            .with_reset_at(NOW + MINUTE);

        let folded = fold_rate_limit(Some(&prior), &RateLimitNotice::new("claude", NOW));
        assert_eq!(folded.model.as_deref(), Some("opus"));
        assert_eq!(folded.limit_window.as_deref(), Some("5h"));
        assert_eq!(folded.reset_at_ms, Some(NOW + MINUTE));
        assert_eq!(folded.used_fraction, EXHAUSTED_FRACTION);
    }

    /// Out-of-order delivery must not walk state backwards.
    #[test]
    fn older_notice_is_ignored() {
        let prior = UsageSnapshot::reported("claude", 0.9, NOW);
        let stale_notice = RateLimitNotice::new("claude", NOW - MINUTE);
        assert_eq!(fold_rate_limit(Some(&prior), &stale_notice), prior);
    }

    #[test]
    fn notice_for_another_agent_leaves_prior_alone() {
        let prior = UsageSnapshot::reported("claude", 0.3, NOW - MINUTE);
        let notice = RateLimitNotice::new("codex", NOW);

        let folded = fold_rate_limit(Some(&prior), &notice);
        assert_eq!(folded.agent_key, "codex");
        assert_eq!(folded.model, None);
        assert_eq!(folded.used_fraction, EXHAUSTED_FRACTION);
        // The prior's own numbers were not folded into the other agent.
        assert_eq!(prior.used_fraction, 0.3);
    }

    // --- is_stale ---

    #[test]
    fn fresh_observation_is_not_stale() {
        let snap = snapshot(0.5, UsageSource::Reported);
        assert!(!is_stale(&snap, NOW + MINUTE, DEFAULT_STALE_AFTER_MS));
    }

    #[test]
    fn observation_past_the_age_limit_is_stale() {
        let snap = snapshot(0.5, UsageSource::Reported);
        assert!(!is_stale(&snap, NOW + 15 * MINUTE, DEFAULT_STALE_AFTER_MS));
        assert!(is_stale(&snap, NOW + 16 * MINUTE, DEFAULT_STALE_AFTER_MS));
    }

    /// Once the window resets, the figure describes a window that no longer
    /// exists — stale however recently it was observed.
    #[test]
    fn passing_the_reset_instant_makes_it_stale() {
        let mut snap = snapshot(0.97, UsageSource::Reported);
        snap.reset_at_ms = Some(NOW + MINUTE);
        assert!(!is_stale(&snap, NOW + MINUTE - 1, DEFAULT_STALE_AFTER_MS));
        assert!(is_stale(&snap, NOW + MINUTE, DEFAULT_STALE_AFTER_MS));
    }

    #[test]
    fn a_backwards_clock_does_not_make_it_stale() {
        let snap = snapshot(0.5, UsageSource::Reported);
        assert!(!is_stale(&snap, NOW - 60 * MINUTE, DEFAULT_STALE_AFTER_MS));
    }

    // --- resolve_display ---

    #[test]
    fn low_usage_is_available() {
        let snap = snapshot(0.2, UsageSource::Reported);
        assert_eq!(
            resolve_display(&snap, NOW, UsageThresholds::default()),
            UsageDisplay::Available
        );
    }

    #[test]
    fn the_low_threshold_is_inclusive() {
        let snap = snapshot(DEFAULT_LOW_FRACTION, UsageSource::Reported);
        assert_eq!(
            resolve_display(&snap, NOW, UsageThresholds::default()),
            UsageDisplay::Low
        );
    }

    #[test]
    fn a_full_window_is_exhausted() {
        let snap = snapshot(1.0, UsageSource::Inferred);
        assert_eq!(
            resolve_display(&snap, NOW, UsageThresholds::default()),
            UsageDisplay::Exhausted
        );
    }

    /// A zero-used `Unknown` snapshot must not read as headroom.
    #[test]
    fn unknown_source_resolves_unknown() {
        let snap = UsageSnapshot::unknown("claude", NOW);
        assert_eq!(
            resolve_display(&snap, NOW, UsageThresholds::default()),
            UsageDisplay::Unknown
        );
    }

    #[test]
    fn stale_snapshot_resolves_unknown_even_when_exhausted() {
        let snap = snapshot(1.0, UsageSource::Reported);
        assert_eq!(
            resolve_display(&snap, NOW + 60 * MINUTE, UsageThresholds::default()),
            UsageDisplay::Unknown
        );
    }

    #[test]
    fn thresholds_are_configurable() {
        let snap = snapshot(0.6, UsageSource::Reported);
        let thresholds = UsageThresholds {
            low_fraction: 0.5,
            stale_after_ms: DEFAULT_STALE_AFTER_MS,
        };
        assert_eq!(resolve_display(&snap, NOW, thresholds), UsageDisplay::Low);
    }

    #[test]
    fn no_snapshot_at_all_resolves_unknown() {
        assert_eq!(
            resolve_display_opt(None, NOW, UsageThresholds::default()),
            UsageDisplay::Unknown
        );
    }

    #[test]
    fn display_state_serializes_kebab_case() {
        let json = serde_json::to_string(&UsageDisplay::Exhausted).unwrap();
        assert_eq!(json, "\"exhausted\"");
        let json = serde_json::to_string(&UsageSource::Inferred).unwrap();
        assert_eq!(json, "\"inferred\"");
    }

    // --- launch_decision ---

    #[test]
    fn no_snapshot_proceeds() {
        assert_eq!(
            launch_decision(None, NOW, UsageThresholds::default()),
            LaunchDecision::Proceed
        );
    }

    #[test]
    fn unknown_source_proceeds() {
        let snap = UsageSnapshot::unknown("claude", NOW);
        assert_eq!(
            launch_decision(Some(&snap), NOW, UsageThresholds::default()),
            LaunchDecision::Proceed
        );
    }

    #[test]
    fn stale_snapshot_proceeds_even_when_exhausted() {
        let snap = snapshot(1.0, UsageSource::Reported);
        assert_eq!(
            launch_decision(Some(&snap), NOW + 60 * MINUTE, UsageThresholds::default()),
            LaunchDecision::Proceed
        );
    }

    #[test]
    fn available_usage_proceeds() {
        let snap = snapshot(0.2, UsageSource::Reported);
        assert_eq!(
            launch_decision(Some(&snap), NOW, UsageThresholds::default()),
            LaunchDecision::Proceed
        );
    }

    #[test]
    fn low_usage_proceeds() {
        let snap = snapshot(DEFAULT_LOW_FRACTION, UsageSource::Reported);
        assert_eq!(
            launch_decision(Some(&snap), NOW, UsageThresholds::default()),
            LaunchDecision::Proceed
        );
    }

    #[test]
    fn exhausted_blocks_and_carries_reset() {
        let mut snap = snapshot(1.0, UsageSource::Reported);
        snap.reset_at_ms = Some(NOW + MINUTE);
        assert_eq!(
            launch_decision(Some(&snap), NOW, UsageThresholds::default()),
            LaunchDecision::Blocked {
                reset_at_ms: Some(NOW + MINUTE)
            }
        );
    }

    #[test]
    fn exhausted_without_reset_blocks_with_none() {
        let snap = snapshot(1.0, UsageSource::Inferred);
        assert_eq!(
            launch_decision(Some(&snap), NOW, UsageThresholds::default()),
            LaunchDecision::Blocked { reset_at_ms: None }
        );
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snap = UsageSnapshot::reported("claude", 0.75, NOW)
            .with_model("opus")
            .with_limit_window("weekly")
            .with_reset_at(NOW + MINUTE);
        let json = serde_json::to_string(&snap).unwrap();
        assert_eq!(serde_json::from_str::<UsageSnapshot>(&json).unwrap(), snap);
    }
}
