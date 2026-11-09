import { describe, expect, it } from "vitest";
import { processStartConfirmationAction, splitCommandLine } from "./command";

describe("splitCommandLine", () => {
  it("keeps quoted arguments together", () => {
    expect(splitCommandLine('npm run dev -- --host "127.0.0.1"')).toEqual([
      "npm", "run", "dev", "--", "--host", "127.0.0.1",
    ]);
  });

  it("scopes dev-server confirmation to the program and full argument list", () => {
    const parts = splitCommandLine("npm run dev -- --host 127.0.0.1");
    expect(processStartConfirmationAction(7, parts)).toBe(
      'process:start:7:npm:["npm","run","dev","--","--host","127.0.0.1"]',
    );
  });
});

