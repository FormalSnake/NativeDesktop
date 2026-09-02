#!/usr/bin/env bun
// scripts/sourcetree-drive.ts [gtk|appkit] — drives examples/sourcetree/main.tsx
// over the automation socket via @nativedesktop/test. Asserts the SourceTree
// wave checklist: rows/roles/testIDs in getTree, selectionChanged by id,
// rowActivated, expand/collapse honoring the controlled `expanded` flags,
// actionClicked {nodeId, actionId} + native disclosure events (AppKit
// pointer/keys leg; GTK cannot synthesize input, -32003), a plain <button>
// event in the same window, hasCommand/hasWidget from the handshake manifest,
// app.isActive() with host replay, and the AppKit `toolbar` structural class
// (screenshot). Leg 7 drives the ND_ST_GEOMETRY probe windows and measures
// row geometry off captures: the disclosure gutter a flat list must not
// reserve, and the row content a hover-visibility action button must not
// move. Prints ND_SOURCETREE_OK on success.
import { inflateSync } from "node:zlib";
import { expect, launchApp, poll } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";
// Every wait in this drive is a UI round trip that settles in well under a
// second on an idle machine. The budget is env-overridable because the same
// drive runs inside scripts/browser-gate.sh, right after a full build on a box
// that may be shared, where 3s is not headroom.
const T = Number(process.env.ND_DRIVE_TIMEOUT_MS ?? 3000);

// ---- row geometry off a capture -------------------------------------------
// getTree carries geometry per WIDGET and never per row, so a capture is the
// only channel that can answer where a row's title actually sits. What
// follows is a minimal reader for the 8-bit non-interlaced PNGs both backends
// write, plus an ink profiler: for one row's scanline band it returns the
// contiguous column runs that contrast with the row fill, in logical units
// relative to the widget's left edge. Every piece of row content — each glyph
// of the title, the badge, the action button — is one run, so two states of
// the same rows can be compared column for column instead of by eye.
interface Png {
  w: number;
  h: number;
  channels: number;
  data: Uint8Array;
}
type Run = [number, number];

function decodePng(bytes: Uint8Array): Png {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let off = 8, w = 0, h = 0, depth = 0, color = 0, interlace = 0;
  const idat: Uint8Array[] = [];
  while (off + 8 <= bytes.length) {
    const len = view.getUint32(off);
    const type = String.fromCharCode(bytes[off + 4]!, bytes[off + 5]!, bytes[off + 6]!, bytes[off + 7]!);
    if (type === "IHDR") {
      w = view.getUint32(off + 8);
      h = view.getUint32(off + 12);
      depth = bytes[off + 16]!;
      color = bytes[off + 17]!;
      interlace = bytes[off + 20]!;
    } else if (type === "IDAT") {
      idat.push(bytes.subarray(off + 8, off + 8 + len));
    } else if (type === "IEND") break;
    off += 12 + len;
  }
  const channels = ({ 0: 1, 2: 3, 4: 2, 6: 4 } as Record<number, number>)[color];
  if (depth !== 8 || interlace !== 0 || !channels) {
    throw new Error(`unsupported PNG (depth=${depth} colorType=${color} interlace=${interlace})`);
  }
  const packed = new Uint8Array(idat.reduce((a, b) => a + b.length, 0));
  let at = 0;
  for (const chunk of idat) { packed.set(chunk, at); at += chunk.length; }
  const raw = inflateSync(packed);
  const stride = w * channels;
  const out = new Uint8Array(h * stride);
  let ri = 0;
  for (let y = 0; y < h; y++) {
    const filter = raw[ri++]!;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? cur[i - channels]! : 0;
      const b = prev ? prev[i]! : 0;
      const c = prev && i >= channels ? prev[i - channels]! : 0;
      let v = raw[ri + i]!;
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) throw new Error(`bad PNG filter ${filter}`);
      cur[i] = v & 0xff;
    }
    ri += stride;
  }
  return { w, h, channels, data: out };
}

function luminance(img: Png, x: number, y: number): number {
  const i = (y * img.w + x) * img.channels;
  if (img.channels <= 2) return img.data[i]!;
  return (img.data[i]! * 299 + img.data[i + 1]! * 587 + img.data[i + 2]! * 114) / 1000;
}

