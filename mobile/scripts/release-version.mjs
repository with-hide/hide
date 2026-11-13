// The version and build number a release build carries, decided once on the machine that starts the command.
//
// The marketing version is the app's own: the `version` of mobile/package.json, three numbers. It moves when the app
// is released to the App Store and not with the desktop release, because the phone app is a shell around a hosted
// phone page: a desktop release changes the phone only through the phone protocol (docs/RELAY_PROTOCOL.md), which
// needs no store release. The build number is the UTC time the run starts, `YYYYMMDDHHMMSS`: it rises with every
// run, a second run on the same commit takes a new one, and it is never written into the repository.
//
// The Android app (`release-version.mjs android`) has its own version, `mobile/android/version.json`, and its version
// code is the Unix minute the run starts: Play takes a whole number up to 2100000000, which the minute stays under until
// the year 5962, and refuses one that is not higher than every earlier upload's.
import { readFileSync } from 'node:fs';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const MOBILE = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const VERSION_FILES = { ios: resolve(MOBILE, 'package.json'), android: resolve(MOBILE, 'android', 'version.json') };

export function appVersion(manifest) {
  const version = manifest?.version;
  if (typeof version !== 'string' || !/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) {
    throw new Error(`the app's version is not three numbers: '${version}'`);
  }
  return version;
}

export function buildNumber(now = new Date()) {
  return now.toISOString().replace(/[-:T]/g, '').slice(0, 14);
}

export function versionCode(now = new Date()) {
  return String(Math.floor(now.getTime() / 60000));
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const platform = process.argv[2] ?? 'ios';
    if (!(platform in VERSION_FILES)) throw new Error('usage: release-version.mjs [ios | android]');
    const version = appVersion(JSON.parse(readFileSync(VERSION_FILES[platform], 'utf8')));
    console.log(`${version} ${platform === 'android' ? versionCode() : buildNumber()}`);
  } catch (error) {
    console.error(`Release version stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
