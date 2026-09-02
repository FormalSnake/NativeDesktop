#!/usr/bin/env bun
// scripts/parity-drive.ts [backend] — drives examples/parity/main.tsx (the
// gpui-parity gallery) over the automation socket via @nativedesktop/test.
// Walks all fourteen sidebar sections and, for each, asserts through getTree
// that the widgets are really there: schema role, enabled, a11y value, and a
// NON-ZERO on-screen rectangle, because the failure this exists to catch is a
// widget that mounts, reports fine, and paints nothing. Then it exercises the
// behaviour the wave-3 widgets shipped with — dialog/sheet open+dismiss,
// chart type switching, `enabled`, ComboBox selection, TabView
// selectionChanged, Table multi-select — and shoots one screenshot per
// section. Prints ND_PARITY_OK on success.
//
// AppKit-only for the interactive legs: crumb/tab/row clicks ride the
// `pointer` and `keys` RPCs, which answer -32003 on GTK (GTK4 removed
// app-constructible events), the same gap gestures-drive.ts documents.
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, hasRealSize, launchApp, renderValue, type AppHandle, type JsonNode } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const ROOT = resolve(import.meta.dir, "..");
const backend = (process.argv[2] as Backend | undefined) ?? "appkit";
const shotDir = process.env.ND_SHOT_DIR ?? resolve(ROOT, "docs/plans/parity-shots");
// The debug shell in this checkout, unless the caller names another host.
// ND_HOST_BINARY wins outright inside @nativedesktop/host too; passing it
// explicitly keeps the default working without exporting anything.
const debugShell = resolve(ROOT, "swift/.build/debug/NDShell");
const hostBinary =
  process.env.ND_HOST_BINARY ?? (backend === "appkit" && existsSync(debugShell) ? debugShell : undefined);

// A screenshot through the automation RPC comes back as an empty frame for
// this app's shape (offscreen render, the blanking screenshot.ts documents),
// so the shots go through ScreenCaptureKit. Focus first: an unfocused capture
// dims the selection and misreads as broken chrome.
const shotVia = (process.env.ND_SHOT_VIA as "rpc" | "ndshot" | undefined) ?? "ndshot";

/// Zero-size nodes that are already broken for reasons outside this wave's
/// widgets, verified by hand against the same run. Everything else that is
/// visible must have a real rectangle, so a new collapse fails the drive.
const KNOWN_ZERO_SIZE: Record<string, string> = {
  // hexpand Slider next to a fixed-width LevelIndicator inside one Row suffix:
  // the pair overflows the suffix and one of the two loses all its width —
  // which one is not stable between runs, so both are listed.
  "display-continuous-slider": "Slider+LevelIndicator overflow one Row suffix",
  "display-continuous-indicator": "Slider+LevelIndicator overflow one Row suffix",
  "composition-otp-cell-0": "OtpInput cells overflow the Row suffix",
  "composition-otp-cell-1": "OtpInput cells overflow the Row suffix",
  "composition-otp-cell-2": "OtpInput cells overflow the Row suffix",
  "composition-otp-cell-3": "OtpInput cells overflow the Row suffix",
  "composition-otp-cell-4": "OtpInput cells overflow the Row suffix",
  "composition-otp-cell-5": "OtpInput cells overflow the Row suffix",
};

/// Widget kinds whose handle is host-only chrome and never a rectangle: an
/// overlay handle presents its own window, a toolbar pane is logical.
const HANDLE_ONLY = new Set(["Dialog", "Sheet", "ToolbarView", "HeaderBar", "Popover", "TrayItem"]);

interface Expect {
  role?: string;
  type?: string;
  text?: string;
  value?: string | number | boolean;
  valueContains?: string;
  enabled?: boolean;
  /** Skip the rectangle check (a handle, or a deliberately hidden node). */
  handle?: boolean;
}

function fail(msg: string): never {
  throw new Error(msg);
}

