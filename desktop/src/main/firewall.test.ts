import { describe, expect, it } from "vitest";
import { allowFirewall, firewallState, type RunChild } from "./firewall";
import type { ChildResult } from "./spawn";

const answer = (code: number | null, stdout = ""): ChildResult => ({ code, signal: null, stdout, stderr: "", timedOut: false, spawnError: null });

/** A run that answers each verb from a table and records each call. */
function runs(table: Record<string, ChildResult>): { run: RunChild; calls: string[][] } {
  const calls: string[][] = [];
  const run: RunChild = (file, args) => {
    calls.push([file, ...args]);
    return Promise.resolve(table[args.join(" ")] ?? answer(2));
  };
  return { run, calls };
}

describe("the firewall ask", () => {
  it("asks nothing outside Windows", async () => {
    const { run, calls } = runs({});
    await expect(allowFirewall("darwin", "hide", run)).resolves.toBe("not_used");
    await expect(firewallState("linux", "hide", run)).resolves.toBe("not_used");
    expect(calls).toEqual([]);
  });

  it("reads what the one ask came to, and anything else as a failure", async () => {
    for (const [ran, expected] of [
      [answer(0, '{"answer":"allowed"}'), "allowed"],
      [answer(0, '{"answer":"declined"}'), "declined"],
      [answer(0, '{"answer":"failed"}'), "failed"],
      [answer(2, '{"answer":"allowed"}'), "failed"],
      [answer(0, "not json"), "failed"],
    ] as const) {
      const { run, calls } = runs({ "always-on firewall ask-allow": ran });
      await expect(allowFirewall("win32", "C:\\hide.exe", run)).resolves.toBe(expected);
      expect(calls).toEqual([["C:\\hide.exe", "always-on", "firewall", "ask-allow"]]);
    }
  });

  it("reads the rules' state from the status line", async () => {
    const { run } = runs({ "always-on status --json": answer(0, '{"firewall":"missing","paired":true}') });
    await expect(firewallState("win32", "hide", run)).resolves.toBe("missing");
    const broken = runs({ "always-on status --json": answer(0, "not json") });
    await expect(firewallState("win32", "hide", broken.run)).resolves.toBe("unknown");
  });
});
