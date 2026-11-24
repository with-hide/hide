// The Mac app's identity as macOS's consent knows it, read from
// contracts/app-identity.json: the one statement of it that the package
// (this folder) and the programs (hide-kit's `app_identity`) both read.
//
// The core a login item runs and the pane holders it starts run outside the
// app bundle. They answer macOS's consent under the app's name, which they
// can only when the identity file beside them says what the app's Info.plist
// says and they are signed so that they meet the requirement the app's
// consent is kept under. So the package signs them as the app and reads
// both back (docs/ARCHITECTURE.md, The platform layer).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const file = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "contracts", "app-identity.json");

/** `{ identifier, name, requirement, usage }` as the contract states them. */
export const APP_IDENTITY = JSON.parse(fs.readFileSync(file, "utf8"));

/** The programs shipped in Contents/Resources that answer consent as the app when they run outside it. */
export const SIGNED_AS_APP = ["hided", "hide-holder"];

const isUsage = (key) => /^NS.*UsageDescription$/.test(key);

/**
 * How an app's Info.plist (its keys and values as an object) differs from
 * the contract, one line per difference; none when the identity file a
 * program writes from the contract says what the app says. A sentence the
 * app has and the file lacks would make macOS refuse that access outside the
 * app without asking.
 */
export function identityDrift(info, identity = APP_IDENTITY) {
  const drift = [];
  if (info.CFBundleIdentifier !== identity.identifier) drift.push(`CFBundleIdentifier is ${JSON.stringify(info.CFBundleIdentifier)}, the contract says ${JSON.stringify(identity.identifier)}`);
  if (info.CFBundleName !== identity.name) drift.push(`CFBundleName is ${JSON.stringify(info.CFBundleName)}, the contract says ${JSON.stringify(identity.name)}`);
  const keys = new Set([...Object.keys(info).filter(isUsage), ...Object.keys(identity.usage)]);
  for (const key of [...keys].sort()) {
    if (info[key] !== identity.usage[key]) drift.push(`${key} is ${JSON.stringify(info[key])}, the contract says ${JSON.stringify(identity.usage[key])}`);
  }
  return drift;
}
