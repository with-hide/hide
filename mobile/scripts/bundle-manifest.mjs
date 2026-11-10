// The list of files the app's `public/` folder may hold, with their digests, made after `cap sync` and checked
// again on the signed app (check-release-app.sh): the app that ships is the app that was assembled.
//
// `assemble.mjs` fills `mobile/www/` and holds every retained file to `retained-bundles.json`; `cap sync` copies that
// folder into the Xcode project and adds the two Cordova files Capacitor always writes. This script reads the copy
// the build compiles, requires it to be that folder plus exactly those two files, requires every pinned file in it to
// match its pin, and writes `{schema: 1, files: {"<path>": "<sha256>"}}`.
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { sha256 } from '../../scripts/phone-release.mjs';
import { PINS, parsePins } from './assemble.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
/** What `cap sync` adds to a copy of the web folder, whatever the web folder holds. */
export const ADDED_BY_CAPACITOR = ['cordova.js', 'cordova_plugins.js'];

async function files(root, prefix = '') {
  const found = [];
  for (const entry of await readdir(resolve(root, prefix), { withFileTypes: true })) {
    const path = prefix + entry.name;
    if (entry.isDirectory()) found.push(...await files(root, `${path}/`));
    else if (entry.isFile()) found.push(path);
    else throw new Error(`Not a regular file: ${path}`);
  }
  return found.sort();
}

async function digests(root) {
  const map = new Map();
  for (const path of await files(root)) map.set(path, sha256(await readFile(resolve(root, path))));
  return map;
}

/**
 * @param {{www: string, publicDir: string, pins: ReturnType<typeof parsePins>}} options
 * @returns {Promise<{schema: 1, files: Record<string, string>}>}
 */
export async function buildManifest({ www, publicDir, pins }) {
  const assembled = await digests(www);
  const copied = await digests(publicDir);
  const problems = [];
  for (const [path, digest] of assembled) {
    if (!copied.has(path)) problems.push(`missing from the app's public/: ${path}`);
    else if (copied.get(path) !== digest) problems.push(`differs from the assembled folder: ${path}`);
  }
  for (const path of copied.keys()) {
    if (!assembled.has(path) && !ADDED_BY_CAPACITOR.includes(path)) problems.push(`in the app's public/ but never assembled: ${path}`);
  }
  for (const files of pins.values()) {
    for (const [path, entry] of Object.entries(files)) {
      if (copied.get(path) !== entry.sha256) problems.push(`pinned file is not in the app as pinned: ${path}`);
    }
  }
  if (problems.length) throw new Error(`The app's web bundle is not the assembled one:\n${problems.map((line) => `  ${line}`).join('\n')}`);
  return { schema: 1, files: Object.fromEntries(copied) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const out = process.argv[2];
  try {
    if (!out) throw new Error('usage: bundle-manifest.mjs <manifest.json>');
    const pins = parsePins(JSON.parse(await readFile(PINS, 'utf8')));
    const manifest = await buildManifest({ www: resolve(ROOT, 'mobile/www'), publicDir: resolve(ROOT, 'mobile/ios/App/App/public'), pins });
    await writeFile(out, `${JSON.stringify(manifest, null, 1)}\n`);
    console.log(`bundle manifest: ${Object.keys(manifest.files).length} files`);
  } catch (error) {
    console.error(`Bundle manifest stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
