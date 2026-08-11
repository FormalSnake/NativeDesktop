// Resilient JSON-RPC 2.0 client over a pluggable Transport. When a handshake
// is configured it must be the first frame on every socket; everything else
// is an id-correlated request/response, and the server pushes JSON-RPC
// notifications (no id) as events.
//
// The reconnect machinery is a port of CanaryOrchestrator's control-plane
// client; its invariants are behavioral, not stylistic, and each carries the
// comment explaining the failure it prevents.

import { ConnectionLadder } from "./backoff.ts";
import type { Transport, TransportFactory, TransportHandlers } from "./transport.ts";

export type RpcState = "connecting" | "ready" | "reconnecting" | "offline" | "closed";

export interface RpcContract {
  methods: Record<string, { params: unknown; result: unknown }>;
  events: Record<string, unknown>;
}

export class RpcError extends Error {
  readonly rpcCode?: number;
  readonly data?: unknown;
  constructor(message: string, rpcCode?: number, data?: unknown) {
    super(message);
    this.name = "RpcError";
    this.rpcCode = rpcCode;
    this.data = data;
  }
}

interface PendingCall {
  resolve: (result: unknown) => void;
  reject: (err: unknown) => void;
}

interface QueuedCall {
  id: number;
  method: string;
  params: unknown;
}

interface JsonRpcIncoming {
  id?: number | string | null;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

/** One dial's transport plus its detach guard. Detaching (setting `detached`
 * before `close()`) is the port of "null the ws handlers before close": a
 * dying socket's late events must never drive recovery logic against the
 * fresh connection dialed after it. */
interface Session {
  transport: Transport;
  detached: boolean;
}

const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
const DEFAULT_CALL_TIMEOUT_MS = 15_000;
const DEFAULT_WATCHDOG_INTERVAL_MS = 5_000;
const DEFAULT_IDLE_PROBE_MS = 20_000;
const DEFAULT_IDLE_CLOSE_MS = 40_000;
const DEFAULT_JITTER = 0.5;

export interface RpcClientOptions<C extends RpcContract> {
  transport: TransportFactory;
  /** First frame on every socket; nothing else is sent until it answers.
   * `fatal` inspects a server-sent handshake error: returning true marks the
   * rejection terminal (wrong protocol, bad credentials), which suppresses
   * the reconnect ladder and leaves the descriptive offline state. */
  handshake?: { method: string; params: unknown | (() => unknown); fatal?: (err: RpcError) => boolean };
  /** Runs after EVERY successful handshake, first dial included (a
   * conditional latch here has a hole). Its failure never stomps a newer
   * state. */
  resume?: (client: RpcClient<C>) => Promise<void>;
  /** Liveness probe sent at idleProbeMs of rx silence. Omit for the silence
   * budget alone. */
  probe?: { method: string; params?: unknown };
  /** Bounds a single connect()/handshake attempt (default 10s) so a socket
   * that never opens (or a handshake that never answers) rejects the caller
   * instead of hanging it forever. */
  connectTimeoutMs?: number;
  /** Bounds every non-handshake call (default 15s), whether in flight or
   * queued during a reconnect window. A half-open socket accepts writes into
   * the kernel buffer and never answers, so without this a call (and the UI
   * action behind it) hangs forever. */
  callTimeoutMs?: number;
  /** Liveness watchdog cadence while ready (default 5s). A wifi/VPN drop
   * with no RST produces no close event; the watchdog is what turns "silent
   * dead socket" into a close so the reconnect ladder can run at all. */
  watchdogIntervalMs?: number;
  /** Rx silence after which the watchdog sends the probe (default 20s). */
  idleProbeMs?: number;
  /** Rx silence after which the socket is declared dead and force-closed into
   * the normal reconnect path (default 40s). */
  idleCloseMs?: number;
  backoff?: { baseMs?: number; maxMs?: number; jitter?: number; stabilityWindowMs?: number };
  /** Injectable clock for tests. */
  now?: () => number;
}

export class RpcClient<C extends RpcContract> {
  #session: Session | undefined;
  #connectCalled = false;
  #nextId = 1;
  #pending = new Map<number, PendingCall>();
  #sendQueue: QueuedCall[] = [];
  #authed = false;
  #everAuthed = false;
  #explicitClose = false;
  #retryForever = false;
  #lastHandshake: unknown | undefined;
  // The one reconnect ladder (backoff.ts) for this client: tracks the attempt count and
  // the stability window, and is the single source #scheduleReconnect computes the real
  // setTimeout delay from. `attempt`/`nextRetryInMs` below expose the same numbers to
  // subscribers so their reported state never drifts from what's actually scheduled.
  #ladder: ConnectionLadder;
  #pendingRetryInMs: number | undefined;
  #reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  // Settles the connect()/handshake promise from outside the open handler: the socket can
  // die (error/close) or never answer (timeout) before the pending handshake call
  // is ever registered, and #rejectAllPending can't reach it there.
  #pendingHandshakeReject: ((err: unknown) => void) | undefined;
  // The #pending id of an in-flight handshake, once registered. Its own reject handler
  // carries side effects (the fatal() check, #setState("offline", ...)) that must fire
  // only for an actual server-sent error response, never for the generic "connection
  // closed" #rejectAllPending sweeps on a plain socket drop, which would otherwise flash
  // an incorrect offline state in the middle of a forever-reconnect run (see
  // #discardPendingHandshake).
  #pendingHandshakeId: number | undefined;
  #connectTimer: ReturnType<typeof setTimeout> | undefined;
  // Set just before tearing the socket down on a fatal handshake rejection, so the close
  // cascade below (#handleClose) treats it as terminal (a reconnect backoff loop against
  // a server we know will refuse us can never succeed) instead of clobbering the
  // descriptive offline state we just set or scheduling another attempt.
  #suppressReconnect = false;
  #eventHandlers = new Map<string, Set<(params: unknown) => void>>();
  #stateHandlers = new Set<(state: RpcState, detail?: string) => void>();
  #reconnectHandlers = new Set<() => void>();
  #state: RpcState = "offline";
  #lastRxAt = 0;
  #lastTickAt = 0;
  #watchdogTimer: ReturnType<typeof setInterval> | undefined;
  readonly #options: RpcClientOptions<C>;
  readonly #connectTimeoutMs: number;
  readonly #callTimeoutMs: number;
  readonly #watchdogIntervalMs: number;
  readonly #idleProbeMs: number;
  readonly #idleCloseMs: number;
  readonly #jitter: number;
  readonly #now: () => number;

