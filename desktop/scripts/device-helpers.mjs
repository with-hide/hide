// The artifact boundary between native helper builds and the device kits a
// release carries. Every platform, its programs, their headers and the
// manifest's format come from contracts/device-kits.json, which the core
// reads too (hide-node/src/ssh/kit/); this file names none of them. No app
// carries the manifest: a core fetches it from its own release
// (with-hide/hide-private#501 decision 1). A release candidate makes every
// kit whose producer is not the private publisher (`build` on each Linux
// runner, then `bundle` once, from those helpers and `compile (windows)`'s
// binaries) and writes no manifest; the publisher makes the Mac kit from the
// signed programs with `makeDeviceKit` and writes the release's manifest
// over all four with `writeDeviceManifest`.
// Inputs live in this checkout, never in a shared build directory.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { gunzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { checkReleaseBinaries } from "./release-binaries.mjs";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const contract = JSON.parse(fs.readFileSync(path.join(repo, "contracts/device-kits.json"), "utf8"));
// What a device runs: the helper, the CLI, the agent hooks and the pane holder
// that keeps its panes (PRD engine-bundle-3-core-owns D-22).
export const DEVICE_PROGRAMS = contract.programs;
export const DEVICE_BUNDLE_MANIFEST = contract.manifest;
export const DEVICE_BUNDLE_FORMAT = contract.format;
/** Every platform a release carries a kit for, as the contract names it. */
export const DEVICE_KITS = contract.kits;
const NODE_OS = { linux: "linux", macos: "darwin", windows: "win32" };
const NODE_ARCH = { x86_64: "x64", aarch64: "arm64" };
/** The kits this repository's `device-helpers` workflow builds, on runners of their own system. */
export const DEVICE_PLATFORMS = Object.fromEntries(Object.entries(contract.kits)
  .filter(([, kit]) => kit.producer === "device-helpers")
  .map(([platform, kit]) => [platform, { target: kit.target, os: NODE_OS[kit.os], arch: NODE_ARCH[kit.arch] }]));
const MAX_KIT = 256 * 1024 * 1024;

/** A kit's archive name in release `version`. */
export function deviceAsset(platform, version) {
  return contract.asset.replace("{platform}", platform).replace("{version}", version);
}

/**
 * A kit's files as the core reads them (hide-node/src/ssh/kit/contract.rs):
 * each program with the platform's suffix, then the runtime files beside
 * them, mapped to whether the file is a program.
 */
export function deviceKitFiles(platform) {
  const kit = contract.kits[platform];
  if (!kit) throw new Error(`no device kit contract for ${platform}`);
  return new Map([
    ...DEVICE_PROGRAMS.map((name) => [`${name}${kit.suffix}`, true]),
    ...Object.keys(kit.runtime).map((name) => [name, false]),
  ]);
}

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

/**
 * Foreign programs cannot be executed on the packager. Check their executable
 * headers as the core does (hide-node/src/ssh/kit/archive.rs): a 64-bit
 * little-endian ELF, a 64-bit Mach-O executable or a PE image, for the
 * contract's processor.
 */
export function assertDeviceExecutable(bytes, platform) {
  const header = contract.kits[platform]?.header;
  const u16 = (at) => (at + 2 <= bytes.length ? bytes.readUInt16LE(at) : -1);
  const u32 = (at) => (at + 4 <= bytes.length ? bytes.readUInt32LE(at) : -1);
  let matches = false;
  if (bytes.length >= 64 && header?.kind === "elf") {
    matches = bytes[0] === 0x7f && bytes.toString("latin1", 1, 4) === "ELF" && bytes[4] === 2 && bytes[5] === 1
      && [2, 3].includes(u16(16)) && u16(18) === header.machine;
  } else if (bytes.length >= 64 && header?.kind === "mach-o") {
    matches = u32(0) === 0xfeedfacf && u32(4) === header.cputype && u32(12) === 2;
  } else if (bytes.length >= 64 && header?.kind === "pe") {
    const offset = u32(0x3c);
    matches = bytes.toString("latin1", 0, 2) === "MZ" && offset >= 0
      && offset + 6 <= bytes.length && bytes.toString("latin1", offset, offset + 4) === "PE\0\0" && u16(offset + 4) === header.machine;
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
    if (Object.keys(manifest.files).sort().join(",") !== [...DEVICE_PROGRAMS].sort().join(",")) throw new Error(`incomplete device helper manifest for ${platform}`);
    return DEVICE_PROGRAMS.map((name) => {
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
    Object.fromEntries(DEVICE_PROGRAMS.map((name) => [name, digest(fs.readFileSync(resources.get(`${name}-${platform}`)))]))]));
}

