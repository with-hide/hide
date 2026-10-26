// The artifact boundary between native helper builds and the Mac package.
// Inputs live in this checkout, never in a shared build directory.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { Buffer } from "node:buffer";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const HELPER_PROGRAMS = ["hided", "hide", "hide-agent-hooks"];
export const DEVICE_PLATFORMS = {
  "macos-x86_64": { target: "x86_64-apple-darwin", os: "darwin", arch: "arm64" },
  "linux-x86_64": { target: "x86_64-unknown-linux-gnu", os: "linux", arch: "x64" },
  "linux-aarch64": { target: "aarch64-unknown-linux-gnu", os: "linux", arch: "arm64" },
};

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** Foreign programs cannot be executed on the packager. Check their executable headers. */
export function assertDeviceExecutable(bytes, platform) {
  let matches = false;
  if (bytes.length >= 64 && platform === "macos-x86_64") {
    matches = bytes.readUInt32LE(0) === 0xfeedfacf && bytes.readUInt32LE(4) === 0x01000007 && bytes.readUInt32LE(12) === 2;
  } else if (bytes.length >= 64 && platform.startsWith("linux-")) {
    matches = bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) && bytes[4] === 2 && bytes[5] === 1
      && [2, 3].includes(bytes.readUInt16LE(16)) && bytes.readUInt16LE(18) === (platform === "linux-x86_64" ? 62 : 183);
  }
  if (!matches) throw new Error(`not a ${platform} executable`);
}

function readProgram(file, platform) {
  const stat = fs.lstatSync(file);
  // Refuse links and oversized input before reading it into the packager.
  if (!stat.isFile() || stat.size === 0 || stat.size > 256 * 1024 * 1024) throw new Error(`invalid device helper file: ${file}`);
  const bytes = fs.readFileSync(file);
  assertDeviceExecutable(bytes, platform);
  return bytes;
}

function assertIdentity(identity) {
  if (!/^[0-9a-f]{40}$/.test(identity.commit) || !/^[0-9]+$/.test(identity.order) || !/^[0-9][0-9A-Za-z.+-]*$/.test(identity.version)) {
    throw new Error("device helper identity needs a full source SHA, version and decimal build order");
  }
}

/** Validates all three complete artifacts before the Mac package starts its native build. */
export function deviceHelperResources(directory, identity) {
  assertIdentity(identity);
  return Object.keys(DEVICE_PLATFORMS).flatMap((platform) => {
    const folder = path.join(directory, platform);
    const manifest = JSON.parse(fs.readFileSync(path.join(folder, "manifest.json"), "utf8"));
    if (manifest.platform !== platform || manifest.commit !== identity.commit || manifest.version !== identity.version || manifest.order !== identity.order) {
      throw new Error(`device helper identity mismatch for ${platform}: rebuild all helpers for this source, version and order`);
    }
    if (Object.keys(manifest.files).sort().join(",") !== [...HELPER_PROGRAMS].sort().join(",")) throw new Error(`incomplete device helper manifest for ${platform}`);
    return HELPER_PROGRAMS.map((name) => {
      const source = path.join(folder, name);
      const bytes = readProgram(source, platform);
      if (digest(bytes) !== manifest.files[name]) throw new Error(`device helper digest mismatch: ${platform}/${name}`);
      return [`${name}-${platform}`, source];
    });
  });
}

/** Builds with the repository's release recipe, on Linux natively and on Mac for Intel. */
function build(platform, identity) {
  assertIdentity(identity);
  const system = DEVICE_PLATFORMS[platform];
  if (!system || process.platform !== system.os || process.arch !== system.arch) throw new Error(`build ${platform} on its workflow runner`);
  const run = (file, args, options = {}) => execFileSync(file, args, { cwd: repo, stdio: "inherit", ...options });
  const commit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repo, encoding: "utf8" }).trim();
  if (commit !== identity.commit) throw new Error("device helper source SHA differs from the checkout");
  const folder = path.join(repo, "desktop/out/device-helpers", platform);
  fs.rmSync(folder, { recursive: true, force: true });
  run("bash", ["scripts/verify-web.sh", "web", "build"]);
  run("bash", ["scripts/verify-cargo.sh", "build", "--release", "--target", system.target, "-p", "hided", "--bins", "-p", "hide-agent-hooks", "--bin", "hide-agent-hooks"], {
    env: { ...process.env, HIDE_VERSION: identity.version, HIDE_COMMIT: identity.commit, HIDE_BUILD_ORDER: identity.order },
  });
  fs.mkdirSync(folder, { recursive: true });
  const files = {};
  for (const name of HELPER_PROGRAMS) {
    const source = path.join(repo, "target", system.target, "release", name);
    files[name] = digest(readProgram(source, platform));
    fs.copyFileSync(source, path.join(folder, name));
    fs.chmodSync(path.join(folder, name), 0o755);
  }
  fs.writeFileSync(path.join(folder, "manifest.json"), `${JSON.stringify({ platform, ...identity, files }, null, 2)}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [action, platform, version, commit, order, ...extra] = process.argv.slice(2);
  if (action !== "build" || extra.length || !order) throw new Error("usage: device-helpers.mjs build <platform> <version> <source-sha> <build-order>");
  build(platform, { version: version.replace(/^v/, ""), commit, order });
}
