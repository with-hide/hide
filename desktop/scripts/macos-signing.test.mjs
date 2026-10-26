import path from "node:path";
import { describe, expect, it } from "vitest";
import { developerIdOptions, staple } from "./macos-signing.mjs";

it("keeps foreign Linux resources out of signing while both Mac architectures use executable entitlements", () => {
  const options = developerIdOptions({ identity: "fixture" }, ["hided", "hided-macos-x86_64", "hided-linux-aarch64"],
    "/entitlements", {}, ["hided-linux-aarch64"]).osxSign;
  const resource = (name) => `/out/hide.app/Contents/Resources/${name}`;
  expect(options.ignore(resource("hided-linux-aarch64"))).toBe(true);
  for (const name of ["hided", "hided-macos-x86_64"]) {
    expect(options.ignore(resource(name))).toBe(false);
    expect(options.optionsForFile(resource(name)).entitlements).toBe(path.join("/entitlements", "executable.plist"));
  }
  expect(options.ignore("/out/hide.app/Contents/MacOS/hided-linux-aarch64")).toBe(false);
});

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
