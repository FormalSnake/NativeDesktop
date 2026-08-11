// Notarization gating: an explicit flag/config wins; the auto default must
// not submit an unsigned bundle even with Apple credentials exported.
import { describe, expect, test } from "bun:test";
import { resolveNotarize } from "./mac.ts";

describe("resolveNotarize", () => {
  test("auto notarizes only with credentials AND a signed bundle", () => {
    expect(resolveNotarize(undefined, true, true)).toBe(true);
    expect(resolveNotarize(undefined, true, false)).toBe(false);
    expect(resolveNotarize(undefined, false, true)).toBe(false);
    expect(resolveNotarize(undefined, false, false)).toBe(false);
  });

  test("an explicit request or refusal wins over the auto default", () => {
    expect(resolveNotarize(true, false, false)).toBe(true);
    expect(resolveNotarize(false, true, true)).toBe(false);
  });
});
