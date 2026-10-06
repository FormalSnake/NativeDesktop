// Pure-TS OS shell helpers. Unlike system.ts, these do NOT round-trip
// through the host or go through ACL — they spawn commands directly in the
// app's own unsandboxed Bun process, the same way any other Node/Bun script
// would (see CLAUDE.md: the Bun child is a full runtime, not a sandboxed
// renderer, so it already has process-spawning access with no IPC boundary).

import { dirname } from "node:path";
import { Platform } from "./platform.ts";

async function run(cmd: string[]): Promise<void> {
  const proc = Bun.spawn(cmd, { stdin: "ignore", stdout: "ignore", stderr: "ignore" });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`${cmd.join(" ")} exited with code ${code}`);
}

/** Opens `url` in the OS default browser/handler (`open` on macOS, `xdg-open` on Linux). */
export async function openExternal(url: string): Promise<void> {
  await run(Platform.os === "macos" ? ["open", url] : ["xdg-open", url]);
}

/** Opens `path` with its OS-registered default application. */
export async function openPath(path: string): Promise<void> {
  await run(Platform.os === "macos" ? ["open", path] : ["xdg-open", path]);
}

/** Reveals `path` in the OS file manager, selecting it if the file manager supports that. On Linux, tries the freedesktop FileManager1 DBus interface first (selects the file) and falls back to opening its containing directory if that fails (e.g. no DBus session or no file manager registered on it). */
export async function revealPath(path: string): Promise<void> {
  if (Platform.os === "macos") {
    await run(["open", "-R", path]);
    return;
  }
  try {
    await run([
      "dbus-send",
      "--session",
      "--dest=org.freedesktop.FileManager1",
      "--type=method_call",
      "/org/freedesktop/FileManager1",
      "org.freedesktop.FileManager1.ShowItems",
      `array:string:file://${path}`,
      "string:",
    ]);
  } catch {
    await run(["xdg-open", dirname(path)]);
  }
}
