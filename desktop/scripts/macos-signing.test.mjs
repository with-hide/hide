import { describe, expect, it } from "vitest";
import { staple } from "./macos-signing.mjs";

/** A stapler answering `statuses` in order, recording each call and each pause. */
function stapler(...statuses) {
  const calls = [];
  const pauses = [];
  const run = (target) => {
    calls.push(target);
    return { status: statuses[calls.length - 1] };
  };
  return { calls, pauses, options: { run, pause: (seconds) => pauses.push(seconds) } };
}

describe("staple", () => {
  it("fetches the ticket again after Apple's ticket service drops the connection", () => {
    const fake = stapler(68, 65, 0);
    staple("/out/hide.app", fake.options);
    expect(fake.calls).toEqual(["/out/hide.app", "/out/hide.app", "/out/hide.app"]);
    expect(fake.pauses).toEqual([10, 20]);
  });

  it("fails after the sixth try that cannot fetch the ticket", () => {
    const fake = stapler(68, 68, 68, 68, 68, 68, 0);
    expect(() => staple("/out/hide.dmg", fake.options)).toThrow("exit 68 after 6 tries");
    expect(fake.calls).toHaveLength(6);
    expect(fake.pauses).toEqual([10, 20, 40, 80, 160]);
  });

  it("fails at once when the target itself cannot be stapled", () => {
    const fake = stapler(1, 0);
    expect(() => staple("/out/hide.app", fake.options)).toThrow("exit 1 after 1 try");
    expect(fake.pauses).toEqual([]);
  });
});
