// The artifact boundary between native helper builds and the packages that
// carry the Linux device kit's manifest: the Mac app, and the Windows app,
// whose WSL distributions are Linux devices (PRD wsl-device D-04). One run
// makes the bundles once (`bundle` below) and both packages take those
// bytes, so the manifest each ships names the archives the release has.
// Inputs live in this checkout, never in a shared build directory.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { Buffer } from "node:buffer";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const HELPER_PROGRAMS = ["hided", "hide", "hide-agent-hooks"];
export const DEVICE_PROGRAMS = [...HELPER_PROGRAMS, "herdr"];
export const DEVICE_BUNDLE_MANIFEST = "device-bundles.json";
export const DEVICE_PLATFORMS = {
  "linux-x86_64": { target: "x86_64-unknown-linux-gnu", os: "linux", arch: "x64" },
  "linux-aarch64": { target: "aarch64-unknown-linux-gnu", os: "linux", arch: "arm64" },
};

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

/** Foreign programs cannot be executed on the packager. Check their executable headers. */
export function assertDeviceExecutable(bytes, platform) {
  let matches = false;
  if (bytes.length >= 64 && Object.hasOwn(DEVICE_PLATFORMS, platform)) {
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

/** Validates both complete artifacts before the Mac package assembles its app. */
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

/** Each platform's helper digests, read after the same checks, for the release candidate's receipt. */
export function deviceHelperDigests(directory, identity) {
  const resources = new Map(deviceHelperResources(directory, identity));
  return Object.fromEntries(Object.keys(DEVICE_PLATFORMS).map((platform) => [platform,
    Object.fromEntries(HELPER_PROGRAMS.map((name) => [name, digest(fs.readFileSync(resources.get(`${name}-${platform}`)))]))]));
}

/** Prepare release assets outside the app; only the resulting manifest ships inside it. */
export function prepareDeviceBundles(directory, identity, output, herdrPaths) {
  const helpers = new Map(deviceHelperResources(directory, identity));
  fs.rmSync(output, { recursive: true, force: true });
  fs.mkdirSync(output, { recursive: true });
  const platforms = {};
  for (const platform of Object.keys(DEVICE_PLATFORMS)) {
    const stage = path.join(output, platform);
    fs.mkdirSync(stage);
    const files = {};
    for (const name of DEVICE_PROGRAMS) {
      const source = name === "herdr" ? herdrPaths[platform] : helpers.get(`${name}-${platform}`);
      readProgram(source, platform);
      const target = path.join(stage, name);
      fs.copyFileSync(source, target);
      fs.chmodSync(target, 0o755);
      const bytes = readProgram(target, platform);
      files[name] = { sha256: digest(bytes), size: bytes.length };
    }
    const asset = `hide-device-${platform}-${identity.version}.tar.gz`;
    const archive = path.join(output, asset);
    // A deliberately small USTAR vocabulary: four regular files, no paths,
    // extended attributes, links or platform-specific metadata records.
    execFileSync("tar", ["--format", "ustar", "-czf", asset, "-C", platform, ...DEVICE_PROGRAMS], {
      cwd: output,
      env: { ...process.env, COPYFILE_DISABLE: "1" },
    });
    const bytes = fs.readFileSync(archive);
    const size = Object.values(files).reduce((sum, file) => sum + file.size, 0);
    if (size > 256 * 1024 * 1024 || bytes.length > 256 * 1024 * 1024) throw new Error(`${platform} device bundle exceeds 256 MiB`);
    platforms[platform] = { asset, sha256: digest(bytes), size: bytes.length, files };
    fs.rmSync(stage, { recursive: true });
  }
  const manifest = path.join(output, DEVICE_BUNDLE_MANIFEST);
  fs.writeFileSync(manifest, `${JSON.stringify({ format: 1, ...identity, platforms }, null, 2)}\n`);
  return manifest;
}

/**
 * The bundles one run already made in `output`, checked before a package
 * ships their manifest: made for this source, version and order, for every
 * Linux platform, each archive there with the size and digest the manifest
 * names. Answers the manifest's path; refuses anything else.
 */
export function readDeviceBundles(output, identity) {
  assertIdentity(identity);
  const file = path.join(output, DEVICE_BUNDLE_MANIFEST);
  const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
  if (manifest.format !== 1 || manifest.commit !== identity.commit || manifest.version !== identity.version || manifest.order !== identity.order) {
    throw new Error("device bundle identity mismatch: make the bundles for this source, version and order");
  }
  if (Object.keys(manifest.platforms).sort().join(",") !== Object.keys(DEVICE_PLATFORMS).sort().join(",")) throw new Error("device bundles do not cover every Linux platform");
  for (const [platform, bundle] of Object.entries(manifest.platforms)) {
    if (bundle.asset !== `hide-device-${platform}-${identity.version}.tar.gz`) throw new Error(`unexpected device bundle name for ${platform}: ${bundle.asset}`);
    const archive = path.join(output, bundle.asset);
    const stat = fs.lstatSync(archive);
    if (!stat.isFile() || stat.size !== bundle.size) throw new Error(`device bundle ${bundle.asset} is missing or another size`);
    if (digest(fs.readFileSync(archive)) !== bundle.sha256) throw new Error(`device bundle digest mismatch: ${bundle.asset}`);
  }
  return file;
}

/** The pinned Herdr for each Linux platform, fetched and verified by the repository's fetch script. */
export function fetchDeviceHerdr() {
  return Object.fromEntries(Object.keys(DEVICE_PLATFORMS).map((name) => [name,
    execFileSync("zsh", ["scripts/fetch-herdr-runtime.sh", "--platform", name], { cwd: repo, encoding: "utf8" }).trim()]));
}

/** Builds with the repository's release recipe, natively on each Linux processor. */
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
  const [action, ...rest] = process.argv.slice(2);
  if (action === "build" && rest.length === 4) {
    const [platform, version, commit, order] = rest;
    build(platform, { version: version.replace(/^v/, ""), commit, order });
  } else if (action === "bundle" && rest.length === 3) {
    // Once per run, from both platforms' helpers in desktop/out/device-helpers.
    const [version, commit, order] = rest;
    const out = path.join(repo, "desktop/out");
    const manifest = prepareDeviceBundles(path.join(out, "device-helpers"), { version: version.replace(/^v/, ""), commit, order }, path.join(out, "device-bundles"), fetchDeviceHerdr());
    console.log(`device_manifest=${manifest}`);
  } else {
    throw new Error("usage: device-helpers.mjs build <platform> <version> <source-sha> <build-order> | bundle <version> <source-sha> <build-order>");
  }
}
