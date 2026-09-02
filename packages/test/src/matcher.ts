// SelectorPart -> predicate over a getTree node, plus the node-field readers
// every layer above (locator, expect, snapshot) shares.
//
// role/text/testID/enabled/visible/value are on the wire today. checked,
// selected, expanded, label, placeholder and options are optional additions:
// where the host does not send one, the reader falls back to what the older
// wire already carried (a boolean `value` is the checked state, `text` is the
// accessible name), so a selector behaves the same against either host.
import type { JsonNode } from "@nativedesktop/react/rpc";
import type { SelectorPart, TextSpec } from "./selectors.ts";

/** getTree's node plus the optional accessibility fields. */
export type NdNode = Omit<JsonNode, "children"> & {
  children: NdNode[];
  checked?: boolean | null;
  selected?: boolean | null;
  expanded?: boolean | null;
  placeholder?: string | null;
  label?: string | null;
  options?: string[] | null;
};

export function asNdNode(node: JsonNode): NdNode {
  return node as NdNode;
}

export function nodeText(node: NdNode): string {
  return node.text ?? "";
}

/** The concatenated text of a node and everything under it, which is what
 * `filter({hasText})` matches against. */
export function subtreeText(node: NdNode): string {
  let out = nodeText(node);
  for (const child of node.children) {
    const inner = subtreeText(child);
    if (inner) out = out ? `${out} ${inner}` : inner;
  }
  return out;
}

/** Accessible name: the label the host sends, else the node's own text, else
 * a string value (Badge and Kbd carry their content there). */
export function nodeName(node: NdNode): string {
  if (typeof node.label === "string" && node.label) return node.label;
  if (node.text) return node.text;
  return typeof node.value === "string" ? node.value : "";
}

export function nodePlaceholder(node: NdNode): string {
  return typeof node.placeholder === "string" ? node.placeholder : "";
}

export function nodeChecked(node: NdNode): boolean | undefined {
  if (typeof node.checked === "boolean") return node.checked;
  if (typeof node.value === "boolean") return node.value;
  return undefined;
}

/** The string rendering the host uses for waitFor's valueEquals, so one
 * comparison fits a TextInput, a Slider and a Checkbox alike. */
export function renderValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  return typeof value === "string" ? value : String(value);
}

export function hasRealSize(node: NdNode): boolean {
  const g = node.geometry;
  return g != null && g.w > 0 && g.h > 0;
}

function normalize(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

export function matchText(spec: TextSpec, actual: string): boolean {
  if (spec.regex) return spec.regex.test(actual);
  const want = spec.value ?? "";
  if (spec.exact) return normalize(actual) === normalize(want);
  return normalize(actual).toLowerCase().includes(normalize(want).toLowerCase());
}

/** True when `part` holds for `node` itself. Structural parts (nth, has,
 * has-not, and) are handled by the selection engine, not here. */
export function matchPart(node: NdNode, part: SelectorPart): boolean {
  switch (part.kind) {
    case "testid":
      return node.testID === part.value;
    case "type":
      return node.type === part.value;
    case "text":
      return matchText(part, nodeText(node));
    case "label":
      return matchText(part, nodeName(node));
    case "placeholder":
      return matchText(part, nodePlaceholder(node) || renderValue(node.value));
    case "has-text":
      return matchText(part, subtreeText(node));
    case "role": {
      if ((node.role ?? "") !== part.role) return false;
      if (part.name && !matchText(part.name, nodeName(node))) return false;
      if (part.checked !== undefined && nodeChecked(node) !== part.checked) return false;
      if (part.disabled !== undefined && node.enabled !== !part.disabled) return false;
      return true;
    }
    default:
      return true;
  }
}

function descendants(node: NdNode, out: NdNode[]): NdNode[] {
  for (const child of node.children) {
    out.push(child);
    descendants(child, out);
  }
  return out;
}

function selfAndDescendants(node: NdNode): NdNode[] {
  return descendants(node, [node]);
}

type PositionalPart = Extract<SelectorPart, { kind: "nth" | "has" | "has-not" | "and" }>;

function isPositional(part: SelectorPart): part is PositionalPart {
  return part.kind === "nth" || part.kind === "has" || part.kind === "has-not" || part.kind === "and";
}

function pickNth(nodes: NdNode[], index: number): NdNode[] {
  const at = index < 0 ? nodes.length + index : index;
  const node = nodes[at];
  return node ? [node] : [];
}

/** Whether one node satisfies every positional-free part (used by has/and). */
function matchesAll(node: NdNode, parts: SelectorPart[]): boolean {
  return parts.every((p) => (isPositional(p) ? true : matchPart(node, p)));
}

/**
 * Resolves a parsed selector against a tree.
 *
 * An engine part descends: the first searches the root and everything under
 * it, each later one searches strictly inside the previous part's matches.
 * nth/has/has-not/and/has-text refine the current set without descending.
 */
export function selectNodes(root: NdNode, parts: SelectorPart[]): NdNode[] {
  let current: NdNode[] = [root];
  let descended = false;
  for (const part of parts) {
    if (part.kind === "has-text") {
      current = current.filter((n) => matchPart(n, part));
      continue;
    }
    if (isPositional(part)) {
      if (part.kind === "nth") {
        current = pickNth(current, part.index);
      } else if (part.kind === "and") {
        current = current.filter((n) => matchesAll(n, part.parts));
      } else {
        const wantInner = part.kind === "has";
        current = current.filter((n) => descendants(n, []).some((d) => matchesAll(d, part.parts)) === wantInner);
      }
      continue;
    }
    const pool: NdNode[] = [];
    const seen = new Set<NdNode>();
    for (const scope of current) {
      for (const node of descended ? descendants(scope, []) : selfAndDescendants(scope)) {
        if (seen.has(node)) continue;
        seen.add(node);
        pool.push(node);
      }
    }
    current = pool.filter((n) => matchPart(n, part));
    descended = true;
  }
  return descended ? current : [];
}

/** Every node in the tree, in document order: the candidate pool an error
 * message ranks. */
export function allNodes(root: NdNode): NdNode[] {
  return selfAndDescendants(root);
}
