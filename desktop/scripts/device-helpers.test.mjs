import { createHash } from "node:crypto";
import { Buffer } from "node:buffer";
import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  DEVICE_KITS, assertDeviceExecutable, deviceAsset, deviceHelperResources, prepareDeviceBundles, readDeviceBundles, stageDeviceKit,
} from "./device-helpers.mjs";
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
  for (const platform of ["linux-x86_64", "linux-aarch64"]) {
    const folder = path.join(root, platform);
    fs.mkdirSync(folder);
    const bytes = Buffer.alloc(64);
    bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]);
    bytes.writeUInt16LE(3, 16);
    bytes.writeUInt16LE(platform === "linux-x86_64" ? 62 : 183, 18);
    const files = {};
    for (const name of ["hided", "hide", "hide-agent-hooks", "hide-holder"]) {
      fs.writeFileSync(path.join(folder, name), bytes);
      files[name] = createHash("sha256").update(bytes).digest("hex");
    }
    fs.writeFileSync(path.join(folder, "manifest.json"), JSON.stringify({ ...identity, platform, files }));
  }
  return root;
}

// The Mac package publishes the one set of bundles a run made beside its installers, and takes them
// only when they are that set; no app carries their manifest (with-hide/hide-private#501).
describe("bundles one run made, as a package takes them", () => {
  function made() {
    const root = artifacts();
    const out = path.join(root, "bundles");
    return { out, manifest: prepareDeviceBundles(root, identity, out) };
  }

  it("takes the manifest of bundles made for this source, version and order", () => {
    const { out, manifest } = made();
    expect(readDeviceBundles(out, identity)).toBe(manifest);
  });

  it.each(["commit", "version", "order"])("refuses bundles made for another %s", (key) => {
    const { out } = made();
    expect(() => readDeviceBundles(out, { ...identity, [key]: key === "commit" ? "b".repeat(40) : "99" })).toThrow("identity mismatch");
  });

  it("refuses an archive that changed after its manifest was made, or is missing", () => {
    const { out, manifest } = made();
    const { platforms } = JSON.parse(fs.readFileSync(manifest, "utf8"));
    const archive = path.join(out, platforms["linux-x86_64"].asset);
    const bytes = fs.readFileSync(archive);
    bytes[bytes.length - 1] ^= 0xff;
    fs.writeFileSync(archive, bytes);
    expect(() => readDeviceBundles(out, identity)).toThrow("digest mismatch");
    fs.rmSync(archive);
    expect(() => readDeviceBundles(out, identity)).toThrow();
  });
});

describe("Mac device helper inputs", () => {
  it("publishes separate four-program archives and a small manifest of their final bytes", () => {
    const root = artifacts();
    const out = path.join(root, "bundles");
    const file = prepareDeviceBundles(root, identity, out);
    expect(fs.statSync(file).size).toBeLessThan(16 * 1024);
    const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
    expect(manifest).toMatchObject({ format: 4, ...identity });
    expect(Object.keys(manifest.platforms).sort()).toEqual(["linux-aarch64", "linux-x86_64"]);
    for (const [platform, bundle] of Object.entries(manifest.platforms)) {
      expect(bundle.asset).toBe(`hide-device-${platform}-1.2.3.tar.gz`);
      const archive = fs.readFileSync(path.join(out, bundle.asset));
      expect(bundle.sha256).toBe(createHash("sha256").update(archive).digest("hex"));
      expect(bundle.size).toBe(archive.length);
      expect(execFileSync("tar", ["-tzf", bundle.asset], { cwd: out, encoding: "utf8" }).trim().split("\n").sort())
        .toEqual(["hide", "hide-agent-hooks", "hide-holder", "hided"].sort());
      expect(Object.values(bundle.files).every((f) => f.size === 64 && /^[0-9a-f]{64}$/.test(f.sha256))).toBe(true);
    }
    expect(fs.readdirSync(out).sort()).toEqual(["device-bundles.json", ...Object.values(manifest.platforms).map((b) => b.asset)].sort());
  });
  it("offers all eight Linux kit programs under names the SSH payload can select", () => {
    const files = deviceHelperResources(artifacts(), identity).map(([name]) => name);
    expect(files.sort()).toEqual([
      "hided-linux-x86_64", "hide-linux-x86_64", "hide-agent-hooks-linux-x86_64", "hide-holder-linux-x86_64",
      "hided-linux-aarch64", "hide-linux-aarch64", "hide-agent-hooks-linux-aarch64", "hide-holder-linux-aarch64",
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
    const program = path.join(root, "linux-x86_64/hide");
    fs.rmSync(program);
    expect(() => deviceHelperResources(root, identity)).toThrow();
    fs.symlinkSync("hided", program);
    expect(() => deviceHelperResources(root, identity)).toThrow("invalid device helper file");
  });
});

/** A program's first bytes for a contract header, as its linker writes them. */
function program(header, tail) {
  const bytes = Buffer.alloc(128);
  if (header.kind === "elf") {
    bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]);
    bytes.writeUInt16LE(3, 16);
    bytes.writeUInt16LE(header.machine, 18);
  } else if (header.kind === "mach-o") {
    bytes.writeUInt32LE(0xfeedfacf, 0);
    bytes.writeUInt32LE(header.cputype, 4);
    bytes.writeUInt32LE(2, 12);
  } else {
    bytes.write("MZ", 0, "latin1");
    bytes.writeUInt32LE(0x40, 0x3c);
    bytes.write("PE\0\0", 0x40, "latin1");
    bytes.writeUInt16LE(header.machine, 0x44);
  }
  return Buffer.concat([bytes, Buffer.from(tail)]);
}

