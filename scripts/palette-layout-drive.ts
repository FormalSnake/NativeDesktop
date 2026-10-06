#!/usr/bin/env bun
// scripts/palette-layout-drive.ts: the command bar's geometry and keyboard
// contract, on whichever backend runs it. Launches scripts/palette-layout-app
// at a normal and a narrow width, captures each state, and asserts through the
// `paletteLayout` RPC: the panel centred in the window, every row's parts
// inside the row, vertically centred and not overlapping, long text truncated,
// and inline completion across typing, Backspace, Tab and Enter.
//
//   ND_HOST_BINARY=<host> [DISPLAY=:N] bun scripts/palette-layout-drive.ts
//
// GTK synthesises no keys over automation, so on Linux the keyboard legs go
// through xdotool and the rig must be an X server (GDK_BACKEND=x11). AppKit
// uses the `keys` RPC. Screenshots land in $PALETTE_OUT
// (default /tmp/nd-palette-layout). Marker: ND_PALETTE_LAYOUT_OK.
import { mkdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { PaletteLayout } from "@nativedesktop/core/rpc";
import { SOLID_PRELOAD, withPreload } from "@nativedesktop/host";
import { type AppHandle, launchApp } from "../packages/test/src/index.ts";
import { decodePng, inkBandHeight, modalFill } from "./capture-ink.ts";

const ROOT = resolve(import.meta.dir, "..");
const OUT = process.env.PALETTE_OUT ?? "/tmp/nd-palette-layout";
const MAC = process.platform === "darwin";
const WIDTHS = [1280, 700];
mkdirSync(OUT, { recursive: true });

const failures: string[] = [];
function check(ok: boolean, what: string): void {
  if (!ok) failures.push(what);
}

type Geo = { x: number; y: number; w: number; h: number };
const right = (g: Geo) => g.x + g.w;
const midY = (g: Geo) => g.y + g.h / 2;
const inside = (a: Geo, b: Geo) => a.x >= b.x && right(a) <= right(b) && a.y >= b.y && a.y + a.h <= b.y + b.h;

function checkLayout(l: PaletteLayout, where: string): void {
  check(l.presented, `${where}: not presented`);
  const win = l.window as Geo | null;
  const panel = l.panel as Geo | null;
  if (!win || !panel || !l.field) return check(false, `${where}: no window/panel/field geometry`);
  const off = panel.x + panel.w / 2 - win.w / 2;
  check(Math.abs(off) <= 1, `${where}: panel off centre by ${off}px (panel ${JSON.stringify(panel)}, window ${win.w})`);
  check(panel.x >= 20 && right(panel) <= win.w - 20, `${where}: panel ${panel.x}..${right(panel)} within 20px of a ${win.w}px window edge`);
  check(inside(l.field as Geo, panel), `${where}: field outside the panel`);
  const top = Math.max(20, Math.round(win.h * 0.18));
  check(Math.abs(panel.y - top) <= 1, `${where}: panel top at ${panel.y}, want ${top} (18% of ${win.h})`);
  const lastRow = l.rows.at(-1)?.row as Geo | undefined;
  if (lastRow && l.rows.length < 10) {
    const gap = panel.y + panel.h - (lastRow.y + lastRow.h);
    check(gap >= 0 && gap <= 16, `${where}: ${gap}px of panel below the last row, the panel does not follow its rows`);
  }
  check(l.dimmed, `${where}: page not dimmed`);
  for (const [i, r] of l.rows.entries()) {
    const row = r.row as Geo;
    const tag = `${where} row ${i}`;
    check(inside(row, panel), `${tag}: row outside the panel`);
    check(Math.abs(row.h - 40) <= 1, `${tag}: row ${row.h}px tall, want 40`);
    const parts = [r.icon, r.title, r.subtitle, r.hint].filter(Boolean) as Geo[];
    for (const p of parts) {
      check(inside(p, row), `${tag}: part ${JSON.stringify(p)} outside row ${JSON.stringify(row)}`);
      check(p.w <= row.w, `${tag}: part wider than its row`);
    }
    if (r.icon) {
      const icon = r.icon as Geo;
      check(icon.w === 16 && icon.h === 16, `${tag}: icon ${icon.w}x${icon.h}, want 16x16`);
      check(Math.abs(midY(icon) - midY(row)) <= 1, `${tag}: icon off vertical centre by ${midY(icon) - midY(row)}`);
      if (r.title) check(right(icon) <= (r.title as Geo).x, `${tag}: icon overlaps title`);
    }
    if (r.title) {
      const t = r.title as Geo;
      check(Math.abs(midY(t) - midY(row)) <= 1, `${tag}: title off vertical centre by ${midY(t) - midY(row)}`);
      if (r.subtitle) {
        const s = r.subtitle as Geo;
        check(right(t) <= s.x, `${tag}: title overlaps subtitle`);
        check(Math.abs(s.y - t.y) <= 1 && Math.abs(s.h - t.h) <= 1, `${tag}: subtitle not on the title's line`);
      }
      if (r.hint) {
        const h = r.hint as Geo;
        const before = (r.subtitle ?? r.title) as Geo;
        check(right(before) <= h.x, `${tag}: hint overlaps the text before it`);
        check(Math.abs(h.y - t.y) <= 1 && Math.abs(h.h - t.h) <= 1, `${tag}: hint not on the title's line`);
        check(right(row) - right(h) <= 24, `${tag}: hint not at the trailing edge (${right(row) - right(h)}px short)`);
      }
    }
  }
}

/// The fixture's third row opens on a colour emoji. A colour emoji font is
/// bitmap strikes, and a strike drawn at its own size (Noto's is 109ppem) makes
/// the row as tall as the glyph. The title must keep a plain title's line
/// height, and the glyph, measured off the capture across the whole row band
/// so ink spilling past the label still counts, must fit inside it.
function checkEmojiRow(l: PaletteLayout, shotPath: string, where: string): void {
  const plain = l.rows[1]?.title as Geo | undefined;
  const row = l.rows[2]?.row as Geo | undefined;
  const title = l.rows[2]?.title as Geo | undefined;
  if (!plain || !row || !title) return check(false, `${where}: no emoji row geometry`);
  check(Math.abs(title.h - plain.h) <= 2, `${where}: the emoji title is ${title.h}px tall, a plain title ${plain.h}px`);
  const img = decodePng(new Uint8Array(readFileSync(shotPath)));
  const scale = img.w / (l.window as Geo).w;
  const fill = modalFill(img, Math.round(row.x * scale), Math.round(right(row) * scale), Math.round(row.y * scale), Math.round((row.y + row.h) * scale));
  const glyph = { x: title.x, y: row.y, w: Math.min(title.w, title.h), h: row.h };
  const ink = inkBandHeight(img, glyph, scale, fill, 20);
  check(ink > 0 && ink <= plain.h, `${where}: the emoji paints ${ink.toFixed(1)}px tall, over a ${plain.h}px text line`);
  console.log(`ND_PALETTE_EMOJI ${where}: emoji ink ${ink.toFixed(1)}px, title ${title.h}px, plain title ${plain.h}px, row ${row.h}px`);
}

function xdo(...args: string[]): void {
  const r = Bun.spawnSync(["xdotool", ...args]);
  if (r.exitCode !== 0) throw new Error(`xdotool ${args.join(" ")}: ${r.stderr.toString().trim()}`);
}

async function focusWindow(): Promise<void> {
  if (MAC) return;
  xdo("search", "--sync", "--name", "Palette layout", "windowfocus", "--sync");
  await Bun.sleep(150);
}

async function press(app: AppHandle, key: "BackSpace" | "Tab" | "Return" | "Escape"): Promise<void> {
  if (MAC) {
    const chord = { BackSpace: "backspace", Tab: "tab", Return: "return", Escape: "escape" }[key];
    await app.keys(chord);
  } else {
    await focusWindow();
    xdo("key", "--clearmodifiers", key);
  }
  await Bun.sleep(250);
}

async function typeText(app: AppHandle, text: string): Promise<void> {
  if (MAC) {
    for (const ch of text) await app.getByTestId("palette").type(ch);
  } else {
    await focusWindow();
    xdo("type", "--delay", "60", text);
  }
  await Bun.sleep(300);
}

/// JSON with its keys sorted: AppKit builds the layout from dictionaries,
/// whose key order changes from one read to the next.
function canonical(v: unknown): string {
  return JSON.stringify(v, (_, x) =>
    x && typeof x === "object" && !Array.isArray(x)
      ? Object.fromEntries(Object.keys(x).sort().map((k) => [k, (x as Record<string, unknown>)[k]]))
      : x,
  );
}

async function layoutWhen(app: AppHandle, ok: (l: PaletteLayout) => boolean, what: string): Promise<PaletteLayout> {
  // Settled, not just matching: the card follows its rows, so a read taken
  // while a new row set is being laid out catches it mid-resize.
  let last: PaletteLayout | undefined;
  let previous = "";
  for (let i = 0; i < 30; i++) {
    last = await app.paletteLayout({ testId: "palette" });
    const now = canonical(last);
    if (ok(last) && now === previous) return last;
    previous = now;
    await Bun.sleep(120);
  }
  failures.push(`${what}: field ${JSON.stringify(last?.fieldText)} sel ${last?.selectionStart}+${last?.selectionLength}`);
  return last!;
}

async function waitText(app: AppHandle, text: string): Promise<void> {
  const w = await app.waitForText(text, { timeoutMs: 4000 });
  check(w.matched, `never saw ${JSON.stringify(text)}`);
}

for (const width of WIDTHS) {
  const app = await launchApp({
    entry: "scripts/palette-layout-app.tsx",
    cwd: ROOT,
    hostBinary: process.env.ND_HOST_BINARY,
    // The fixture sits outside any Solid package, so the register preload is passed by hand.
    env: { PALETTE_WIDTH: String(width), ND_AUTOMATION_CAPTURE: MAC ? "region" : undefined, BUN_OPTIONS: withPreload(process.env.BUN_OPTIONS, SOLID_PRELOAD) },
    logPath: `${OUT}/host-${width}.log`,
  });
  const shot = async (name: string) => {
    await Bun.sleep(250);
    await app.screenshot(`${OUT}/${width}-${name}.png`);
  };
  try {
    const where = `${width}px`;
    const open = await layoutWhen(app, (l) => l.presented && l.rows.length > 0, `${where}: palette never presented`);
    checkLayout(open, `${where} empty`);
    check(open.rows[0]?.truncated === true, `${where}: the long title/URL row is not truncated`);
    check(open.rows[0]?.highlighted === true, `${where}: the top row is not highlighted`);
    await shot("empty");
    checkEmojiRow(open, `${OUT}/${width}-empty.png`, `${where} empty`);

    await typeText(app, "git");
    await waitText(app, "Query: git");
    const done = await layoutWhen(
      app,
      (l) => l.fieldText === "github.com" && l.selectionStart === 3 && l.selectionLength === 7,
      `${where}: typing "git" did not complete to github.com with "hub.com" selected`,
    );
    checkLayout(done, `${where} completed`);
    await shot("completed");

    await press(app, "BackSpace");
    await layoutWhen(app, (l) => l.fieldText === "git" && l.selectionLength === 0, `${where}: Backspace did not remove the completion`);
    await Bun.sleep(400);
    await layoutWhen(app, (l) => l.fieldText === "git", `${where}: the completion came back after Backspace`);
    await press(app, "Return");
    await waitText(app, "Last: submit git");

    // The controlled query is still "", so this present must not show "git".
    await app.click("reopen-empty");
    await layoutWhen(app, (l) => l.presented && l.fieldText === "", `${where}: a reopen kept the earlier typed text`);
    await typeText(app, "gi");
    await layoutWhen(app, (l) => l.fieldText === "github.com" && l.selectionStart === 2, `${where}: "gi" did not complete`);
    await press(app, "Tab");
    await layoutWhen(
      app,
      (l) => l.fieldText === "github.com" && l.selectionLength === 0 && l.selectionStart === 10,
      `${where}: Tab did not accept the completion`,
    );
    await waitText(app, "Query: github.com");
    await press(app, "Return");
    await waitText(app, "Last: activate open:github.com");

    await app.click("reopen");
    const seeded = await layoutWhen(
      app,
      (l) => l.presented && l.fieldText === "https://example.com/current",
      `${where}: a seeded open did not show the seed`,
    );
    check(seeded.selectionStart === 0 && seeded.selectionLength === seeded.fieldText.length, `${where}: the seed is not all selected`);
    await shot("seeded");
    await press(app, "Escape");
    await waitText(app, "Last: cancel");
  } catch (e) {
    failures.push(`${width}px: ${(e as Error).message}`);
  } finally {
    await app.close();
  }
}

if (failures.length) {
  console.error(`ND_PALETTE_LAYOUT_FAIL\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(`ND_PALETTE_LAYOUT_OK screenshots in ${OUT}`);
