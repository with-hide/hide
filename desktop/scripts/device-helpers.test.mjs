import { createHash } from "node:crypto";
import { Buffer } from "node:buffer";
import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { deviceHelperResources, prepareDeviceBundles } from "./device-helpers.mjs";
import { execFileSync } from "node:child_process";

const identity = { commit: "a".repeat(40), version: "1.2.3", order: "42" };
const folders = [];
afterEach(() => folders.splice(0).forEach((folder) => fs.rmSync(folder, { recursive: true, force: true })));

// Minimal executable headers stand in for foreign compiler output, an OS boundary.
function artifacts() {
  const parent = path.resolve("out/device-helper-tests");
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(parent, "kit-"));
  folders.push(root);
  for (const platform of ["macos-x86_64", "linux-x86_64", "linux-aarch64"]) {
    const folder = path.join(root, platform);
    fs.mkdirSync(folder);
    const bytes = Buffer.alloc(64);
    if (platform === "macos-x86_64") {
      bytes.writeUInt32LE(0xfeedfacf, 0);
      bytes.writeUInt32LE(0x01000007, 4);
      bytes.writeUInt32LE(2, 12);
    } else {
      bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]);
      bytes.writeUInt16LE(3, 16);
      bytes.writeUInt16LE(platform === "linux-x86_64" ? 62 : 183, 18);
    }
    const files = {};
    for (const name of ["hided", "hide", "hide-agent-hooks"]) {
      fs.writeFileSync(path.join(folder, name), bytes);
      files[name] = createHash("sha256").update(bytes).digest("hex");
    }
    fs.writeFileSync(path.join(folder, "manifest.json"), JSON.stringify({ ...identity, platform, files }));
  }
  return root;
}

describe("Mac device helper inputs", () => {
  it("publishes separate four-program archives and a small manifest of their final bytes", () => {
    const root = artifacts();
    const herdr = Object.fromEntries(["macos-x86_64", "linux-x86_64", "linux-aarch64"].map((p) => [p, path.join(root, p, "hided")]));
    const out = path.join(root, "bundles");
    const file = prepareDeviceBundles(root, identity, out, herdr);
    expect(fs.statSync(file).size).toBeLessThan(16 * 1024);
    const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
    expect(manifest).toMatchObject({ format: 1, ...identity });
    for (const [platform, bundle] of Object.entries(manifest.platforms)) {
      expect(bundle.asset).toBe(`hide-device-${platform}-1.2.3.tar.gz`);
      const archive = fs.readFileSync(path.join(out, bundle.asset));
      expect(bundle.sha256).toBe(createHash("sha256").update(archive).digest("hex"));
      expect(bundle.size).toBe(archive.length);
      expect(execFileSync("tar", ["-tzf", bundle.asset], { cwd: out, encoding: "utf8" }).trim().split("\n").sort())
        .toEqual(["hide", "hide-agent-hooks", "hided", "herdr"].sort());
      expect(Object.values(bundle.files).every((f) => f.size === 64 && /^[0-9a-f]{64}$/.test(f.sha256))).toBe(true);
    }
    expect(fs.readdirSync(out).sort()).toEqual(["device-bundles.json", ...Object.values(manifest.platforms).map((b) => b.asset)].sort());
  });
  it("offers all nine kit programs under names the SSH payload can select", () => {
    const files = deviceHelperResources(artifacts(), identity).map(([name]) => name);
    expect(files.sort()).toEqual([
      "hided-macos-x86_64", "hide-macos-x86_64", "hide-agent-hooks-macos-x86_64",
      "hided-linux-x86_64", "hide-linux-x86_64", "hide-agent-hooks-linux-x86_64",
      "hided-linux-aarch64", "hide-linux-aarch64", "hide-agent-hooks-linux-aarch64",
    ].sort());
  });

  it.each(["commit", "version", "order"])("refuses an artifact from another %s", (key) => {
    const root = artifacts();
    const wrong = { ...identity, [key]: key === "commit" ? "b".repeat(40) : "99" };
    expect(() => deviceHelperResources(root, wrong)).toThrow("identity mismatch");
  });

  it("refuses a missing platform instead of offering a partial kit", () => {
    const root = artifacts();
    fs.rmSync(path.join(root, "linux-aarch64"), { recursive: true });
    expect(() => deviceHelperResources(root, identity)).toThrow();
  });

  it("refuses a changed binary even when its architecture still matches", () => {
    const root = artifacts();
    fs.appendFileSync(path.join(root, "linux-x86_64/hide"), "changed");
    expect(() => deviceHelperResources(root, identity)).toThrow("digest mismatch");
  });

  it("refuses a program for another processor before accepting its digest", () => {
    const root = artifacts();
    fs.copyFileSync(path.join(root, "linux-x86_64/hide"), path.join(root, "linux-aarch64/hide"));
    expect(() => deviceHelperResources(root, identity)).toThrow("not a linux-aarch64 executable");
  });

  it("refuses a missing kit member and linked executable", () => {
    const root = artifacts();
    const program = path.join(root, "macos-x86_64/hide");
    fs.rmSync(program);
    expect(() => deviceHelperResources(root, identity)).toThrow();
    fs.symlinkSync("hided", program);
    expect(() => deviceHelperResources(root, identity)).toThrow("invalid device helper file");
  });
});