/// Column runs per row band inside `rect` (logical, window top-left space).
/// The row fill is taken as the rect's modal luminance rather than a fixed
/// threshold, so the profile reads the same in dark appearance and under a
/// hover highlight.
function profileRows(img: Png, rect: { x: number; y: number; w: number; h: number }, scale: number): Run[][] {
  const x0 = Math.round(rect.x * scale), x1 = Math.min(img.w, Math.round((rect.x + rect.w) * scale));
  const y0 = Math.round(rect.y * scale), y1 = Math.min(img.h, Math.round((rect.y + rect.h) * scale));
  const hist = new Map<number, number>();
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const l = luminance(img, x, y) & ~3;
      hist.set(l, (hist.get(l) ?? 0) + 1);
    }
  }
  let fill = 0, best = -1;
  for (const [l, count] of hist) if (count > best) { best = count; fill = l; }
  const isInk = (x: number, y: number): boolean => Math.abs(luminance(img, x, y) - fill) > 60;
  const bands: { y0: number; y1: number }[] = [];
  for (let y = y0; y < y1; y++) {
    let ink = false;
    for (let x = x0; x < x1; x++) if (isInk(x, y)) { ink = true; break; }
    if (!ink) continue;
    const last = bands[bands.length - 1];
    if (last && last.y1 === y - 1) last.y1 = y;
    else bands.push({ y0: y, y1: y });
  }
  return bands.map((band) => {
    const runs: Run[] = [];
    let start = -1;
    for (let x = x0; x < x1; x++) {
      let ink = false;
      for (let y = band.y0; y <= band.y1; y++) if (isInk(x, y)) { ink = true; break; }
      if (ink && start < 0) start = x;
      if (!ink && start >= 0) { runs.push([(start - x0) / scale, (x - 1 - x0) / scale]); start = -1; }
    }
    if (start >= 0) runs.push([(start - x0) / scale, (x1 - 1 - x0) / scale]);
    return runs;
  });
}

const fmtRun = (r: Run): string => `${r[0].toFixed(1)}-${r[1].toFixed(1)}`;

