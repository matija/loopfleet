// A tiny subsequence fuzzy matcher with lightweight ranking — enough to power
// the ⌘K command palette without pulling a dependency (PRD non-goal: no new
// surface beyond the existing tokens). Scoring rewards consecutive matches and
// matches at word boundaries (space, slash, dash, underscore), so "plan" hits
// "Plan tree" and "cmp" hits "compare". Returns the matched char indices so the
// palette can highlight them.

export type FuzzyMatch = {
  /// `-1` when the query is not a subsequence of the target. Higher is better.
  score: number;
  matched: boolean;
  /// Character indices in `target` that the query consumed (for highlighting).
  indices: number[];
};

/// Word-boundary chars: a match starting right after one gets a bonus.
const BOUNDARY = /[\s\-/_.,]/;

export function fuzzyMatch(query: string, target: string): FuzzyMatch {
  if (!query) return { score: 0, matched: true, indices: [] };
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  const indices: number[] = [];
  let qi = 0;
  let score = 0;
  let prevMatch = -2;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      indices.push(ti);
      score += ti === prevMatch + 1 ? 4 : 1;
      if (ti === 0 || BOUNDARY.test(t[ti - 1])) score += 5;
      prevMatch = ti;
      qi++;
    }
  }
  if (qi < q.length) return { score: -1, matched: false, indices: [] };
  // Mild preference for shorter targets (exact-ish matches outrank long ones).
  score += Math.max(0, 24 - t.length) * 0.1;
  return { score, matched: true, indices };
}
