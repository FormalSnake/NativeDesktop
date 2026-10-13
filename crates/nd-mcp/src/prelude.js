// Runs inside QuickJS before any agent code. Natives (`__nd_*`) answer with a JSON envelope,
// `{"ok": ..}` or `{"error": ".."}`, so every failure surfaces as a JS exception with the message.
(() => {
  const call = (name, ...args) => {
    const reply = JSON.parse(globalThis[name](...args));
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
  const flatten = (node, out = []) => {
    out.push(node);
    node.children.forEach((c) => flatten(c, out));
    return out;
  };
  const refOf = (target) => {
    if (typeof target === "number") return target;
    if (typeof target === "string" && /^e\d+$/.test(target)) return Number(target.slice(1));
    if (target && typeof target.id === "number") return target.id;
    throw new Error("click takes a ref number, an \"eN\" string, or a node from app.find()");
  };

  globalThis.state = {};
  globalThis.console = {};
  for (const level of ["log", "info", "warn", "error", "debug"]) {
    console[level] = (...args) => __nd_log(args.map(show).join(" "));
  }

  globalThis.app = {
    /** The semantic tree as nested objects: { id, role, name, value, test_id, enabled, focused, children }. */
    tree: () => call("__nd_tree"),
    /** The compact text tree, one line per node with its [ref=eN]. */
    snapshot: (options = {}) => call("__nd_snapshot", JSON.stringify(options)),
    /** Nodes matching every given field: role, name, testId (exact) and text (substring of name or value). */
    find: (query = {}) =>
      flatten(app.tree()).filter(
        (n) =>
          (query.role === undefined || n.role === query.role) &&
          (query.name === undefined || n.name === query.name) &&
          (query.testId === undefined || n.test_id === query.testId) &&
          (query.text === undefined ||
            (n.name ?? "").includes(query.text) ||
            (n.value ?? "").includes(query.text)),
      ),
    /** Semantic click on a ref (number or "eN") or a node from find(). */
    click: (target) => call("__nd_click", refOf(target)),
  };

  // Indirect eval: the completion value of the last expression statement is the result.
  globalThis.__nd_run = (code) => {
    const value = (0, eval)(code);
    return value === undefined ? "null" : JSON.stringify(value);
  };
})();
