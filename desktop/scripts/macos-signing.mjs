// How the macOS package is signed, chosen by HIDE_MACOS_SIGNING.
//
// `adhoc` gives the bundle an ad-hoc signature, as every pull-request and
// nightly package does, and a downloaded copy opens only after the Gatekeeper
// step docs/INSTALL.md gives. Unset, which is a local package, it means
// `local`: the bundle is signed with the one Developer ID Application identity
// a keychain holds, under the hardened runtime and without notarization, and
// with `adhoc` when there is none. An ad-hoc signature changes with every
// build, and macOS keys a privacy grant such as Screen Recording to the
// signature, so each install of an ad-hoc build asks for every grant again;
// a Developer ID signature is keyed to its team and keeps them. `developer-id` is what a
// release's package.yml run asks for: every executable in the bundle (the
// Electron helpers, the notification permission helper, the bundled CLIs and
// Herdr) is signed with the Developer ID certificate under the hardened
// runtime, Apple notarizes the app and its ticket is stapled (`notarizeApp`),
// so a downloaded copy opens like any other app; the dmg around it is signed,
// notarized and stapled the same way (`notarizeDiskImage`). Herdr ships with only the linker's ad-hoc
// signature and notarization refuses an executable without a Developer ID one,
// so hide's signature replaces it; Herdr's code is not changed.
//
// The keys below are the whole contract, read here and nowhere else. Every
// missing or malformed one is reported at once, before the build starts.

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

/** The keys `developer-id` needs, each with the test a usable value passes. */
const DEVELOPER_ID_KEYS = {
  // The certificate as `security find-identity -v -p codesigning` lists it:
  // its full name or its SHA-1. Its keychain must be on the search list.
  HIDE_SIGN_IDENTITY: (value) => /^Developer ID Application: .+ \([A-Z0-9]{10}\)$/.test(value) || /^[0-9A-F]{40}$/.test(value),
  // The App Store Connect API key notarytool submits with: the .p8 file,
  // its key ID and its issuer ID.
  HIDE_NOTARY_KEY_PATH: (value) => path.isAbsolute(value) && fs.statSync(value, { throwIfNoEntry: false })?.isFile() === true,
  HIDE_NOTARY_KEY_ID: (value) => /^[A-Z0-9]{10}$/.test(value),
  HIDE_NOTARY_ISSUER: (value) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value),
};

/**
 * Reads the signing choice from `env`: `{ mode: "adhoc" }`, a local package's
 * choice (`localSigning`), or the Developer ID identity and notary
 * credentials once the identity is found in a keychain codesign searches and
 * notarytool is installed. Anything else throws.
 */
export function macosSigning(env) {
  const mode = env.HIDE_MACOS_SIGNING ?? "local";
  if (mode === "adhoc") return { mode };
  if (mode === "local") return localSigning(codesigningIdentities(), env.HIDE_SIGN_IDENTITY);
  if (mode !== "developer-id") throw new Error(`HIDE_MACOS_SIGNING must be adhoc, local or developer-id (got '${mode}')`);
  const unusable = Object.entries(DEVELOPER_ID_KEYS)
    .filter(([key, usable]) => !env[key] || !usable(env[key]))
    .map(([key]) => key);
  if (unusable.length > 0) throw new Error(`HIDE_MACOS_SIGNING=developer-id needs these set and well formed: ${unusable.join(", ")}`);
  const identities = codesigningIdentities();
  if (!identities.includes(env.HIDE_SIGN_IDENTITY)) throw new Error(`no keychain on the search list holds the signing identity '${env.HIDE_SIGN_IDENTITY}'`);
  execFileSync("/usr/bin/xcrun", ["--find", "notarytool"], { stdio: "ignore" });
  return {
    mode,
    identity: env.HIDE_SIGN_IDENTITY,
    notary: { appleApiKey: env.HIDE_NOTARY_KEY_PATH, appleApiKeyId: env.HIDE_NOTARY_KEY_ID, appleApiIssuer: env.HIDE_NOTARY_ISSUER },
  };
}

