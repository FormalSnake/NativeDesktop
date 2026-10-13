// Runs inside QuickJS before any agent code. `__nd(op)` answers with a JSON envelope,
// `{"ok": ..}` or `{"error": ".."}`, so every failure surfaces as a JS exception with the message.
(() => {
  const call = (op) => {
    const reply = JSON.parse(__nd(JSON.stringify(op)));
    if (reply.error !== undefined) throw new Error(reply.error);
    return reply.ok;
  };
  const show = (v) => {
    if (typeof v === "string") return v;
    try {
      return JSON.stringify(v);
    } catch {
      return String(v);
    }
  };
  const quote = (s) => JSON.stringify(String(s));

  const locator = (selector) => {
    const act = (action, arg) => call({ op: "locator", selector, action, arg });
    const chain = (part) => locator(selector ? `${selector} >> ${part}` : part);
    return {
      selector,
      locator: (sub) => chain(sub),
      getByRole: (role, o = {}) =>
        chain(`role=${role}${o.name !== undefined ? `[name=${quote(o.name)}]` : ""}${o.exact ? "[exact]" : ""}`),
      getByTestId: (id) => chain(`testid=${id}`),
      getByText: (t) => chain(`text=${quote(t)}`),
      getByLabel: (t) => chain(`label=${quote(t)}`),
      getByPlaceholder: (t) => chain(`placeholder=${quote(t)}`),
      first: () => chain("first"),
      last: () => chain("last"),
      nth: (i) => chain(`nth=${i}`),
      count: () => act("count"),
      all: () => act("all"),
      node: () => act("node"),
      text: () => act("text"),
      value: () => act("value"),
      isVisible: () => act("is_visible"),
      isEnabled: () => act("is_enabled"),
      isChecked: () => act("is_checked"),
      isFocused: () => act("is_focused"),
      click: () => act("click"),
      dblclick: () => act("dblclick"),
      rightClick: () => act("right_click"),
      hover: () => act("hover"),
      focus: () => act("focus"),
      fill: (value) => act("fill", value),
      type: (text) => act("type", text),
      press: (keys) => act("press", keys),
      scrollIntoView: () => act("scroll_into_view"),
      /** state: "attached" | "detached" | "visible" | "hidden" */
      waitFor: (state = "visible") => act("wait_for", state),
    };
  };

  globalThis.state = {};
  globalThis.console = {};
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    console[level] = (...args) => __nd_log(args.map(show).join(" "));
  }

  globalThis.app = {
    /** Compact tree, one line per node with its [ref=eN]. */
    snapshot: (window) => call({ op: "snapshot", window }),
    /** The semantic tree of every window as nested objects. */
    tree: (window) => call({ op: "tree", window }),
    windows: () => call({ op: "rpc", method: "windows", params: {} }),
    locator: (selector) => locator(selector),
    getByRole: (role, o) => locator("").getByRole(role, o),
    getByTestId: (id) => locator("").getByTestId(id),
    getByText: (t) => locator("").getByText(t),
    getByLabel: (t) => locator("").getByLabel(t),
    getByPlaceholder: (t) => locator("").getByPlaceholder(t),
    keyboard: {
      /** GPUI key syntax: "cmd-a", "shift-tab", "enter". */
      press: (keys) => call({ op: "key", keys }),
      type: (text) => call({ op: "type", text }),
    },
    screenshot: (path, window) => call({ op: "rpc", method: "screenshot", params: { path, window } }),
    /** Raw automation RPC, for methods not wrapped above. */
    rpc: (method, params = {}) => call({ op: "rpc", method, params }),
  };

  // Indirect eval: the completion value of the last expression statement is the result.
  globalThis.__nd_run = (code) => {
    const value = (0, eval)(code);
    return value === undefined ? "null" : JSON.stringify(value);
  };
})();
