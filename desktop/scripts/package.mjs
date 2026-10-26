// Packages the release app for the system this runs on: the Electron host
// with the release daemon, the hide CLI, the hook helper, the
// pinned Herdr in its resources folder (the Electron release app PRD, D-03),
// wrapped in that system's installers and the files its updater reads
// (installers.mjs; PRD auto-update D-08, D-35), each installer beside a
// SHA-256 checksum, all in desktop/out/release.
//
// macOS builds hide.app, with the notification permission helper compiled
// into Contents/MacOS, either ad-hoc signed or signed with the Developer ID
// certificate and notarized when HIDE_MACOS_SIGNING asks for it
// (macos-signing.mjs), as a dmg and the zip the updater installs. Windows x64
// builds an unsigned per-user installer and Linux x64 an unsigned deb and rpm
// (operator decision 2026-10-03, PRD auto-update D-07: no paid signing outside
// macOS). Each is built on its own system, because the release binaries come from that
// system's cargo build. A Mac also consumes the three device helper artifacts
// built by device-helpers.yml at this exact source, version and build order.
//
// The CLI finds `hided` beside its own file and the daemon offers its own
// program to a device, which runs it as `hided node serve`, so all of them
// ship flat in the resources
// folder. A binary that is missing or cannot run stops the build here, by
// name, before any app exists (B12); nothing is substituted.

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { packager } from "@electron/packager";
import { applyFuses, assertFuses } from "./fuses.mjs";
import { APP_ID, buildInstallers } from "./installers.mjs";
import { assertNotarized, developerIdOptions, macosSigning, notarizeApp } from "./macos-signing.mjs";
import { orderOnMain, resolveVersion } from "./build-identity.mjs";
import { DEVICE_PLATFORMS, assertDeviceExecutable, deviceHelperResources } from "./device-helpers.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(root, "..");
const out = path.join(root, "out");
const resources = path.join(root, "resources");

/** Runs a tool in the repository; its stdout is the answer, unless `stdio` shows it instead. */
const run = (file, args, options = {}) => execFileSync(file, args, { cwd: repo, stdio: ["ignore", "pipe", "inherit"], encoding: "utf8", ...options })?.trim() ?? "";

// Windows starts pnpm's `.cmd` shim only through a shell; the arguments here
// are fixed words, so the shell has nothing to reinterpret.
const pnpm = (args) => run("pnpm", args, { stdio: "inherit", shell: process.platform === "win32" });

/**
 * What differs per system. `arch` is the one architecture the pinned Herdr
 * asset for that system serves (`contracts/herdr-bundle.json`), so another
 * machine is refused rather than given a Herdr it cannot run. `herdr` fetches
 * the verified asset and answers its executable and the files that must sit
 * beside it.
 */
const SYSTEMS = {
  darwin: {
    label: "macos",
    arch: "arm64",
    herdr: () => ({ executable: run("zsh", ["scripts/fetch-herdr-runtime.sh"]), companions: [] }),
  },
  win32: {
    label: "windows",
    arch: "x64",
    // The Windows asset is a zip: herdr.exe finds its ConPTY runtime in the
    // `conpty` folder beside it, and the zip carries that runtime's notices.
    // Every entry ships as upstream laid it out.
    herdr: () => {
      const executable = run("pwsh", ["-NoProfile", "-File", "scripts/fetch-herdr-runtime.ps1"]);
      const folder = path.dirname(executable);
      const companions = fs.readdirSync(folder).filter((entry) => entry !== path.basename(executable)).map((entry) => path.join(folder, entry));
      return { executable, companions };
    },
  },
  linux: {
    label: "linux",
    arch: "x64",
    herdr: () => ({ executable: run("zsh", ["scripts/fetch-herdr-runtime.sh", "--platform", "linux-x86_64"]), companions: [] }),
  },
};

const platform = process.platform;
const system = SYSTEMS[platform];
if (!system) throw new Error(`hide has no package for ${platform}`);
const arch = process.arch;
if (arch !== system.arch) throw new Error(`the pinned Herdr serves ${system.label} ${system.arch} only, and this machine is ${arch}`);
const exe = platform === "win32" ? ".exe" : "";
const version = resolveVersion();
// Read before the build, so a release missing a signing secret fails in
// seconds rather than after it.
const signing = platform === "darwin" ? macosSigning(process.env) : null;

