// §1.4's waitFor sugar is a thin pass-through to the `waitFor` RPC: the host
// polls its own retained tree at ~50ms (§1.1) and this client never
// re-implements that loop. poll() below is the one piece of genuine
// client-side polling in the harness, for conditions the waitFor vocabulary
// can't express (window count).
export interface WaitOpts {
  timeoutMs?: number;
  window?: number;
}

/** Generic poll: calls `fn` until `pred` holds or `timeoutMs` elapses. */
export async function poll<T>(
  fn: () => Promise<T>,
  pred: (value: T) => boolean,
  opts: { timeoutMs?: number; intervalMs?: number } = {},
): Promise<T> {
  const timeoutMs = opts.timeoutMs ?? 3000;
  const intervalMs = opts.intervalMs ?? 100;
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await fn();
    if (pred(value)) return value;
    if (Date.now() >= deadline) throw new Error(`poll: predicate never held within ${timeoutMs}ms`);
    await new Promise((r) => setTimeout(r, intervalMs));
  }
}

/** Renders a value the same way the host stringifies `value` for
 * valueEquals/valueContains: numbers stringified, bools "true"/"false". */
export function renderWaitValue(value: string | number | boolean): string {
  return typeof value === "string" ? value : String(value);
}
