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

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Clicks the docked inspector's own close button, in the frontend attached to
 * `frontend`, and answers where it was. Null means the toolbar has no close
 * control, which is what an inspector that was never told it can dock looks
 * like: the button is in the DOM either way, carrying `hidden` and no box.
 *
 * A real mouse event rather than a DOM `.click()`, so the frontend's own
 * toolbar handling runs.
 */
export async function clickDevToolsClose(frontend: Session): Promise<Box | null> {
  return await clickIn(frontend, DEVTOOLS_CLOSE);
}

type Handler = (params: Record<string, unknown>) => void;

export class Session {
  private ws: WebSocket;
  private next = 1;
  private pending = new Map<number, { resolve: (v: Record<string, unknown>) => void; reject: (e: Error) => void }>();
  private handlers = new Map<string, Handler[]>();
  private gone: Error | null = null;

  private constructor(ws: WebSocket) {
    this.ws = ws;
    // A browser that dies takes the socket with it, and a call left pending on
    // a dead socket is a driver that never finishes rather than one that
    // reports what happened.
    const drop = (why: string) => {
      this.gone ??= new Error(why);
      for (const call of this.pending.values()) call.reject(this.gone);
      this.pending.clear();
    };
    ws.addEventListener("close", () => drop("the debugger socket closed"), { once: true });
    ws.addEventListener("error", () => drop("the debugger socket failed"), { once: true });
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
    if (this.gone) return Promise.reject(this.gone);
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

/**
 * What the docked frontend reserved for the inspected page, and how wide the
 * frontend's own document is, read from `frontend`.
 *
 * A frontend told it can dock lays its panels out around a hole the embedder
 * is supposed to draw the page into, and announces that rectangle with
 * `setInspectedPageBounds` (in device mode it is the device's rectangle, which
 * is how the phone ends up inside the page area). The message goes out through
 * `DevToolsHost.sendMessageToEmbedder`, so this patches that channel and asks
 * the frontend to lay out again.
 */
export async function inspectedPageBounds(
  frontend: Session,
): Promise<{ bounds: Box | null; width: number; height: number }> {
  const raw = await frontend.eval<string>(`(() => {
    const host = window.DevToolsHost;
    if (host && host.sendMessageToEmbedder && !host.__ndProbeHook) {
      const original = host.sendMessageToEmbedder.bind(host);
      host.sendMessageToEmbedder = (message) => {
        try {
          const parsed = typeof message === 'string' ? JSON.parse(message) : message;
          if (parsed && parsed.method === 'setInspectedPageBounds') window.__ndProbeBounds = parsed.params[0];
        } catch (error) {}
        return original(message);
      };
      host.__ndProbeHook = true;
    }
    window.dispatchEvent(new Event('resize'));
    return new Promise((done) => setTimeout(() => done(JSON.stringify({
      bounds: window.__ndProbeBounds ?? null,
      width: innerWidth,
      height: innerHeight,
    })), 250));
  })()`);
  return JSON.parse(raw) as { bounds: Box | null; width: number; height: number };
}

/** The frontend's device-toolbar toggle, the phone icon, as a `frontendBox` finder. */
export const DEVICE_TOOLBAR = `(() => {
  let hit = null;
  const walk = (root) => {
    for (const el of root.querySelectorAll('*')) {
      const label = el.getAttribute && el.getAttribute('aria-label');
      if (!hit && label && /device toolbar/i.test(label)) hit = el;
      if (el.shadowRoot) walk(el.shadowRoot);
    }
  };
  walk(document);
  return hit;
})()`;

/** The docked frontend's own splitter between the page and its panels. */
export const FRONTEND_SPLITTER = `(() => {
  let hit = null;
  const walk = (root) => {
    for (const el of root.querySelectorAll('*')) {
      const cls = typeof el.className === 'string' ? el.className : '';
      if (!hit && /resizer/.test(cls)) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.width < 20 && r.height > innerHeight - 4) hit = el;
      }
      if (el.shadowRoot) walk(el.shadowRoot);
    }
  };
  walk(document);
  return hit;
})()`;

/** The docked inspector's own close button. */
export const DEVTOOLS_CLOSE = `(() => {
  let hit = null;
  const walk = (root) => {
    for (const el of root.querySelectorAll('*')) {
      if (!hit && el.classList && el.classList.contains('close-devtools')) hit = el;
      if (el.shadowRoot) walk(el.shadowRoot);
    }
  };
  walk(document);
  return hit;
})()`;

/**
 * Where the element `finder` answers with sits in the frontend's viewport, in
 * CSS pixels, or null when there is none or it has no box. A docked frontend
 * fills the view, so this plus the view's origin is where a real pointer goes.
 */
export async function frontendBox(frontend: Session, finder: string): Promise<Box | null> {
  return await frontend.eval<Box | null>(`(() => {
    const el = ${finder};
    if (!el) return null;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  })()`);
}

/**
 * Clicks the frontend's device-toolbar toggle. Device mode is the case the
 * hole stops being a column and becomes the device's own rectangle, which is
 * where Chrome draws the phone.
 */
export async function clickDeviceToolbar(frontend: Session): Promise<Box | null> {
  return await clickIn(frontend, DEVICE_TOOLBAR);
}

/**
 * Drags the docked frontend's own splitter, which is what a user does to make
 * the inspector wider: the hole moves with it and the page has to follow.
 */
export async function dragFrontendSplitter(frontend: Session, dx: number): Promise<boolean> {
  const box = await frontendBox(frontend, FRONTEND_SPLITTER);
  if (!box) return false;
  const from = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await frontend.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...from, button: "none", buttons: 0 });
  await frontend.send("Input.dispatchMouseEvent", { type: "mousePressed", ...from, button: "left", buttons: 1, clickCount: 1 });
  // In steps: the split widget follows the pointer rather than the drop, and a
  // single jump is a gesture its handler never sees the middle of.
  for (let step = 1; step <= 4; step++) {
    await frontend.send("Input.dispatchMouseEvent", {
      type: "mouseMoved", x: from.x + (dx * step) / 4, y: from.y, button: "left", buttons: 1,
    });
  }
  await frontend.send("Input.dispatchMouseEvent", {
    type: "mouseReleased", x: from.x + dx, y: from.y, button: "left", buttons: 0, clickCount: 1,
  });
  return true;
}

/// A real mouse click on whatever `finder` answers with, in the frontend.
async function clickIn(frontend: Session, finder: string): Promise<Box | null> {
  const box = await frontendBox(frontend, finder);
  if (!box) return null;
  const at = { x: box.x + box.width / 2, y: box.y + box.height / 2, button: "left", clickCount: 1 };
  await frontend.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...at, button: "none", buttons: 0 });
  await frontend.send("Input.dispatchMouseEvent", { type: "mousePressed", ...at, buttons: 1 });
  await frontend.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...at, buttons: 0 });
  return box;
}