/** A kit file's bytes, refused when it is a link, empty or above the kit cap; a program must have its platform's header. */
function readKitFile(file, platform, program) {
  if (program) return readProgram(file, platform);
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.size === 0 || stat.size > MAX_KIT) throw new Error(`invalid device kit file: ${file}`);
  return fs.readFileSync(file);
}

/**
 * Archives one kit from `folder`, which holds exactly the kit's files under
 * their kit names (deviceKitFiles), as `output/<asset>`, and answers its
 * manifest entry read back from the archive: `{ asset, sha256, size, files }`
 * with each file's sha256 and size. The bytes are taken as they are; a Mac
 * kit is the signed programs the Mac app ships, never signed again.
 */
export function makeDeviceKit(platform, folder, version, output) {
  const files = deviceKitFiles(platform);
  const digests = new Map([...files].map(([name, program]) => [name, digest(readKitFile(path.join(folder, ...name.split("/")), platform, program))]));
  const asset = deviceAsset(platform, version);
  fs.mkdirSync(output, { recursive: true });
  const archive = path.join(path.resolve(output), asset);
  fs.rmSync(archive, { force: true });
  // A deliberately small USTAR vocabulary: the kit's regular files under
  // relative `/` names, no folder entries, extended attributes, links or
  // platform-specific metadata records.
  execFileSync("tar", ["--format", "ustar", "-czf", archive, ...[...files.keys()].sort()], {
    cwd: folder,
    env: { ...process.env, COPYFILE_DISABLE: "1" },
  });
  const entry = readDeviceKit(platform, archive, version);
  for (const [name, sha256] of digests) {
    if (entry.files[name].sha256 !== sha256) throw new Error(`device file ${name} changed while ${asset} was made`);
  }
  return entry;
}

/**
 * The manifest entry of one kit's archive, computed from its bytes after the
 * core's checks (hide-node/src/ssh/kit/archive.rs): the release's name for
 * the platform, at most 256 MiB packed and unpacked, exactly the kit's files
 * as regular entries and every program with its platform's header.
 */
export function readDeviceKit(platform, archive, version) {
  const kit = contract.kits[platform];
  if (!kit) throw new Error(`no device kit contract for ${platform}`);
  const asset = deviceAsset(platform, version);
  if (path.basename(archive) !== asset) throw new Error(`${archive} is not the ${platform} kit of ${version} (${asset})`);
  const stat = fs.lstatSync(archive);
  if (!stat.isFile() || stat.size === 0 || stat.size > MAX_KIT) throw new Error(`${asset} is not an archive of at most 256 MiB`);
  const bytes = fs.readFileSync(archive);
  const entries = readUstar(gunzipSync(bytes, { maxOutputLength: MAX_KIT + 64 * 1024 }));
  const expected = deviceKitFiles(platform);
  if (JSON.stringify([...entries.keys()].sort()) !== JSON.stringify([...expected.keys()].sort())) {
    throw new Error(`${asset} does not hold exactly the ${platform} kit's files`);
  }
  let total = 0;
  const files = {};
  for (const name of [...expected.keys()].sort()) {
    const content = entries.get(name);
    if (content.length === 0) throw new Error(`device file ${name} in ${asset} is empty`);
    if (expected.get(name)) assertDeviceExecutable(content, platform);
    total += content.length;
    files[name] = { sha256: digest(content), size: content.length };
  }
  if (total > MAX_KIT) throw new Error(`${platform} device kit exceeds 256 MiB`);
  return { asset, sha256: digest(bytes), size: bytes.length, files };
}

/**
 * Writes `output/device-bundles.json` over `entries`, one per platform of the
 * contract, each the entry of an archive already in `output` with that size
 * and digest; answers its path. The private publisher writes a release's
 * manifest with it, over the candidate's kits and its own Mac kit.
 */
export function writeDeviceManifest(output, identity, entries) {
  assertIdentity(identity);
  if (JSON.stringify(Object.keys(entries).sort()) !== JSON.stringify(Object.keys(contract.kits).sort())) {
    throw new Error(`a device manifest names every kit of contracts/device-kits.json, not [${Object.keys(entries).join(", ")}]`);
  }
  const platforms = {};
  for (const platform of Object.keys(contract.kits).sort()) {
    const entry = entries[platform];
    if (entry.asset !== deviceAsset(platform, identity.version)) throw new Error(`unexpected device bundle name for ${platform}: ${entry.asset}`);
    const archive = path.join(output, entry.asset);
    const stat = fs.lstatSync(archive, { throwIfNoEntry: false });
    if (!stat?.isFile() || stat.size !== entry.size || digest(fs.readFileSync(archive)) !== entry.sha256) {
      throw new Error(`device bundle ${entry.asset} is missing or differs from its entry`);
    }
    platforms[platform] = { asset: entry.asset, sha256: entry.sha256, size: entry.size, files: entry.files };
  }
  const file = path.join(output, DEVICE_BUNDLE_MANIFEST);
  fs.writeFileSync(file, `${JSON.stringify({ format: DEVICE_BUNDLE_FORMAT, ...identity, platforms }, null, 2)}\n`);
  return file;
}

