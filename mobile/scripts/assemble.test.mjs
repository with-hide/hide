import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { assemble, parsePins } from './assemble.mjs';
import { pin } from './pin-bundles.mjs';

const POLICY = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' wss://relay.withhide.dev";
const page = (name, policy = POLICY) => `<!doctype html><head><meta http-equiv="Content-Security-Policy" content="${policy}"></head>${name}`;
const sha = (text) => createHash('sha256').update(text).digest('hex');
const entries = (files) => Object.fromEntries(Object.entries(files).map(([path, text]) => [path, { sha256: sha(text), size: Buffer.byteLength(text) }]));

const OLD = { 'p/3/index.html': page('three'), 'p/3/assets/a-1.js': 'old script' };
const pins = () => parsePins({ schema: 1, protocols: { 3: entries(OLD) } });

let root;
beforeEach(async () => { root = await mkdtemp(join(tmpdir(), 'assemble-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });

/**
 * The phone site: `files` it serves, `protocols` it lists, and, as `inventory`, what its own phone-site.json says,
 * which the assembler must never believe.
 */
function site({ files = { ...OLD, 'p/5/index.html': page('five-published') }, protocols = [3, 5], inventory } = {}) {
  const served = { ...files, 'protocols.json': JSON.stringify(protocols), 'phone-site.json': JSON.stringify(inventory ?? { schema: 1, source: 'a'.repeat(40), files: entries(files) }) };
  return async (url) => {
    const path = new URL(url).pathname.slice(1);
    return path in served ? new Response(Buffer.from(served[path])) : new Response('missing', { status: 404 });
  };
}

async function build() {
  const dist = join(root, 'dist');
  await mkdir(join(dist, 'p/5'), { recursive: true });
  await writeFile(join(dist, 'index.html'), page('loader'));
  await writeFile(join(dist, 'sw.js'), 'worker');
  await mkdir(join(dist, '.well-known'), { recursive: true });
  await writeFile(join(dist, '.well-known/apple-app-site-association'), '{}');
  await writeFile(join(dist, 'p/5/index.html'), page('five-built'));
  await writeFile(join(dist, 'protocols.json'), '[5]');
  return dist;
}

const run = async (fetcher, pinned = pins()) => assemble({ dist: await build(), out: join(root, 'www'), pins: pinned, fetcher });

describe('the folder the app serves', () => {
  it('holds this source\'s bundle, the pinned older one, their union and no service worker', async () => {
    expect(await run(site())).toEqual([3, 5]);
    const out = join(root, 'www');
    expect(JSON.parse(await readFile(join(out, 'protocols.json'), 'utf8'))).toEqual([3, 5]);
    expect(await readFile(join(out, 'p/5/index.html'), 'utf8')).toContain('five-built');
    expect(await readFile(join(out, 'p/3/assets/a-1.js'), 'utf8')).toBe('old script');
    await expect(readFile(join(out, 'sw.js'))).rejects.toThrow();
    // The file iOS reads from the hosted site to open the app is the site's, not the app's.
    await expect(readFile(join(out, '.well-known/apple-app-site-association'))).rejects.toThrow();
  });

  it('refuses a phone build made from another revision than the source it is assembled with', async () => {
    await expect(assemble({ dist: await build(), out: join(root, 'www'), pins: pins(), expected: 6, fetcher: site() })).rejects.toThrow(/speaks protocol 5 but this source speaks 6/);
    expect(await assemble({ dist: await build(), out: join(root, 'www'), pins: pins(), expected: 5, fetcher: site() })).toEqual([3, 5]);
  });

  it('permits only the named loopback relay for a simulator assembly, keeping the release policy strict', async () => {
    const dist = await build();
    const options = { dist, out: join(root, 'www'), pins: pins(), fetcher: site() };
    await expect(assemble({ ...options, simulatorRelayPort: 43179 })).rejects.toThrow(/fixture relay/);
    const local = page('simulator', POLICY.replace('wss://relay.withhide.dev', 'ws://127.0.0.1:43179'));
    await writeFile(join(dist, 'index.html'), local);
    await writeFile(join(dist, 'p/5/index.html'), local);
    await expect(assemble(options)).rejects.toThrow(/production relay/);
    await expect(assemble({ ...options, simulatorRelayPort: 43180 })).rejects.toThrow(/fixture relay/);
    await expect(assemble({ ...options, simulatorRelayPort: 43179 })).resolves.toEqual([3, 5]);
    expect(await readFile(join(root, 'www/index.html'), 'utf8')).toContain('ws://127.0.0.1:43179');
    await expect(assemble({ ...options, simulatorRelayPort: 1 })).rejects.toThrow(/Invalid simulator/);
    await writeFile(join(dist, 'index.html'), local.replace("script-src 'self'", 'script-src *'));
    await expect(assemble({ ...options, simulatorRelayPort: 43179 })).rejects.toThrow(/block external scripts/);
  });

  it('stops when a pinned file differs, even when the site\'s own inventory agrees with the changed file', async () => {
    const changed = { ...OLD, 'p/3/assets/a-1.js': 'tampered!!!', 'p/5/index.html': page('five') };
    await expect(run(site({ files: changed }))).rejects.toThrow(/differs from its pin: p\/3\/assets\/a-1\.js/);
  });

  it('stops when a pinned protocol or file is gone from the site', async () => {
    await expect(run(site({ files: { 'p/5/index.html': page('five') }, protocols: [5] }))).rejects.toThrow(/gone from the phone site: p\/3\//);
    await expect(run(site({ files: { 'p/3/index.html': OLD['p/3/index.html'], 'p/5/index.html': page('five') } }))).rejects.toThrow(/gone from the phone site: p\/3\/assets\/a-1\.js/);
  });

  it('stops when the site serves a protocol nobody pinned, rather than shipping without it', async () => {
    await expect(run(site({ protocols: [2, 3, 5] }))).rejects.toThrow(/serves protocol 2 that retained-bundles\.json does not pin/);
  });

  it('refuses a pin for the protocol this source builds itself', async () => {
    const own = parsePins({ schema: 1, protocols: { 5: entries({ 'p/5/index.html': page('five') }) } });
    await expect(run(site(), own)).rejects.toThrow(/this source's own/);
  });

  it('stops when a pinned page does not keep the phone site\'s content policy', async () => {
    const weak = { 'p/3/index.html': page('three', 'default-src *') };
    await expect(run(site({ files: { ...weak, 'p/5/index.html': page('five') } }), parsePins({ schema: 1, protocols: { 3: entries(weak) } }))).rejects.toThrow();
  });
});

describe('the pin list', () => {
  it('refuses a malformed list', () => {
    const good = entries(OLD);
    expect(() => parsePins({ schema: 2, protocols: {} })).toThrow();
    expect(() => parsePins({ schema: 1, protocols: { 3: { 'p/3/a.js': good['p/3/assets/a-1.js'] } } })).toThrow(/no entry page/);
    expect(() => parsePins({ schema: 1, protocols: { 3: { ...good, 'p/4/x.js': good['p/3/assets/a-1.js'] } } })).toThrow(/outside its protocol folder/);
    expect(() => parsePins({ schema: 1, protocols: { 3: { ...good, 'p/3/../../x': good['p/3/assets/a-1.js'] } } })).toThrow();
    expect(() => parsePins({ schema: 1, protocols: { 3: { ...good, 'p/3/b.js': { sha256: 'xyz', size: 1 } } } })).toThrow(/valid digest/);
  });

  it('is written from the site only when every file agrees with the site\'s own inventory', async () => {
    const files = { ...OLD, 'p/5/index.html': page('five') };
    const value = await pin({ current: 5, fetcher: site({ files }) });
    expect(Object.keys(value.protocols)).toEqual(['3']);
    expect(value.protocols[3]).toEqual(entries(OLD));
    const lying = { schema: 1, source: 'a'.repeat(40), files: entries({ ...files, 'p/3/assets/a-1.js': 'something else' }) };
    await expect(pin({ current: 5, fetcher: site({ files, inventory: lying }) })).rejects.toThrow(/differs from its own inventory/);
  });
});
