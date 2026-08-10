import { describe, expect, test } from "bun:test";
import { ConnectionLadder, backoffDelayMs } from "./backoff.ts";

describe("backoffDelayMs", () => {
  test("doubles from the base and caps at the max", () => {
    expect([1, 2, 3, 4, 5, 6, 7].map((n) => backoffDelayMs(n))).toEqual([
      500, 1000, 2000, 4000, 8000, 16000, 16000,
    ]);
  });

  test("clamps attempt < 1 to the base", () => {
    expect(backoffDelayMs(0)).toBe(500);
    expect(backoffDelayMs(-3)).toBe(500);
  });

  test("honors custom base and max", () => {
    expect(backoffDelayMs(1, 100, 400)).toBe(100);
    expect(backoffDelayMs(3, 100, 400)).toBe(400);
    expect(backoffDelayMs(9, 100, 400)).toBe(400);
  });
});

describe("ConnectionLadder", () => {
  test("nextDelayMs advances one rung per call", () => {
    const ladder = new ConnectionLadder();
    expect(ladder.attempt).toBe(0);
    expect(ladder.nextDelayMs()).toBe(500);
    expect(ladder.nextDelayMs()).toBe(1000);
    expect(ladder.attempt).toBe(2);
  });

  test("stability window >= 30s resets the ladder on disconnect", () => {
    let t = 0;
    const ladder = new ConnectionLadder({ now: () => t });
    ladder.nextDelayMs();
    ladder.nextDelayMs();
    ladder.nextDelayMs();
    expect(ladder.attempt).toBe(3);
    ladder.noteConnected();
    t += 30_000;
    ladder.noteDisconnected();
    expect(ladder.attempt).toBe(0);
    expect(ladder.nextDelayMs()).toBe(500); // fresh outage starts at the bottom
  });

  test("a drop before the stability window inherits the rung", () => {
    let t = 0;
    const ladder = new ConnectionLadder({ now: () => t });
    ladder.nextDelayMs();
    ladder.nextDelayMs();
    ladder.nextDelayMs();
    ladder.noteConnected();
    t += 29_999;
    ladder.noteDisconnected();
    expect(ladder.attempt).toBe(3);
    expect(ladder.nextDelayMs()).toBe(4000); // continues the old outage's backoff
  });

  test("a disconnect with no prior connect keeps the rung", () => {
    const ladder = new ConnectionLadder();
    ladder.nextDelayMs();
    ladder.noteDisconnected();
    expect(ladder.attempt).toBe(1);
  });

  test("reset() zeroes unconditionally", () => {
    let t = 0;
    const ladder = new ConnectionLadder({ now: () => t });
    ladder.nextDelayMs();
    ladder.noteConnected();
    ladder.reset();
    expect(ladder.attempt).toBe(0);
    t += 60_000;
    ladder.noteDisconnected(); // must not throw or reference the cleared timestamp
    expect(ladder.attempt).toBe(0);
  });

  test("custom stabilityWindowMs is honored", () => {
    let t = 0;
    const ladder = new ConnectionLadder({ now: () => t, stabilityWindowMs: 100 });
    ladder.nextDelayMs();
    ladder.noteConnected();
    t += 100;
    ladder.noteDisconnected();
    expect(ladder.attempt).toBe(0);
  });
});
