// Minimal CDP client for the Chrome-style gate. The host's own
// `executeDevToolsMethod` plumbing speaks to one browser at a time through the
// app; this speaks to the remote debugging port, which is the only way to reach
// a service worker target, Chromium's input pipeline and chrome:// pages.

export interface TargetInfo {
  id: string;
  type: string;
  title: string;
  url: string;
  webSocketDebuggerUrl?: string;
}

export async function targets(port: number): Promise<TargetInfo[]> {
  const res = await fetch(`http://127.0.0.1:${port}/json/list`);
  return await res.json() as TargetInfo[];
}

/** Polls the target list until `match` finds one, or throws after timeoutMs. */
export async function waitForTarget(
  port: number,
  match: (t: TargetInfo) => boolean,
  timeoutMs = 30000,
): Promise<TargetInfo> {
  const deadline = Date.now() + timeoutMs;
  let last: TargetInfo[] = [];
  while (Date.now() < deadline) {
    try {
      last = await targets(port);
      const hit = last.find(match);
      if (hit) return hit;
    } catch {
      // The port is not listening yet.
    }
    await Bun.sleep(250);
  }
  throw new Error(`no matching CDP target after ${timeoutMs}ms; saw ${JSON.stringify(last.map((t) => `${t.type} ${t.url}`))}`);
}

type Handler = (params: Record<string, unknown>) => void;

export class Session {
  private ws: WebSocket;
  private next = 1;
  private pending = new Map<number, { resolve: (v: Record<string, unknown>) => void; reject: (e: Error) => void }>();
  private handlers = new Map<string, Handler[]>();

  private constructor(ws: WebSocket) {
    this.ws = ws;
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(String((event as MessageEvent).data)) as {
        id?: number;
        result?: Record<string, unknown>;
        error?: { message: string };
        method?: string;
        params?: Record<string, unknown>;
      };
      if (msg.id !== undefined) {
        const call = this.pending.get(msg.id);
        if (!call) return;
        this.pending.delete(msg.id);
        if (msg.error) call.reject(new Error(msg.error.message));
        else call.resolve(msg.result ?? {});
        return;
      }
      if (!msg.method) return;
      for (const h of this.handlers.get(msg.method) ?? []) h(msg.params ?? {});
    });
  }

  static async open(wsUrl: string): Promise<Session> {
    // CEF only accepts a debugger socket whose Origin it was told to allow, and
    // the launch path passes --remote-allow-origins=*.
    const ws = new WebSocket(wsUrl, { headers: { Origin: "http://127.0.0.1" } } as unknown as string[]);
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error(`could not open ${wsUrl}`)), { once: true });
    });
    return new Session(ws);
  }

  send(method: string, params: Record<string, unknown> = {}): Promise<Record<string, unknown>> {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  on(method: string, handler: Handler): void {
    const list = this.handlers.get(method) ?? [];
    list.push(handler);
    this.handlers.set(method, list);
  }

  /** Runtime.evaluate with the promise awaited, answering the JSON value. */
  async eval<T = unknown>(expression: string, userGesture = false): Promise<T> {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture,
    }) as { result?: { value?: T }; exceptionDetails?: { text?: string; exception?: { description?: string } } };
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? "evaluate failed");
    }
    return result.result?.value as T;
  }

  close(): void {
    this.ws.close();
  }
}
