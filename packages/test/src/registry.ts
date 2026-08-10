// Process-lifetime registry of launched host binaries. A thrown assertion in
// a drive script (or a test runner's own SIGINT) must not orphan a native
// window — killAll() is wired to fire on every exit path exactly once.

const live = new Set<import("bun").Subprocess>();
let wired = false;

export function registerProcess(proc: import("bun").Subprocess): void {
  live.add(proc);
  wireSignalHandlers();
}

export function unregisterProcess(proc: import("bun").Subprocess): void {
  live.delete(proc);
}

/** Kills every still-registered host process. Safe to call more than once. */
export function killAll(): void {
  for (const proc of live) {
    try {
      proc.kill("SIGKILL");
    } catch {
      // already dead
    }
  }
  live.clear();
}

function wireSignalHandlers(): void {
  if (wired) return;
  wired = true;
  process.on("exit", killAll);
  process.on("SIGINT", () => {
    killAll();
    process.exit(130);
  });
  process.on("SIGTERM", () => {
    killAll();
    process.exit(143);
  });
}
