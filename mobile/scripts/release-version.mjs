// The version and build number a release build carries, decided once on the machine that starts the command.
//
// The marketing version is the numeric part of the repository's own version: `HIDE_VERSION` or the newest `v<version>`
// tag, the way desktop/scripts/package.mjs names a build (`0.4.0-rc.1` and `0.4.0-main.<commit>` are both `0.4.0`,
// as the macOS bundle version is). The build number is the UTC time the run starts, `YYYYMMDDHHMMSS`: it rises with
// every run, a second run on the same commit takes a new one, and it is never written into the repository.
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';
import { resolveVersion } from '../../desktop/scripts/build-identity.mjs';

export function marketingVersion(full) {
  const numeric = /^([0-9]+\.[0-9]+\.[0-9]+)(?:[-+].*)?$/.exec(full)?.[1];
  if (!numeric) throw new Error(`Not a version with three numbers: '${full}'`);
  return numeric;
}

export function buildNumber(now = new Date()) {
  return now.toISOString().replace(/[-:T]/g, '').slice(0, 14);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    console.log(`${marketingVersion(resolveVersion())} ${buildNumber()}`);
  } catch (error) {
    console.error(`Release version stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
