// Tree-walking helpers factored out of the four consumer drive scripts, which
// each hand-rolled the same find/mustFind pair. resolveTarget() is the
// client-side half of §1.2a's targeting: a caller passes a testID, a raw ref,
// or an explicit {testId?, ref?, window?}, and every action RPC accepts
// exactly one of ref/testId (validated host-side).
import type { JsonNode } from "../../react/src/generated/rpc.ts";

export function findNode(node: JsonNode, testId: string): JsonNode | null {
  if (node.testID === testId) return node;
  for (const child of node.children) {
    const found = findNode(child, testId);
    if (found) return found;
  }
  return null;
}

export function findAllNodes(node: JsonNode, testId: string, out: JsonNode[] = []): JsonNode[] {
  if (node.testID === testId) out.push(node);
  for (const child of node.children) findAllNodes(child, testId, out);
  return out;
}

export function findMatchingNode(node: JsonNode, pred: (n: JsonNode) => boolean): JsonNode | null {
  if (pred(node)) return node;
  for (const child of node.children) {
    const found = findMatchingNode(child, pred);
    if (found) return found;
  }
  return null;
}

/** A widget reference for the action RPCs: a bare testID, a bare ref, or an explicit descriptor. */
export type Target = string | number | { testId?: string; ref?: number; window?: number };

export function resolveTarget(t: Target): { testId?: string; ref?: number; window?: number } {
  if (typeof t === "string") return { testId: t };
  if (typeof t === "number") return { ref: t };
  return { ...t };
}
