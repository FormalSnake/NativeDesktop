#!/usr/bin/env bun
// scripts/headerfield-drive.ts [gtk|appkit] -- drives examples/headerfield
// via @nativedesktop/test. Acceptance for the two header-bar gaps:
//
//   leg 1  the address field takes the whole run between the start and end
//          packs, at three window widths. Asserted on allocations, not by
//          eye: the field's left edge within TOL of the rightmost start
//          child's right edge, its right edge within TOL of the leftmost end
//          child's left edge.
//   leg 2  the leading icon inside the field is interactive: a click on it
//          (the real cursor on AppKit) delivers onLeadingIconClicked to the
//          app, and the popover that named the icon slot opens against the
//          icon rather than the field.
//   leg 3  the automation setValue path still emits `changed`, which is what
//          the search-entry-to-entry swap in the GTK backend could have
//          broken.
//
// Runs either way round: scripts/headless-headerfield.sh owns the host and
// hands the socket over in ND_AUTOMATION_SOCKET, and a bare
// `bun scripts/headerfield-drive.ts` launches one itself.
import { connectApp, findNode, launchApp, poll } from "../packages/test/src/index.ts";
import { pngSize } from "../packages/test/src/png.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const gtk = process.env.ND_BACKEND === "gtk";
const attached = process.env.ND_AUTOMATION_SOCKET != null;
const T = 6000;
// The packs sit one header spacing away from the field: 6px on GTK, and on
// AppKit the toolbar's own item gap, 8. Both are far below the ~460px the
// centred title used to waste.
const TOL = 8;
// The two weight buttons ride a wrapping <box> so the font leg has a
// container to style. GTK measures that box; AppKit promotes the plain button
// inside it to a system-drawn toolbar item and measures the item, leaving the
// box itself at zero, so each backend names the node it can answer for.
const START = gtk
  ? ["nav-back", "nav-forward", "nav-reload", "weight-box", "weight-plain-box"]
  : ["nav-back", "nav-forward", "nav-reload", "weight-styled", "weight-plain"];
const END = ["end-menu", "end-add"];

const app = attached ? await connectApp() : await launchApp({ entry: "examples/headerfield/main.tsx", backend });
const hostPid = Number(process.env.ND_HOST_PID ?? ("pid" in app ? app.pid : 0));

type CensusWindow = { layer: number; alpha: number; x: number; y: number; width: number; height: number };
// The window server's view of the host's on-screen windows (scripts/mac/window-census.swift).
const census = async (pid: number): Promise<CensusWindow[]> => {
  const proc = Bun.spawn(["swift", "scripts/mac/window-census.swift", String(pid)], {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined },
  });
  const text = await new Response(proc.stdout).text();
  return text.split("\n").filter((l) => l.trim().startsWith("{")).map((l) => JSON.parse(l) as CensusWindow);
};
const activate = (pid: number) => {
  Bun.spawnSync(["osascript", "-e", `tell application "System Events" to set frontmost of (first process whose unix id is ${pid}) to true`]);
};

type Rect = { x: number; y: number; w: number; h: number };
const mustFind = async (testId: string) => {
  const node = findNode((await app.tree()).root, testId);
  if (!node) throw new Error(`${testId} not found in tree`);
  return node;
};
const rectOf = async (testId: string): Promise<Rect> => {
  const g = (await mustFind(testId)).geometry;
  if (!g || g.w <= 0) throw new Error(`${testId} never laid out (${JSON.stringify(g)})`);
  return g;
};
const label = async (testId: string) => (await mustFind(testId)).text ?? "";