const app = await launchApp({ entry: "examples/sourcetree/main.tsx", backend });
try {
  // ---- leg 1: getTree shape — role, itemCount, rows with testIDs ------------
  const tree = await app.mustFind("st-tree");
  if (tree.role !== "tree") throw new Error(`st-tree role=${tree.role}, want "tree"`);
  if (tree.itemCount !== 9) throw new Error(`st-tree itemCount=${tree.itemCount}, want 9`);
  const rows = tree.rows ?? [];
  const byTestId = new Map(rows.map((r) => [r.testID, r]));
  for (const id of ["st-sec-hosts", "st-host-mac", "st-proj-nd", "st-run-1", "st-run-old"]) {
    if (!byTestId.has(id)) throw new Error(`getTree rows missing testID ${id} (got: ${rows.map((r) => r.testID).join(",")})`);
  }
  if (byTestId.get("st-run-1")!.title !== "fix sidebar") throw new Error("st-run-1 row title mismatch");
  if (byTestId.get("st-run-1")!.badge !== "3") throw new Error("st-run-1 row badge mismatch");
  if (byTestId.get("st-host-mac")!.iconName !== "computer-symbolic") throw new Error("st-host-mac row iconName mismatch");
  // Rows carry their own identity, not just an index: a drive names a row.
  if (byTestId.get("st-run-1")!.id !== "run-1") throw new Error(`st-run-1 row id=${byTestId.get("st-run-1")!.id}, want run-1`);
  console.log(`ND_ST_TREE_OK role=tree rows=${rows.length} testIDs and ids present`);

  // iconData: raw image bytes where a freedesktop icon name cannot reach (a
  // favicon). Only the capture can prove the pixels, so this asserts the row
  // survived the decode and leaves the pixels to the screenshot legs below.
  if (!byTestId.has("st-run-2")) throw new Error("st-run-2 (the iconData row) missing from getTree rows");
  console.log("ND_ST_ICONDATA_OK row with iconData rendered");

  // ---- leg 1b: a plain <button> in the same window still fires --------------
  // The toolbar button's handler is `onClick`, the name the Button schema
  // declares. An `on*` prop the schema does NOT declare (`onClicked` here)
  // registers no listener at all, while the click RPC keeps answering
  // dispatched:true and the native `clicked` signal keeps being emitted, so
  // the failure looks like a dead widget rather than a typo. Nothing else in
  // this drive exercises a button event, which is what let that read as a
  // sourcetree-specific defect.
  await app.getByTestId("st-toolbar-refresh").click();
  await app.waitForText("toolbar refresh", { timeoutMs: T });
  console.log("ND_ST_BUTTON_OK <button> onClick fired next to the tree");

  // ---- leg 2: handshake manifest + activation replay ------------------------
  await app.waitForText("caps present=true nope=false sourcetree=true", { timeoutMs: T });
  // The host records the launch activation state and replays it right after
  // HelloAck; a background spawn legitimately starts inactive, so first assert
  // only that the replay landed, then frontmost the process (works for both
  // backends; nd-hello runs via Quartz on macOS) and wait for the live flip.
  // The replay frame races the child's FIRST render (under load it lands
  // after commit 0) and the readout only recomputes on a re-render: poke one
  // with a selection round-trip before asserting.
  await app.getByTestId("st-tree").fill("run-1");
  await app.waitForText("replay=yes", { timeoutMs: T });
  await app.getByTestId("st-tree").fill("");
  await app.waitForText("sel (none)", { timeoutMs: T });
  // The live flip needs the process frontmost, which only macOS can be asked
  // for; a headless weston seat never activates a window, so the GTK leg
  // asserts the replay and stops there rather than failing on a missing
  // osascript.
  if (app.backend === "appkit") {
    const frontmost = Bun.spawnSync([
      "osascript", "-e",
      `tell application "System Events" to set frontmost of (first application process whose unix id is ${app.pid}) to true`,
    ]);
    if (frontmost.exitCode !== 0) throw new Error(`osascript frontmost failed: ${frontmost.stderr.toString()}`);
    await app.waitForText("active true replay=yes", { timeoutMs: T * 2 });
    console.log("ND_ST_CAPS_OK hasCommand true/false + hasWidget + isActive replay/live-flip verified");
  } else {
    console.log("ND_ST_CAPS_OK hasCommand true/false + hasWidget + isActive replay verified (no live flip: headless has no seat)");
  }

  // ---- leg 3: selection by node id (setValue -> selectionChanged -> a11y) ---
  await app.getByTestId("st-tree").fill("run-1");
  await app.waitForText("sel run-1", { timeoutMs: T });
  await expect(app.getByTestId("st-tree")).toHaveValue("run-1");
  console.log("ND_ST_SELECT_OK selectionChanged {nodeId} + a11y value by id");

  // ---- leg 3b: selectedId "" clears the selection ---------------------------
  // GTK regression: unselectAll is a documented no-op in browse mode, so the
  // clear path goes through selectRow(null). Re-select before leg 4, whose
  // semantic click activates the selected row.
  await app.getByTestId("st-tree").fill("");
  await app.waitForText("sel (none)", { timeoutMs: T });
  await app.getByTestId("st-tree").fill("run-1");
  await app.waitForText("sel run-1", { timeoutMs: T });
  console.log('ND_ST_CLEAR_OK setValue("") deselected; re-select landed');

  // ---- leg 4: semantic click activates the selected row ---------------------
  await app.getByTestId("st-tree").click();
  await app.waitForText("act run-1", { timeoutMs: T });
  console.log("ND_ST_ACTIVATE_OK rowActivated {nodeId}");

  // ---- leg 5: expansion honors the controlled `expanded` flags --------------
  // sec-settled starts collapsed: its child row does not exist natively, so
  // an id-addressed setValue must fail loudly rather than select nothing.
  let collapsedRejected = false;
  try {
    await app.getByTestId("st-tree").fill("run-old");
  } catch {
    collapsedRejected = true;
  }
  if (!collapsedRejected) throw new Error("setValue(run-old) inside a collapsed section should have failed");
  await app.getByTestId("st-settled-toggle").click(); // app state -> expanded flag -> rebuild
  // Rows live in the widget's `rows` payload, not as tree nodes, so the
  // reveal is waited on there. The click RPC returns before the toggle's JS
  // round-trip lands the nodes update.
  await poll(
    () => app.getByTestId("st-tree").node(),
    (n) => (n.rows ?? []).some((r) => r.testID === "st-run-old"),
    { timeoutMs: T, intervalMs: 100 },
  );
  await app.getByTestId("st-tree").fill("run-old");
  await app.waitForText("sel run-old", { timeoutMs: T });
  console.log("ND_ST_EXPAND_PROP_OK collapsed shelf hides rows; expanding reveals them");

  // ---- leg 5b: semantic row action (both backends) ---------------------------
  // click {testId: <row testID>, action} dispatches the named trailing action
  // without a real pointer, the path GTK needs (no input synthesis) and the
  // one CanaryOrchestrator-style e2e flows use for row-level buttons.
  await app.click({ testId: "st-run-1", action: "close-run" });
  await app.waitForText("action close-run@run-1", { timeoutMs: T });
  // An action the node does not declare must fail loudly, not silently no-op.
  let undeclaredRejected = false;
  try {
    await app.click({ testId: "st-run-1", action: "new-run" });
  } catch {
    undeclaredRejected = true;
  }
  if (!undeclaredRejected) throw new Error("click(st-run-1, action new-run) should have failed: node does not declare it");
  console.log("ND_ST_ROWACTION_OK click {testId, action} dispatched actionClicked; undeclared action rejected");

  // ---- leg 6: native gestures (AppKit only; GTK4 cannot synthesize input) ---
  if (app.backend === "appkit") {
    const g = (await app.mustFind("st-tree")).geometry;
    if (!g) throw new Error("st-tree has no geometry");
    // Row 0 is proj-nd (26pt-ish single-line source rows). A real click both
    // selects it and makes the outline first responder for the keys below.
    const row0y = g.y + 14;
    await app.rpc.call("pointer", { phase: "down", x: g.x + 80, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: g.x + 80, y: row0y });
    await app.waitForText("sel proj-nd", { timeoutMs: T });
    console.log("ND_ST_POINTER_SELECT_OK pointer click selected proj-nd");

    await app.keyboard.press("ArrowLeft"); // collapse the selected parent
    await app.waitForText("expand collapsed:proj-nd", { timeoutMs: T });
    await app.keyboard.press("ArrowRight"); // expand it again
    await app.waitForText("expand expanded:proj-nd", { timeoutMs: T });
    console.log("ND_ST_EXPAND_EVENT_OK nodeCollapsed/nodeExpanded from native disclosure keys");

    // Trailing action button on row 0 ("New Run", labeled inline button at
    // the row's right edge; actionVisibility "always" keeps it clickable).
    const actionX = g.x + g.w - 45;
    await app.rpc.call("pointer", { phase: "down", x: actionX, y: row0y });
    await app.rpc.call("pointer", { phase: "up", x: actionX, y: row0y });
    await app.waitForText("action new-run@proj-nd", { timeoutMs: T });
    console.log("ND_ST_ACTION_OK actionClicked {nodeId, actionId}");

    const shot = await app.screenshot(`${shotDir}/sourcetree-appkit.png`, { minBytes: 2000 });
    console.log(`ND_ST_TOOLBAR_SHOT ${shot.path} ${shot.width}x${shot.height}`);
  } else {
    console.log("ND_ST_GESTURES_SKIP gtk: input synthesis unsupported (-32003); disclosure/action click paths verified on the appkit leg");
    const shot = await app.screenshot(`${shotDir}/sourcetree-gtk.png`, { minBytes: 2000 });
    console.log(`ND_ST_SHOT ${shot.path} ${shot.width}x${shot.height}`);
  }

  // Notification click needs a real user click on an OS banner; the data
  // correlation map is covered by packages/react/src/system.test.ts instead.
  console.log("ND_ST_NOTIFICATION_SKIP data echo covered by unit tests (OS banner click is not synthesizable)");
} finally {
  await app.close();
}

