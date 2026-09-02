#!/usr/bin/env bun
// scripts/settings-drive.ts [gtk|appkit] — drives examples/settings/main.tsx
// via @nativedesktop/test. Acceptance for the boxed-list wave: the settings
// pages are real <settingsgroup title>/<row>/<switchrow> widgets (SwitchRow
// a11y role/value + setValue toggle path, Row suffix controls, activatable
// Row activation, SettingsGroup role), navigation runs through the
// <sourcelist> sidebar, and React state survives page remounts. Prints
// ND_ROWS_OK on success.
import { expect, launchApp } from "../packages/test/src/index.ts";
import type { Backend } from "@nativedesktop/host";

const backend = process.argv[2] as Backend | undefined;
const shotDir = process.env.ND_SHOT_DIR ?? "/tmp";

const app = await launchApp({ entry: "examples/settings/main.tsx", backend });
try {
  // ---- leg 1: boxed-list shape — group + rows with roles ---------------------
  await expect(app.getByTestId("general-card")).toHaveAttribute("role", "group");
  await expect(app.getByTestId("setting-launch")).toHaveAttribute("role", "switch");
  await app.waitForValue("setting-launch", true, { timeoutMs: 3000 });
  console.log("ND_ROWS_SHAPE_OK settingsgroup=group switchrow=switch");

  // ---- leg 2: switchrow setValue -> toggled -> React state -------------------
  await app.getByTestId("setting-launch").uncheck();
  await app.waitForText("launch false", { timeoutMs: 3000 });
  await app.waitForValue("setting-launch", false, { timeoutMs: 3000 });
  console.log("ND_ROWS_TOGGLE_OK switchrow setValue round-trip");

  // ---- leg 3: sidebar navigation via <sourcelist> ----------------------------
  await app.getByTestId("settings-categories").selectOption(1);
  await app.waitForPresent("appearance-card", { timeoutMs: 3000 });
  const slider = app.getByTestId("setting-textsize");
  await slider.fill(21);
  await app.waitForValue("setting-textsize", 21, { timeoutMs: 3000 });
  console.log(`ND_ROWS_NAV_OK sourcelist page switch; row-suffix slider ref=${await slider.ref()} accepts setValue`);

  // ---- leg 4: state survives remounts; activatable row; reset ----------------
  await app.getByTestId("settings-categories").selectOption(2);
  await app.waitForPresent("advanced-card", { timeoutMs: 3000 });
  await app.getByTestId("setting-devmode").check();
  await app.waitForText("devmode true", { timeoutMs: 3000 });
  await app.getByTestId("settings-categories").selectOption(1);
  await app.waitForPresent("appearance-card", { timeoutMs: 3000 });
  await app.waitForValue("setting-textsize", 21, { timeoutMs: 3000 }); // remount kept React state
  await app.getByTestId("settings-categories").selectOption(2);
  await app.waitForPresent("advanced-card", { timeoutMs: 3000 });
  await app.getByTestId("check-updates-row").click();
  await app.waitForText("updates checked", { timeoutMs: 3000 });
  await app.getByTestId("reset-button").click();
  await app.waitForText("settings reset", { timeoutMs: 3000 });
  await app.waitForValue("setting-devmode", false, { timeoutMs: 3000 });
  console.log("ND_ROWS_STATE_OK remount persistence + activatable row + reset");

  const shot = await app.screenshot(`${shotDir}/settings-rows-${app.backend}.png`, { minBytes: 2000 });
  console.log(`ND_ROWS_SHOT ${shot.path} ${shot.width}x${shot.height}`);

  console.log(`ND_ROWS_OK backend=${app.backend}`);
} finally {
  await app.close();
}
