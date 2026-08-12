// Regression test for AppHandle.close() hanging when a crashed host leaves
// its own bun child (spawned with `.stderr = .inherit`, src/runtime.zig)
// orphaned and still holding the stderr pipe's write end open. Drives a fake
// host fixture instead of a real nd-hello/NDShell so the test doesn't need a
// native build.
import { afterEach, expect, test } from "bun:test";
import { launchApp } from "../src/launch.ts";

const FAKE_HOST = new URL("./fixtures/fake-crashing-host.ts", import.meta.url).pathname;
const CLOSE_BOUND_MS = 5000;

let grandchildPid: number | undefined;

afterEach(() => {
  if (grandchildPid !== undefined) {
    try {
      process.kill(grandchildPid, "SIGKILL");
    } catch {
      // already gone
    }
    grandchildPid = undefined;
  }
});

test("close() resolves within the bound even when the host's own child outlives it", async () => {
  const handle = await launchApp({
    entry: "unused",
    hostBinary: FAKE_HOST,
    readyMarkers: ["FAKE_READY"],
    retries: 0,
    onStderr: (line) => {
      const m = /GRANDCHILD_PID=(\d+)/.exec(line);
      if (m) grandchildPid = Number(m[1]);
    },
  });

  expect(grandchildPid).toBeDefined();

  const start = Date.now();
  await Promise.race([
    handle.close(),
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error(`close() did not resolve within ${CLOSE_BOUND_MS}ms`)), CLOSE_BOUND_MS),
    ),
  ]);
  expect(Date.now() - start).toBeLessThan(CLOSE_BOUND_MS);

  // The grandchild is a real orphaned process (not killed by close(), which
  // only owns the direct child) -- confirm it's still alive so the test
  // actually exercised the held-open-pipe case, not a no-op.
  expect(() => process.kill(grandchildPid!, 0)).not.toThrow();
});