function checkNode(node: JsonNode, testId: string, e: Expect): void {
  const where = `${testId} (${node.type})`;
  if (e.type && node.type !== e.type) fail(`${where}: type=${node.type}, want ${e.type}`);
  if (e.role && node.role !== e.role) fail(`${where}: role=${node.role}, want ${e.role}`);
  if (e.text !== undefined && node.text !== e.text) fail(`${where}: text=${JSON.stringify(node.text)}, want ${JSON.stringify(e.text)}`);
  if (e.value !== undefined && node.value !== e.value) fail(`${where}: value=${JSON.stringify(node.value)}, want ${JSON.stringify(e.value)}`);
  if (e.valueContains !== undefined && !renderValue(node.value).includes(e.valueContains)) {
    fail(`${where}: value does not contain ${JSON.stringify(e.valueContains)} (got ${JSON.stringify(node.value)})`);
  }
  const wantEnabled = e.enabled ?? true;
  if (node.enabled !== wantEnabled) fail(`${where}: enabled=${node.enabled}, want ${wantEnabled}`);
  if (e.handle) return;
  if (!node.visible) fail(`${where}: visible=false — the widget mounted but is not on screen`);
  if (!hasRealSize(node)) {
    const g = node.geometry;
    fail(g ? `${where}: renders as a ${g.w}x${g.h} box at (${g.x},${g.y})` : `${where}: no geometry`);
  }
}

async function expectWidgets(app: AppHandle, spec: Record<string, Expect>): Promise<void> {
  const tree = await app.tree();
  const byId = new Map<string, JsonNode>();
  const walk = (n: JsonNode): void => {
    if (n.testID && !byId.has(n.testID)) byId.set(n.testID, n);
    n.children.forEach(walk);
  };
  walk(tree.root);
  for (const [testId, e] of Object.entries(spec)) {
    const node = byId.get(testId);
    if (!node) fail(`${testId} is not in the tree`);
    checkNode(node, testId, e);
  }
}

/// Every visible, testID'd node in the current section must occupy real
/// pixels. This is the blanket net under the per-widget expectations above.
async function expectNothingCollapsed(app: AppHandle, section: string): Promise<void> {
  const tree = await app.tree();
  const bad: string[] = [];
  const walk = (n: JsonNode): void => {
    const g = n.geometry;
    if (n.testID && n.visible && !HANDLE_ONLY.has(n.type) && g && !hasRealSize(n)) {
      if (!(n.testID in KNOWN_ZERO_SIZE)) bad.push(`${n.testID} (${n.type}) ${g.w}x${g.h}`);
    }
    n.children.forEach(walk);
  };
  walk(tree.root);
  if (bad.length) fail(`${section}: ${bad.length} visible widget(s) render at zero size: ${bad.join(", ")}`);
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/// The widget's rectangle, for the legs that aim a raw pointer at a
/// sub-region a locator cannot name (a crumb, a tab strip, a table row band).
async function boxOf(app: AppHandle, testId: string): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await app.getByTestId(testId).boundingBox();
  if (!box) fail(`${testId}: no geometry to aim at`);
  return box;
}

const sections = [
  "display", "input", "navigation", "composition", "data", "overlays", "richtext",
  "charts", "progress", "loading", "dock", "tiles", "dragdrop", "codeeditor",
] as const;

/// One node per section that only exists while that section is mounted, so
/// the sidebar selection is confirmed before anything is measured.
const sectionAnchor: Record<string, string> = {
  display: "display-avatar-group", input: "input-group", navigation: "nav-breadcrumb-group",
  composition: "composition-accordion-group", data: "data-table", overlays: "overlays-dialog-group",
  richtext: "richtext-group", charts: "charts-tabs", progress: "progress-live-group",
  loading: "loading-group", dock: "dock-section", tiles: "tiles-section",
  dragdrop: "dragdrop-group", codeeditor: "codeeditor-widget",
};

mkdirSync(shotDir, { recursive: true });
const app = await launchApp({ entry: "examples/parity/main.tsx", backend, hostBinary, logPath: process.env.ND_HOST_LOG });
const shots: string[] = [];

