// Puts the phone web app inside the iOS app: the folder Capacitor serves from the app's own scheme.
//
// The app is one binary that has to work with every desktop version in use, and a desktop reports the phone
// protocol it speaks (docs/ARCHITECTURE.md, The mobile companion). So the folder carries the phone site's own
// structure: the loader at the root, the current protocol's bundle built from this source, and every older
// `p/<n>/` bundle the production phone site still serves, with `protocols.json` their union. Nothing is fetched
// when the app runs; the older bundles are fetched here, at build time, and each file is held to the size and
// SHA-256 pinned in `mobile/retained-bundles.json`, a file a person reviews and changes in a pull request
// (`pin-bundles.mjs` writes it). The site's own inventory (`phone-site.json`) is never believed: it is served by
// the same host as the files, so agreeing with it would prove only that the host agrees with itself. A file that
// differs, a pinned file or protocol the site no longer has, or a protocol the site serves that is not pinned
// stops the build and says which.
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { ORIGIN, contentPolicy, filePath, framePolicy, get, sha256 } from '../../scripts/phone-release.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
export const PINS = resolve(ROOT, 'mobile/retained-bundles.json');
const MAX_FILE = 16 * 1024 * 1024;
// Files of the browser build that do nothing inside the app: the app registers no service worker, and the file iOS reads
// from the hosted site to hand it a link is the site's, not the app's.
const NOT_IN_APP = new Set(['sw.js']);
const notInApp = (path) => NOT_IN_APP.has(path) || path === '.well-known' || path.startsWith('.well-known/');

// A read of a published file is the same read every time, so a network stumble is tried again, a few times.
const ATTEMPTS = 3;
export async function read(url, fetcher) {
  for (let attempt = 1; ; attempt += 1) {
    try { return await get(url, fetcher); }
    catch (error) { if (attempt === ATTEMPTS) throw error; }
  }
}

