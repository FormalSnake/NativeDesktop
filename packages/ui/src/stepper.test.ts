import { describe, expect, test } from "bun:test";
import { stepState } from "./stepper.ts";

describe("stepState", () => {
  test("classifies steps relative to the active index", () => {
    expect(stepState(0, 2)).toBe("completed");
    expect(stepState(1, 2)).toBe("completed");
    expect(stepState(2, 2)).toBe("active");
    expect(stepState(3, 2)).toBe("pending");
  });

  test("activeIndex 0: only the first step is active, nothing completed", () => {
    expect(stepState(0, 0)).toBe("active");
    expect(stepState(1, 0)).toBe("pending");
  });
});
