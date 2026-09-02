#!/usr/bin/env bun
// scripts/command-palette-drive.ts: drives examples/command-palette over the
// automation socket. The palette is mounted beside other content and the app
// re-renders on a background tick, so this proves the routed palette actions
// (type/click/setValue string|index|bool -> queryChanged/activate/submit) reach
// React reliably under the same churn that broke Return/click before the fix.
import { connectApp } from "@nativedesktop/test";

const app = await connectApp();

async function waitText(s: string): Promise<void> {
  const w = await app.waitForText(s, { timeoutMs: 4000 });
  if (!w.matched) throw new Error(`waitFor ${JSON.stringify(s)} did not match`);
}

const palette = app.getByTestId("palette");

// 1. Open the palette (real button click), then confirm it presents: the
//    palette node reports visible only while presented.
await app.getByTestId("open-button").click();
const paletteRef = await palette.ref();
const vis = await app.waitFor({ refVisible: paletteRef }, { timeoutMs: 4000 });
if (!vis.matched) throw new Error("palette did not present");

// 2. type -> queryChanged (append into the real search entry).
await palette.type("Dev");
await waitText("Query: Dev");

// 3. click -> activate the highlighted row (the single "Developer" match): a
//    directory, so the app drills in and keeps the palette open.
await palette.click();
await waitText("Folder: /Users/kyan/Developer");

// 4. setValue string -> replace the query text.
await palette.fill("NativeDesktop");
await waitText("Query: NativeDesktop");

// 4b. setValue OVER A NON-EMPTY query: the controlled round trip. Regression
//     guard: GtkEditable.setText is a delete plus an insert, and while the
//     entry's change signal was GtkSearchEntry's debounced "search-changed",
//     the delete's synchronous empty reached the app, came back as the `query`
//     prop, and that programmatic set killed the pending timer carrying the
//     real value: the field collapsed. Every other setValue here runs against
//     an empty query (the app clears it on activation), which is exactly why
//     the collapse went unnoticed.
await palette.fill("NativeDeskto");
await waitText("Query: NativeDeskto");
await palette.fill("NativeDesktop");
await waitText("Query: NativeDesktop");

// 5. setValue integer -> activate the row at that index (drills again).
await palette.fill(0);
await waitText("Folder: /Users/kyan/Developer/NativeDesktop");

// 6. setValue string then bool -> submit the raw query as-is (no matching row).
await palette.fill("/tmp/typed-path");
await waitText("Query: /tmp/typed-path");
await palette.fill(true);
await waitText("Picked: /tmp/typed-path");

console.log("CP_DRIVE_OK palette driven under churn: type/click/setValue string|index|submit");
await app.close();
