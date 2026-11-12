import { describe, expect, it } from "vitest";
import { toMilli } from "./CloudCost";

describe("cloud cost amounts", () => {
  it("converts euros typed by the user to thousandths, empty meaning not set", () => {
    expect(toMilli("")).toBe(0);
    expect(toMilli("0,15")).toBe(150);
    expect(toMilli("2.5")).toBe(2500);
    expect(toMilli("1000")).toBe(1_000_000);
  });

  it("rejects negative, too large and non-numeric amounts", () => {
    expect(toMilli("-1")).toBeUndefined();
    expect(toMilli("1000.001")).toBeUndefined();
    expect(toMilli("abc")).toBeUndefined();
  });
});
