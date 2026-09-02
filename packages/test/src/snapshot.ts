// Compact accessibility-tree rendering, shared by the MCP `snapshot` tool and
// anything that wants a tree in an error or a log.
//
// One line per node, indented by depth:
//
//   - window "Notes" [ref=e1]
//     - button "Save" [ref=e12] [testid=save-btn] [enabled]
//
// A node with no role, text or testID carries no information a caller can act
// on, so it collapses: its children move up to its own depth. That turns the
// layout boxes that make up most of a real tree into nothing at all.
import { hasRealSize, nodeChecked, nodeName, renderValue, type NdNode } from "./matcher.ts";

export interface SnapshotOptions {
  /** Keep only nodes an agent can act on (a role, plus visible and enabled),
   * and the ancestors that lead to them. */
  interactiveOnly?: boolean;
  /** Cap the emitted lines; the renderer notes how many it dropped. */
  maxLines?: number;
}

const NON_INTERACTIVE_ROLES = new Set(["window", "group", "label", "image", "toolbar", "list", "tree", "table"]);

function identifies(node: NdNode): boolean {
  return Boolean(node.role || node.testID || nodeName(node));
}

function actionable(node: NdNode): boolean {
  if (!node.role || NON_INTERACTIVE_ROLES.has(node.role)) return false;
  return node.visible && node.enabled && hasRealSize(node);
}

function line(node: NdNode, depth: number): string {
  const bits = [`- ${node.role ?? node.type.toLowerCase()}`];
  const name = nodeName(node);
  if (name) bits.push(JSON.stringify(name));
  bits.push(`[ref=e${node.ref}]`);
  if (node.testID) bits.push(`[testid=${node.testID}]`);
  const checked = nodeChecked(node);
  if (checked !== undefined) bits.push(`[checked=${checked}]`);
  else {
    const value = renderValue(node.value);
    if (value && value !== name) bits.push(`[value=${JSON.stringify(value)}]`);
  }
  if (!node.visible) bits.push("[hidden]");
  bits.push(node.enabled ? "[enabled]" : "[disabled]");
  return `${"  ".repeat(depth)}${bits.join(" ")}`;
}

/** True when the node or anything under it survives the interactive filter. */
function keeps(node: NdNode, interactiveOnly: boolean): boolean {
  if (!interactiveOnly) return true;
  if (actionable(node)) return true;
  return node.children.some((c) => keeps(c, interactiveOnly));
}

export function renderSnapshot(root: NdNode, opts: SnapshotOptions = {}): string {
  const out: string[] = [];
  const interactiveOnly = opts.interactiveOnly ?? false;
  const walk = (node: NdNode, depth: number): void => {
    if (!keeps(node, interactiveOnly)) return;
    const show = identifies(node) && (!interactiveOnly || actionable(node) || node.children.length > 0);
    if (show) out.push(line(node, depth));
    for (const child of node.children) walk(child, show ? depth + 1 : depth);
  };
  walk(root, 0);
  const max = opts.maxLines;
  if (max !== undefined && out.length > max) {
    return [...out.slice(0, max), `... ${out.length - max} more nodes (raise maxLines or scope to a window)`].join("\n");
  }
  return out.join("\n");
}
