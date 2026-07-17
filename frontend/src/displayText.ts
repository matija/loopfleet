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
