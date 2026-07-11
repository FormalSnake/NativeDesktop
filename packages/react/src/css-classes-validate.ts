import { cssClassSpec } from "./generated/intrinsics.ts";

export class CssClassError extends Error {}

function nearest(bad: string, valid: string[]): string | null {
  let best: string | null = null, bestD = Infinity;
  for (const v of valid) {
    const d = lev(bad, v);
    if (d < bestD) { bestD = d; best = v; }
  }
  return bestD <= 3 ? best : null;
}

function lev(a: string, b: string): number {
  const m = a.length, n = b.length;
  const dp: number[][] = Array.from({ length: m + 1 }, (_, i) => [i, ...Array(n).fill(0)]);
  for (let j = 0; j <= n; j++) dp[0]![j] = j;
  for (let i = 1; i <= m; i++) for (let j = 1; j <= n; j++)
    dp[i]![j] = Math.min(dp[i - 1]![j]! + 1, dp[i]![j - 1]! + 1, dp[i - 1]![j - 1]! + (a[i - 1] === b[j - 1] ? 0 : 1));
  return dp[m]![n]!;
}

export function validateCssClasses(classes: unknown): void {
  if (classes == null) return;
  if (!Array.isArray(classes)) throw new CssClassError(`cssClasses must be a string[] — got ${typeof classes}. See docs/styling.md`);
  for (const c of classes) {
    if (typeof c !== "string") throw new CssClassError(`cssClasses entries must be strings — got ${typeof c}. See docs/styling.md`);
    if (!cssClassSpec.includes(c)) {
      const near = nearest(c, cssClassSpec);
      const hint = near ? `Did you mean "${near}"?` : `Valid classes: ${cssClassSpec.join(", ")}.`;
      throw new CssClassError(`Unknown CSS class "${c}" — only Adwaita/GTK design-system classes are allowed. ${hint} See docs/styling.md`);
    }
  }
}
