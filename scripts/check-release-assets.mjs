// File integrity and release inventory, not installation or native UI evidence.
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { DEVICE_PLATFORMS, DEVICE_PROGRAMS, DEVICE_BUNDLE_MANIFEST } from "../desktop/scripts/device-helpers.mjs";

/**
 * The tag is the one source of a release's kind: `vX.Y.Z` is a stable
 * release, published as the latest; `vX.Y.Z-rc.N` is a pre-release, never the
 * latest. Any other tag is refused.
 */
export function releaseKind(tag) {
  const match = typeof tag === "string" && /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-rc\.[1-9]\d*)?$/.exec(tag);
  if (!match) throw new Error("a release requires a vX.Y.Z or vX.Y.Z-rc.N tag without other suffixes");
  return { prerelease: match[4] !== undefined };
}

/** Each system's label in a file name, as `desktop/scripts/package.mjs` names the system it runs on. */
export const SYSTEMS = Object.freeze(["macos", "windows", "linux"]);

/**
 * What one system ships for `version` (PRD auto-update D-35): the installers
 * a person downloads, each with a `.sha256` naming only it; the update file
 * the app reads (`latest-mac.yml`, `latest.yml`, `latest-linux.yml`), which
 * names `updates`; and the blockmaps a differential download reads. The
 * macOS app updates from its zip, never the dmg; a Linux app updates from the
 * entry of the package type it was installed as.
 */
export function systemFiles(version, system) {
  const base = `hide-v${version}`;
  switch (system) {
    case "macos":
      return { installers: [`${base}-macos-arm64.dmg`, `${base}-macos-arm64.zip`], updates: [`${base}-macos-arm64.zip`], updateInfo: "latest-mac.yml", blockmaps: [`${base}-macos-arm64.zip.blockmap`] };
    case "windows":
      return { installers: [`${base}-windows-x64-setup.exe`], updates: [`${base}-windows-x64-setup.exe`], updateInfo: "latest.yml", blockmaps: [`${base}-windows-x64-setup.exe.blockmap`] };
    case "linux":
      return { installers: [`${base}-linux-x64.deb`, `${base}-linux-x64.rpm`], updates: [`${base}-linux-x64.deb`, `${base}-linux-x64.rpm`], updateInfo: "latest-linux.yml", blockmaps: [] };
    default:
      throw new Error(`hide ships no package for ${system}`);
  }
}

/** The install/update files buildInstallers writes before device kits are added, sorted. */
export function systemAssetNames(version, system) {
  const files = systemFiles(version, system);
  return [...files.installers.flatMap((name) => [name, `${name}.sha256`]), files.updateInfo, ...files.blockmaps].sort();
}

export function expectedAssetNames(tag) {
  releaseKind(tag);
  const version = tag.slice(1);
  return [
    ...SYSTEMS.flatMap((system) => systemAssetNames(version, system)),
    DEVICE_BUNDLE_MANIFEST,
    ...Object.keys(DEVICE_PLATFORMS).map((platform) => `hide-device-${platform}-${version}.tar.gz`),
  ].sort();
}

const UPDATE_INFO_LIMIT = 64 * 1024;

/**
 * The fields of an update file electron-updater reads, from the one shape
 * `desktop/scripts/installers.mjs` writes for every system: top-level
 * `key: value` lines and a `files:` list of `- key: value` items. Anything
 * else is refused rather than guessed at.
 */
export function parseUpdateInfo(text) {
  const scalar = (raw) => {
    const value = raw.trim();
    const quoted = /^'(.*)'$/.exec(value) ?? /^"(.*)"$/.exec(value);
    return quoted ? quoted[1] : value;
  };
  const info = { files: [] };
  let inFiles = false;
  let item = null;
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() === "") continue;
    const top = /^([A-Za-z0-9]+):(.*)$/.exec(line);
    if (top) {
      inFiles = top[1] === "files" && top[2].trim() === "";
      item = null;
      if (!inFiles) info[top[1]] = scalar(top[2]);
      continue;
    }
    const start = /^ {2}- ([A-Za-z0-9]+):(.*)$/.exec(line);
    const field = /^ {4}([A-Za-z0-9]+):(.*)$/.exec(line);
    if (inFiles && start) {
      item = { [start[1]]: scalar(start[2]) };
      info.files.push(item);
    } else if (inFiles && item && field) {
      item[field[1]] = scalar(field[2]);
    } else {
      throw new Error(`unexpected update file line: ${JSON.stringify(line.slice(0, 80))}`);
    }
  }
  return info;
}

