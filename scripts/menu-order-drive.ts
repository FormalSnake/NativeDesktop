#!/usr/bin/env bun
// scripts/menu-order-drive.ts: drives examples/notes/menu-order-probe.tsx over
// the automation socket on BOTH backends (scripts/headless-menu-order.sh on
// Linux, scripts/mac/mac-menu-order.sh on macOS).
//
// What it proves: the NATIVE menu model matches React's child order after
// every structural shape React can emit for a keyed list. `menuModel` reads
// the model the owner is actually carrying (the GMenuModel on GTK, the NSMenu
// on AppKit), so a tree that reads right while the drawn menu is wrong fails
// here. A reorder is the interesting one: React moves a child with a bare
// insertBefore and no preceding remove, and an owner that appends instead of
// moving draws the item twice.
//
// Both owners carry the same keyed list: a <menubutton> (whose model is its
// own) and a menubar <menu> (whose items live in the app menu, under the
// "Tabs > " path). The menubutton's list is followed by a separator and a
// nested submenu, so a middle insert has to land before both.
import { connectApp, poll } from "../packages/test/src/index.ts";

const app = await connectApp();

const ownerRef = await app.getByTestId("probe-owner").ref();
const menubarRef = await app.getByTestId("probe-menubar").ref();

const TAIL = ["---", "More", "More > Nested one", "More > Nested two"];

async function reactOrder(): Promise<string[]> {
  const text = (await app.getByTestId("probe-order").node()).text ?? "";
  return text.replace(/^order=/, "").split("|");
}

async function nativeItems(ref: number): Promise<string[]> {
  return (await app.rpc.call("menuModel", { ref })).items;
}

/// The menubar model carries the platform's own menus too (the App menu on
/// AppKit), so the declared <menu> is read through its path prefix.
function tabsOnly(items: string[]): string[] {
  return items.filter((i) => i === "Tabs" || i.startsWith("Tabs > "));
}

async function assertBothMatch(step: string): Promise<void> {
  const order = await reactOrder();
  const wantOwner = [...order, ...TAIL];
  const wantBar = ["Tabs", ...order.map((n) => `Tabs > ${n}`)];
  // AppKit coalesces menu rebuilds onto the next main-queue turn, so the model
  // is polled rather than read once.
  const owner = await poll(() => nativeItems(ownerRef), (items) => items.join("|") === wantOwner.join("|"))
    .catch(async () => {
      throw new Error(`${step}: menubutton drew [${(await nativeItems(ownerRef)).join(", ")}], React said [${wantOwner.join(", ")}]`);
    });
  const bar = await poll(async () => tabsOnly(await nativeItems(menubarRef)), (items) => items.join("|") === wantBar.join("|"))
    .catch(async () => {
      throw new Error(`${step}: menubar drew [${tabsOnly(await nativeItems(menubarRef)).join(", ")}], React said [${wantBar.join(", ")}]`);
    });
  console.log(`ND_MENU_ORDER_STEP ${step}: owner=[${owner.join(", ")}] bar=[${bar.join(", ")}]`);
}

// ---- leg 1: the mounted order -----------------------------------------------
await app.waitForText("order=Alpha|Bravo|Charlie|Delta");
await assertBothMatch("mounted");

// ---- leg 2: a MOVE (insertBefore of an already-mounted child, no remove) ----
await app.getByTestId("probe-reorder").click();
await app.waitForText("order=Bravo|Charlie|Alpha|Delta");
await assertBothMatch("reordered");

// ---- leg 3: a middle remove -------------------------------------------------
await app.getByTestId("probe-remove").click();
await app.waitForText("order=Bravo|Alpha|Delta");
await assertBothMatch("removed-middle");

// ---- leg 4: a middle insert -------------------------------------------------
await app.getByTestId("probe-insert").click();
await app.waitForText("order=Bravo|Echo|Alpha|Delta");
await assertBothMatch("inserted-middle");

console.log("ND_MENU_ORDER_OK the native menu matched React after a move, a middle remove and a middle insert");
await app.close();
