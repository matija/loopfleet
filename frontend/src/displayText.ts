// Task text comes from Markdown plans. Keep the authored source intact for agent
// prompts, but remove lightweight formatting syntax from compact UI labels.
export function normalizeDisplayText(value: string): string {
  return value
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}
