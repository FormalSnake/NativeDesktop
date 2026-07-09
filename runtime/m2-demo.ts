// M2 demo: plain TypeScript (no React) builds Window -> Box(vertical) -> [Label, Button, Label]
// over NDP, reacts to button-click events, and drives a timer commit so headless CI can
// assert ND_COMMIT_APPLIED markers without input synthesis.

import { Ndp, type Op } from "./ndp";

const WIN = 1,
  BOX = 2,
  CLICKS = 3,
  BUTTON = 4,
  UPTIME = 5;

const ndp = await Ndp.connect();
await ndp.handshake({ name: "bun", version: Bun.version });

let commitId = 0;
const commit = (ops: Op[]) => ndp.sendCommit({ commitId: commitId++, generation: 0, ops });

// Initial tree.
commit([
  { op: "create", id: WIN, widget: "Window", props: { title: "NativeDesktop M2", defaultWidth: 480, defaultHeight: 320 } },
  { op: "create", id: BOX, widget: "Box", props: { orientation: "vertical", spacing: 8 } },
  { op: "append", parent: WIN, child: BOX },
  { op: "create", id: CLICKS, widget: "Label", props: { text: "Clicks: 0" } },
  { op: "append", parent: BOX, child: CLICKS },
  { op: "create", id: BUTTON, widget: "Button", props: { label: "Click me" } },
  { op: "append", parent: BOX, child: BUTTON },
  { op: "create", id: UPTIME, widget: "Label", props: { text: "Uptime: 0s" } },
  { op: "append", parent: BOX, child: UPTIME },
]);

let clicks = 0;
ndp.onEvent((e) => {
  if (e.name === "clicked" && e.nodeId === BUTTON) {
    clicks++;
    commit([{ op: "setText", id: CLICKS, text: `Clicks: ${clicks}` }]);
  }
});

// Drive commits without input so headless CI can assert ND_COMMIT_APPLIED.
let seconds = 0;
setInterval(() => {
  seconds++;
  commit([{ op: "setText", id: UPTIME, text: `Uptime: ${seconds}s` }]);
}, 500);
