#!/usr/bin/env bun
// Minimal pluginCommand driver for headless-m10.sh leg 3. `pluginCommand`
// isn't a typed Ndp method (M10 scope kept runtime/ndp.ts untouched for this
// task), so this opens the NDP socket directly and hand-frames two messages:
// a `hello` handshake (required before the host will route anything) and one
// `pluginCommand` invoking the demo plugin's `greet` command. Works for both
// the granted and ungranted runs — the host prints either
// ND_PLUGIN_COMMAND_OK or ND_ACL_DENY depending on the ACL grants it was
// started with; this driver only needs to deliver the frame.
const path = process.env.ND_SOCKET!;
const socket = await Bun.connect({ unix: path, socket: { data() {}, close() {} } });
function frame(obj: unknown): Uint8Array {
  const body = new TextEncoder().encode(JSON.stringify(obj));
  const f = new Uint8Array(4 + body.length);
  new DataView(f.buffer).setUint32(0, body.length, true); f.set(body, 4); return f;
}
socket.write(frame({ type: "hello", ndpVersion: 1, runtime: { name: "bun", version: Bun.version } }));
await Bun.sleep(300);
socket.write(frame({ type: "pluginCommand", plugin: "hello", command: "greet", arg: { name: "m10" } }));
await Bun.sleep(500);
console.error("M10_PLUGIN_DRIVE_DONE");
process.exit(0);
