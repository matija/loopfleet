// Display logic for per-agent limit headroom: the pure half of the usage meter.
//
// The backend hands the UI a `UsageSnapshot` (`core::usage`) and nothing more —
// which bucket that lands in, how the figure reads, when the window resets in
// the viewer's own timezone, and how to admit the number has gone stale are all
// presentation decisions, so they live here rather than in a component. Every
// function takes `now` (epoch millis) instead of reading the clock, which keeps
// the module pure and directly testable; the ticking is the component's job.
//
// The bucket rules deliberately mirror `core::usage::resolve_display` so the
// chip and any backend-side launch guardrail agree about what "exhausted"
// means. Keep the two in step if either moves.

import type { UsageSnapshot } from "./types";

/// A used fraction at or above this reads as exhausted. `core::usage`.
export const EXHAUSTED_FRACTION = 1.0;

/// Used fraction at or above which headroom reads as `"low"`. `core::usage`.
export const DEFAULT_LOW_FRACTION = 0.8;

/// Age (15 minutes) after which a snapshot no longer describes reality.
/// `core::usage`.
export const DEFAULT_STALE_AFTER_MS = 15 * 60 * 1_000;

/// What the meter shows for an agent. Mirrors `core::UsageDisplay`'s kebab-case
/// wire form, so a bucket the backend computes and one computed here are the
/// same token.
export type UsageDisplay = "available" | "low" | "exhausted" | "unknown";

/// Where the boundaries between buckets sit. Overridable so a surface with a
/// different appetite for warnings (or a test) can move them.
export type UsageThresholds = {
  lowFraction: number;
  staleAfterMs: number;
};

export const DEFAULT_THRESHOLDS: UsageThresholds = {
  lowFraction: DEFAULT_LOW_FRACTION,
  staleAfterMs: DEFAULT_STALE_AFTER_MS,
};

/// How the reset instant should be spelled: in whose timezone, and — when the
/// host's own locale isn't wanted (tests, screenshots) — in whose locale.
/// Both default to the viewer's, which is the point: a reset time is only
/// useful read against the clock on the wall.
export type ClockFormat = {
  timeZone?: string;
  locale?: string;
};

/// Whether a snapshot has stopped describing reality as of `now`.
///
/// Two ways to go stale: the observation aged past `staleAfterMs`, or the
/// window it measured has since reset — "97% used" from before a reset says
/// nothing about the fresh window. A clock that runs backwards (`now` before
/// the observation) is not staleness; the snapshot is still the newest thing
/// we hold.
export function isStale(
  snapshot: UsageSnapshot,
  nowMs: number,
  staleAfterMs: number = DEFAULT_STALE_AFTER_MS,
): boolean {
  const resetAt = snapshot.reset_at_ms;
  if (
    resetAt !== null &&
    nowMs >= resetAt &&
    resetAt >= snapshot.observed_at_ms
  ) {
    return true;
  }
  return Math.max(0, nowMs - snapshot.observed_at_ms) > staleAfterMs;
}

/// Which bucket an agent's headroom falls in as of `now`.
///
/// A missing snapshot, an `"unknown"`-sourced one, and a stale one all resolve
/// to `"unknown"`: the meter says "no idea" rather than inventing headroom out
/// of a zero it was never told.
export function headroomBucket(
  snapshot: UsageSnapshot | null | undefined,
  nowMs: number,
  thresholds: UsageThresholds = DEFAULT_THRESHOLDS,
): UsageDisplay {
  if (!snapshot) return "unknown";
  if (snapshot.source === "unknown") return "unknown";
  if (isStale(snapshot, nowMs, thresholds.staleAfterMs)) return "unknown";
  if (snapshot.used_fraction >= EXHAUSTED_FRACTION) return "exhausted";
  if (snapshot.used_fraction >= thresholds.lowFraction) return "low";
  return "available";
}

