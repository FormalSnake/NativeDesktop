// Locator failures, rendered so the next step is obvious from the message
// alone: what was waited for, how many nodes answered, and which nodes in the
// window came closest.
import { hasRealSize, nodeName, renderValue, type NdNode } from "./matcher.ts";

export class LocatorError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "LocatorError";
  }
}

/** More than one node matched a selector used for a single-target action.
 * Never retried: a second element is a selector bug, not a timing one. */
export class StrictModeError extends LocatorError {
  constructor(
    readonly selector: string,
    readonly matches: NdNode[],
    windowLabel: string,
  ) {
    super(
      `strict mode violation: ${selector} resolved to ${matches.length} elements in window ${windowLabel}.\n` +
        `${renderCandidates(matches, matches.length)}\n` +
        `Narrow it with .first(), .nth(i), or .filter({ hasText: ... }).`,
    );
    this.name = "StrictModeError";
  }
}

export class TimeoutError extends LocatorError {
  constructor(
    readonly call: string,
    readonly timeoutMs: number,
    readonly callLog: string[],
    candidates: NdNode[],
    windowLabel: string,
    extra?: string,
  ) {
    const lines = [`${call}: Timeout ${timeoutMs}ms exceeded.`];
    if (extra) lines.push(extra);
    lines.push("Call log:");
    for (const entry of callLog) lines.push(`  - ${entry}`);
    if (candidates.length) {
      lines.push(`Nearest candidates in window ${windowLabel} (${candidates.length}):`);
      lines.push(renderCandidates(candidates, MAX_CANDIDATES));
    }
    super(lines.join("\n"));
    this.name = "TimeoutError";
  }
}

const MAX_CANDIDATES = 5;

function quote(s: string): string {
  return JSON.stringify(s.length > 40 ? `${s.slice(0, 39)}…` : s);
}

/** One candidate line: type, role, name, testID, state flags, rectangle. */
export function describeNode(node: NdNode): string {
  const bits = [node.type.padEnd(14), `role=${node.role ?? "-"}`.padEnd(18)];
  const name = nodeName(node);
  if (name) bits.push(`text=${quote(name)}`);
  if (node.testID) bits.push(`testID=${node.testID}`);
  const value = renderValue(node.value);
  if (value && value !== name) bits.push(`value=${quote(value)}`);
  bits.push([node.visible ? "visible" : "hidden", node.enabled ? "enabled" : "disabled"].join(" "));
  const g = node.geometry;
  bits.push(g ? `(${g.x},${g.y} ${g.w}x${g.h})` : "(no geometry)");
  return `  ${bits.join("  ")}`;
}

export function renderCandidates(nodes: NdNode[], limit: number): string {
  const shown = nodes.slice(0, limit).map(describeNode);
  if (nodes.length > limit) shown.push(`  ... and ${nodes.length - limit} more`);
  return shown.join("\n");
}

/** Levenshtein distance, capped at the longer string's length. Only used to
 * order a handful of candidates, so the plain two-row implementation is
 * cheaper than anything smarter. */
export function distance(a: string, b: string): number {
  if (a === b) return 0;
  if (!a.length || !b.length) return Math.max(a.length, b.length);
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      row[j] = Math.min(row[j - 1]! + 1, prev[j]! + 1, prev[j - 1]! + cost);
    }
    prev = row;
  }
  return prev[b.length]!;
}

/**
 * Orders every identifiable node in the tree by how close its testID or name
 * is to what the selector asked for. Actionable nodes win ties, so the first
 * suggestion is one a click would actually reach.
 */
export function rankCandidates(nodes: NdNode[], intended: string | undefined, limit = MAX_CANDIDATES): NdNode[] {
  const identifiable = nodes.filter((n) => n.testID || nodeName(n) || n.role);
  const want = (intended ?? "").toLowerCase();
  const score = (node: NdNode): number => {
    if (!want) return node.testID ? 0 : 1;
    const keys = [node.testID ?? "", nodeName(node), node.role ?? "", node.type];
    let best = Number.POSITIVE_INFINITY;
    for (const key of keys) {
      const lower = key.toLowerCase();
      if (!lower) continue;
      const d = lower.includes(want) || want.includes(lower) ? 0 : distance(lower, want);
      if (d < best) best = d;
    }
    return best;
  };
  return identifiable
    .map((node) => ({ node, score: score(node), actionable: node.visible && node.enabled && hasRealSize(node) }))
    .sort((a, b) => a.score - b.score || Number(b.actionable) - Number(a.actionable))
    .slice(0, limit)
    .map((e) => e.node);
}
