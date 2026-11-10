// Two facts about what is linked into the app, checked without Xcode so every pull request can run them.
//
// 1. The Capacitor version is written in three places that must agree: this package, the plugin package `cap sync`
//    manages, and the app's own native package. A mismatch builds two bridges into one app.
// 2. The app links Capacitor itself and no plugin. A plugin runs with the whole app's rights and is reachable from
//    every script the app serves, so adding one is a change to this list, made with `BridgeGate.appPlugins` in the
//    same pull request, never a side effect of an install.
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const LINKED_PACKAGES = ['@capacitor/cli', '@capacitor/core', '@capacitor/ios'];
export const LINKED_SWIFT_PRODUCTS = ['Capacitor', 'Cordova'];
const pin = /capacitor-swift-pm\.git",\s*exact:\s*"([^"]+)"/;

/** @returns {Promise<string[]>} what is wrong, empty when nothing is */
export async function check(root = ROOT) {
  const problems = [];
  const manifest = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
  const capApp = await readFile(resolve(root, 'ios/App/CapApp-SPM/Package.swift'), 'utf8');
  const shell = await readFile(resolve(root, 'ios/App/HideShell/Package.swift'), 'utf8');

  const found = {
    'package.json @capacitor/ios': manifest.dependencies['@capacitor/ios'],
    'package.json @capacitor/core': manifest.dependencies['@capacitor/core'],
    'package.json @capacitor/cli': manifest.dependencies['@capacitor/cli'],
    'CapApp-SPM': capApp.match(pin)?.[1],
    HideShell: shell.match(pin)?.[1],
  };
  if (new Set(Object.values(found)).size !== 1 || Object.values(found).includes(undefined)) problems.push(`Capacitor versions disagree: ${JSON.stringify(found)}`);

  const dependencies = Object.keys(manifest.dependencies).sort();
  if (JSON.stringify(dependencies) !== JSON.stringify(LINKED_PACKAGES)) problems.push(`mobile/package.json links ${dependencies.join(', ')}; the app links only ${LINKED_PACKAGES.join(', ')}`);

  for (const [name, text] of [['CapApp-SPM', capApp], ['HideShell', shell]]) {
    const packages = [...text.matchAll(/\.package\(url:\s*"([^"]+)"/g)].map((match) => match[1]);
    if (packages.length !== 1 || !packages[0].endsWith('/capacitor-swift-pm.git')) problems.push(`${name} depends on ${packages.join(', ') || 'nothing'}; the app depends on capacitor-swift-pm alone`);
    const products = [...new Set([...text.matchAll(/\.product\(name:\s*"([^"]+)"/g)].map((match) => match[1]))].sort();
    if (JSON.stringify(products) !== JSON.stringify(LINKED_SWIFT_PRODUCTS)) problems.push(`${name} links ${products.join(', ')}; the app links only ${LINKED_SWIFT_PRODUCTS.join(', ')}`);
  }
  return problems;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const problems = await check();
  for (const problem of problems) console.error(problem);
  if (problems.length) process.exitCode = 1;
}
