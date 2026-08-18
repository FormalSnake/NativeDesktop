import { describe, expect, test } from "bun:test";
import { otpCellChanged, otpChars } from "./otp.ts";

describe("otpChars", () => {
  test("pads and truncates to length", () => {
    expect(otpChars("12", 6)).toEqual(["1", "2", "", "", "", ""]);
    expect(otpChars("1234567", 6)).toEqual(["1", "2", "3", "4", "5", "6"]);
    expect(otpChars("", 4)).toEqual(["", "", "", ""]);
  });
});

describe("otpCellChanged", () => {
  test("typing a digit fills the cell and advances", () => {
    expect(otpCellChanged("", 6, 0, "1")).toEqual({ value: "1", activeIndex: 1 });
    expect(otpCellChanged("1", 6, 1, "2")).toEqual({ value: "12", activeIndex: 2 });
  });

  test("advancing stops at the last cell", () => {
    expect(otpCellChanged("12345", 6, 5, "6")).toEqual({ value: "123456", activeIndex: 5 });
  });

  test("a multi-character box (paste) distributes across the following cells", () => {
    expect(otpCellChanged("", 6, 0, "123456")).toEqual({ value: "123456", activeIndex: 5 });
    expect(otpCellChanged("ab", 6, 2, "cdef")).toEqual({ value: "abcdef", activeIndex: 5 });
  });

  test("a paste longer than the remaining cells is truncated", () => {
    expect(otpCellChanged("", 4, 2, "999999")).toEqual({ value: "99", activeIndex: 3 });
  });

  test("clearing a filled cell (backspace) stays on that cell", () => {
    expect(otpCellChanged("123456", 6, 5, "")).toEqual({ value: "12345", activeIndex: 5 });
  });

  test("clearing an already-empty cell steps back one", () => {
    expect(otpCellChanged("12", 6, 2, "")).toEqual({ value: "12", activeIndex: 1 });
    expect(otpCellChanged("", 6, 0, "")).toEqual({ value: "", activeIndex: 0 });
  });

  test("typing into a mid box that already had a character overwrites it", () => {
    expect(otpCellChanged("123456", 6, 2, "9")).toEqual({ value: "129456", activeIndex: 3 });
  });
});
