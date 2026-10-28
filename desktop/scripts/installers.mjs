// Wraps a packaged app in the installers a person downloads and the files the
// app's updater reads (PRD auto-update D-08, D-14, D-35), with electron-builder
// reading the app @electron/packager already made (`--prepackaged`), so the
// app, its bundled runtime and its macOS signature stay exactly what
// package.mjs built and checked:
//
//   macOS    hide-v<v>-macos-arm64.dmg (hide and an Applications link) and
//            the .zip the updater installs, with latest-mac.yml naming the
//            zip and the zip's blockmap. A Developer ID build also signs, notarizes and
//            staples the dmg (macos-signing.mjs).
//   Windows  hide-v<v>-windows-x64-setup.exe, a per-user NSIS installer into
//            %LOCALAPPDATA%\Programs\hide, with latest.yml and its blockmap.
//            Unsigned (D-07).
//   Linux    hide-v<v>-linux-x64.deb and .rpm into /opt/hide, with one
//            latest-linux.yml naming both. Unsigned (D-07).
//
// The update files are written here, not by electron-builder: a prepackaged
// macOS zip left none on the package lane, and Linux builds each type on its
// own, so one writer gives every system the same file from the same facts.
//
// Each installer gets a `.sha256` naming only it. `out` ends holding exactly
// the system's release files (`systemAssetNames`); anything else there is
// removed first, and a failure leaves no partial set behind.
//
// Usage, for a second build of an app already packaged (the CI update test):
//   node desktop/scripts/installers.mjs <packaged app> <version> <out>
// which first gives the app's own package.json that version, since the
// updater reads the running version from there. On Linux this test updates
// through the deb only, so it builds no test-only rpm.

import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createPackage, extractAll } from "@electron/asar";
import { Arch, build, Platform } from "electron-builder";
import { systemAssetNames, systemFiles } from "../../scripts/check-release-assets.mjs";
import { notarizeDiskImage } from "./macos-signing.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const resources = path.join(root, "resources");

/** The bundle identifier; Windows derives the uninstall entry's GUID from it, so it never changes once released. */
export const APP_ID = "me.grab.hide.desktop";

/** Where the update files say the files are; the app pins its own feed (`desktop/src/main/updater.ts`), so this only fills the field electron-builder requires. */
const FEED = "https://github.com/with-hide/hide/releases/latest/download";

const SYSTEM_OF = { darwin: "macos", win32: "windows", linux: "linux" };

/** The electron-builder configuration shared by every system; `version` names the release. */
function baseConfig(version, output) {
  return {
    appId: APP_ID,
    productName: "hide",
    executableName: "hide",
    // The packaged app already holds its own package.json; these fill what
    // the installers say about it. `name` is the Windows install folder.
    extraMetadata: {
      name: "hide",
      version,
      description: "Run more agents. Hold less.",
      homepage: "https://withhide.dev",
      author: { name: "Modakbul", email: "hello@withhide.dev" },
    },
    directories: { output, buildResources: resources },
    npmRebuild: false,
    publish: [{ provider: "generic", url: FEED }],
    generateUpdatesFilesForAllChannels: false,
  };
}

function sha256File(file) {
  return createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function sha512Base64(file) {
  return createHash("sha512").update(fs.readFileSync(file)).digest("base64");
}

/**
 * Gives the packaged app's own package.json `version`. The updater reads the
 * running version there (Electron's `app.getVersion()`), so a second build of
 * the same app for the update test must carry the newer one inside it.
 */
export async function setAppVersion(app, version) {
  const asar = process.platform === "darwin" ? path.join(app, "Contents", "Resources", "app.asar") : path.join(app, "resources", "app.asar");
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "hide-asar-"));
  try {
    extractAll(asar, scratch);
    const manifest = path.join(scratch, "package.json");
    const metadata = JSON.parse(fs.readFileSync(manifest, "utf8"));
    metadata.version = version;
    fs.writeFileSync(manifest, `${JSON.stringify(metadata, null, 2)}\n`);
    // Awaited inside the try: the archive reads the scratch files after this returns otherwise.
    await createPackage(scratch, asar);
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

async function buildMacos(app, version, work, signing) {
  await build({
    targets: Platform.MAC.createTarget(["dmg", "zip"], Arch.arm64),
    prepackaged: app,
    projectDir: root,
    publish: "never",
    config: {
      ...baseConfig(version, work),
      // package.mjs signed (and, for a release, notarized) the app; nothing here signs it again.
      mac: { identity: null, notarize: false, category: "public.app-category.developer-tools", artifactName: `hide-v${version}-macos-arm64.\${ext}` },
      // The dmg is for a first install; the updater reads the zip.
      dmg: { writeUpdateInfo: false, sign: false, title: "hide" },
    },
  });
  if (signing?.mode === "developer-id") notarizeDiskImage(path.join(work, `hide-v${version}-macos-arm64.dmg`), signing, work);
}

async function buildWindows(app, version, work) {
  await build({
    targets: Platform.WINDOWS.createTarget(["nsis"], Arch.x64),
    prepackaged: app,
    projectDir: root,
    publish: "never",
    config: {
      ...baseConfig(version, work),
      // Unsigned (D-07); the packager already set the executable's icon and version.
      win: { signAndEditExecutable: false, icon: path.join(resources, "hide.ico") },
      nsis: {
        oneClick: true,
        perMachine: false,
        artifactName: `hide-v${version}-windows-x64-setup.\${ext}`,
        installerIcon: path.join(resources, "hide.ico"),
        uninstallerIcon: path.join(resources, "hide.ico"),
        shortcutName: "hide",
        // Settings > Apps lists hide by name, with the version in its own column (B2).
        uninstallDisplayName: "${productName}",
        deleteAppDataOnUninstall: false,
        differentialPackage: true,
        include: path.join(resources, "windows", "installer.nsh"),
      },
    },
  });
}

/**
 * Opens the packaged app to every account, as a system package installs it:
 * folders 0755, files readable and, where the owner may run them, runnable.
 * @electron/packager builds the app in a private temporary folder (0700) and
 * renames it into place, so without this `/opt/hide` would be root's alone
 * and no one else could start the app or reach `/usr/bin/hide`.
 */
function openToEveryAccount(dir) {
  fs.chmodSync(dir, 0o755);
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) openToEveryAccount(file);
    else if (entry.isFile()) {
      const mode = fs.statSync(file).mode & 0o7777;
      fs.chmodSync(file, mode & 0o100 ? mode | 0o555 : mode | 0o444);
    }
  }
}