describe("device kit headers", () => {
  it("passes each kit's programs for their own platform only", () => {
    for (const [platform, kit] of Object.entries(DEVICE_KITS)) {
      for (const other of Object.keys(DEVICE_KITS)) {
        const check = () => assertDeviceExecutable(program(kit.header, "x"), other);
        if (DEVICE_KITS[other].header.kind === kit.header.kind && JSON.stringify(DEVICE_KITS[other].header) === JSON.stringify(kit.header)) {
          expect(check).not.toThrow();
        } else {
          expect(check, `${platform} as ${other}`).toThrow(`not a ${other} executable`);
        }
      }
    }
  });
});

describe("a kit staged into the developer folder", () => {
  /** A release's archive and manifest for `platform`, made from `files` (name to bytes) with the system tar. */
  function release(platform, files, listed = files) {
    const parent = path.resolve("out/device-helper-tests");
    fs.mkdirSync(parent, { recursive: true });
    const root = fs.mkdtempSync(path.join(parent, "stage-"));
    folders.push(root);
    const source = path.join(root, "source");
    for (const [name, bytes] of Object.entries(files)) {
      const target = path.join(source, ...name.split("/"));
      fs.mkdirSync(path.dirname(target), { recursive: true });
      if (bytes === "link") fs.symlinkSync("elsewhere", target);
      else fs.writeFileSync(target, bytes);
    }
    const asset = deviceAsset(platform, identity.version);
    // Relative paths only: the GNU tar on a Windows runner reads `D:\...` as a
    // remote host's name.
    execFileSync("tar", ["--format", "ustar", "-czf", asset, "-C", "source", ...Object.keys(files)], {
      cwd: root,
      env: { ...process.env, COPYFILE_DISABLE: "1" },
    });
    const archive = fs.readFileSync(path.join(root, asset));
    const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
    const manifest = path.join(root, "device-bundles.json");
    fs.writeFileSync(manifest, JSON.stringify({
      format: 4, ...identity,
      platforms: { [platform]: { asset, sha256: digest(archive), size: archive.length,
        files: Object.fromEntries(Object.entries(listed).map(([name, bytes]) => [name, { sha256: digest(bytes), size: bytes.length }])) } },
    }));
    return { root, manifest, archive: path.join(root, asset) };
  }

  function kitFiles(platform) {
    const kit = DEVICE_KITS[platform];
    return Object.fromEntries([
      ...["hided", "hide", "hide-agent-hooks", "hide-holder"].map((name) => [`${name}${kit.suffix}`, program(kit.header, name)]),
      ...Object.keys(kit.runtime).map((name) => [name, Buffer.from(`runtime ${name}`)]),
    ]);
  }

  it("writes every file under the name a core finds before any release", () => {
    const { root, manifest, archive } = release("windows-x86_64", kitFiles("windows-x86_64"));
    const folder = path.join(root, "developer");
    expect(stageDeviceKit(manifest, archive, folder)).toEqual([
      "hide-agent-hooks-windows-x86_64", "hide-holder-windows-x86_64", "hide-windows-x86_64", "hided-windows-x86_64",
      "holder-conpty-windows-x86_64/arm64/OpenConsole.exe", "holder-conpty-windows-x86_64/conpty-files.json",
      "holder-conpty-windows-x86_64/conpty.dll", "holder-conpty-windows-x86_64/x64/OpenConsole.exe",
    ]);
    expect(fs.readFileSync(path.join(folder, "hided-windows-x86_64"))).toEqual(kitFiles("windows-x86_64")["hided.exe"]);
    if (process.platform !== "win32") {
      expect(fs.statSync(path.join(folder, "hided-windows-x86_64")).mode & 0o777).toBe(0o755);
      expect(fs.statSync(path.join(folder, "holder-conpty-windows-x86_64/conpty.dll")).mode & 0o777).toBe(0o644);
    }
  });

  it("refuses another platform's program, an unlisted file and a changed archive", () => {
    const files = kitFiles("linux-aarch64");
    const wrong = { ...files, hided: program(DEVICE_KITS["linux-x86_64"].header, "hided") };
    const header = release("linux-aarch64", wrong);
    expect(() => stageDeviceKit(header.manifest, header.archive, path.join(header.root, "out"))).toThrow("not a linux-aarch64 executable");
    const extra = release("linux-aarch64", { ...files, extra: Buffer.from("x") }, files);
    expect(() => stageDeviceKit(extra.manifest, extra.archive, path.join(extra.root, "out"))).toThrow("exactly");
    const changed = release("linux-aarch64", files);
    const bytes = fs.readFileSync(changed.archive);
    bytes[bytes.length - 1] ^= 0xff;
    fs.writeFileSync(changed.archive, bytes);
    expect(() => stageDeviceKit(changed.manifest, changed.archive, path.join(changed.root, "out"))).toThrow("digest mismatch");
  });

  it.skipIf(process.platform === "win32")("refuses a link in the archive", () => {
    const files = kitFiles("linux-x86_64");
    const linked = release("linux-x86_64", { ...files, hide: "link" }, files);
    expect(() => stageDeviceKit(linked.manifest, linked.archive, path.join(linked.root, "out"))).toThrow("not a regular file");
  });
});