/// A used fraction as a whole-percent string.
///
/// Rounds to the nearest point, except at the two ends: a window with anything
/// left never rounds up to `"100%"`, and any consumption at all never rounds
/// down to `"0%"` — both would misread the one number the user acts on.
export function formatUsedPercent(fraction: number): string {
  const clamped = Number.isFinite(fraction)
    ? Math.min(Math.max(fraction, 0), EXHAUSTED_FRACTION)
    : 0;
  let percent = Math.round(clamped * 100);
  if (percent === 100 && clamped < EXHAUSTED_FRACTION) percent = 99;
  if (percent === 0 && clamped > 0) percent = 1;
  return `${percent}%`;
}

/// The headroom left, as a whole-percent string — the complement of
/// `formatUsedPercent`, for surfaces that count down rather than up.
export function formatRemainingPercent(fraction: number): string {
  const clamped = Number.isFinite(fraction)
    ? Math.min(Math.max(fraction, 0), EXHAUSTED_FRACTION)
    : 0;
  return formatUsedPercent(EXHAUSTED_FRACTION - clamped);
}

const MINUTE_MS = 60 * 1_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/// How long until the window resets, at the coarsest useful grain: `"2h 15m"`,
/// `"45m"`, `"1d 3h"`. A reset less than a minute out is `"<1m"` rather than
/// `"0m"`, which would read as "already reset"; one already past is `"now"`,
/// since the snapshot describing the spent window is the stale thing, not the
/// clock.
export function formatCountdown(resetAtMs: number, nowMs: number): string {
  const remaining = resetAtMs - nowMs;
  if (remaining <= 0) return "now";
  if (remaining < MINUTE_MS) return "<1m";
  if (remaining < HOUR_MS) return `${Math.floor(remaining / MINUTE_MS)}m`;
  if (remaining < DAY_MS) {
    const hours = Math.floor(remaining / HOUR_MS);
    const minutes = Math.floor((remaining % HOUR_MS) / MINUTE_MS);
    return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
  }
  const days = Math.floor(remaining / DAY_MS);
  const hours = Math.floor((remaining % DAY_MS) / HOUR_MS);
  return hours === 0 ? `${days}d` : `${days}d ${hours}h`;
}

/// The wall-clock reset instant in the viewer's timezone.
///
/// Carries only as much date as the distance demands: a reset later today is
/// bare time (`"3:00 PM"`), one within the coming week gains a weekday
/// (`"Tue 3:00 PM"`), and anything further gains a date (`"Sep 2, 3:00 PM"`).
/// The day comparison is made in the display timezone, so "today" means the
/// user's today and not UTC's.
export function formatResetTime(
  resetAtMs: number,
  nowMs: number,
  clock: ClockFormat = {},
): string {
  const time: Intl.DateTimeFormatOptions = {
    hour: "numeric",
    minute: "2-digit",
    timeZone: clock.timeZone,
  };
  const days = calendarDaysBetween(nowMs, resetAtMs, clock.timeZone);
  if (days !== 0) {
    // Beyond a week a weekday name is ambiguous ("Tue" — which one?).
    Object.assign(
      time,
      days >= 1 && days <= 6
        ? { weekday: "short" }
        : { month: "short", day: "numeric" },
    );
  }
  return new Intl.DateTimeFormat(clock.locale, time).format(
    new Date(resetAtMs),
  );
}

/// Whole calendar days from `fromMs` to `toMs` as counted in `timeZone` — the
/// difference the user perceives ("tomorrow"), not the number of 24-hour spans
/// between the instants. `en-CA` is used only because it formats as
/// `YYYY-MM-DD`, which subtracts cleanly.
function calendarDaysBetween(
  fromMs: number,
  toMs: number,
  timeZone?: string,
): number {
  const day = new Intl.DateTimeFormat("en-CA", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    timeZone,
  });
  const midnightOf = (ms: number) => Date.parse(`${day.format(new Date(ms))}T00:00:00Z`);
  return Math.round((midnightOf(toMs) - midnightOf(fromMs)) / DAY_MS);
}

