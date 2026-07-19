// Line-level text diff for reviewing a plan edit before it is written. There is
// no backend command that diffs arbitrary text (the run diffs come from git),
// so the PRD edit review computes its own: a unified-diff-style patch string
// whose lines are prefixed " " (context), "+" (added), or "-" (removed). That
// is exactly what the run timeline's `Patch` renderer already colors, so the
// two diff surfaces read the same.
//
// No hunking or context collapsing: a plan document is small enough to show
// whole, and full context keeps the review unambiguous. Alignment is a standard
// longest-common-subsequence over lines.

export type TextDiff = {
  patch: string;
  insertions: number;
  deletions: number;
};

export function diffLines(oldText: string, newText: string): TextDiff {
  const a = oldText.split("\n");
  const b = newText.split("\n");
  const m = a.length;
  const n = b.length;

  // lcs[i][j] = length of the longest common subsequence of a[i..] and b[j..].
  const lcs: number[][] = Array.from({ length: m + 1 }, () =>
    new Array<number>(n + 1).fill(0),
  );
  for (let i = m - 1; i >= 0; i--) {
    for (let j = n - 1; j >= 0; j--) {
      lcs[i][j] =
        a[i] === b[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const lines: string[] = [];
  let insertions = 0;
  let deletions = 0;
  let i = 0;
  let j = 0;
  while (i < m && j < n) {
    if (a[i] === b[j]) {
      lines.push(" " + a[i]);
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      lines.push("-" + a[i]);
      deletions++;
      i++;
    } else {
      lines.push("+" + b[j]);
      insertions++;
      j++;
    }
  }
  for (; i < m; i++) {
    lines.push("-" + a[i]);
    deletions++;
  }
  for (; j < n; j++) {
    lines.push("+" + b[j]);
    insertions++;
  }

  return { patch: lines.join("\n"), insertions, deletions };
}