/** The kits a release candidate makes: every contract kit whose producer is not the private publisher. */
export const CANDIDATE_KITS = Object.keys(contract.kits).filter((platform) => contract.kits[platform].producer !== "publisher");

/**
 * Each producer's checked output in this checkout, as the contract's
 * `producer` names it: a kit's files by kit name, mapped to the file that
 * holds its bytes. `device-helpers` is a Linux helper artifact
 * (`<out>/device-helpers/<platform>/`), checked against its manifest;
 * `windows-compile` is `compile (windows)`'s artifact in
 * `<out>/windows-compile/`, the release binaries the Windows app ships
 * (`target/release/`) checked against their record
 * (`desktop/out/release-binaries.json`), with the pinned ConPTY runtime
 * `scripts/fetch-conpty.py --out <out>/windows-compile/holder-conpty` wrote.
 */
const PRODUCERS = {
  "device-helpers": (platform, identity, out) => {
    const helpers = new Map(deviceHelperResources(path.join(out, "device-helpers"), identity));
    return new Map(DEVICE_PROGRAMS.map((name) => [name, helpers.get(`${name}-${platform}`)]));
  },
  "windows-compile": (platform, identity, out) => {
    const input = path.join(out, "windows-compile");
    const { suffix, runtime } = contract.kits[platform];
    const binaries = path.join(input, "target", "release");
    const names = DEVICE_PROGRAMS.map((name) => `${name}${suffix}`);
    checkReleaseBinaries(path.join(input, "desktop", "out", "release-binaries.json"), binaries, names, identity);
    return new Map([
      ...names.map((name) => [name, path.join(binaries, name)]),
      ...Object.keys(runtime).map((name) => [name, path.join(input, ...name.split("/"))]),
    ]);
  },
};

/** One candidate kit's files by kit name, from its producer's checked output under `out`. */
export function candidateKitSources(platform, identity, out) {
  assertIdentity(identity);
  const producer = PRODUCERS[contract.kits[platform]?.producer];
  if (!producer || !CANDIDATE_KITS.includes(platform)) throw new Error(`a release candidate makes no ${platform} kit`);
  return producer(platform, identity, out);
}

/**
 * Makes every kit a release candidate produces into `output`, from the
 * producers' output under `out`, and answers their entries by platform. It
 * writes no manifest: the publisher writes the release's over these archives
 * as they are, beside its own Mac kit.
 */
export function makeCandidateKits(out, identity, output) {
  fs.rmSync(output, { recursive: true, force: true });
  fs.mkdirSync(output, { recursive: true });
  const entries = {};
  for (const platform of CANDIDATE_KITS) {
    const stage = path.join(output, `${platform}.stage`);
    for (const [name, source] of candidateKitSources(platform, identity, out)) {
      const target = path.join(stage, ...name.split("/"));
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.copyFileSync(source, target);
    }
    entries[platform] = makeDeviceKit(platform, stage, identity.version, output);
    fs.rmSync(stage, { recursive: true });
  }
  return entries;
}

/**
 * The regular files of a USTAR archive, by name. A link, a folder or any
 * other entry type, a name too long for a plain USTAR header and a duplicate
 * are refused: a kit's archive holds exactly its files (the core's rule,
 * hide-node/src/ssh/kit/archive.rs).
 */
function readUstar(bytes) {
  const files = new Map();
  const field = (block, start, length) => block.toString("latin1", start, start + length).replace(/\0.*$/s, "");
  for (let at = 0; at + 512 <= bytes.length;) {
    const block = bytes.subarray(at, at + 512);
    if (block.every((byte) => byte === 0)) return files;
    const prefix = field(block, 345, 155);
    const name = prefix ? `${prefix}/${field(block, 0, 100)}` : field(block, 0, 100);
    const size = Number.parseInt(field(block, 124, 12).trim() || "0", 8);
    const type = String.fromCharCode(block[156]);
    if (!["0", "\0"].includes(type)) throw new Error(`device archive entry ${name} is not a regular file`);
    if (files.has(name)) throw new Error(`device archive holds ${name} twice`);
    const start = at + 512;
    if (!Number.isSafeInteger(size) || start + size > bytes.length) throw new Error(`device archive entry ${name} is truncated`);
    files.set(name, bytes.subarray(start, start + size));
    at = start + Math.ceil(size / 512) * 512;
  }
  throw new Error("device archive has no end");
}