// A failed run leaves no app or installer behind, stale or otherwise (B12).
const packaged = path.join(out, `hide-${platform}-${arch}`);
const release = path.join(out, "release");
fs.rmSync(packaged, { recursive: true, force: true });
fs.rmSync(release, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });

// Release hided embeds web/dist, so the web shell is built first; the cargo
// wrapper reuses the machine's toolchain and keeps output in this worktree.
pnpm(["--dir", "web", "build"]);
// The binaries report the version the app ships and the commit it was built
// from (`hide version`); a package that cannot name its commit is refused.
const commit = run("git", ["rev-parse", "HEAD"]);
if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error(`the package's commit could not be read (got '${commit}')`);
// Its place in main's history orders two builds of one release, so a newer
// screen updates its core (hided/src/build_order.rs): the first-parent count
// of the last commit it shares with main, so a branch's own commits never
// order it past a later main build. A shallow checkout cannot count it and
// names it in HIDE_BUILD_ORDER, or is refused.
const order = process.env.HIDE_BUILD_ORDER || orderOnMain();
if (!/^[0-9]+$/.test(order)) throw new Error(`the package's build order could not be read (got '${order}')`);
// Required artifacts are checked before the native release build. No partial
// Mac app may silently leave another supported system without its kit.
const deviceHelpers = platform === "darwin"
  ? deviceHelperResources(path.join(out, "device-helpers"), { version, commit, order })
  : [];
run("bash", ["scripts/verify-cargo.sh", "release"], { stdio: "inherit", env: { ...process.env, HIDE_VERSION: version, HIDE_COMMIT: commit, HIDE_BUILD_ORDER: order } });
const herdr = system.herdr();

const cargoRelease = path.join(repo, "target", "release");
const shipped = [
  [`hided${exe}`, path.join(cargoRelease, `hided${exe}`)],
  [`hide${exe}`, path.join(cargoRelease, `hide${exe}`)],
  [`hide-agent-hooks${exe}`, path.join(cargoRelease, `hide-agent-hooks${exe}`)],
  [`herdr${exe}`, herdr.executable],
  ...deviceHelpers,
];
if (platform === "darwin") {
  for (const devicePlatform of Object.keys(DEVICE_PLATFORMS)) {
    const source = run("zsh", ["scripts/fetch-herdr-runtime.sh", "--platform", devicePlatform]);
    assertDeviceExecutable(fs.readFileSync(source), devicePlatform);
    shipped.push([`herdr-${devicePlatform}`, source]);
  }
}
const missing = shipped.filter(([name, source]) => {
  try {
    // Downloaded workflow artifacts lose executable bits; staging restores
    // them. Native programs must already be runnable by this builder.
    fs.accessSync(source, name.includes("-macos-") || name.includes("-linux-") ? fs.constants.R_OK : fs.constants.X_OK);
    return !fs.statSync(source).isFile();
  } catch {
    return true;
  }
});
if (missing.length > 0) {
  throw new Error(`cannot package hide: not an executable file: ${missing.map(([name, source]) => `${name} (${source})`).join(", ")}`);
}

// Staged under out/ so a name inside the package can differ from the build's.
const staged = path.join(out, "resources");
fs.rmSync(staged, { recursive: true, force: true });
fs.mkdirSync(staged, { recursive: true });
const extraResource = shipped.map(([name, source]) => {
  const target = path.join(staged, name);
  fs.copyFileSync(source, target);
  fs.chmodSync(target, 0o755);
  return target;
});
for (const companion of herdr.companions) {
  const target = path.join(staged, path.basename(companion));
  fs.cpSync(companion, target, { recursive: true });
  extraResource.push(target);
}
const notices = path.join(resources, "THIRD_PARTY_NOTICES");
if (!fs.existsSync(notices)) throw new Error(`third-party notices are missing: ${notices}`);
extraResource.push(notices);
// The updater's own settings, which electron-builder writes only when it packs the app itself.
extraResource.push(path.join(resources, "app-update.yml"));

