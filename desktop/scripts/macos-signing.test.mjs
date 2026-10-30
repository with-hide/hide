import { describe, expect, it } from "vitest";
import { localSigning, staple } from "./macos-signing.mjs";

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

const HIDE = ["3A4C4ACB82CFE3BA6A34ADDE16DE24420B4C21A0", "Developer ID Application: hoyeon lee (GH6F6ZA497)"];
const OTHER = ["0123456789ABCDEF0123456789ABCDEF01234567", "Developer ID Application: Someone Else (ABCDE12345)"];
/** `security find-identity -v -p codesigning` output listing `identities`. */
const listing = (...identities) =>
  `${identities.map(([sha, name], index) => `  ${index + 1}) ${sha} "${name}"`).join("\n")}\n     ${identities.length} valid identities found\n`;

describe("localSigning", () => {
  it("signs a local package with the only Developer ID Application identity", () => {
    const development = ["89ABCDEF0123456789ABCDEF0123456789ABCDEF", "Apple Development: hoyeon lee (ZZZZZZZZZZ)"];
    expect(localSigning(listing(development, HIDE))).toEqual({ mode: "local", identity: HIDE[0], name: HIDE[1] });
  });

  it("stays ad-hoc for a contributor whose keychains hold no Developer ID", () => {
    expect(localSigning(listing())).toEqual({ mode: "adhoc" });
  });

  it("refuses to guess between two Developer ID identities", () => {
    expect(() => localSigning(listing(HIDE, OTHER))).toThrow("name one with HIDE_SIGN_IDENTITY");
  });

  it("uses the identity HIDE_SIGN_IDENTITY names, by name or SHA-1", () => {
    expect(localSigning(listing(HIDE, OTHER), OTHER[1]).identity).toBe(OTHER[0]);
    expect(localSigning(listing(HIDE, OTHER), HIDE[0]).identity).toBe(HIDE[0]);
    expect(() => localSigning(listing(HIDE), OTHER[1])).toThrow("no keychain on the search list holds");
  });
});
