import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkReleaseBinaries, recordReleaseBinaries } from "./release-binaries.mjs";

const identity = { commit: "a".repeat(40), version: "1.2.3", order: "42" };
const names = ["hided", "hide", "hide-agent-hooks"];
const folders = [];
afterEach(() => folders.splice(0).forEach((folder) => fs.rmSync(folder, { recursive: true, force: true })));

// What `compile` leaves, then what a workflow artifact gives back: the same bytes without their mode.
function compiled() {
  const parent = path.resolve("out/release-binaries-tests");
  fs.mkdirSync(parent, { recursive: true });
  const folder = fs.mkdtempSync(path.join(parent, "release-"));
  folders.push(folder);
  for (const name of names) fs.writeFileSync(path.join(folder, name), `${name} binary`, { mode: 0o644 });
  const record = path.join(folder, "release-binaries.json");
  recordReleaseBinaries(record, folder, names, identity);
  return { folder, record };
}

describe("release binaries between compile and assemble", () => {
  it("hands assemble the recorded binaries, executable again", () => {
    const { folder, record } = compiled();
    checkReleaseBinaries(record, folder, names, identity);
    for (const name of names) expect(fs.statSync(path.join(folder, name)).mode & 0o111).toBe(0o111);
  });

  it.each(["commit", "version", "order"])("refuses binaries built for another %s", (key) => {
    const { folder, record } = compiled();
    expect(() => checkReleaseBinaries(record, folder, names, { ...identity, [key]: key === "commit" ? "b".repeat(40) : "99" }))
      .toThrow("compile them again");
  });

  it("refuses a changed, missing or linked binary", () => {
    const changed = compiled();
    fs.appendFileSync(path.join(changed.folder, "hided"), "changed");
    expect(() => checkReleaseBinaries(changed.record, changed.folder, names, identity)).toThrow("differs from what compile recorded");
    const missing = compiled();
    fs.rmSync(path.join(missing.folder, "hide"));
    expect(() => checkReleaseBinaries(missing.record, missing.folder, names, identity)).toThrow();
    const linked = compiled();
    fs.renameSync(path.join(linked.folder, "hide"), path.join(linked.folder, "hide-real"));
    fs.symlinkSync("hide-real", path.join(linked.folder, "hide"));
    expect(() => checkReleaseBinaries(linked.record, linked.folder, names, identity)).toThrow("differs from what compile recorded");
  });

  it("refuses a record of other programs and a package that was never compiled", () => {
    const { folder, record } = compiled();
    expect(() => checkReleaseBinaries(record, folder, ["hided", "hide"], identity)).toThrow("other programs");
    fs.rmSync(record);
    expect(() => checkReleaseBinaries(record, folder, names, identity)).toThrow();
  });
});