/// How the snapshot's age should be admitted, or `null` while it is still
/// fresh enough to state plainly.
///
/// Two different admissions: a window that has already reset is describing
/// something that no longer exists, while a merely old observation may still be
/// roughly right — so they get different wording rather than one flat "stale".
export function formatStaleness(
  snapshot: UsageSnapshot,
  nowMs: number,
  staleAfterMs: number = DEFAULT_STALE_AFTER_MS,
): string | null {
  if (!isStale(snapshot, nowMs, staleAfterMs)) return null;
  const resetAt = snapshot.reset_at_ms;
  if (
    resetAt !== null &&
    nowMs >= resetAt &&
    resetAt >= snapshot.observed_at_ms
  ) {
    return "window has since reset";
  }
  return `last measured ${formatAge(nowMs - snapshot.observed_at_ms)} ago`;
}

/// An elapsed span at the same grain as `formatCountdown`, for text that looks
/// backwards ("last measured 22m ago").
function formatAge(elapsedMs: number): string {
  const elapsed = Math.max(0, elapsedMs);
  if (elapsed < MINUTE_MS) return "moments";
  return formatCountdown(elapsed, 0);
}

/// Everything the compact headroom chip renders, resolved in one place: the
/// bucket that colors it, the short text it shows, and the sentence behind its
/// tooltip. Components stay declarative — no formatting decisions at the JSX
/// call site.
export type UsageIndicator = {
  display: UsageDisplay;
  /// The chip's own text — short enough to sit beside a version string.
  label: string;
  /// The full story: what was measured, of which window, and when it resets.
  title: string;
  /// How the figure has aged, when it has; `null` while fresh.
  staleness: string | null;
};

const LABELS: Record<UsageDisplay, string> = {
  available: "usage",
  low: "usage",
  exhausted: "limit reached",
  unknown: "usage unknown",
};

/// Resolve a snapshot (or its absence) into the chip's whole rendering.
export function usageIndicator(
  snapshot: UsageSnapshot | null | undefined,
  nowMs: number,
  thresholds: UsageThresholds = DEFAULT_THRESHOLDS,
  clock: ClockFormat = {},
): UsageIndicator {
  const display = headroomBucket(snapshot, nowMs, thresholds);
  const staleness = snapshot
    ? formatStaleness(snapshot, nowMs, thresholds.staleAfterMs)
    : null;

  if (!snapshot || display === "unknown") {
    // Say why we don't know when we can — "never reported" and "the figure we
    // had went stale" are different problems, and only one of them is fixed by
    // waiting.
    const why = !snapshot
      ? "no snapshot yet"
      : (staleness ?? "never reported a limit figure");
    return {
      display,
      label: LABELS.unknown,
      title: `Limit headroom unknown — ${why}.`,
      staleness,
    };
  }

  const used = formatUsedPercent(snapshot.used_fraction);
  const window = snapshot.limit_window
    ? `${snapshot.limit_window} limit`
    : "limit";
  const scope = snapshot.model ? ` (${snapshot.model})` : "";
  const label = display === "exhausted" ? LABELS.exhausted : used;

  const parts = [`${used} of the ${window}${scope} used`];
  if (snapshot.reset_at_ms !== null) {
    parts.push(
      `resets in ${formatCountdown(snapshot.reset_at_ms, nowMs)} (${formatResetTime(snapshot.reset_at_ms, nowMs, clock)})`,
    );
  }
  // An inferred figure is our reading of a rate-limit notice, not the agent's
  // word; the tooltip should not launder it into a measurement.
  if (snapshot.source === "inferred") {
    parts.push("inferred from a rate-limit notice");
  }
  return { display, label, title: `${parts.join(" · ")}.`, staleness };
}
