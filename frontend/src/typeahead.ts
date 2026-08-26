// Type-ahead search for option lists: the "press s-e-t to jump to Settings"
// behavior a native select gets for free and Select.tsx has to reimplement.
// Two pure pieces — the keystroke buffer that decides whether a keypress
// extends the current search or starts a new one, and the wrap-around scan
// that turns a search string into an option index.

/// The accumulated search string and the timestamp of the keystroke that last
/// extended it.
export type TypeaheadState = { text: string; at: number };

/// Type-ahead resets if the user pauses this long between keystrokes — matches
/// the informal ~1s window most native listboxes use before starting a fresh
/// search rather than extending the current one.
export const TYPEAHEAD_RESET_MS = 1000;

/// The empty buffer to start from.
export const emptyTypeahead: TypeaheadState = { text: "", at: 0 };

/// Fold a keystroke into the buffer: appended to the current search if it came
/// within `TYPEAHEAD_RESET_MS` of the last one, otherwise starting a new
/// search from just this character.
export function extendTypeahead(
  state: TypeaheadState,
  char: string,
  now: number,
): TypeaheadState {
  const continues = now - state.at < TYPEAHEAD_RESET_MS;
  return { text: continues ? state.text + char : char, at: now };
}

/// The enabled option whose label starts with `query`, searching forward from
/// just after `from` and wrapping around through `from` itself last — so
/// repeated presses of the same letter cycle through same-initial options, the
/// way native selects do. Case-insensitive. Returns undefined when nothing
/// matches.
export function matchTypeahead(
  labels: readonly string[],
  indices: readonly number[],
  from: number,
  query: string,
): number | undefined {
  if (query === "") return undefined;
  const lower = query.toLowerCase();
  const startAfter = indices.indexOf(from);
  const ordered = [...indices.slice(startAfter + 1), ...indices.slice(0, startAfter + 1)];
  return ordered.find((i) => labels[i]?.toLowerCase().startsWith(lower));
}