try {
  // ---- leg 1: the field fills the free run at three widths ----------------
  for (const width of [1400, 1000, 760]) {
    await app.setWindowSize(width, 420);
    // The relayout lands a frame after the resize; wait for the field to stop
    // moving rather than for a fixed delay.
    let last = -1;
    await poll(async () => (await rectOf("address")).w, (w) => {
      const settled = w === last;
      last = w;
      return settled;
    }, { timeoutMs: T });

    const field = await rectOf("address");
    let startRight = -Infinity;
    for (const id of START) {
      const r = await rectOf(id);
      startRight = Math.max(startRight, r.x + r.w);
    }
    let endLeft = Infinity;
    for (const id of END) endLeft = Math.min(endLeft, (await rectOf(id)).x);

    const gapL = field.x - startRight;
    const gapR = endLeft - (field.x + field.w);
    const line = `width=${width} field=[${field.x}..${field.x + field.w}] w=${field.w} gapL=${gapL} gapR=${gapR}`;
    if (gapL < -TOL || gapL > TOL) throw new Error(`the field does not reach the start pack: ${line}`);
    if (gapR < -TOL || gapR > TOL) throw new Error(`the field does not reach the end pack: ${line}`);
    console.log(`  ND_HEADERFIELD_FILL_OK ${line}`);
  }

  // ---- leg 2: the leading icon is interactive, and anchors the popover ----
  if ((await label("icon-count")) !== "icon clicks: 0") throw new Error("the icon fired before anything touched it");
  const field = await rectOf("address");
  // Stage Manager draws a background app's windows as thumbnails and reports
  // their frames that way, and a transient popover closes when its app loses
  // focus, so the host comes forward before anything opens.
  // On AppKit the real cursor clicks the padlock, so the window server's hit
  // test has to land on the icon overlay rather than on the text field under
  // it. GTK4 refuses synthesized pointer input, so there the app's own button
  // fires the icon through activateLeadingIcon.
  if (gtk) {
    await app.getByTestId("fire-icon").click();
  } else {
    activate(hostPid);
    await app.cursor.click({ x: field.x + 14, y: field.y + field.h / 2 });
  }
  await poll(() => label("icon-count"), (v) => v === "icon clicks: 1", { timeoutMs: T });
  console.log("  ND_HEADERFIELD_ICON_OK the leading icon delivered onLeadingIconClicked");

  // The panel opens against the icon: its origin sits in the field's LEADING
  // quarter, not at the field's centre where a whole-widget anchor puts it.
  // GTK puts a popover in its own GdkSurface, which never maps under a
  // headless compositor with no seat, so the readable signal there is the
  // geometry getTree reports for the panel: none at all when it is unplaced
  // (the popover-anchor gate's finding), an origin on the anchor when placed.
  if (gtk) {
    const panel = await poll(async () => findNode((await app.tree()).root, "site-info-body")?.geometry ?? null,
      (g) => g != null, { timeoutMs: T });
    if (panel!.x > field.x + field.w / 4) {
      throw new Error(`the site-info panel opened at ${panel!.x}, past the field's leading quarter (field ${field.x}..${field.x + field.w})`);
    }
    console.log(`  ND_HEADERFIELD_ANCHOR_OK the panel opened at ${panel!.x}, on the icon at the field's leading edge (field ${field.x}..${field.x + field.w})`);
  } else {
    // AppKit presents the panel in a popover window of its own and getTree
    // reports its content in that window's space, so the readable signal is
    // where the window server put that window. Its midpoint is where the arrow
    // points, and it has to land in the field's leading quarter.
    const main = (await app.windows()).windows[0]?.geometry;
    if (!main) throw new Error("the app reports no window geometry");
    // Each census compiles its Swift script, so the budget is several reads.
    let seen: CensusWindow[] = [];
    const popover = await poll(async () => {
      seen = await census(hostPid);
      return seen.find((w) => w.alpha > 0 && w.layer === 0 && w.width < main.w) ?? null;
    }, (w) => w != null, { timeoutMs: 20000 }).catch(() => {
      throw new Error(`no popover window on screen (host windows: ${JSON.stringify(seen)})`);
    });
    const mid = popover!.x + popover!.width / 2 - main.x;
    if (mid < field.x || mid > field.x + field.w / 4) {
      throw new Error(`the site-info popover points at ${mid}, outside the field's leading quarter (field ${field.x}..${field.x + field.w})`);
    }
    console.log(`  ND_HEADERFIELD_ANCHOR_OK the popover points at ${mid}, on the icon at the field's leading edge (field ${field.x}..${field.x + field.w})`);

    // The panel holds its content: the popover window is at least the label
    // plus the panel's 12pt insets on each side. An empty panel collapsed to
    // the insets alone.
    const text = await rectOf("site-info-label");
    const body = text.h + 24;
    if (popover!.width < text.w + 24 || popover!.height < body) {
      throw new Error(`the site-info popover is ${popover!.width}x${popover!.height}, too small for its ${text.w}x${text.h} label`);
    }
    console.log(`  ND_HEADERFIELD_POPOVER_SIZE_OK the popover is ${popover!.width}x${popover!.height} around a ${text.w}x${text.h} label`);

    // And it paints: inside the panel body, away from the arrow and the glass
    // edge, the label's glyphs make some rows differ from the fill.
    if (process.env.ND_NDSHOT && process.env.ND_REGION_SHOT_PATH) {
      const path = process.env.ND_REGION_SHOT_PATH;
      const shot = Bun.spawnSync([process.env.ND_NDSHOT, "capture", "--pid", String(hostPid), "--region", "--out", path]);
      if (shot.exitCode !== 0) throw new Error(`ndshot capture failed: ${shot.stderr.toString().trim()}`);
      const scale = (await pngSize(path)).width / main.w;
      const x = (popover!.x - main.x + 8) * scale;
      const y = (popover!.y + popover!.height - body + 4 - main.y) * scale;
      const w = (popover!.width - 16) * scale;
      const h = (body - 8) * scale;
      const probe = Bun.spawnSync(["swift", "scripts/mac/png-probe.swift", path, ...[x, y, w, h].map((v) => String(Math.round(v)))], {
        env: { ...process.env, SDKROOT: undefined, DEVELOPER_DIR: undefined },
      });
      if (probe.exitCode !== 0) throw new Error(`png-probe failed: ${probe.stderr.toString().trim()}`);
      const bands = (JSON.parse(probe.stdout.toString()) as { bands: number[][] }).bands.map(([r, g, b]) => (r! + g! + b!) / 3);
      const spread = Math.max(...bands) - Math.min(...bands);
      if (spread < 12) throw new Error(`the site-info popover body is flat (band luminance spread ${spread.toFixed(1)}), nothing painted in it`);
      console.log(`  ND_HEADERFIELD_POPOVER_INK_OK the popover body paints its label (band luminance spread ${spread.toFixed(1)})`);
      console.log(`  capture ${path}`);
    }
  }

  // ---- leg 3: setValue still emits `changed` -----------------------------
  await app.getByTestId("address").fill("nativedesktop.dev");
  await poll(() => label("url-label"), (v) => v === "nativedesktop.dev", { timeoutMs: T });
  console.log("  ND_HEADERFIELD_SETVALUE_OK setValue still round-trips through `changed`");

  // ---- leg 4: a `font` style reaches the button under the styled box -----
  // Adwaita declares `font-weight: bold` on the button NODE, and an explicit
  // declaration beats a value inherited from an ancestor, so a `font` style on
  // the wrapping <box> used to leave the button bold. AppKit's toolbar draws
  // its item titles heavier than the system font too. Same string in both
  // buttons, so the widths differ by the weight drawn.
  const styled = await rectOf("weight-styled");
  const plain = await rectOf("weight-plain");
  if (styled.w >= plain.w) {
    throw new Error(`fontWeight "normal" on the wrapping box did not reach the button: styled=${styled.w}px, default=${plain.w}px`);
  }
  console.log(`  ND_HEADERFIELD_FONT_OK a header button under a box at fontWeight normal measures ${styled.w}px against ${plain.w}px at the header's default`);

  if (process.env.ND_SHOT_PATH) {
    await app.screenshot(process.env.ND_SHOT_PATH);
    console.log(`  capture ${process.env.ND_SHOT_PATH}`);
  }
  console.log("ND_HEADERFIELD_OK header title fill, leading icon and setValue all hold");
} finally {
  // Closing an attached client only drops this end of the socket; the gate
  // script owns the host either way.
  await app.close();
}
