#!/usr/bin/env bun
// scripts/decoration-drive.ts: how much the window draws in the trailing end
// of its header bar, which is where GTK packs the close button. Run against a
// host that is already up (ND_AUTOMATION_SOCKET), once per decoration layout
// under test; scripts/headless-decoration.sh is the rig. Prints
// ND_DECO_INK <pixels> so the caller can compare two layouts.
import { AutomationClient } from "../packages/test/src/socket.ts";
import { decodePng, luminance, modalFill } from "./capture-ink.ts";

// The trailing end of the header bar, in logical units from the window's
// top-right corner. Wide enough for a window-controls run and short enough to
// stay clear of a centred title, on a window sized by the rig. Inset on every
// side, because the window's own 1px border and the header bar's bottom
// hairline both cross this corner and are drawn whatever the layout says.
const corner = { w: 92, right: 8, y: 8, h: 30 };

const c = await AutomationClient.connect();
const tree: any = await c.call("getTree" as never);
const win = tree.root.geometry;
if (!win) throw new Error("the root window reports no geometry");
const path = process.env.ND_DECO_SHOT ?? "/tmp/nd-decoration.png";
await c.call("screenshot" as never, { path } as never);
c.close();

const img = decodePng(new Uint8Array(await Bun.file(path).arrayBuffer()));
const scale = img.w / win.w;
const y0 = Math.round(corner.y * scale), y1 = Math.round((corner.y + corner.h) * scale);
// The header bar's own fill, read across its full width so a run of window
// controls cannot become the reference it is measured against.
const fill = modalFill(img, 0, img.w, y0, y1);
const x0 = Math.max(0, img.w - Math.round(corner.w * scale)), x1 = img.w - Math.round(corner.right * scale);
let ink = 0;
for (let y = y0; y < y1; y++) {
  for (let x = x0; x < x1; x++) if (Math.abs(luminance(img, x, y) - fill) > 12) ink++;
}
console.log(`ND_DECO_INK ${ink} (window ${win.w}x${win.h}, corner ${x0}-${x1} x ${y0}-${y1}, fill ${fill}, shot ${path})`);
