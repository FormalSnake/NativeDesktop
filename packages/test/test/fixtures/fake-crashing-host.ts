#!/usr/bin/env bun
// Fixture for launch-close.test.ts: reproduces src/runtime.zig's `.stderr =
// .inherit` bun-child spawn without needing a real host binary. Opens an
// automation socket, announces ready, spawns a grandchild that inherits this
// process's stderr and outlives it, then exits (simulating a crash) --
// leaving the grandchild as the sole holder of the pipe's write end, exactly
// the shape that used to make AppHandle.close() hang forever.
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const sockPath = join(mkdtempSync(join(tmpdir(), "nd-fake-host-")), "sock");
Bun.listen({ unix: sockPath, socket: { open() {}, data() {}, close() {} } });

const grandchild = Bun.spawn(["sleep", "30"], { stdin: "ignore", stdout: "ignore", stderr: "inherit" });

console.error(`GRANDCHILD_PID=${grandchild.pid}`);
console.error(`ND_AUTOMATION_LISTENING path=${sockPath}`);
console.error("FAKE_READY");

// Give the parent's stderr pump a chance to read the lines above before the
// pipe's other end (this process) closes, so `waitUntilReady` doesn't race
// `proc.exited` against the not-yet-processed marker.
await Bun.sleep(50);
process.exit(0);
