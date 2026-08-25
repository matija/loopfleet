import { describe, expect, it } from "vitest";
import type { UsageSnapshot, UsageSource } from "./types";
import {
  DEFAULT_LOW_FRACTION,
  DEFAULT_STALE_AFTER_MS,
  formatCountdown,
  formatRemainingPercent,
  formatResetTime,
  formatStaleness,
  formatUsedPercent,
  headroomBucket,
  isStale,
  launchHeadroom,
  usageIndicator,
} from "./usage";

const MINUTE = 60 * 1_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
/// 2023-11-14T22:13:20Z — a Tuesday, which the weekday assertions lean on.
const NOW = 1_700_000_000_000;

/// Every reset-time assertion pins locale and timezone; the formatter's whole
/// job is to follow the viewer's, so the tests have to name one to be stable.
const CLOCK = { locale: "en-US", timeZone: "UTC" };

function snapshot(over: Partial<UsageSnapshot> = {}): UsageSnapshot {
  return {
    agent_key: "claude",
    model: null,
    limit_window: null,
    used_fraction: 0.5,
    reset_at_ms: null,
    observed_at_ms: NOW,
    source: "reported" as UsageSource,
    ...over,
  };
}

describe("isStale", () => {
  it("holds a fresh observation up to the age limit", () => {
    expect(isStale(snapshot(), NOW + 15 * MINUTE, DEFAULT_STALE_AFTER_MS)).toBe(
      false,
    );
    expect(isStale(snapshot(), NOW + 16 * MINUTE, DEFAULT_STALE_AFTER_MS)).toBe(
      true,
    );
  });

  it("goes stale once the measured window has reset", () => {
    const snap = snapshot({ used_fraction: 0.97, reset_at_ms: NOW + MINUTE });
    expect(isStale(snap, NOW + MINUTE - 1)).toBe(false);
    expect(isStale(snap, NOW + MINUTE)).toBe(true);
  });

  it("does not read a backwards clock as staleness", () => {
    expect(isStale(snapshot(), NOW - 60 * MINUTE)).toBe(false);
  });
});

describe("headroomBucket", () => {
  it("buckets a reported fraction", () => {
    expect(headroomBucket(snapshot({ used_fraction: 0.2 }), NOW)).toBe(
      "available",
    );
    expect(
      headroomBucket(snapshot({ used_fraction: DEFAULT_LOW_FRACTION }), NOW),
    ).toBe("low");
    expect(headroomBucket(snapshot({ used_fraction: 1 }), NOW)).toBe(
      "exhausted",
    );
  });

  it("never dresses an absent or unsourced figure up as headroom", () => {
    expect(headroomBucket(null, NOW)).toBe("unknown");
    expect(headroomBucket(undefined, NOW)).toBe("unknown");
    // Zero used, but nothing was ever reported — not "plenty left".
    expect(
      headroomBucket(snapshot({ used_fraction: 0, source: "unknown" }), NOW),
    ).toBe("unknown");
  });

  it("forgets a stale figure however alarming it was", () => {
    const snap = snapshot({ used_fraction: 1 });
    expect(headroomBucket(snap, NOW + 60 * MINUTE)).toBe("unknown");
  });

  it("takes its boundaries from the thresholds it is given", () => {
    const snap = snapshot({ used_fraction: 0.6 });
    expect(
      headroomBucket(snap, NOW, {
        lowFraction: 0.5,
        staleAfterMs: DEFAULT_STALE_AFTER_MS,
      }),
    ).toBe("low");
  });
});

describe("formatUsedPercent", () => {
  it("rounds to whole points", () => {
    expect(formatUsedPercent(0)).toBe("0%");
    expect(formatUsedPercent(0.424)).toBe("42%");
    expect(formatUsedPercent(0.426)).toBe("43%");
    expect(formatUsedPercent(1)).toBe("100%");
  });

  it("keeps the two ends honest", () => {
    // Something left must not read as spent, and something spent must not
    // read as untouched.
    expect(formatUsedPercent(0.999)).toBe("99%");
    expect(formatUsedPercent(0.0001)).toBe("1%");
  });

  it("clamps whatever it is handed", () => {
    expect(formatUsedPercent(1.7)).toBe("100%");
    expect(formatUsedPercent(-0.5)).toBe("0%");
    expect(formatUsedPercent(Number.NaN)).toBe("0%");
  });

  it("counts down as well as up", () => {
    expect(formatRemainingPercent(0.25)).toBe("75%");
    expect(formatRemainingPercent(1)).toBe("0%");
    expect(formatRemainingPercent(0)).toBe("100%");
  });
});

describe("formatCountdown", () => {
  it("uses the coarsest useful grain", () => {
    expect(formatCountdown(NOW + 45 * MINUTE, NOW)).toBe("45m");
    expect(formatCountdown(NOW + 2 * HOUR + 15 * MINUTE, NOW)).toBe("2h 15m");
    expect(formatCountdown(NOW + 2 * HOUR, NOW)).toBe("2h");
    expect(formatCountdown(NOW + DAY + 3 * HOUR, NOW)).toBe("1d 3h");
    expect(formatCountdown(NOW + 2 * DAY, NOW)).toBe("2d");
  });

  it("does not round a reset that is still coming down to zero", () => {
    expect(formatCountdown(NOW + 30_000, NOW)).toBe("<1m");
  });

  it("says now for a reset already past", () => {
    expect(formatCountdown(NOW, NOW)).toBe("now");
    expect(formatCountdown(NOW - HOUR, NOW)).toBe("now");
  });
});

