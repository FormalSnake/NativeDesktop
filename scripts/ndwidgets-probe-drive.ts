#!/usr/bin/env bun
// scripts/ndwidgets-probe-drive.ts: drives examples/ndwidgets-probe over the
// automation socket and asserts the 2026-07-17 nd-widgets wave on GTK: C3
// icon-only vs icon+label button text routing, both `<window>`s present in
// the a11y tree, and the C5 "present" widgetCommand dispatching without
// error via a semantic click (GTK has no synthetic pointer/hover input:
// -32003, src/gtk/backend.zig's vtSemanticAction, so real onHoverChanged
// interaction is verified on AppKit instead, where `hover` posts a real
// mouseMoved).
import { connectApp, expect } from "../packages/test/src/index.ts";

const app = await connectApp();

// C3: iconName + label="" is icon-only (no text); iconName + non-empty
// label renders icon AND text.
await expect(app.getByTestId("icon-only-btn")).toHaveText("");
await expect(app.getByTestId("icon-label-btn")).toHaveText("Refresh");
console.log("ND_C3_OK icon-only vs icon+label button text routing correct");

// Both probe windows exist (root A + orphaned B, tabs-drive.ts's counting idiom).
await expect(app.getByRole("window")).toHaveCount(2);
console.log("ND_WINDOWS_OK both probe windows present in the a11y tree");

// C5: "present" dispatches cleanly through the GTK tabs.zig arm (a real
// pointer-driven raise/focus is verified on AppKit, see the Mac recipe).
await app.getByTestId("present-b-btn").click();
console.log('ND_C5_OK Window "present" widgetCommand dispatched with no error');

// navigation-sidebar reach check: both host sections + all three run rows
// (nested two levels below the classed box) resolve in the tree and their
// clicks dispatch (GTK gets this "for free": a CSS class cascades
// regardless of nesting depth); the AppKit native-table-row equivalent is
// verified on the Mac recipe (SidebarTable.swift's recursive row collection).
for (const id of ["host-a", "host-b", "run-a1", "run-a2", "run-b1"]) {
  await expect(app.getByTestId(id)).toBeAttached();
}
await app.getByTestId("run-b1").click();
console.log("ND_SIDEBAR_OK nested navigation-sidebar rows resolved and clickable");

await app.screenshot(process.env.ND_SHOT_PATH ?? "/tmp/nd-widgets-probe.png");

console.log("ND_WIDGETS_PROBE_OK C3 button parity + both windows + present dispatch verified");
await app.close();
