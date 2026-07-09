import type { Op } from "../../../runtime/ndp.ts";

export class Batch {
  private ops: Op[] = [];
  push(op: Op): void { this.ops.push(op); }
  drain(): Op[] { const o = this.ops; this.ops = []; return o; }
  get length(): number { return this.ops.length; }
}

export type Handler = (payload?: unknown) => void;

export interface NodeRecord {
  id: number;
  type: string;
  props: Record<string, unknown>;
  handlers: Record<string, Handler>; // event name -> latest render's handler
}

export class NodeRegistry {
  private byId = new Map<number, NodeRecord>();
  register(rec: NodeRecord): void { this.byId.set(rec.id, rec); }
  get(id: number): NodeRecord | undefined { return this.byId.get(id); }
  unregister(id: number): void { this.byId.delete(id); }
}