describe("formatResetTime", () => {
  it("shows bare time for a reset later the same day", () => {
    expect(formatResetTime(NOW + 30 * MINUTE, NOW, CLOCK)).toBe("10:43 PM");
  });

  it("adds a weekday once the reset lands on another day", () => {
    // NOW is a Tuesday at 22:13 UTC; +2h crosses midnight into Wednesday.
    expect(formatResetTime(NOW + 2 * HOUR, NOW, CLOCK)).toBe("Wed 12:13 AM");
  });

  it("switches to a date past the week, where a weekday is ambiguous", () => {
    expect(formatResetTime(NOW + 9 * DAY, NOW, CLOCK)).toBe("Nov 23, 10:13 PM");
  });

  it("counts the day boundary in the viewer's timezone, not UTC", () => {
    // The same instant is still Tuesday evening in New York but already
    // Wednesday in UTC.
    const reset = NOW + 30 * MINUTE;
    expect(
      formatResetTime(reset, NOW, { locale: "en-US", timeZone: "UTC" }),
    ).toBe("10:43 PM");
    expect(
      formatResetTime(NOW + 2 * HOUR, NOW, {
        locale: "en-US",
        timeZone: "America/New_York",
      }),
    ).toBe("7:13 PM");
  });
});

describe("formatStaleness", () => {
  it("says nothing while the figure is fresh", () => {
    expect(formatStaleness(snapshot(), NOW + MINUTE)).toBeNull();
  });

  it("distinguishes an aged observation from a spent window", () => {
    expect(formatStaleness(snapshot(), NOW + 22 * MINUTE)).toBe(
      "last measured 22m ago",
    );
    const withReset = snapshot({ reset_at_ms: NOW + MINUTE });
    expect(formatStaleness(withReset, NOW + 2 * MINUTE)).toBe(
      "window has since reset",
    );
  });
});

describe("usageIndicator", () => {
  it("labels a live figure with its percentage", () => {
    const indicator = usageIndicator(
      snapshot({
        used_fraction: 0.84,
        limit_window: "weekly",
        model: "opus",
        reset_at_ms: NOW + 2 * HOUR,
      }),
      NOW,
      undefined,
      CLOCK,
    );
    expect(indicator.display).toBe("low");
    expect(indicator.label).toBe("84%");
    expect(indicator.title).toBe(
      "84% of the weekly limit (opus) used · resets in 2h (Wed 12:13 AM).",
    );
    expect(indicator.staleness).toBeNull();
  });

  it("names the state rather than the number once the window is spent", () => {
    const indicator = usageIndicator(
      snapshot({ used_fraction: 1, source: "inferred" }),
      NOW,
    );
    expect(indicator.display).toBe("exhausted");
    expect(indicator.label).toBe("limit reached");
    expect(indicator.title).toBe(
      "100% of the limit used · inferred from a rate-limit notice.",
    );
  });

  it("says why it does not know, when it can", () => {
    expect(usageIndicator(null, NOW).title).toBe(
      "Limit headroom unknown — no snapshot yet.",
    );
    expect(
      usageIndicator(snapshot({ source: "unknown" }), NOW).title,
    ).toBe("Limit headroom unknown — never reported a limit figure.");
    expect(usageIndicator(snapshot(), NOW + 22 * MINUTE).title).toBe(
      "Limit headroom unknown — last measured 22m ago.",
    );
  });

  it("reports a stale figure as unknown while still admitting its age", () => {
    const indicator = usageIndicator(snapshot(), NOW + 22 * MINUTE);
    expect(indicator.display).toBe("unknown");
    expect(indicator.label).toBe("usage unknown");
    expect(indicator.staleness).toBe("last measured 22m ago");
  });
});

describe("launchHeadroom", () => {
  it("leaves the launch unremarked while the agent has room", () => {
    const readout = launchHeadroom("claude", snapshot(), NOW, undefined, CLOCK);
    expect(readout.display).toBe("available");
    expect(readout.label).toBe("50%");
    expect(readout.warning).toBeNull();
  });

  it("says nothing extra for a low window — visible, but not a wait", () => {
    const readout = launchHeadroom(
      "claude",
      snapshot({ used_fraction: DEFAULT_LOW_FRACTION }),
      NOW,
    );
    expect(readout.display).toBe("low");
    expect(readout.warning).toBeNull();
  });

  it("spells out the wait, with the reset, when the window is spent", () => {
    const readout = launchHeadroom(
      "claude",
      snapshot({ used_fraction: 1, reset_at_ms: NOW + 2 * HOUR }),
      NOW,
      undefined,
      CLOCK,
    );
    expect(readout.display).toBe("exhausted");
    expect(readout.label).toBe("limit reached");
    expect(readout.warning).toBe(
      "claude has no limit headroom left — a run started now waits until the window resets in 2h (Wed 12:13 AM).",
    );
  });

  it("still warns when the agent never said when it resets", () => {
    const readout = launchHeadroom(
      "pi",
      snapshot({ agent_key: "pi", used_fraction: 1 }),
      NOW,
    );
    expect(readout.warning).toBe(
      "pi has no limit headroom left — a run started now waits until the window resets.",
    );
  });

  it("does not warn about a figure it no longer believes", () => {
    const stale = snapshot({ used_fraction: 1 });
    const readout = launchHeadroom("claude", stale, NOW + 22 * MINUTE);
    expect(readout.display).toBe("unknown");
    expect(readout.warning).toBeNull();
  });
});