// ---- leg 7: row geometry, measured ----------------------------------------
// Each variant is the SAME window with one property changed, so two captures
// are directly comparable. The probe hosts run one at a time, after the main
// host is closed: a GApplication is single-instance per id, so two live GTK
// hosts sharing the gate's ND_APP_ID would collide.
const rowProfile = async (variant: string, hoverFirst = false): Promise<Run[][]> => {
  const label = hoverFirst ? `${variant}+hover` : variant;
  const probe = await launchApp({
    entry: "examples/sourcetree/main.tsx",
    backend,
    env: { ND_ST_GEOMETRY: variant, ND_APP_ID: `dev.nativedesktop.stGeo${process.pid}${variant}` },
  });
  try {
    const widget = await probe.mustFind("st-geo");
    const win = (await probe.tree()).root.geometry;
    if (!widget.geometry || !win) throw new Error(`st-geo (${label}) has no geometry`);
    if (hoverFirst) {
      await probe.getByTestId("st-geo").hover();
      await new Promise((r) => setTimeout(r, 300));
    }
    const shot = await probe.screenshot(`${shotDir}/sourcetree-geo-${label}.png`, { minBytes: 2000 });
    const img = decodePng(new Uint8Array(await Bun.file(shot.path).arrayBuffer()));
    const rows = profileRows(img, widget.geometry, img.w / win.w);
    if (rows.length < 2) throw new Error(`st-geo (${label}) profiled ${rows.length} row bands, want at least 2`);
    return rows;
  } finally {
    await probe.close();
  }
};