try {
  // A capture of an unfocused window shows a dimmed selection and misreads as
  // broken chrome, so front the app once before any screenshot.
  const frontmost = Bun.spawnSync([
    "osascript", "-e",
    `tell application "System Events" to set frontmost of (first application process whose unix id is ${app.pid}) to true`,
  ]);
  if (frontmost.exitCode !== 0) throw new Error(`osascript frontmost failed: ${frontmost.stderr.toString()}`);
  await sleep(400);

  async function show(section: string): Promise<void> {
    await app.getByTestId("parity-nav").fill(section);
    await app.waitForPresent(sectionAnchor[section]!, { timeoutMs: 4000 });
    await sleep(250); // one layout pass after the pane content swaps
  }

  async function shoot(index: number, section: string): Promise<void> {
    const path = `${shotDir}/${String(index).padStart(2, "0")}-${section}.png`;
    const shot = await app.screenshot(path, { via: shotVia, minBytes: 2000 });
    shots.push(`${section} ${shot.width}x${shot.height}`);
    console.log(`ND_PARITY_SHOT ${section} ${shot.path} ${shot.width}x${shot.height}`);
  }

  // ---- display: Avatar / Badge / Tag / Kbd ---------------------------------
  await show("display");
  await expectWidgets(app, {
    "display-avatar-24": { type: "Avatar", role: "image", text: "Ada Lovelace" },
    "display-avatar-64": { type: "Avatar", role: "image", text: "Ada Lovelace" },
    "display-badge-live": { type: "Badge", role: "label", text: "Live", value: "Live" },
    "display-badge-neutral": { type: "Badge", role: "label", value: "neutral" },
    "display-badge-error": { type: "Badge", role: "label", value: "error" },
    "display-badge-dot": { type: "Badge", role: "label" },
    "display-tag-t1": { type: "Tag", role: "label", text: "React" },
    "display-tag-t3": { type: "Tag", role: "label", text: "Swift" },
    "display-kbd-palette": { type: "Kbd", role: "label", value: "⌘K" },
    "display-kbd-rename": { type: "Kbd", role: "label", value: "F2" },
  });
  await expectNothingCollapsed(app, "display");
  // Avatar follows its `text` prop; Badge follows the variant Select.
  await app.getByTestId("display-avatar-input").fill("Grace Hopper");
  await expect(app.getByTestId("display-avatar-48")).toHaveText("Grace Hopper");
  await app.getByTestId("display-badge-select").selectOption(4);
  await expect(app.getByTestId("display-badge-live")).toHaveText("Live");
  // Tag add/remove is real state, not decoration.
  await app.getByTestId("display-tag-input").fill("Bun");
  await app.getByTestId("display-tag-add-button").click();
  await app.waitForPresent("display-tag-t4", { timeoutMs: 3000 });
  await expectWidgets(app, { "display-tag-t4": { type: "Tag", role: "label", text: "Bun" } });
  console.log("ND_PARITY_DISPLAY_OK avatar/badge/tag/kbd present, sized, and state-driven");
  await shoot(1, "display");

  // ---- input: ComboBox + the universal `enabled` prop ----------------------
  await show("input");
  await expectWidgets(app, {
    "input-combobox": { type: "ComboBox", role: "combobox", value: "Apple" },
    "input-enabled-button": { type: "Button", enabled: true },
  });
  await expectNothingCollapsed(app, "input");
  // ComboBox selection: an index picks a list row (selectionChanged ->
  // changed), a string is free text; both must reach app state.
  await app.getByTestId("input-combobox").selectOption(2);
  await expect(app.getByTestId("input-combobox-readout")).toHaveText("Cherry");
  await app.getByTestId("input-combobox").selectOption(4);
  await expect(app.getByTestId("input-combobox-readout")).toHaveText("Elderberry");
  await app.getByTestId("input-combobox").fill("Kiwi");
  await expect(app.getByTestId("input-combobox-readout")).toHaveText("Kiwi");
  let rangeRejected = false;
  await app.getByTestId("input-combobox").selectOption(99).catch(() => { rangeRejected = true; });
  if (!rangeRejected) fail("combobox setValue(99) should have been rejected: only 5 options");
  // `enabled` off must show up in the a11y tree on every control kind.
  await app.getByTestId("input-enabled-toggle").uncheck();
  await app.waitForDisabled("input-enabled-button", { timeoutMs: 3000 });
  await expectWidgets(app, {
    "input-enabled-button": { enabled: false },
    "input-enabled-textinput": { enabled: false },
    "input-enabled-select": { enabled: false },
    "input-enabled-slider": { enabled: false },
  });
  await app.getByTestId("input-enabled-toggle").check();
  await app.waitForEnabled("input-enabled-button", { timeoutMs: 3000 });
  console.log("ND_PARITY_INPUT_OK combobox drove selection + free text; enabled=false reached the a11y tree");
  await shoot(2, "input");

  // ---- navigation: Breadcrumb ---------------------------------------------
  await show("navigation");
  await expectWidgets(app, { "nav-breadcrumb": { type: "Breadcrumb", role: "toolbar" } });
  await expectNothingCollapsed(app, "navigation");
  {
    const g = await boxOf(app, "nav-breadcrumb");
    await expect(app.getByTestId("nav-breadcrumb-readout")).toHaveText("Parity");
    await app.mouse.click(g.x + 18, g.y + g.height / 2); // first crumb
    await expect(app.getByTestId("nav-breadcrumb-readout")).toHaveText("Home");
  }
  console.log("ND_PARITY_NAV_OK breadcrumb itemActivated moved app state");
  await shoot(3, "navigation");

  // ---- composition: @nativedesktop/ui layer over the same widgets ----------
  await show("composition");
  await expectNothingCollapsed(app, "composition");
  await shoot(4, "composition");

  // ---- data: Table multi-select -------------------------------------------
  await show("data");
  await expectWidgets(app, { "data-table": { type: "Table", role: "table" } });
  await expectNothingCollapsed(app, "data");
  {
    const g = await boxOf(app, "data-table");
    await expect(app.getByTestId("data-selected-readout")).toHaveText("Selected: (none)");
    await app.mouse.click(g.x + 60, g.y + 40); // header is ~24pt; this is row 0
    await expect(app.getByTestId("data-selected-readout")).toHaveText("Selected: Ada Lovelace");
    await app.keyboard.press("Shift+ArrowDown");
    await expect(app.getByTestId("data-selected-readout")).toHaveText("Selected: Ada Lovelace, Grace Hopper");
    await app.keyboard.press("Shift+ArrowDown");
    await expect(app.getByTestId("data-selected-readout")).toHaveText("Selected: Ada Lovelace, Grace Hopper, Alan Turing");
  }
  console.log("ND_PARITY_TABLE_OK selectionMode=multiple reported the index set {0,1,2}");
  await shoot(5, "data");

  // ---- overlays: Dialog + Sheet -------------------------------------------
  await show("overlays");
  // Both handles are host-only (they present their own window), so they carry
  // a role and no rectangle; the CONTENT is what has to appear and vanish.
  await expectWidgets(app, {
    "overlays-dialog": { type: "Dialog", role: "dialog", handle: true },
    "overlays-sheet": { type: "Sheet", role: "dialog", handle: true },
  });
  await expectNothingCollapsed(app, "overlays");
  await app.getByTestId("overlays-dialog-open-button").click();
  await app.waitFor({ testId: "overlays-dialog-name-input", state: "visible" }, { timeoutMs: 4000 });
  await expectWidgets(app, {
    "overlays-dialog-name-input": { type: "TextInput", role: "textbox" },
    "overlays-dialog-save": { type: "Button", role: "button", text: "Save" },
  });
  await shoot(6, "overlays");
  await app.getByTestId("overlays-dialog-name-input").fill("Ada");
  await app.getByTestId("overlays-dialog-save").click();
  await app.waitFor({ testId: "overlays-dialog-name-input", state: "gone" }, { timeoutMs: 4000 }).catch(async () => {
    if (await app.getByTestId("overlays-dialog-name-input").isVisible()) {
      fail("dialog content is still visible after Save closed the dialog");
    }
  });
  await expect(app.getByTestId("overlays-dialog-readout")).toHaveText("Saved Ada <(empty)>");
  // Sheet: one instance, its edge switched per button. The dismissal is
  // asserted through app state, not `visible`: a closed sheet leaves the
  // screen (checked against a capture) but its content nodes keep reporting
  // visible=true, since node_visible does not account for the overlay window
  // the sheet content lives in. The Dialog above does flip correctly.
  for (const edge of ["bottom", "trailing"]) {
    await app.getByTestId(`overlays-sheet-open-${edge}`).click();
    await expect(app.getByTestId("overlays-sheet-readout")).toHaveText(`Open from ${edge}`);
    await app.waitFor({ testId: "overlays-sheet-title", state: "visible" }, { timeoutMs: 4000 });
    await expect(app.getByTestId("overlays-sheet-title")).toHaveText(`Sheet from ${edge}`);
    await app.getByTestId("overlays-sheet-close").click();
    await expect(app.getByTestId("overlays-sheet-readout")).toHaveText("Closed");
  }

  // Overlay: the floating bar must not move or resize the content under it,
  // which is the widget's entire contract. minHeight rides along: the content
  // box asks for 120 and must actually get it.
  const stackBefore = await app.getByTestId("overlays-stack-content").boundingBox();
  if (!stackBefore || stackBefore.height < 120) {
    fail(`overlay content ignored minHeight 120: ${JSON.stringify(stackBefore)}`);
  }
  await app.getByTestId("overlays-stack-toggle").click();
  await app.waitFor({ testId: "overlays-stack-bar", state: "visible" }, { timeoutMs: 4000 });
  const stackAfter = await app.getByTestId("overlays-stack-content").boundingBox();
  const barGeom = await app.getByTestId("overlays-stack-bar").boundingBox();
  if (!stackAfter || stackAfter.y !== stackBefore.y || stackAfter.height !== stackBefore.height) {
    fail(`the floating bar resized the content: ${JSON.stringify(stackBefore)} -> ${JSON.stringify(stackAfter)}`);
  }
  if (!barGeom || barGeom.height <= 0 || barGeom.y > stackAfter.y + stackAfter.height / 2) {
    fail(`the bar is not floating over the content's top half: ${JSON.stringify(barGeom)} over ${JSON.stringify(stackAfter)}`);
  }
  await app.getByTestId("overlays-stack-toggle").click();
  await app.waitFor({ testId: "overlays-stack-bar", state: "gone" }, { timeoutMs: 4000 });
  console.log("ND_PARITY_OVERLAYS_OK dialog opened, saved and dismissed; sheet slid in from two edges; the overlay bar floated without resizing the content");

  // ---- richtext -----------------------------------------------------------
  await show("richtext");
  await expectWidgets(app, {
    "richtext-view": { type: "RichText", role: "label", valueContains: "A read-only, natively parsed Markdown view" },
  });
  await expectNothingCollapsed(app, "richtext");
  await app.getByTestId("richtext-selectable-toggle").uncheck();
  await sleep(200);
  await expectWidgets(app, { "richtext-view": { type: "RichText", role: "label", valueContains: "Code block" } });
  await app.getByTestId("richtext-selectable-toggle").check();
  console.log("ND_PARITY_RICHTEXT_OK markdown rendered with a real height and survived a selectable flip");
  await shoot(7, "richtext");

  // ---- charts: six types, TabView selectionChanged -------------------------
  await show("charts");
  await expectWidgets(app, {
    "charts-tabs": { type: "TabView", role: "tablist" },
    "charts-line": { type: "Chart", role: "custom" },
    "charts-area": { type: "Chart", role: "custom" },
    "charts-bar": { type: "Chart", role: "custom" },
    "charts-pie": { type: "Chart", role: "custom" },
    "charts-scatter": { type: "Chart", role: "custom" },
    "charts-candlestick": { type: "Chart", role: "custom" },
  });
  await expectNothingCollapsed(app, "charts");
  await shoot(8, "charts");
  {
    // The tab strip is a segmented control centred on the TabView's top edge.
    const g = await boxOf(app, "charts-tabs");
    if (await app.getByTestId("charts-live-chart").isVisible()) {
      fail("charts: the Live tab is showing before its tab was clicked");
    }
    await app.mouse.click(g.x + g.width * 0.54, g.y + 6);
    await app.waitFor({ testId: "charts-live-chart", state: "visible" }, { timeoutMs: 4000 });
    if (await app.getByTestId("charts-types-grid").isVisible()) {
      fail("charts: the Types page is still showing after switching to Live");
    }
    // The live chart must survive every type switch, still on screen and sized.
    for (let i = 0; i < 6; i++) {
      await app.getByTestId("charts-live-type-select").selectOption(i);
      await sleep(250);
      await expectWidgets(app, { "charts-live-chart": { type: "Chart", role: "custom" } });
    }
    for (const toggle of ["charts-live-legend-toggle", "charts-live-grid-toggle", "charts-live-animated-toggle"]) {
      await app.getByTestId(toggle).uncheck();
      await sleep(150);
      await expectWidgets(app, { "charts-live-chart": { type: "Chart", role: "custom" } });
      await app.getByTestId(toggle).check();
    }
    await shoot(9, "charts-live");
    // Back to the Types page, proving the strip drives both ways.
    await app.mouse.click(g.x + g.width * 0.46, g.y + 6);
    await app.waitFor({ testId: "charts-types-grid", state: "visible" }, { timeoutMs: 4000 });
  }
  console.log("ND_PARITY_CHARTS_OK six chart types rendered; tab strip switched pages both ways; live chart survived all six type switches");

  // ---- progress: ProgressCircle -------------------------------------------
  await show("progress");
  await expectWidgets(app, {
    "progress-live-circle": { type: "ProgressCircle", role: "progressbar" },
    "progress-circle-labeled-50": { type: "ProgressCircle", role: "progressbar" },
    "progress-circle-unlabeled-100": { type: "ProgressCircle", role: "progressbar" },
  });
  await expectNothingCollapsed(app, "progress");
  await app.getByTestId("progress-live-slider").fill(0.75);
  await expect(app.getByTestId("progress-live-readout")).toHaveText("75%");
  await expectWidgets(app, { "progress-live-circle": { type: "ProgressCircle", role: "progressbar" } });
  console.log("ND_PARITY_PROGRESS_OK progresscircle tracked the slider");
  await shoot(10, "progress");

  // ---- loading: Skeleton --------------------------------------------------
  await show("loading");
  await expectWidgets(app, {
    "loading-row-r1-avatar-skeleton": { type: "Skeleton", role: "custom" },
    "loading-row-r1-name-skeleton": { type: "Skeleton", role: "custom" },
  });
  await expectNothingCollapsed(app, "loading");
  await shoot(11, "loading");
  await app.getByTestId("loading-toggle").check();
  await app.waitForPresent("loading-row-r1-avatar", { timeoutMs: 3000 });
  await app.waitForGone("loading-row-r1-avatar-skeleton", { timeoutMs: 3000 });
  await expectNothingCollapsed(app, "loading (loaded)");
  await app.getByTestId("loading-toggle").uncheck();
  await app.waitForPresent("loading-row-r1-avatar-skeleton", { timeoutMs: 3000 });
  console.log("ND_PARITY_LOADING_OK skeletons swapped for real content and back");

  // ---- dock / tiles / dragdrop: panes models over the same widgets ---------
  await show("dock");
  await expectNothingCollapsed(app, "dock");
  await shoot(12, "dock");
  await show("tiles");
  await expectNothingCollapsed(app, "tiles");
  await shoot(13, "tiles");
  await show("dragdrop");
  await expectNothingCollapsed(app, "dragdrop");
  await shoot(14, "dragdrop");

  // ---- codeeditor ---------------------------------------------------------
  await show("codeeditor");
  await expectWidgets(app, {
    "codeeditor-widget": { type: "CodeEditor", role: "textbox", valueContains: "function greet" },
  });
  await expectNothingCollapsed(app, "codeeditor");
  await app.getByTestId("codeeditor-linenumbers-toggle").uncheck();
  await sleep(200);
  await expectWidgets(app, { "codeeditor-widget": { type: "CodeEditor", role: "textbox", valueContains: "function greet" } });
  await app.getByTestId("codeeditor-linenumbers-toggle").check();
  await app.getByTestId("codeeditor-language-select").selectOption(1);
  await sleep(200);
  await expectWidgets(app, { "codeeditor-widget": { type: "CodeEditor", role: "textbox", valueContains: "function greet" } });
  console.log("ND_PARITY_CODEEDITOR_OK editor rendered with a real height and survived language/gutter switches");
  await shoot(15, "codeeditor");

  // Every section must still be reachable after all of that.
  for (const section of sections) {
    await show(section);
    await expectNothingCollapsed(app, `${section} (second pass)`);
  }
  console.log(`ND_PARITY_SECTIONS_OK all ${sections.length} sections re-rendered on a second pass`);

  console.log(`ND_PARITY_SHOTS ${shots.join(" | ")}`);
  console.log(`ND_PARITY_OK backend=${app.backend} sections=${sections.length} shots=${shots.length} dir=${shotDir}`);
} finally {
  await app.close();
}
