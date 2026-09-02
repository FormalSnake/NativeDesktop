#!/usr/bin/env bun
// scripts/menubar-drive.ts: drives examples/notes/menubar-probe.tsx over the
// automation socket (mirrors scripts/threepane-drive.ts / notes-drive.ts). M13
// Feature A: proves the <menubar>/<menu>/<menuitem> machinery on BOTH backends.
//
// (a) getTree surfaces the menubar/menu/menuitem nodes with their testIDs and
//     labels-as-text.
// (b) semanticClick on the File>New Thing menuitem fires onSelect: the counter
//     label increments (count=0 -> count=1).
// (c) the disabled item's semanticClick does NOT increment: the item's GAction
//     (GTK) / NSMenuItem validation (mac) blocks its onSelect, so a second
//     enabled click yields count=2, never count=102. Contract: a disabled
//     menuitem's click dispatches but its onSelect never fires.
import { connectApp, expect } from "../packages/test/src/index.ts";

const app = await connectApp();

// The initial counter label text is a stable settle signal.
await app.waitForText("count=0");

// (a) menu tree present, labels surfaced as text.
await expect(app.getByTestId("probe-menubar")).toBeAttached();
const fileMenu = app.getByTestId("probe-file-menu");
const probeMenu = app.getByTestId("probe-menu");
const newThing = app.getByTestId("probe-new-thing");
const disabled = app.getByTestId("probe-disabled");
await expect(app.getByTestId("probe-about")).toBeAttached();
await expect(fileMenu).toHaveText("File");
await expect(probeMenu).toHaveText("Probe");
await expect(newThing).toHaveText("New Thing");
await expect(disabled).toHaveText("Disabled Item");

// (b) click New Thing: onSelect increments the counter.
await newThing.click();
await app.waitForText("count=1");

// (c) click the disabled item: onSelect must NOT fire (would add 100). The
// item is disabled, so the click goes through the raw ref instead of the
// locator's actionability gate, which would otherwise wait forever for it to
// become enabled. An immediate read must still show count=1...
const disabledRef = await disabled.ref();
await app.rpc.call("click", { ref: disabledRef });
await expect(app.getByTestId("probe-counter")).toHaveText("count=1");

// ...and a second enabled click yields count=2 (never 102), settling the race:
// had the disabled onSelect fired (+100), this would be count=102 and the wait
// would time out.
await newThing.click();
await app.waitForText("count=2");

console.log(
  `ND_MENUBAR_PROBE_OK menubar+menu+menuitem in tree, onSelect fired (count=2), disabled click was a no-op`,
);
await app.close();