const flat = await rowProfile("flat");
const deep = await rowProfile("deep");
const flatX = flat[0]![0]![0], deepX = deep[0]![0]![0];
// Row 0 is a leaf at depth 0 in BOTH trees and differs only in whether the
// tree holds an expandable node at all (deep's sits last, off screen). A tree
// with no branch anywhere owes its rows no disclosure gutter; one with a
// branch keeps it on every row so branch and leaf titles share an origin.
if (!(flatX < deepX - 4)) {
  throw new Error(`flat-list title x=${flatX} did not clear the disclosure gutter (branching tree x=${deepX})`);
}
if (Math.abs(deepX - deep[1]![0]![0]) > 0.5) {
  throw new Error(`branching tree rows 0/1 disagree on title x (${deepX} vs ${deep[1]![0]![0]})`);
}
console.log(`ND_ST_INDENT_OK flat-list title x=${flatX}, same rows under a branching tree x=${deepX} (gutter ${deepX - flatX})`);

// Appearing actions must not change the allocation. Neither backend can
// synthesize a real pointer crossing — GTK4 has no app-constructible input at
// all (-32003), and AppKit's `hover` posts an NSEvent, which never produces
// the NSTrackingArea crossing an NSTableRowView reveals its actions from — so
// the two allocation states are reached through the prop instead:
// actionVisibility "hover" un-hovered (actions hidden) against "always"
// (actions drawn). The fix makes the hidden state keep the slot, so every run
// of row content has to land on the same columns in both, and the drawn
// button is the only run the "always" profile adds.
const hidden = await rowProfile("hover");
const shown = await rowProfile("always");
const hiddenRow = hidden[0]!, shownRow = shown[0]!;
for (let i = 0; i < hiddenRow.length; i++) {
  const a = hiddenRow[i]!, b = shownRow[i];
  if (!b || Math.abs(a[0] - b[0]) > 0.5 || Math.abs(a[1] - b[1]) > 0.5) {
    throw new Error(
      `hover-visibility actions shifted the row at run ${i}: hidden ${fmtRun(a)} vs shown ${b ? fmtRun(b) : "(missing)"}` +
        ` — hidden row [${hiddenRow.map(fmtRun).join(" ")}], shown row [${shownRow.map(fmtRun).join(" ")}]`,
    );
  }
}
if (shownRow.length <= hiddenRow.length) throw new Error("actionVisibility=always drew no extra run: the action button is missing");
console.log(
  `ND_ST_ACTIONSLOT_OK ${hiddenRow.length} content runs identical with actions hidden and drawn` +
    ` (title x=${hiddenRow[0]![0]}, last run ends ${hiddenRow[hiddenRow.length - 1]![1]});` +
    ` the drawn action adds ${fmtRun(shownRow[shownRow.length - 1]!)}`,
);