/**
 * Checks one system's update file against the files beside it: it names the
 * release's version, lists every update file of that system once and no file
 * outside the system's installers, and each entry's sha512 and size are the
 * file's own, which is what electron-updater checks a download against.
 */
export async function verifyUpdateInfo(version, system, directory) {
  const files = systemFiles(version, system);
  const file = path.join(directory, files.updateInfo);
  const stat = await fs.lstat(file);
  if (!stat.isFile() || stat.size === 0 || stat.size > UPDATE_INFO_LIMIT) throw new Error(`${files.updateInfo} must be a nonempty file under 64 KiB`);
  const info = parseUpdateInfo(await fs.readFile(file, "utf8"));
  if (info.version !== version) throw new Error(`${files.updateInfo} names version ${info.version}, not ${version}`);
  const urls = info.files.map((entry) => entry.url);
  if (new Set(urls).size !== urls.length || urls.some((url) => !files.installers.includes(url))) {
    throw new Error(`${files.updateInfo} lists [${urls.join(", ")}], which are not ${system}'s installers`);
  }
  const absent = files.updates.filter((name) => !urls.includes(name));
  if (absent.length > 0) throw new Error(`${files.updateInfo} does not list ${absent.join(", ")}`);
  for (const entry of info.files) {
    const target = path.join(directory, entry.url);
    const size = (await fs.stat(target)).size;
    if (String(size) !== entry.size) throw new Error(`${files.updateInfo} gives ${entry.url} ${entry.size} bytes; the file has ${size}`);
    const hash = createHash("sha512");
    for await (const chunk of createReadStream(target)) hash.update(chunk);
    if (hash.digest("base64") !== entry.sha512) throw new Error(`${files.updateInfo}'s sha512 for ${entry.url} does not match the file`);
  }
  const first = info.files[0];
  if (info.path !== first.url || info.sha512 !== first.sha512) throw new Error(`${files.updateInfo}'s path and sha512 must name its first file`);
  return info;
}

/** Match the device download contract to the release assets before any upload. */
async function verifyDeviceBundles(version, directory, assets, sourceSha) {
  const manifest = JSON.parse(await fs.readFile(path.join(directory, DEVICE_BUNDLE_MANIFEST), "utf8"));
  const platforms = Object.keys(DEVICE_PLATFORMS).sort();
  const sameNames = (value, names) => value !== null && typeof value === "object" && !Array.isArray(value)
    && JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...names].sort());
  const digest = (value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
  const size = (value, minimum = 1) => Number.isSafeInteger(value) && value >= minimum && value <= 256 * 1024 * 1024;
  if (!sameNames(manifest, ["format", "version", "commit", "order", "platforms"])
    || manifest.format !== 1 || manifest.version !== version
    || typeof manifest.commit !== "string" || !/^[a-f0-9]{40}$/.test(manifest.commit)
    || (sourceSha !== undefined && manifest.commit !== sourceSha)
    || typeof manifest.order !== "string" || !/^[0-9]+$/.test(manifest.order)
    || !sameNames(manifest.platforms, platforms)) {
    throw new Error("device-bundles.json must identify this release, source and both Linux platforms");
  }
  for (const platform of platforms) {
    const bundle = manifest.platforms[platform];
    const asset = assets.find(({ name }) => name === `hide-device-${platform}-${version}.tar.gz`);
    if (!sameNames(bundle, ["asset", "sha256", "size", "files"])
      || bundle.asset !== asset.name || !size(bundle.size) || bundle.size !== asset.size
      || !digest(bundle.sha256) || `sha256:${bundle.sha256}` !== asset.digest
      || !sameNames(bundle.files, DEVICE_PROGRAMS)) {
      throw new Error(`${platform} device bundle differs from its manifest`);
    }
    let total = 0;
    for (const file of Object.values(bundle.files)) {
      if (!sameNames(file, ["size", "sha256"]) || !size(file.size, 64) || !digest(file.sha256)) {
        throw new Error(`${platform} device program has invalid size or SHA-256`);
      }
      total += file.size;
    }
    if (total > 256 * 1024 * 1024) throw new Error(`${platform} device kit exceeds 256 MiB`);
  }
}