/**
 * One Linux package type. Each is built on its own: electron-builder writes
 * the type the updater reads (`resources/package-type`) into the app it wraps
 * just before packing, so two types built at once could pack each other's.
 */
async function buildLinux(app, version, work, type) {
  const scripts = { afterInstall: path.join(resources, "linux", "after-install.tpl"), afterRemove: path.join(resources, "linux", "after-remove.tpl") };
  await build({
    targets: Platform.LINUX.createTarget([type], Arch.x64),
    prepackaged: app,
    projectDir: root,
    publish: "never",
    config: {
      ...baseConfig(version, work),
      linux: {
        executableName: "hide",
        category: "Development",
        synopsis: "Run more agents. Hold less.",
        maintainer: "Modakbul <hello@withhide.dev>",
        vendor: "Modakbul",
        icon: path.join(resources, "hide.icns"),
        artifactName: `hide-v${version}-linux-x64.\${ext}`,
      },
      deb: scripts,
      rpm: scripts,
    },
  });
}

/**
 * The system's update file, naming each file its updater installs: the zip on
 * macOS, setup.exe on Windows, and both Linux packages, of which
 * electron-updater picks the type the app was installed as.
 */
function writeUpdateInfo(out, version, files) {
  const entries = files.updates.map((name) => ({ url: name, sha512: sha512Base64(path.join(out, name)), size: fs.statSync(path.join(out, name)).size }));
  const lines = [`version: ${version}`, "files:"];
  for (const entry of entries) lines.push(`  - url: ${entry.url}`, `    sha512: ${entry.sha512}`, `    size: ${entry.size}`);
  lines.push(`path: ${entries[0].url}`, `sha512: ${entries[0].sha512}`, `releaseDate: '${new Date().toISOString()}'`);
  fs.writeFileSync(path.join(out, files.updateInfo), `${lines.join("\n")}\n`);
}

/**
 * Builds this system's installers for the packaged `app` into `out` and
 * answers the paths of what landed there. `signing` is macos-signing.mjs's
 * choice, read only on macOS. `updateTest` limits Linux's throwaway update
 * feed to the deb; the default still builds and checks the full release set.
 */
export async function buildInstallers({ app, version, out, signing = null, updateTest = false }) {
  const system = SYSTEM_OF[process.platform];
  if (!system) throw new Error(`hide has no installers for ${process.platform}`);
  const files = systemFiles(version, system);
  if (updateTest && system === "linux") {
    files.installers = files.installers.filter((name) => name.endsWith(".deb"));
    files.updates = files.updates.filter((name) => name.endsWith(".deb"));
  }
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  const work = fs.mkdtempSync(path.join(path.dirname(out), "installers-"));
  try {
    const kept = [...files.installers, ...files.blockmaps];
    if (system === "macos") await buildMacos(app, version, work, signing);
    else if (system === "windows") await buildWindows(app, version, work);
    else {
      openToEveryAccount(app);
      for (const type of updateTest ? ["deb"] : ["deb", "rpm"]) await buildLinux(app, version, path.join(work, type), type);
    }
    for (const name of kept) {
      const source = system === "linux" ? path.join(work, name.endsWith(".rpm") ? "rpm" : "deb", name) : path.join(work, name);
      if (!fs.statSync(source, { throwIfNoEntry: false })?.isFile()) throw new Error(`electron-builder did not write ${name}`);
      fs.renameSync(source, path.join(out, name));
    }
    writeUpdateInfo(out, version, files);
    for (const name of files.installers) fs.writeFileSync(path.join(out, `${name}.sha256`), `${sha256File(path.join(out, name))}  ${name}\n`);
    const landed = fs.readdirSync(out).sort();
    const expected = updateTest && system === "linux"
      ? [...files.installers.flatMap((name) => [name, `${name}.sha256`]), files.updateInfo].sort()
      : systemAssetNames(version, system);
    if (JSON.stringify(landed) !== JSON.stringify(expected)) throw new Error(`installers differ from the release set: got [${landed.join(", ")}], expected [${expected.join(", ")}]`);
    return Object.fromEntries(files.installers.map((name) => [name, path.join(out, name)]));
  } catch (error) {
    fs.rmSync(out, { recursive: true, force: true });
    throw error;
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const [app, version, out] = process.argv.slice(2);
  if (!app || !version || !out || process.argv.length !== 5) throw new Error("usage: node desktop/scripts/installers.mjs <packaged app> <version> <out>");
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$/.test(version)) throw new Error(`not a version: ${version}`);
  // Rewriting a signed bundle's archive would break its signature, and a
  // macOS update needs a Developer ID build anyway (the PRD's stated risk).
  if (process.platform === "darwin") throw new Error("a second build of a packaged app is for the Windows and Linux update test only");
  await setAppVersion(path.resolve(app), version);
  const built = await buildInstallers({ app: path.resolve(app), version, out: path.resolve(out), updateTest: true });
  for (const file of Object.values(built)) console.log(`installer=${file}`);
}
