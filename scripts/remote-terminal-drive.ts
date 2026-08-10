#!/usr/bin/env bun
// scripts/remote-terminal-drive.ts [backend] — drives the <terminal remote>
// example over @nativedesktop/test against scripts/remote-terminal-fake-
// server.ts. Starts the fake byte-plane server itself (ephemeral port),
// launches examples/remote-terminal with the matching ND_REMOTE_* env, and
// asserts: the terminal node is present; connectionState reached ATTACHED;
// the scripted title/bell/exit events were observed (surfaced into labels);
// the FLAG_RESET snapshot was fed (its title "snapshot-ready" arrives only
// after the reset); and a screenshot renders with non-zero dimensions (the
// banner bytes painted — the terminal grid itself has no getTree text).
//
// GTK builds/runs need the nix devshell (pkg-config + brew GTK). AppKit
// needs only zig+swift on PATH.
import { resolve } from "node:path";
import { launchApp } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const outPng = process.env.ND_SHOT_PATH ?? "/tmp/remote-terminal-shot.png";
const repoRoot = resolve(import.meta.dir, "..");

async function startFakeServer(): Promise<{ port: number; proc: import("bun").Subprocess }> {
  const proc = Bun.spawn(["bun", "scripts/remote-terminal-fake-server.ts", "0"], {
    cwd: repoRoot,
    stdout: "pipe",
    stderr: "inherit",
  });
  const reader = proc.stdout.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) throw new Error("fake server exited before printing FAKE_SERVER_LISTENING");
    buf += decoder.decode(value, { stream: true });
    const match = /FAKE_SERVER_LISTENING port=(\d+)/.exec(buf);
    if (match?.[1]) return { port: Number(match[1]), proc };
  }
}

const { port, proc: fakeServer } = await startFakeServer();
console.log(`fake server on port ${port}`);

const app = await launchApp({
  entry: "examples/remote-terminal/main.tsx",
  backend,
  env: {
    ND_REMOTE_HOST: "127.0.0.1",
    ND_REMOTE_PORT: String(port),
    ND_REMOTE_SESSION: "sess-demo",
    ND_REMOTE_TICKET: "ticket-demo",
  },
});

try {
  const term = await app.findMatching((n) => n.testID === "remote-term" || n.type === "terminal");
  if (!term) throw new Error("terminal node not found in getTree");

  // connectionState -> ATTACHED (surfaced as `conn: attached`).
  await app.waitForText("conn: attached", { timeoutMs: 8000 });
  // onTitleChanged for the post-reset snapshot title — arrives only after the
  // FLAG_RESET frame cleared the grid and its bytes were fed (proves reset).
  await app.waitForText("title: snapshot-ready", { timeoutMs: 8000 });
  // onBell.
  await app.waitForText("bells: 1", { timeoutMs: 8000 });
  // onExited (code 0).
  await app.waitForText("exit: 0", { timeoutMs: 8000 });

  const shot = await app.screenshot(outPng);
  if (shot.width <= 0 || shot.height <= 0) throw new Error("screenshot has no dimensions");

  console.log(`REMOTE_DRIVE_OK term=#${term.ref} attached title bell exit reset png=${shot.path} ${shot.width}x${shot.height}`);
} finally {
  await app.close();
  fakeServer.kill();
}
