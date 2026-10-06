#!/usr/bin/env bun
// scripts/counter-solid-drive.ts: launches examples/counter-solid under
// NATIVE_AUTOMATION and checks the Solid renderer end to end: the tree mounts,
// a click routes through the registry to the handler, the signal write lands
// as one update on the label, and the <Loading> fallback gives way to the
// resolved badge. ND_HOST_BINARY picks the host (e.g. swift/.build/release/NDShell).
import { launchApp, expect } from "../packages/test/src/index.ts";

const app = await launchApp({ entry: "examples/counter-solid/main.tsx", hostBinary: process.env.ND_HOST_BINARY });
try {
  const label = app.getByTestId("clicks-label");
  await expect(label).toHaveText("Clicks: 0");
  await app.getByTestId("increment-button").click();
  await expect(label).toHaveText("Clicks: 1");
  await app.getByTestId("increment-button").click();
  await expect(label).toHaveText("Clicks: 2");
  await expect(app.getByTestId("badge-label")).toHaveText("ready:loading-resolved");
  console.log("ND_COUNTER_SOLID_OK mount, click -> label update, Loading fallback resolved");
} finally {
  await app.close();
}