export async function verifyAssets(tag, directory, { sourceSha } = {}) {
  const expected = expectedAssetNames(tag);
  const actual = (await fs.readdir(directory)).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    const missing = expected.filter((name) => !actual.includes(name));
    const unexpected = actual.filter((name) => !expected.includes(name));
    throw new Error(`release asset set differs: missing [${missing.join(", ")}]; unexpected [${unexpected.join(", ")}]`);
  }
  const manifest = [];
  for (const name of expected) {
    const file = path.join(directory, name);
    const stat = await fs.lstat(file);
    if (!stat.isFile() || stat.size === 0) throw new Error(`${name} must be a nonempty ordinary file`);
    if (name === DEVICE_BUNDLE_MANIFEST && stat.size > 16 * 1024) throw new Error(`${name} exceeds 16 KiB`);
    if (name.startsWith("hide-device-") && stat.size > 256 * 1024 * 1024) throw new Error(`${name} exceeds 256 MiB`);
    if (name.endsWith(".sha256") && stat.size > 512) throw new Error(`${name} exceeds the checksum size limit`);
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(file)) hash.update(chunk);
    manifest.push({ name, file, size: stat.size, digest: `sha256:${hash.digest("hex")}` });
  }
  for (const asset of manifest.filter(({ name }) => manifest.some((other) => other.name === `${name}.sha256`))) {
    const checksum = await fs.readFile(`${asset.file}.sha256`, "utf8");
    const match = /^([a-f0-9]{64})  ([^\r\n]+)\r?\n?$/.exec(checksum);
    if (!match || match[2] !== asset.name) throw new Error(`${asset.name}.sha256 must name only its exact file`);
    if (`sha256:${match[1]}` !== asset.digest) throw new Error(`${asset.name} does not match its SHA-256`);
  }
  for (const system of SYSTEMS) await verifyUpdateInfo(tag.slice(1), system, directory);
  await verifyDeviceBundles(tag.slice(1), directory, manifest, sourceSha);
  return manifest;
}

export function selectDraft(tag, pages) {
  const expected = expectedAssetNames(tag);
  if (!Array.isArray(pages) || pages.some((page) => !Array.isArray(page))) {
    throw new Error("release inventory must contain every page from the authenticated release list");
  }
  const releases = pages.flat();
  if (releases.some((release) => release === null || typeof release !== "object" || typeof release.tag_name !== "string")) {
    throw new Error("release inventory contains an invalid release record");
  }
  const matching = releases.filter((release) => release.tag_name === tag);
  if (matching.length > 1) throw new Error("multiple releases use this tag; review them before retrying");
  if (!matching.length) return null;
  const release = matching[0];
  if (release.draft !== true || release.prerelease !== releaseKind(tag).prerelease) {
    throw new Error("only an existing draft of this exact tag and its release kind may be used");
  }
  if (!Array.isArray(release.assets)) throw new Error("the existing draft has no asset inventory");
  const names = release.assets.map((asset) => asset?.name);
  if (new Set(names).size !== names.length || names.some((name) => !expected.includes(name))) {
    throw new Error("the existing draft contains duplicate or unexpected assets; review it before retrying");
  }
  return release;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [tag, directory, releasePagesFile] = process.argv.slice(2);
  if (!tag || !directory || process.argv.length > 5) throw new Error("usage: node scripts/check-release-assets.mjs <vX.Y.Z[-rc.N]> <directory> [release-pages.json]");
  const manifest = await verifyAssets(tag, directory);
  if (releasePagesFile) {
    if ((await fs.stat(releasePagesFile)).size > 16 * 1024 * 1024) throw new Error("release inventory exceeds 16 MiB");
    selectDraft(tag, JSON.parse(await fs.readFile(releasePagesFile, "utf8")));
  }
  console.log(`release assets verified: ${tag}, ${SYSTEMS.join(", ")}, ${manifest.length} files with matching checksums and update files`);
}