/** What `security find-identity -v -p codesigning` lists: the valid identities of every keychain codesign searches. */
function codesigningIdentities() {
  return execFileSync("/usr/bin/security", ["find-identity", "-v", "-p", "codesigning"], { encoding: "utf8" });
}

/**
 * The signing of a local package, from `identities` as `security find-identity`
 * lists them: `{ mode: "local", identity }` for the Developer ID Application
 * identity named by `requested` (HIDE_SIGN_IDENTITY) or, unnamed, the only one
 * listed, and `{ mode: "adhoc" }` when none is listed. Two or more without a
 * name, or a name that is not listed, throw.
 */
export function localSigning(identities, requested) {
  const listed = new Map();
  for (const [, sha, name] of identities.matchAll(/^\s*\d+\) ([0-9A-F]{40}) "(Developer ID Application: [^"]+)"$/gm)) listed.set(sha, name);
  if (requested) {
    const found = [...listed].find(([sha, name]) => requested === sha || requested === name);
    if (!found) throw new Error(`no keychain on the search list holds the Developer ID Application identity '${requested}'`);
    return { mode: "local", identity: found[0], name: found[1] };
  }
  if (listed.size === 0) return { mode: "adhoc" };
  if (listed.size > 1) throw new Error(`the keychains hold ${listed.size} Developer ID Application identities (${[...listed.values()].join(", ")}); name one with HIDE_SIGN_IDENTITY, or set HIDE_MACOS_SIGNING=adhoc`);
  const [[sha, name]] = listed;
  return { mode: "local", identity: sha, name };
}

/**
 * The packager's `osxSign` options for a Developer ID package, local or
 * release; a release's signed app is notarized afterwards by `notarizeApp`. `executables` names the programs shipped flat in
 * Contents/Resources, `entitlements` is the folder holding the two
 * entitlement files, and `helpers` maps each program placed beside the app's
 * executable in Contents/MacOS to the code-signing identifier it is signed
 * under.
 */
export function developerIdOptions(signing, executables, entitlements, helpers = {}) {
  return {
    osxSign: {
      identity: signing.identity,
      // The packager only warns about a failed signature unless told
      // otherwise, and an unsigned app must not reach an archive.
      continueOnError: false,
      optionsForFile: (file) => {
        if (path.basename(path.dirname(file)) === "Resources" && executables.includes(path.basename(file))) {
          return { entitlements: path.join(entitlements, "executable.plist") };
        }
        if (path.basename(path.dirname(file)) === "MacOS" && Object.hasOwn(helpers, path.basename(file))) {
          return { entitlements: path.join(entitlements, "executable.plist"), additionalArguments: ["--identifier", helpers[path.basename(file)]] };
        }
        // Electron's helpers keep the Chromium entitlements @electron/osx-sign gives each.
        if (file.includes(".app/Contents/Frameworks/")) return null;
        return { entitlements: path.join(entitlements, "app.plist") };
      },
    },
  };
}

/** Submits `file` with notarytool, waits for Apple's answer and throws unless Apple accepted it. */
function submitForNotarization(file, signing, cwd) {
  const { appleApiKey, appleApiKeyId, appleApiIssuer } = signing.notary;
  // notarytool exits 0 for a rejected submission too; its status says which.
  const submitted = JSON.parse(execFileSync("/usr/bin/xcrun", ["notarytool", "submit", file, "--key", appleApiKey, "--key-id", appleApiKeyId, "--issuer", appleApiIssuer, "--wait", "--timeout", "60m", "--output-format", "json"], { encoding: "utf8", cwd }));
  if (submitted.status !== "Accepted") throw new Error(`Apple did not accept ${file} for notarization: ${submitted.status} (submission ${submitted.id}; read its log with notarytool log)`);
}

// stapler's exit codes when it cannot fetch a ticket Apple has issued: 65
// while the ticket's record is not served yet, 68 when the connection to
// Apple's ticket service fails. A GitHub macOS runner lost that connection on
// two release builds in a row on 2026-10-21, each after Apple had accepted the
// app. Any other code is a failure of the target itself.
const TICKET_NOT_FETCHED = new Set([65, 68]);
// The pause before each further try, in seconds: six tries over about five minutes.
const STAPLE_RETRY_WAITS = [10, 20, 40, 80, 160];

