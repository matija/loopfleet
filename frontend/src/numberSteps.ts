// The arithmetic behind NumberField: what a +/- press or a typed-then-blurred
// string turns into once min/max/step have their say. Kept out of the
// component because the interesting cases (a step that lands past `max`, a
// half-typed "-", 0.1 + 0.2 drift) are all value-in/value-out and deserve
// tests that never touch the DOM.

/// The bounds a field's value has to satisfy. `step` is also the size of one
/// +/- press and the precision every committed value is rounded to.
export type StepBounds = { min?: number; max?: number; step: number };

/// Pull `n` inside whichever of `min`/`max` were given.
export function clampToRange(n: number, min?: number, max?: number): number {
  let v = n;
  if (min !== undefined) v = Math.max(min, v);
  if (max !== undefined) v = Math.min(max, v);
  return v;
}

/// Round to the step's own decimal precision so repeated +/- presses don't
/// accumulate floating-point noise (the classic 0.1 + 0.2 drift). Precision is
/// read off the step's decimal digits, so a step written in exponent notation
/// (1e-7) rounds to whole numbers.
export function roundToStep(n: number, step: number): number {
  const decimals = (step.toString().split(".")[1] ?? "").length;
  return Number(n.toFixed(decimals));
}

/// What the field actually commits for a candidate value: rounded to the
/// step's precision, then held inside the bounds.
export function normalizeValue(n: number, { min, max, step }: StepBounds): number {
  return clampToRange(roundToStep(n, step), min, max);
}

/// The value one +/- press (or arrow key) away from `base`.
export function stepValue(base: number, delta: number, bounds: StepBounds): number {
  return normalizeValue(base + delta, bounds);
}

/// Read what the user typed. Blank, whitespace-only, and unparseable input
/// (including the intermediate "-" or "1e") all come back null, meaning "no
/// number here" — the caller falls back to the committed value rather than
/// treating it as 0.
export function parseFieldValue(text: string): number | null {
  if (text.trim() === "") return null;
  const parsed = Number(text);
  return Number.isFinite(parsed) ? parsed : null;
}

/// Whether the value sits at (or past) a bound, i.e. whether the matching
/// stepper button has anything left to do. An absent bound is never reached.
export function atMin(value: number, min?: number): boolean {
  return min !== undefined && value <= min;
}

export function atMax(value: number, max?: number): boolean {
  return max !== undefined && value >= max;
}