// The hover RPC cannot reach the reveal (see above), so this only proves the
// row does not move under it; the pair above carries the real proof.
if (app.backend === "appkit") {
  const hoveredRow = (await rowProfile("hover", true))[0]!;
  for (let i = 0; i < hiddenRow.length; i++) {
    if (Math.abs(hiddenRow[i]![0] - (hoveredRow[i]?.[0] ?? NaN)) > 0.5) {
      throw new Error(`row content moved under hover at run ${i}: ${fmtRun(hiddenRow[i]!)} vs ${hoveredRow[i] ? fmtRun(hoveredRow[i]!) : "(missing)"}`);
    }
  }
  console.log(`ND_ST_HOVER_OK title x=${hoveredRow[0]![0]} unchanged across the hover RPC`);
} else {
  console.log("ND_ST_HOVER_SKIP gtk: no synthetic pointer, so the hidden/drawn pair above is the whole proof");
}

// ---- leg 8: a narrow row's two lines use the row, and stop at it -----------
// The `long` variant is a browser's tab list: titles and captions both longer
// than the row. Two rules, one profile. Nothing may reach the row's trailing
// margin, because the caption used to have no right edge at all and ran under
// the action button and out to where the scroll view finally clipped it. And
// the rightmost glyph has to land near the action slot rather than well short
// of it, because the row owes its text everything but the icon and the
// actions.
const longProbe = await launchApp({
  entry: "examples/sourcetree/main.tsx",
  backend,
  env: { ND_ST_GEOMETRY: "long", ND_APP_ID: `dev.nativedesktop.stLong${process.pid}` },
});
try {
  const widget = (await longProbe.mustFind("st-geo")).geometry;
  const win = (await longProbe.tree()).root.geometry;
  if (!widget || !win) throw new Error("st-geo (long) has no geometry");
  const shot = await longProbe.screenshot(`${shotDir}/sourcetree-geo-long.png`, { minBytes: 2000 });
  const img = decodePng(new Uint8Array(await Bun.file(shot.path).arrayBuffer()));
  const bands = profileRows(img, { x: 0, y: widget.y, w: win.w, h: widget.h }, img.w / win.w);
  const row = bands[0];
  if (!row || row.length < 3) throw new Error(`st-geo (long) row 0 profiled ${row?.length ?? 0} runs, want at least 3`);
  const right = widget.x + widget.w;
  const last = row[row.length - 1]!;
  // The action button is the rightmost thing a row draws, and it sits inset
  // from the row's own edge. Ink closer than that inset means something ran
  // through the action slot to the clip.
  if (right - last[1] < 8) {
    throw new Error(
      `row content runs to x=${last[1]}, ${(right - last[1]).toFixed(1)}pt from the tree's right edge at ${right}` +
        ` (row runs: ${row.map(fmtRun).join(" ")})`,
    );
  }
  // The run before the action button is the rightmost glyph of a title or a
  // caption, and it has to reach the action rather than stop well short of it.
  // What "reach" is measured FROM differs by backend, because the chrome
  // between the last glyph and the tree's own edge does. An AppKit source-list
  // cell spends about 26pt there, so the tree's right edge is a fair mark. A
  // libadwaita row spends 60 before a glyph is possible at all: a 34px
  // `.circular` chip, plus the 26px its header box is inset from the list edge
  // (6px row margin, 20px row inset), both read off the live allocations. GTK
  // takes the same 48pt reach from the action's leading ink instead, which
  // still catches the regression this leg exists for: a suffix holding width
  // out of the title's share moves the text about 20pt left.
  const text = row[row.length - 2]!;
  const reach = longProbe.backend === "appkit" ? right : last[0];
  if (text[1] < reach - 48) {
    throw new Error(
      `rightmost text ends at ${text[1]}, ${(reach - text[1]).toFixed(1)}pt short of ${reach}` +
        ` (row runs: ${row.map(fmtRun).join(" ")})`,
    );
  }
  console.log(
    `ND_ST_CLIP_OK narrow row: text ends ${fmtRun(text)}, action ${fmtRun(last)}, tree right edge ${right}`,
  );
} finally {
  await longProbe.close();
}

console.log(`ND_SOURCETREE_OK backend=${app.backend}`);
