#!/usr/bin/env bun
// scripts/headerfield-drive.ts [gtk|appkit] -- drives examples/headerfield
// via @nativedesktop/test. Acceptance for the two header-bar gaps:
//
//   leg 1  the address field takes the whole run between the start and end
//          packs, at three window widths. Asserted on allocations, not by
//          eye: the field's left edge within TOL of the rightmost start
//          child's right edge, its right edge within TOL of the leftmost end
//          child's left edge.
//   leg 2  the leading icon inside the field is interactive: firing it
//          delivers onLeadingIconClicked to the app, and the popover that
//          named the icon slot opens against the icon rather than the field.
//   leg 3  the automation setValue path still emits `changed`, which is what
//          the search-entry-to-entry swap in the GTK backend could have
//          broken.
//
// Runs either way round: scripts/headless-headerfield.sh owns the host and
// hands the socket over in ND_AUTOMATION_SOCKET, and a bare
// `bun scripts/headerfield-drive.ts` launches one itself.
import { connectApp, findNode, launchApp, poll } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const gtk = process.env.ND_BACKEND === "gtk";
const attached = process.env.ND_AUTOMATION_SOCKET != null;
const T = 6000;
// The packs sit one header spacing away from the field on GTK (6px measured)
// and one toolbar gap away on AppKit; 8 is the budget the gate allows for
// that, and it is far below the ~460px the centred title used to waste.
const TOL = 8;
const START = ["nav-back", "nav-forward", "nav-reload", "weight-styled", "weight-plain"];
const END = ["end-menu", "end-add"];

const app = attached ? await connectApp() : await launchApp({ entry: "examples/headerfield/main.tsx", backend });

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
  await app.getByTestId("fire-icon").click();
  await poll(() => label("icon-count"), (v) => v === "icon clicks: 1", { timeoutMs: T });
  console.log("  ND_HEADERFIELD_ICON_OK the leading icon delivered onLeadingIconClicked");

  // The panel opens against the icon: its origin sits in the field's LEADING
  // quarter, not at the field's centre where a whole-widget anchor puts it.
  // GTK puts a popover in its own GdkSurface, which never maps under a
  // headless compositor with no seat, so the readable signal there is the
  // geometry getTree reports for the panel: none at all when it is unplaced
  // (the popover-anchor gate's finding), an origin on the anchor when placed.
  const panel = await poll(async () => findNode((await app.tree()).root, "site-info-body")?.geometry ?? null,
    (g) => g != null, { timeoutMs: T });
  if (panel!.x > field.x + field.w / 4) {
    throw new Error(`the site-info panel opened at ${panel!.x}, past the field's leading quarter (field ${field.x}..${field.x + field.w})`);
  }
  console.log(`  ND_HEADERFIELD_ANCHOR_OK the panel opened at ${panel!.x}, on the icon at the field's leading edge (field ${field.x}..${field.x + field.w})`);

  // ---- leg 3: setValue still emits `changed` -----------------------------
  await app.getByTestId("address").fill("nativedesktop.dev");
  await poll(() => label("url-label"), (v) => v === "nativedesktop.dev", { timeoutMs: T });
  console.log("  ND_HEADERFIELD_SETVALUE_OK setValue still round-trips through `changed`");

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