export async function json(url, fetcher) {
  return JSON.parse((await read(url, fetcher)).bytes);
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
 * The pinned bundles, checked for shape: `{schema: 1, protocols: {"<n>": {"p/<n>/<file>": {sha256, size}}}}`.
 * @returns {Map<number, Record<string, {sha256: string, size: number}>>}
 */
export function parsePins(value) {
  if (value?.schema !== 1 || !value.protocols || typeof value.protocols !== 'object' || Array.isArray(value.protocols)) {
    throw new Error('retained-bundles.json is not a schema 1 pin list');
  }
  const pins = new Map();
  for (const [key, files] of Object.entries(value.protocols)) {
    const number = Number(key);
    if (!/^[1-9][0-9]*$/.test(key) || !Number.isSafeInteger(number)) throw new Error(`Pinned protocol is not a number: ${key}`);
    if (!files || typeof files !== 'object' || !(`p/${number}/index.html` in files)) throw new Error(`Pinned protocol ${number} has no entry page`);
    for (const [path, entry] of Object.entries(files)) {
      filePath(path);
      if (!path.startsWith(`p/${number}/`)) throw new Error(`Pinned file is outside its protocol folder: ${path}`);
      if (!/^[a-f0-9]{64}$/.test(entry?.sha256) || !Number.isSafeInteger(entry.size) || entry.size < 1 || entry.size > MAX_FILE) {
        throw new Error(`Pinned file has no valid digest or size: ${path}`);
      }
    }
    pins.set(number, files);
  }
  return pins;
}

/** The one place the protocol this source speaks is read: `PHONE_PROTOCOL`, which the phone build and the pin generator both follow. */
export async function sourceProtocol(root = ROOT) {
  const source = await readFile(resolve(root, 'web/src/mobile/phoneVersion.ts'), 'utf8');
  const current = Number(source.match(/PHONE_PROTOCOL = (\d+)/)?.[1]);
  if (!Number.isSafeInteger(current) || current < 1) throw new Error('phone protocol missing');
  return current;
}

/**
 * @param {{dist: string, out: string, pins: ReturnType<typeof parsePins>, origin?: string, fetcher?: typeof fetch, expected?: number, simulatorRelayPort?: number}} options
 * @returns {Promise<number[]>} the protocols the folder carries
 */
export async function assemble({ dist, out, pins, expected, simulatorRelayPort, origin = ORIGIN, fetcher = fetch }) {
  if (simulatorRelayPort !== undefined && (!Number.isInteger(simulatorRelayPort) || simulatorRelayPort < 1024 || simulatorRelayPort > 65535)) throw new Error('Invalid simulator relay port');
  const built = JSON.parse(await readFile(resolve(dist, 'protocols.json'), 'utf8'));
  if (!Array.isArray(built) || built.length !== 1 || !Number.isSafeInteger(built[0]) || built[0] < 1) {
    throw new Error('The phone build must contain exactly its own protocol');
  }
  const [current] = built;
  // The build's own protocol is the source's: a phone-dist built from another revision is refused, not shipped.
  if (expected !== undefined && current !== expected) throw new Error(`web/phone-dist speaks protocol ${current} but this source speaks ${expected}; rebuild it`);
  // The current protocol is built from this source; a pin for it would let the site's copy stand in for the source.
  if (pins.has(current)) throw new Error(`Protocol ${current} is this source's own; remove it from retained-bundles.json`);

  // Only the list of numbers is read from the site, to refuse a retained protocol nobody pinned: shipping without it
  // would send that desktop's phones to the update screen with no failure anywhere. Its files are never trusted.
  const listed = await json(`${origin}/protocols.json`, fetcher);
  if (!Array.isArray(listed) || !listed.every((number) => Number.isSafeInteger(number) && number > 0)) throw new Error('The phone site does not list its protocols');
  const unpinned = listed.filter((number) => number !== current && !pins.has(number));
  if (unpinned.length) throw new Error(`The phone site serves protocol ${unpinned.join(', ')} that retained-bundles.json does not pin; run \`pnpm --dir mobile pin\` and review the change`);

  // A fresh folder: a file left by an earlier build must never ride along.
  await rm(out, { recursive: true, force: true });
  await mkdir(out, { recursive: true });
  await cp(dist, out, { recursive: true, filter: (source) => !notInApp(source.slice(dist.length + 1)) });

  for (const [number, files] of pins) {
    for (const [path, entry] of Object.entries(files)) {
      let bytes;
      try { ({ bytes } = await read(`${origin}/${path}`, fetcher)); }
      catch (error) { throw new Error(`Pinned file is gone from the phone site: ${path} (${error.message})`); }
      if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) throw new Error(`Retained phone file differs from its pin: ${path}`);
      await write(out, path, bytes);
    }
  }

  const protocols = [current, ...pins.keys()].sort((a, b) => a - b);
  await write(out, 'protocols.json', JSON.stringify(protocols));

  // Every page the app can open keeps the phone site's content policy: scripts from the app alone, connections to
  // the app and the production relay alone. The frame that draws agent-made HTML keeps its own.
  for (const page of await htmlFiles(out)) {
    const html = await readFile(resolve(out, page), 'utf8');
    if (/^p\/[1-9][0-9]*\/look-frame\.html$/.test(page)) framePolicy(html);
    else {
      const local = `connect-src 'self' ws://127.0.0.1:${simulatorRelayPort}`;
      if (simulatorRelayPort !== undefined && (page === 'index.html' || page.startsWith(`p/${current}/`)) && !html.includes(local)) {
        throw new Error(`Simulator page does not use the fixture relay: ${page}`);
      }
      contentPolicy(simulatorRelayPort === undefined ? html : html.replace(local, "connect-src 'self' wss://relay.withhide.dev"));
    }
  }
  for (const number of protocols) await readFile(resolve(out, `p/${number}/index.html`));
  return protocols;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const pins = parsePins(JSON.parse(await readFile(PINS, 'utf8')));
    const args = process.argv.slice(2);
    if (args.length && (args.length !== 2 || args[0] !== '--simulator-relay-port')) throw new Error('Unknown assembly argument');
    const protocols = await assemble({ dist: resolve(ROOT, 'web/phone-dist'), out: resolve(ROOT, 'mobile/www'), pins, expected: await sourceProtocol(), simulatorRelayPort: args.length ? Number(args[1]) : undefined });
    console.log(`phone bundles in the app: ${protocols.join(', ')}`);
  } catch (error) {
    console.error(`App bundle assembly stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
