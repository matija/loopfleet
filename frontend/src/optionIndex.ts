// Index arithmetic shared by the keyboard-navigable option lists (Select's
// listbox, SegmentedControl's radiogroup). Both walk a list where some entries
// are disabled and must be skipped, and both need that walk to be testable
// without mounting a component — so the arithmetic lives here and the
// components only translate key events into calls.

/// The only thing these helpers need to know about an option.
export type SkippableOption = { disabled?: boolean };

/// Positions of the options a keyboard user can land on, in list order.
/// Disabled options keep their slot in the original list but drop out here, so
/// the returned numbers still index into `options`.
export function enabledIndices(options: readonly SkippableOption[]): number[] {
  return options.map((o, i) => (o.disabled ? -1 : i)).filter((i) => i >= 0);
}

/// The option to highlight when opening at `index`: `index` itself if it is
/// enabled, otherwise the nearest enabled option at or after it, wrapping to
/// the first enabled option when none follow. With nothing enabled there is no
/// better answer than `index`.
export function clampToEnabled(indices: readonly number[], index: number): number {
  if (indices.length === 0) return index;
  if (indices.includes(index)) return index;
  return indices.find((i) => i >= index) ?? indices[0];
}

/// How movement behaves at the ends of the list: `clamp` stops on the first or
/// last enabled option (a listbox's arrow keys), `wrap` continues around to the
/// other end (a radio group's).
export type MoveMode = "clamp" | "wrap";

/// Move `delta` enabled options from `from`, skipping disabled ones. `from` is
/// an index into the original option list; the result is too. A `from` that is
/// not itself enabled counts as sitting at the first enabled option. Returns
/// `from` unchanged when nothing is enabled, since there is nowhere to go.
export function moveIndex(
  indices: readonly number[],
  from: number,
  delta: number,
  mode: MoveMode,
): number {
  if (indices.length === 0) return from;
  const pos = indices.indexOf(from);
  const basePos = pos === -1 ? 0 : pos;
  const target = basePos + delta;
  const len = indices.length;
  const nextPos =
    mode === "wrap"
      ? ((target % len) + len) % len
      : Math.min(Math.max(target, 0), len - 1);
  return indices[nextPos];
}
