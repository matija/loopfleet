// Task text comes from Markdown plans. Keep the authored source intact for agent
// prompts, but remove lightweight formatting syntax from compact UI labels.
export function normalizeDisplayText(value: string): string {
  return value
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    // A task cut off mid-`**bold**` (or mid-`code`) leaves an unmatched
    // delimiter the paired passes can't reach; drop any survivors so no raw
    // Markdown syntax leaks into a label.
    .replace(/\*\*/g, "")
    .replace(/`/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

const SUMMARY_LIMIT = 90;

// Compact one-line summary of a task: the first sentence, or the first 90
// characters cut at a word boundary, whichever is shorter. Ends with an
// ellipsis whenever text was dropped. Measured against the normalized text so
// Markdown syntax never counts toward the limit.
export function taskSummary(value: string): string {
  const text = normalizeDisplayText(value);

  // Punctuation only ends a sentence when more text follows; a lone trailing
  // period means the whole text is one sentence and nothing is dropped.
  const boundary = /[.!?]+(?=\s)/.exec(text);
  if (boundary !== null) {
    const end = boundary.index + boundary[0].length;
    if (end <= SUMMARY_LIMIT) {
      return text.slice(0, end).replace(/[.!?]+$/, "") + "…";
    }
  }

  if (text.length <= SUMMARY_LIMIT) {
    return text;
  }

  const cut = text.lastIndexOf(" ", SUMMARY_LIMIT);
  // No space to back off to (one unbroken run); a hard cut is the only option.
  const head = cut > 0 ? text.slice(0, cut) : text.slice(0, SUMMARY_LIMIT);
  return head.replace(/[\s,;:.!?]+$/, "") + "…";
}
