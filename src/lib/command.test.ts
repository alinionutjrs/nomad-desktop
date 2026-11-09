import { describe, expect, it } from "vitest";
import { splitCommandLine } from "./command";

describe("splitCommandLine", () => {
  it("keeps quoted arguments together", () => {
    expect(splitCommandLine('npm run dev -- --host "127.0.0.1"')).toEqual([
      "npm", "run", "dev", "--", "--host", "127.0.0.1",
    ]);
  });
});

