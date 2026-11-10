import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { assemble } from './assemble.mjs';

const POLICY = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' wss://relay.withhide.dev";
const page = (name) => `<!doctype html><head><meta http-equiv="Content-Security-Policy" content="${POLICY}"></head>${name}`;
const sha = (text) => createHash('sha256').update(text).digest('hex');

let root;
beforeEach(async () => { root = await mkdtemp(join(tmpdir(), 'assemble-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });

/** The phone site as it serves an older protocol 3 beside the current 5, with `lie` altering what a file's bytes are. */
function site({ lie = {} } = {}) {
  const files = { 'p/3/index.html': page('three'), 'p/3/assets/a-1.js': 'old script', 'p/5/index.html': page('five-published') };
  const inventory = { schema: 1, source: 'a'.repeat(40), files: Object.fromEntries(Object.entries(files).map(([path, text]) => [path, { sha256: sha(text), size: Buffer.byteLength(text) }])) };
  const served = { ...files, ...lie, 'phone-site.json': JSON.stringify(inventory), 'protocols.json': JSON.stringify([3, 5]) };
  return async (url) => {
    const path = new URL(url).pathname.slice(1);
    if (!(path in served)) return new Response('missing', { status: 404 });
    return new Response(Buffer.from(served[path]));
  };
}

async function build() {
  const dist = join(root, 'dist');
  await mkdir(join(dist, 'p/5'), { recursive: true });
  await writeFile(join(dist, 'index.html'), page('loader'));
  await writeFile(join(dist, 'sw.js'), 'worker');
  await writeFile(join(dist, 'p/5/index.html'), page('five-built'));
  await writeFile(join(dist, 'protocols.json'), '[5]');
  return dist;
}

describe('the folder the app serves', () => {
  it('holds this source\'s bundle, the retained older one, their union and no service worker', async () => {
    const out = join(root, 'www');
    const protocols = await assemble({ dist: await build(), out, fetcher: site() });
    expect(protocols).toEqual([3, 5]);
    expect(JSON.parse(await readFile(join(out, 'protocols.json'), 'utf8'))).toEqual([3, 5]);
    expect(await readFile(join(out, 'p/5/index.html'), 'utf8')).toContain('five-built');
    expect(await readFile(join(out, 'p/3/assets/a-1.js'), 'utf8')).toBe('old script');
    await expect(readFile(join(out, 'sw.js'))).rejects.toThrow();
  });

  it('stops when a retained file is not the bytes the site recorded', async () => {
    const out = join(root, 'www');
    await expect(assemble({ dist: await build(), out, fetcher: site({ lie: { 'p/3/assets/a-1.js': 'tampered!!!' } }) }))
      .rejects.toThrow(/differs from the site inventory: p\/3\/assets\/a-1\.js/);
  });

  it('stops when a retained page does not keep the phone site\'s content policy', async () => {
    const weak = '<!doctype html><head><meta http-equiv="Content-Security-Policy" content="default-src *"></head>';
    const out = join(root, 'www');
    await expect(assemble({ dist: await build(), out, fetcher: site({ lie: { 'p/3/index.html': weak } }) })).rejects.toThrow();
  });
});
