// The version and build number a release build carries, decided once on the machine that starts the command.
//
// The marketing version is the app's own: the `version` of mobile/package.json, three numbers. It moves when the app
// is released to the App Store and not with the desktop release, because the phone app is a shell around a hosted
// phone page: a desktop release changes the phone only through the phone protocol (docs/RELAY_PROTOCOL.md), which
// needs no store release. The build number is the UTC time the run starts, `YYYYMMDDHHMMSS`: it rises with every
// run, a second run on the same commit takes a new one, and it is never written into the repository.
import { readFileSync } from 'node:fs';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const MANIFEST = resolve(dirname(fileURLToPath(import.meta.url)), '..', 'package.json');

export function appVersion(manifest) {
  const version = manifest?.version;
  if (typeof version !== 'string' || !/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) {
    throw new Error(`mobile/package.json version is not three numbers: '${version}'`);
  }
  return version;
}

export function buildNumber(now = new Date()) {
  return now.toISOString().replace(/[-:T]/g, '').slice(0, 14);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    console.log(`${appVersion(JSON.parse(readFileSync(MANIFEST, 'utf8')))} ${buildNumber()}`);
  } catch (error) {
    console.error(`Release version stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
