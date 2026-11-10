// Puts the phone web app inside the iOS app: the folder Capacitor serves from the app's own scheme.
//
// The app is one binary that has to work with every desktop version in use, and a desktop reports the phone
// protocol it speaks (docs/ARCHITECTURE.md, The mobile companion). So the folder carries the phone site's own
// structure: the loader at the root, the current protocol's bundle built from this source, and every older
// `p/<n>/` bundle the production phone site still serves, with `protocols.json` their union. Nothing is fetched
// when the app runs; the older bundles are fetched here, at build time, and each file is held to the size and
// SHA-256 the site's own inventory (`phone-site.json`) records. A file that differs or is missing stops the build.
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { ORIGIN, contentPolicy, filePath, framePolicy, get, inventory, sha256 } from '../../scripts/phone-release.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
// Files of the browser build that do nothing inside the app: the app registers no service worker.
const NOT_IN_APP = new Set(['sw.js']);

async function json(url, fetcher) {
  return JSON.parse((await get(url, fetcher)).bytes);
}

async function write(root, path, bytes) {
  const target = resolve(root, filePath(path));
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
}

async function htmlFiles(root, prefix = '') {
  const found = [];
  for (const entry of await readdir(resolve(root, prefix), { withFileTypes: true })) {
    const path = prefix + entry.name;
    if (entry.isDirectory()) found.push(...await htmlFiles(root, `${path}/`));
    else if (entry.name.endsWith('.html')) found.push(path);
  }
  return found;
}

/**
 * @param {{dist: string, out: string, origin?: string, fetcher?: typeof fetch}} options
 * @returns {Promise<number[]>} the protocols the folder carries
 */
export async function assemble({ dist, out, origin = ORIGIN, fetcher = fetch }) {
  const built = JSON.parse(await readFile(resolve(dist, 'protocols.json'), 'utf8'));
  if (!Array.isArray(built) || built.length !== 1 || !Number.isSafeInteger(built[0]) || built[0] < 1) {
    throw new Error('The phone build must contain exactly its own protocol');
  }
  const [current] = built;

  const site = await json(`${origin}/phone-site.json`, fetcher);
  const published = inventory(site);
  if (JSON.stringify(await json(`${origin}/protocols.json`, fetcher)) !== JSON.stringify(published)) {
    throw new Error('The phone site disagrees with itself about its protocols');
  }

  // A fresh folder: a file left by an earlier build must never ride along.
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });
  await cp(dist, out, { recursive: true, filter: (source) => !NOT_IN_APP.has(source.slice(dist.length + 1)) });

  const retained = published.filter((number) => number !== current);
  for (const number of retained) {
    for (const [path, entry] of Object.entries(site.files).filter(([path]) => path.startsWith(`p/${number}/`))) {
      const { bytes } = await get(`${origin}/${path}`, fetcher);
      if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) throw new Error(`Retained phone file differs from the site inventory: ${path}`);
      await write(out, path, bytes);
    }
  }

  const protocols = [current, ...retained].sort((a, b) => a - b);
  await write(out, 'protocols.json', JSON.stringify(protocols));

  // Every page the app can open keeps the phone site's content policy: scripts from the app alone, connections to
  // the app and the production relay alone. The frame that draws agent-made HTML keeps its own.
  for (const page of await htmlFiles(out)) {
    const html = await readFile(resolve(out, page), 'utf8');
    if (/^p\/[1-9][0-9]*\/look-frame\.html$/.test(page)) framePolicy(html);
    else contentPolicy(html);
  }
  for (const number of protocols) await readFile(resolve(out, `p/${number}/index.html`));
  return protocols;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const protocols = await assemble({ dist: resolve(ROOT, 'web/phone-dist'), out: resolve(ROOT, 'mobile/www') });
    console.log(`phone bundles in the app: ${protocols.join(', ')}`);
  } catch (error) {
    console.error(`App bundle assembly stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