const APP_BUNDLE_ID = APP_ID;

/**
 * macOS's notification permission helper (PRD first-run-onboarding D-12):
 * one Swift file compiled for this machine's architecture, with the app's
 * identifier embedded as its Info.plist, since macOS keys notification
 * settings to it. A failed compile stops the package; nothing is shipped in
 * its place.
 */
function buildNotificationHelper() {
  const source = path.join(root, "native", "notification-status", "main.swift");
  const work = path.join(out, "notification-status");
  fs.rmSync(work, { recursive: true, force: true });
  fs.mkdirSync(work, { recursive: true });
  const plist = path.join(work, "Info.plist");
  fs.writeFileSync(
    plist,
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>${APP_BUNDLE_ID}</string>
  <key>CFBundleName</key>
  <string>hide-notification-status</string>
</dict>
</plist>
`,
  );
  const binary = path.join(work, "hide-notification-status");
  run("/usr/bin/xcrun", ["swiftc", "-O", source, "-target", `${system.arch}-apple-macosx11.0`, "-o", binary, "-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker", plist], { stdio: "inherit" });
  return binary;
}
const notificationHelper = platform === "darwin" ? buildNotificationHelper() : null;

const [outputDir] = await packager({
  dir: root,
  out,
  name: "hide",
  appBundleId: APP_BUNDLE_ID,
  appVersion: version,
  // macOS's CFBundleVersion and Windows' file version must be dotted
  // integers; the numeric prefix is that.
  buildVersion: /^[0-9.]+/.exec(version)[0],
  // No extension: the packager takes hide.icns on macOS and hide.ico on
  // Windows; a Linux executable carries no icon.
  icon: path.join(resources, "hide"),
  platform,
  arch,
  overwrite: true,
  // The packager's own default (native modules unpacked), plus the status page: with the shipped
  // fuses `loadFile` cannot read a file inside the archive (`src/main/statusPage.ts`).
  asar: { unpack: "**/{.**,**}/**/*.node", unpackDir: "dist/static" },
  prune: true,
  extraResource,
  // Only the bundles and the status page ship; sources, tests and tooling do not.
  ignore: (file) => file !== "" && file !== "/package.json" && !file.startsWith("/dist"),
  // The app reads its version from its own package.json (Electron's
  // `app.getVersion()`), and the updater compares releases with it, so the
  // copy that ships names the release rather than the workspace's 0.1.0.
  afterCopy: [
    async ({ buildPath }) => {
      const manifest = path.join(buildPath, "package.json");
      const metadata = JSON.parse(fs.readFileSync(manifest, "utf8"));
      fs.writeFileSync(manifest, `${JSON.stringify({ ...metadata, version }, null, 2)}\n`);
    },
  ],
  // The helper goes beside the app's executable, where the host looks for it
  // and where macOS resolves its bundle, before the packager signs anything.
  // It is signed under the app's identifier here, since macOS keys
  // notification settings to it, so it is already signed when a Developer ID
  // signature seals the bundle around it, and that signature replaces this one.
  afterCopyExtraResources: [
    // The fuses first, before anything signs the bundle: flipping one changes the binary.
    async ({ buildPath }) => {
      await applyFuses(platform === "darwin" ? path.join(buildPath, "hide.app") : path.join(buildPath, platform === "win32" ? "hide.exe" : "hide"), { adHocSign: platform === "darwin" });
    },
    ...(notificationHelper
    ? [
        async ({ buildPath }) => {
          const target = path.join(buildPath, "hide.app", "Contents", "MacOS", path.basename(notificationHelper));
          fs.copyFileSync(notificationHelper, target);
          fs.chmodSync(target, 0o755);
          run("/usr/bin/codesign", ["--force", "--sign", "-", "--identifier", APP_BUNDLE_ID, "--timestamp=none", target], { stdio: "inherit" });
        },
      ]
    : []),
  ],
  ...(signing?.mode === "developer-id"
    ? developerIdOptions(signing, shipped.map(([name]) => name), path.join(resources, "entitlements"), { [path.basename(notificationHelper)]: APP_BUNDLE_ID })
    : {}),
});

// The packager answers with the per-platform directory. On macOS the bundle
// is inside it and is what ships; elsewhere the directory itself ships, with
// the Electron executable at its top and the resources folder beside it.
const bundle = platform === "darwin" ? path.join(outputDir, "hide.app") : outputDir;
// What ships has the fuses, read back from the built app rather than trusted from the hook.
await assertFuses(platform === "darwin" ? bundle : path.join(bundle, platform === "win32" ? "hide.exe" : "hide"));
const bundledResources = platform === "darwin" ? path.join(bundle, "Contents", "Resources") : path.join(bundle, "resources");
for (const [name] of shipped) {
  const file = path.join(bundledResources, name);
  fs.accessSync(file, fs.constants.X_OK);
  if (!fs.statSync(file).isFile()) throw new Error(`${name} did not land as a file in ${bundledResources}`);
  if (name.includes("-macos-") || name.includes("-linux-")) console.log(`device_resource=${name} bytes=${fs.statSync(file).size}`);
}
for (const companion of herdr.companions) {
  const name = path.basename(companion);
  if (!fs.existsSync(path.join(bundledResources, name))) throw new Error(`Herdr's ${name} did not land in ${bundledResources}`);
}
if (platform === "darwin") {
  const helper = path.join(bundle, "Contents", "MacOS", path.basename(notificationHelper));
  if (!fs.statSync(helper, { throwIfNoEntry: false })?.isFile()) throw new Error(`the notification permission helper did not land at ${helper}`);
  // Linux ELFs are inert data here. Both Mac architectures are executable
  // code and must carry the same signature path as the native programs.
  const macPrograms = shipped.filter(([name]) => !name.includes("-linux-")).map(([name]) => path.join(bundledResources, name));
  if (signing.mode === "adhoc") {
    for (const file of macPrograms) run("/usr/bin/codesign", ["--force", "--sign", "-", "--timestamp=none", file], { stdio: "inherit" });
    run("/usr/bin/codesign", ["--force", "--deep", "--sign", "-", "--timestamp=none", bundle], { stdio: "inherit" });
  }
  for (const file of macPrograms) {
    run("/usr/bin/codesign", ["--verify", "--strict", file], { stdio: "inherit" });
    if (signing.mode === "developer-id") {
      const signature = spawnSync("/usr/bin/codesign", ["-dv", file], { encoding: "utf8" }).stderr;
      if (!signature.includes("Authority=Developer ID Application:") || !/flags=0x[0-9a-f]+\([^)]*runtime/.test(signature)) {
        throw new Error(`${path.basename(file)} needs a Developer ID hardened-runtime signature`);
      }
    }
  }
  run("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=2", bundle], { stdio: "inherit" });
  // Signing the bundle again must not have given the helper another identity.
  const signed = spawnSync("/usr/bin/codesign", ["-dv", helper], { encoding: "utf8" }).stderr;
  if (!signed.includes(`Identifier=${APP_BUNDLE_ID}\n`)) throw new Error(`the notification permission helper is not signed as ${APP_BUNDLE_ID}: ${signed.trim()}`);
  if (signing.mode === "developer-id" && !/flags=0x[0-9a-f]+\([^)]*runtime/.test(signed)) throw new Error(`the notification permission helper is not signed under the hardened runtime: ${signed.trim()}`);
  // Apple notarizes the checked bundle before any installer wraps it.
  if (signing.mode === "developer-id") notarizeApp(bundle, signing, out);
}

// The installers wrap the app as it stands; a Developer ID build's dmg is
// signed and notarized there too.
const installers = await buildInstallers({ app: bundle, version, out: release, signing });
// What a downloader of the update gets is the zip, so Gatekeeper judges the app expanded from it.
if (signing?.mode === "developer-id") {
  const zip = Object.values(installers).find((file) => file.endsWith(".zip"));
  try {
    assertNotarized(zip, out);
  } catch (error) {
    fs.rmSync(release, { recursive: true, force: true });
    throw error;
  }
}

console.log(`bundle=${bundle}`);
console.log(`hide_version=${version}`);
console.log(`release=${release}`);
for (const file of Object.values(installers)) console.log(`installer=${file}`);