/**
 * Unpacks a release's or a CI run's kit archive into `folder` under the
 * developer names a core finds before any release (`<name>-<os>-<arch>`,
 * and `holder-conpty-<os>-<arch>/...` for the Windows runtime;
 * hide-node/src/ssh/kit/select.rs), after checking it as a core does
 * (readDeviceKit) and, given a release's `manifestFile`, against the entry
 * there. A local build then reaches a device of that platform without
 * cross-compiling. Answers the files written.
 */
export function stageDeviceKit(manifestFile, archiveFile, folder) {
  const manifest = JSON.parse(fs.readFileSync(manifestFile, "utf8"));
  if (manifest.format !== DEVICE_BUNDLE_FORMAT) throw new Error(`device manifest format ${manifest.format} is not ${DEVICE_BUNDLE_FORMAT}`);
  const found = Object.entries(manifest.platforms ?? {}).find(([, bundle]) => bundle.asset === path.basename(archiveFile));
  if (!found) throw new Error(`${manifestFile} names no kit ${path.basename(archiveFile)}`);
  const [platform, bundle] = found;
  const stat = fs.lstatSync(archiveFile);
  if (!stat.isFile() || stat.size !== bundle.size || digest(fs.readFileSync(archiveFile)) !== bundle.sha256) {
    throw new Error(`device archive digest mismatch: ${archiveFile}`);
  }
  const entry = readDeviceKit(platform, archiveFile, manifest.version);
  if (JSON.stringify(entry.files) !== JSON.stringify(Object.fromEntries(Object.keys(bundle.files ?? {}).sort().map((name) => [name, { sha256: bundle.files[name].sha256, size: bundle.files[name].size }])))) {
    throw new Error(`${archiveFile} does not hold exactly the ${platform} kit's files the manifest names`);
  }
  const kit = contract.kits[platform];
  const programs = new Map(DEVICE_PROGRAMS.map((name) => [`${name}${kit.suffix}`, name]));
  const files = readUstar(gunzipSync(fs.readFileSync(archiveFile), { maxOutputLength: MAX_KIT + 64 * 1024 }));
  const written = [];
  fs.mkdirSync(folder, { recursive: true });
  for (const [name, bytes] of files) {
    if (digest(bytes) !== entry.files[name].sha256) throw new Error(`device file ${name} differs from the manifest`);
    const [first, ...rest] = name.split("/");
    const developer = programs.has(name) ? `${programs.get(name)}-${platform}` : [`${first}-${platform}`, ...rest].join("/");
    const target = path.join(folder, ...developer.split("/"));
    const existing = fs.lstatSync(target, { throwIfNoEntry: false });
    if (existing && !existing.isFile()) throw new Error(`${target} is not a file`);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, bytes, { mode: 0o644 });
    fs.chmodSync(target, programs.has(name) || kit.runtime[name] ? 0o755 : 0o644);
    written.push(developer);
  }
  return written.sort();
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
  run("bash", ["scripts/verify-cargo.sh", "build", "--release", "--target", system.target, "-p", "hided", "--bins", "-p", "hide-agent-hooks", "--bin", "hide-agent-hooks", "-p", "hide-holder", "--bin", "hide-holder"], {
    env: { ...process.env, HIDE_VERSION: identity.version, HIDE_COMMIT: identity.commit, HIDE_BUILD_ORDER: identity.order },
  });
  fs.mkdirSync(folder, { recursive: true });
  const files = {};
  for (const name of DEVICE_PROGRAMS) {
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
    // Once per run, from every candidate producer's output under desktop/out.
    const [version, commit, order] = rest;
    const out = path.join(repo, "desktop/out");
    const entries = makeCandidateKits(out, { version: version.replace(/^v/, ""), commit, order }, path.join(out, "device-bundles"));
    for (const [platform, entry] of Object.entries(entries)) console.log(`device_kit ${platform} ${entry.asset} ${entry.sha256} ${entry.size}`);
  } else if (action === "stage" && rest.length === 3) {
    const [manifest, archive, folder] = rest;
    for (const name of stageDeviceKit(manifest, archive, folder)) console.log(path.join(folder, name));
  } else {
    throw new Error("usage: device-helpers.mjs build <platform> <version> <source-sha> <build-order> | bundle <version> <source-sha> <build-order> | stage <device-bundles.json> <archive> <folder>");
  }
}
