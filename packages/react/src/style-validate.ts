import { styleKeySpec } from "./generated/intrinsics.ts";

export class StyleError extends Error {}

const validTop = Object.keys(styleKeySpec);

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

function reject(bad: string, valid: string[], where: string): never {
  const near = nearest(bad, valid);
  const hint = near ? `Did you mean "${near}"?` : `Valid keys: ${valid.join(", ")}.`;
  throw new StyleError(`Invalid style key "${bad}"${where} — GTK styling is not web CSS. ${hint} See docs/styling.md`);
}

export function validateStyle(style: unknown): void {
  if (style == null || typeof style !== "object") return;
  for (const [k, v] of Object.entries(style as Record<string, unknown>)) {
    if (!(k in styleKeySpec)) reject(k, validTop, "");
    const nested = styleKeySpec[k];
    if (nested && v != null && typeof v === "object" && !Array.isArray(v)) {
      for (const nk of Object.keys(v as Record<string, unknown>)) {
        if (!nested.includes(nk)) reject(nk, nested, ` in "${k}"`);
      }
    }
  }
}