function runStapler(target) {
  const result = spawnSync("/usr/bin/xcrun", ["stapler", "staple", target], { encoding: "utf8" });
  process.stdout.write(`${result.stdout ?? ""}${result.stderr ?? ""}`);
  return { status: result.status };
}

const pauseSeconds = (seconds) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, seconds * 1000);

/**
 * Staples the ticket Apple issued for `target`. Call it only after Apple
 * accepted the submission: the ticket then exists, so stapler failing to
 * fetch it is tried again after each of STAPLE_RETRY_WAITS, and the last
 * failure throws. A test gives `run` (one stapler call) and `pause`.
 */
export function staple(target, { run = runStapler, pause = pauseSeconds } = {}) {
  for (let tries = 1; ; tries += 1) {
    const { status } = run(target);
    if (status === 0) return;
    if (!TICKET_NOT_FETCHED.has(status) || tries > STAPLE_RETRY_WAITS.length) {
      throw new Error(`stapler could not staple ${target}: exit ${status} after ${tries} ${tries === 1 ? "try" : "tries"}`);
    }
    console.warn(`stapler could not fetch the ticket for ${target} (exit ${status}); trying again in ${STAPLE_RETRY_WAITS[tries - 1]} s`);
    pause(STAPLE_RETRY_WAITS[tries - 1]);
  }
}

/**
 * Notarizes the signed `app` and staples its ticket, throwing on a rejection.
 * notarytool takes an archive, so the app is zipped into a new folder under
 * `scratch`, which is removed on every path.
 */
export function notarizeApp(app, signing, scratch) {
  const folder = fs.mkdtempSync(path.join(scratch, "notarize-"));
  try {
    const archive = path.join(folder, `${path.basename(app, ".app")}.zip`);
    execFileSync("/usr/bin/ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", app, archive]);
    submitForNotarization(archive, signing, folder);
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
  staple(app);
}

/**
 * Signs the disk image a person downloads with the Developer ID identity,
 * notarizes it, staples its ticket and throws unless Gatekeeper accepts it as
 * notarized, so a downloaded dmg opens without a warning (PRD auto-update
 * B1). The app inside was notarized before the image was made.
 */
export function notarizeDiskImage(dmg, signing, scratch) {
  execFileSync("/usr/bin/codesign", ["--force", "--sign", signing.identity, "--timestamp", dmg], { stdio: "inherit" });
  submitForNotarization(dmg, signing, scratch);
  staple(dmg);
  const assessment = spawnSync("/usr/sbin/spctl", ["--assess", "--type", "open", "--context", "context:primary-signature", "--verbose=4", dmg], { encoding: "utf8" });
  const answer = `${assessment.stdout}${assessment.stderr}`.trim();
  if (assessment.status !== 0 || !answer.includes("source=Notarized Developer ID")) {
    throw new Error(`Gatekeeper does not accept ${dmg} as notarized: ${answer || `spctl exited ${assessment.status}`}`);
  }
}

/**
 * Throws unless Gatekeeper accepts the app inside `archive` as notarized
 * Developer ID software, the way it judges a downloaded copy. Expands the
 * archive into a new folder under `scratch` and removes it on every path.
 */
export function assertNotarized(archive, scratch) {
  const folder = fs.mkdtempSync(path.join(scratch, "notarized-"));
  try {
    execFileSync("/usr/bin/ditto", ["-x", "-k", archive, folder]);
    const app = path.join(folder, "hide.app");
    execFileSync("/usr/bin/xcrun", ["stapler", "validate", app], { stdio: "inherit" });
    // spctl answers on stderr: "accepted" or "rejected", and the source it trusted.
    const assessment = spawnSync("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=4", app], { encoding: "utf8" });
    const answer = `${assessment.stdout}${assessment.stderr}`.trim();
    if (assessment.status !== 0 || !answer.includes("source=Notarized Developer ID")) {
      throw new Error(`Gatekeeper does not accept the app in ${archive} as notarized: ${answer || `spctl exited ${assessment.status}`}`);
    }
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
}
