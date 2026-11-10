// The Capacitor version is written in three places that must agree: this package, the plugin package
// `cap sync` manages, and the app's own native package. A mismatch builds two bridges into one app.
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pin = /capacitor-swift-pm\.git",\s*exact:\s*"([^"]+)"/;
const manifest = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
const found = {
  'package.json @capacitor/ios': manifest.dependencies['@capacitor/ios'],
  'package.json @capacitor/core': manifest.dependencies['@capacitor/core'],
  'package.json @capacitor/cli': manifest.dependencies['@capacitor/cli'],
  'CapApp-SPM': (await readFile(resolve(root, 'ios/App/CapApp-SPM/Package.swift'), 'utf8')).match(pin)?.[1],
  HideShell: (await readFile(resolve(root, 'ios/App/HideShell/Package.swift'), 'utf8')).match(pin)?.[1],
};
if (new Set(Object.values(found)).size !== 1 || Object.values(found).includes(undefined)) {
  console.error(`Capacitor versions disagree: ${JSON.stringify(found)}`);
  process.exitCode = 1;
}
