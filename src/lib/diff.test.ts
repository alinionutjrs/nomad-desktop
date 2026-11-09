import { describe, expect, it } from "vitest";
import { createLineDiff } from "./diff";

describe("createLineDiff", () => {
  it("marks additions and removals", () => {
    const diff = createLineDiff("a\nb", "a\nc");
    expect(diff.map((line) => [line.kind, line.text])).toEqual([
      ["same", "a"],
      ["remove", "b"],
      ["add", "c"],
    ]);
  });
});