  constructor(options: RpcClientOptions<C>) {
    this.#options = options;
    this.#connectTimeoutMs = options.connectTimeoutMs ?? DEFAULT_CONNECT_TIMEOUT_MS;
    this.#callTimeoutMs = options.callTimeoutMs ?? DEFAULT_CALL_TIMEOUT_MS;
    this.#watchdogIntervalMs = options.watchdogIntervalMs ?? DEFAULT_WATCHDOG_INTERVAL_MS;
    this.#idleProbeMs = options.idleProbeMs ?? DEFAULT_IDLE_PROBE_MS;
    this.#idleCloseMs = options.idleCloseMs ?? DEFAULT_IDLE_CLOSE_MS;
    this.#jitter = options.backoff?.jitter ?? DEFAULT_JITTER;
    this.#now = options.now ?? Date.now;
    this.#ladder = new ConnectionLadder({
      now: this.#now,
      baseMs: options.backoff?.baseMs,
      maxMs: options.backoff?.maxMs,
      stabilityWindowMs: options.backoff?.stabilityWindowMs,
    });
  }

  get state(): RpcState {
    return this.#state;
  }

  /** The result of the most recent successful handshake, so a subscriber can
   * pick it up when the FIRST success arrives via the reconnect ladder (a
   * retryForever dial whose initial connect() promise already rejected). */
  get handshakeResult(): unknown | undefined {
    return this.#lastHandshake;
  }

  /** The current rung of the reconnect ladder (0 while connected/idle), readable
   * synchronously from inside an onStateChange callback. */
  get attempt(): number {
    return this.#ladder.attempt;
  }

  /** The delay of the currently scheduled reconnect attempt, undefined once that attempt
   * starts dialing or none is pending. */
  get nextRetryInMs(): number | undefined {
    return this.#pendingRetryInMs;
  }

  connect(opts: { retryForever?: boolean } = {}): Promise<unknown> {
    // Idempotent: a connect() during a reconnect window (or over a live socket)
    // must not leak the pending backoff timer or the old socket. Detach the old
    // socket's handlers before closing it so its imminent close event can't drive
    // recovery logic against the fresh connection opened below.
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#clearConnectAttempt(new RpcError("connect superseded"));
    // The superseded attempt may have an in-flight handshake registered in
    // #pending; with its socket detached below, no close event will ever
    // sweep it, so the entry would sit there until a LATER #rejectAllPending
    // invokes its reject handler, which sets a spurious offline state and
    // closes whatever session is current by then. Discard it now.
    this.#discardPendingHandshake();
    this.#detachSession();
    this.#connectCalled = true;
    this.#explicitClose = false;
    this.#retryForever = opts.retryForever ?? false;
    // #everAuthed deliberately survives connect(): manual retry paths re-enter
    // through connect() (not reconnectNow()), and a once-ready client must keep
    // the forever-reconnect promise there too; resetting it made every manual
    // retry on those paths a one-shot. It resets only in close().
    this.#suppressReconnect = false;
    this.#pendingRetryInMs = undefined;
    this.#ladder.reset();
    return this.#openAndHandshake(false);
  }

  /** Manual retry: re-dials immediately, bypassing any pending backoff wait.
   * No-op if connect() was never called. Unlike connect(), this does NOT reset
   * #everAuthed: a manual retry that itself fails must still rejoin the
   * forever-reconnect ladder, not fall back to the never-ready fail-fast path. */
  reconnectNow(): void {
    if (!this.#connectCalled) return;
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#pendingRetryInMs = undefined;
    this.#ladder.reset();
    this.#clearConnectAttempt(new RpcError("retry superseded"));
    this.#discardPendingHandshake(); // same stranded-handshake hazard as connect() above
    this.#detachSession();
    void this.#openAndHandshake(true).catch(() => {
      // #handleClose already rescheduled the next attempt on failure.
    });
  }

  call<M extends keyof C["methods"] & string>(
    method: M,
    params: C["methods"][M]["params"],
  ): Promise<C["methods"][M]["result"]> {
    return this.#call(method, params) as Promise<C["methods"][M]["result"]>;
  }

  #call(method: string, params: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      // No socket: fail fast only when there is no reconnect run to wait for.
      // During a ladder run the call queues instead: a UI action fired in a
      // reconnect window rides out the blip (bounded by the call deadline
      // below) rather than failing on millisecond timing.
      if (!this.#session && (this.#explicitClose || !(this.#everAuthed || this.#retryForever))) {
        reject(new RpcError("not connected"));
        return;
      }
      const id = this.#nextId++;
      // Bounds the call whether in flight, queued during a dial, or queued
      // across a reconnect window: a half-open socket answers nothing, and a
      // queued call on a link that stays down must not strand its caller.
      const deadline = setTimeout(() => {
        if (!this.#pending.has(id)) return;
        this.#pending.delete(id);
        this.#sendQueue = this.#sendQueue.filter((q) => q.id !== id);
        reject(new RpcError(`${method} timed out after ${this.#callTimeoutMs}ms`));
      }, this.#callTimeoutMs);
      this.#pending.set(id, {
        resolve: (result) => {
          clearTimeout(deadline);
          resolve(result);
        },
        reject: (err) => {
          clearTimeout(deadline);
          reject(err);
        },
      });
      const queued: QueuedCall = { id, method, params };
      if (this.#authed && this.#session) this.#sendRaw(queued);
      else this.#sendQueue.push(queued);
    });
  }

  on<E extends keyof C["events"] & string>(event: E, handler: (params: C["events"][E]) => void): () => void {
    let handlers = this.#eventHandlers.get(event);
    if (!handlers) {
      handlers = new Set();
      this.#eventHandlers.set(event, handlers);
    }
    const wrapped = handler as (params: unknown) => void;
    handlers.add(wrapped);
    return () => handlers.delete(wrapped);
  }

  onStateChange(cb: (state: RpcState, detail?: string) => void): () => void {
    this.#stateHandlers.add(cb);
    return () => this.#stateHandlers.delete(cb);
  }

  /** Fires after a successful post-drop reconnect (handshake + resume done), so the app
   * can re-hydrate whatever it may have missed while disconnected. */
  onReconnected(cb: () => void): () => void {
    this.#reconnectHandlers.add(cb);
    return () => this.#reconnectHandlers.delete(cb);
  }

  close(): void {
    this.#explicitClose = true;
    this.#stopWatchdog();
    if (this.#reconnectTimer) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = undefined;
    }
    this.#pendingRetryInMs = undefined;
    this.#clearConnectAttempt(new RpcError("client closed"));
    this.#discardPendingHandshake();
    this.#rejectAllPending(new RpcError("client closed"));
    this.#detachSession();
    this.#authed = false;
    this.#everAuthed = false;
    this.#setState("closed");
  }

  /** Marks the current session dead and closes its transport. After this, no
   * event from that transport reaches the client (the Transport contract
   * plus the `detached` guard), the port of nulling ws handlers pre-close. */
  #detachSession(): void {
    const session = this.#session;
    if (!session) return;
    this.#session = undefined;
    session.detached = true;
    session.transport.close();
  }

  #openAndHandshake(isReconnect: boolean): Promise<unknown> {
    return new Promise((resolve, reject) => {
      this.#setState(isReconnect ? "reconnecting" : "connecting");
      const session: Session = { transport: undefined as unknown as Transport, detached: false };
      // An already-open transport (a child process's stdio, a test mock) may
      // fire onOpen from inside the factory call, before `session.transport`
      // or the connect timer exist — #handleOpen would then send the
      // handshake through an undefined transport, and the resulting throw
      // would ride the factory-failure branch below as a permanent dial
      // failure. Events fired during the factory call are buffered and
      // replayed once the session is fully wired.
      let preWireBuffer: Array<() => void> | undefined = [];
      const deliver = (event: () => void): void => {
        if (preWireBuffer) preWireBuffer.push(event);
        else event();
      };
      const handlers: TransportHandlers = {
        onOpen: () =>
          deliver(() => {
            if (!session.detached) this.#handleOpen(isReconnect, resolve, reject);
          }),
        onMessage: (frame) =>
          deliver(() => {
            if (!session.detached) this.#handleMessage(frame);
          }),
        onClose: () =>
          deliver(() => {
            if (!session.detached) this.#handleClose();
          }),
      };
      let transport: Transport;
      try {
        transport = this.#options.transport(handlers);
      } catch (err) {
        // A synchronous factory throw (malformed URL, exhausted fds) skips
        // every socket event, so nothing downstream would ever re-arm the
        // timer; without this branch the ladder dies silently and the state
        // sticks at "reconnecting" forever.
        if (this.#everAuthed || this.#retryForever) {
          this.#scheduleReconnect();
          this.#setState("reconnecting");
        } else {
          this.#setState("offline", err instanceof Error ? err.message : String(err));
        }
        reject(err instanceof Error ? err : new RpcError(String(err)));
        return;
      }
      session.transport = transport;
      this.#session = session;
      this.#authed = false;

      // Settle-once wiring for THIS attempt's handshake. If the socket dies
      // before it opens, #handleClose calls this; if it opens but the
      // handshake never answers, the timer below does. Either way connect()
      // rejects instead of hanging.
      this.#pendingHandshakeReject = (err) => {
        this.#pendingHandshakeReject = undefined;
        this.#clearConnectTimer();
        reject(err);
      };
      this.#connectTimer = setTimeout(() => {
        this.#connectTimer = undefined;
        const rejectHandshake = this.#pendingHandshakeReject;
        if (!rejectHandshake) return;
        this.#pendingHandshakeReject = undefined;
        // Detach + close the wedged socket so its late events can't drive
        // recovery against a connection nobody is waiting for. With close
        // detached, #handleClose never runs for this socket, so its duties
        // move here: sweep the in-flight handshake and any calls queued
        // during the dial (a queued call would otherwise neither reject nor
        // clear, and then be REPLAYED by #flushQueue on a much later
        // reconnect), and keep the "retry forever once ready" promise for a
        // dial that hangs rather than being refused (a sleeping host
        // black-holing the SYN must not end the ladder any more than an RST
        // does).
        session.detached = true;
        session.transport.close();
        if (this.#session === session) this.#session = undefined;
        this.#discardPendingHandshake();
        if (this.#everAuthed || this.#retryForever) {
          // Unsent calls queued during the dial survive to the next attempt;
          // their own deadlines bound the total wait.
          this.#rejectAllPending(new RpcError("connect timed out"), { keepQueued: true });
          this.#scheduleReconnect();
          this.#setState("reconnecting");
        } else {
          this.#rejectAllPending(new RpcError("connect timed out"));
          this.#setState("offline", "connect timed out");
        }
        rejectHandshake(new RpcError(`rpc connect timed out after ${this.#connectTimeoutMs}ms`));
      }, this.#connectTimeoutMs);

      // Replay AFTER the reject/timer wiring above: a replayed onOpen with
      // no handshake configured clears both on its way to ready, and wiring
      // them afterwards would arm a connect timer against an already-ready
      // session.
      const replay = preWireBuffer;
      preWireBuffer = undefined;
      for (const event of replay) event();
    });
  }

  #handleOpen(isReconnect: boolean, resolve: (result: unknown) => void, reject: (err: unknown) => void): void {
    const handshake = this.#options.handshake;
    if (!handshake) {
      // No handshake configured: the transport opening IS readiness.
      this.#pendingHandshakeReject = undefined;
      this.#clearConnectTimer();
      this.#becomeReady(undefined, isReconnect, resolve);
      return;
    }
    const id = this.#nextId++;
    this.#pendingHandshakeId = id;
    this.#pending.set(id, {
      resolve: (result) => {
        this.#pendingHandshakeId = undefined;
        this.#pendingHandshakeReject = undefined;
        this.#clearConnectTimer();
        this.#becomeReady(result, isReconnect, resolve);
      },
      reject: (err) => {
        this.#pendingHandshakeId = undefined;
        this.#pendingHandshakeReject = undefined;
        this.#clearConnectTimer();
        if (err instanceof RpcError && handshake.fatal?.(err)) this.#suppressReconnect = true;
        this.#setState("offline", err instanceof Error ? err.message : String(err));
        // The source closed the raw socket and let its close event run
        // #handleClose; the Transport contract suppresses onClose after
        // close(), so run the cascade directly.
        this.#detachSession();
        this.#handleClose();
        reject(err);
      },
    });
    const params = typeof handshake.params === "function" ? (handshake.params as () => unknown)() : handshake.params;
    this.#sendRaw({ id, method: handshake.method, params });
  }

  #becomeReady(result: unknown, isReconnect: boolean, resolve: (result: unknown) => void): void {
    this.#authed = true;
    this.#everAuthed = true;
    this.#lastHandshake = result;
    this.#ladder.noteConnected();
    this.#startWatchdog();
    this.#setState("ready");
    this.#flushQueue();
    resolve(result);
    void this.#afterConnected(isReconnect);
  }

  #clearConnectTimer(): void {
    if (this.#connectTimer) {
      clearTimeout(this.#connectTimer);
      this.#connectTimer = undefined;
    }
  }

  /** Cancels an in-flight connect()/handshake attempt, rejecting its pending promise
   * so it never strands; used when a fresh connect() supersedes it or close()
   * tears the client down. */
  #clearConnectAttempt(err: Error): void {
    this.#clearConnectTimer();
    const rejectHandshake = this.#pendingHandshakeReject;
    this.#pendingHandshakeReject = undefined;
    rejectHandshake?.(err);
  }

  /** Drops a still-in-flight handshake's #pending entry WITHOUT invoking its reject
   * handler: that handler's side effects (the fatal() check, #setState("offline", ...))
   * belong only to an explicit error response from the server (via #handleMessage),
   * never to the socket merely closing. Called before the generic #rejectAllPending
   * sweep in both close() and #handleClose() so a plain drop mid-handshake can't
   * masquerade as a protocol-level rejection and short-circuit the forever-reconnect
   * decision that follows. */
  #discardPendingHandshake(): void {
    if (this.#pendingHandshakeId !== undefined) {
      this.#pending.delete(this.#pendingHandshakeId);
      this.#pendingHandshakeId = undefined;
    }
  }

  async #afterConnected(isReconnect: boolean): Promise<void> {
    // resume runs after EVERY successful handshake, first dial included: a
    // conditional first-connect latch has a hole (a first-connect resume
    // swept by a drop would leave the latch unset, and every future
    // reconnect would then skip it). Idempotency is the resume callback's
    // contract with its server.
    const resume = this.#options.resume;
    if (resume) {
      try {
        await resume(this);
      } catch (err) {
        // If the socket dropped while this was in flight, #handleClose
        // already flipped #authed false and moved to 'reconnecting'; don't
        // stomp that back to 'ready'. The next reconnect's resume retries.
        if (!this.#authed) return;
        this.#setState("ready", err instanceof Error ? err.message : "resume failed");
      }
    }
    if (isReconnect) for (const handler of this.#reconnectHandlers) handler();
  }

  #handleClose(): void {
    const wasExplicit = this.#explicitClose;
    const suppressReconnect = this.#suppressReconnect;
    this.#suppressReconnect = false;
    this.#authed = false;
    this.#detachSession();
    this.#stopWatchdog();
    this.#discardPendingHandshake();
    const ridesLadder = !wasExplicit && !suppressReconnect && (this.#everAuthed || this.#retryForever);
    // In-flight calls reject (their fate on the wire is unknown); calls still
    // queued (never sent) survive a ladder-bound drop and flush on the next
    // successful handshake, bounded by their own deadlines.
    this.#rejectAllPending(new RpcError("connection closed"), { keepQueued: ridesLadder });
    // A socket that dies before or during the handshake never registered a
    // pending call, so #rejectAllPending can't reach the connect() promise;
    // settle it here or it hangs forever (#openAndHandshake only wires the
    // handshake call inside the open handler).
    const rejectHandshake = this.#pendingHandshakeReject;
    if (rejectHandshake) {
      this.#pendingHandshakeReject = undefined;
      this.#clearConnectTimer();
      rejectHandshake(new RpcError("connection closed before authentication"));
    }
    if (wasExplicit) {
      this.#setState("closed");
      return;
    }
    // The handshake reject handler already set a descriptive 'offline' state
    // for a fatal rejection: a reconnect backoff loop against a server we
    // know will refuse us can never succeed, so leave that state alone
    // rather than retrying.
    if (suppressReconnect) return;
    // Fail fast only on a client that has NEVER been ready (and isn't marked
    // retryForever): an interactive first connect() that never succeeds rolls
    // back instead of leaving a permanent reconnect loop against a
    // dead/unreachable endpoint. Once ready at least once, or dialed with
    // retryForever, retry forever (no attempt cap): a link that drops
    // mid-session should always come back on its own.
    if (!this.#everAuthed && !this.#retryForever) {
      this.#setState("offline");
      return;
    }
    // Advance the ladder + arm the timer BEFORE notifying listeners, so a
    // subscriber reading attempt/nextRetryInMs from inside its state-change
    // callback sees the rung this very 'reconnecting' transition represents.
    this.#scheduleReconnect();
    this.#setState("reconnecting");
  }

  #scheduleReconnect(): void {
    if (this.#reconnectTimer) return;
    this.#ladder.noteDisconnected();
    // Jitter is folded in BEFORE publishing: nextRetryInMs must report the
    // delay actually armed (a countdown built on it would otherwise hit zero
    // and sit there a large fraction of the rung while the timer still runs).
    const base = this.#ladder.nextDelayMs();
    const delay = base + Math.random() * base * this.#jitter;
    this.#pendingRetryInMs = delay;
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = undefined;
      this.#pendingRetryInMs = undefined;
      void this.#openAndHandshake(true).catch(() => {
        // #handleClose already rescheduled the next attempt on failure.
      });
    }, delay);
  }

  #flushQueue(): void {
    const queue = this.#sendQueue;
    this.#sendQueue = [];
    for (const item of queue) this.#sendRaw(item);
  }

  #sendRaw(msg: QueuedCall): void {
    this.#session!.transport.send(JSON.stringify({ jsonrpc: "2.0", ...msg }));
  }

  /** Liveness watchdog (runs only while ready). Any received message
   * refreshes #lastRxAt; at `idleProbeMs` of silence the configured probe
   * goes out (its reply, even an error, counts as rx); at `idleCloseMs` the
   * socket is declared dead and force-closed into the normal #handleClose
   * recovery path. A wall-clock jump between ticks (laptop suspend) forces
   * an immediate probe or close instead of waiting out the silence budget on
   * a socket that almost certainly died while asleep. */
  #startWatchdog(): void {
    this.#stopWatchdog();
    this.#lastRxAt = this.#now();
    this.#lastTickAt = this.#now();
    this.#watchdogTimer = setInterval(() => this.#watchdogTick(), this.#watchdogIntervalMs);
  }

  #stopWatchdog(): void {
    if (this.#watchdogTimer) {
      clearInterval(this.#watchdogTimer);
      this.#watchdogTimer = undefined;
    }
  }

  #watchdogTick(): void {
    if (!this.#authed || !this.#session) return;
    const now = this.#now();
    const resumed = now - this.#lastTickAt > this.#watchdogIntervalMs * 3;
    this.#lastTickAt = now;
    const silence = now - this.#lastRxAt;
    if (silence >= this.#idleCloseMs) {
      // Detach first so the real close event (if the stack ever delivers
      // one) can't run recovery twice against the fresh connection.
      this.#detachSession();
      this.#handleClose();
      return;
    }
    const probe = this.#options.probe;
    if (probe && (silence >= this.#idleProbeMs || resumed)) {
      void this.#call(probe.method, probe.params ?? {}).catch(() => {
        // Probe failure is not a verdict; the silence budget is.
      });
    }
  }

  #handleMessage(raw: string): void {
    // Every rx, including an error reply, refreshes the silence budget.
    this.#lastRxAt = this.#now();
    let msg: JsonRpcIncoming;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (msg.method && (msg.id === undefined || msg.id === null)) {
      const handlers = this.#eventHandlers.get(msg.method);
      if (handlers) for (const handler of handlers) handler(msg.params);
      return;
    }
    if (typeof msg.id !== "number") return;
    const pending = this.#pending.get(msg.id);
    if (!pending) return;
    this.#pending.delete(msg.id);
    if (msg.error) pending.reject(new RpcError(msg.error.message, msg.error.code, msg.error.data));
    else pending.resolve(msg.result);
  }

  #rejectAllPending(err: unknown, opts?: { keepQueued?: boolean }): void {
    if (opts?.keepQueued) {
      const queuedIds = new Set(this.#sendQueue.map((q) => q.id));
      for (const [id, pending] of [...this.#pending]) {
        if (queuedIds.has(id)) continue;
        this.#pending.delete(id);
        pending.reject(err);
      }
      return;
    }
    for (const pending of this.#pending.values()) pending.reject(err);
    this.#pending.clear();
    this.#sendQueue = [];
  }

  #setState(state: RpcState, detail?: string): void {
    this.#state = state;
    for (const handler of this.#stateHandlers) handler(state, detail);
  }
}
