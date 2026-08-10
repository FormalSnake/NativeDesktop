// Reconnect backoff ladder. Pure, framework-free, no React, no ND import.
// `RpcClient` (client.ts) is the sole owner of a `ConnectionLadder`: it drives
// the real reconnect schedule and exposes the resulting attempt/delay so a
// subscriber can report it without maintaining a second, competing notion of
// "how many attempts has this client made".

export const RECONNECT_BASE_MS = 500;
export const RECONNECT_MAX_MS = 16_000;
export const STABILITY_WINDOW_MS = 30_000;

/** Pure: 500, 1000, 2000, 4000, 8000, 16000, 16000, ... Clamps attempt < 1 to the base. */
export function backoffDelayMs(attempt: number, baseMs = RECONNECT_BASE_MS, maxMs = RECONNECT_MAX_MS): number {
  const n = Math.max(attempt, 1);
  return Math.min(baseMs * 2 ** (n - 1), maxMs);
}

export interface LadderOptions {
  now?: () => number;
  baseMs?: number;
  maxMs?: number;
  stabilityWindowMs?: number;
}

/** The ladder itself, deterministic and injectable-clock so it is unit-testable. Tracks two
 * things: how many rungs deep the current reconnect run is (`attempt`), and whether the last
 * connected span was long enough (stabilityWindowMs) that a fresh drop should start back at
 * the bottom instead of inheriting an old outage's backoff. */
export class ConnectionLadder {
  #attempt = 0;
  #connectedAtMs: number | undefined;
  readonly #now: () => number;
  readonly #baseMs: number;
  readonly #maxMs: number;
  readonly #stabilityWindowMs: number;

  constructor(opts: LadderOptions = {}) {
    this.#now = opts.now ?? Date.now;
    this.#baseMs = opts.baseMs ?? RECONNECT_BASE_MS;
    this.#maxMs = opts.maxMs ?? RECONNECT_MAX_MS;
    this.#stabilityWindowMs = opts.stabilityWindowMs ?? STABILITY_WINDOW_MS;
  }

  get attempt(): number {
    return this.#attempt;
  }

  /** Advances the ladder one rung and returns the delay for that attempt. */
  nextDelayMs(): number {
    this.#attempt += 1;
    return backoffDelayMs(this.#attempt, this.#baseMs, this.#maxMs);
  }

  /** Call when a connection reaches `ready`. Records the timestamp. */
  noteConnected(): void {
    this.#connectedAtMs = this.#now();
  }

  /** Call when a connected socket drops. If it stayed connected for >= stabilityWindowMs the
   * ladder resets to 0. */
  noteDisconnected(): void {
    if (this.#connectedAtMs !== undefined && this.#now() - this.#connectedAtMs >= this.#stabilityWindowMs) {
      this.#attempt = 0;
    }
    this.#connectedAtMs = undefined;
  }

  /** Manual retry: reset to 0 unconditionally. */
  reset(): void {
    this.#attempt = 0;
    this.#connectedAtMs = undefined;
  }
}
