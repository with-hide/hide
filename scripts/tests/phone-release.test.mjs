import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { preparePhone, confirmPhone, credentials, requireMain, SECRETS, ORIGIN } from '../phone-release.mjs';

const runs = resolve('agents/runs/phone-release-tests');
const source = 'a'.repeat(40);
const policy = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self' wss://relay.withhide.dev";
const page = `<head><meta http-equiv="Content-Security-Policy" content="${policy}"></head>`;
// An older bundle was built with an older policy, and keeps being served with it.
const oldPolicy = policy.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'");
const oldPage = page.replace(policy, oldPolicy);
// The phone's real look frame, so a policy the release would refuse fails here first.
const realFrame = await readFile('web/public/m/look-frame.html', 'utf8');
const framePolicy = realFrame.match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)"/)[1];
const frame = (csp = framePolicy) => realFrame.replace(framePolicy, csp);
const secrets = { PHONE_VERCEL_TOKEN: 'fixture-token', PHONE_VERCEL_ORG_ID: 'team_fixture', PHONE_VERCEL_PROJECT_ID: 'prj_fixture' };
const ci = { GITHUB_ACTIONS: 'true', GITHUB_REPOSITORY: 'with-hide/hide', GITHUB_REF: 'refs/heads/main',
  GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_WORKFLOW_REF: 'with-hide/hide/.github/workflows/release-candidate.yml@refs/heads/main' };
async function fixture(t, protocol = 2) {
  await mkdir(runs, { recursive: true });
  const root = await mkdtemp(resolve(runs, 'case-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const dist = resolve(root, 'dist');
  await mkdir(resolve(dist, `p/${protocol}/assets`), { recursive: true });
  await writeFile(resolve(dist, 'index.html'), page);
  await writeFile(resolve(dist, 'protocols.json'), JSON.stringify([protocol]));
  await writeFile(resolve(dist, `p/${protocol}/index.html`), page);
  await writeFile(resolve(dist, `p/${protocol}/assets/new.js`), 'new bundle');
  const oldFiles = new Map([['p/1/index.html', oldPage], ['p/1/assets/old.js', 'old bundle'],
    ['assets/relay_bg-old.wasm', 'old WASM'], ['assets/connect-old.js', 'old lazy module']]);
  const manifest = { schema: 1, source: 'b'.repeat(40), files: Object.fromEntries([...oldFiles].map(([path, bytes]) =>
    [path, { size: Buffer.byteLength(bytes), sha256: createHash('sha256').update(bytes).digest('hex') }])) };
  const state = {
    project: { id: secrets.PHONE_VERCEL_PROJECT_ID, accountId: secrets.PHONE_VERCEL_ORG_ID,
      targets: { production: { readyState: 'READY', url: 'previous.vercel.app' } } },
    previous: new Map([...oldFiles, ['phone-site.json', JSON.stringify(manifest)], ['protocols.json', '[1]']]),
    live: new Map(), routes: [], badHeader: false, ancestor: true,
  };
  // The live host answers the headers the deployed routes give a path, as Vercel does: exactly one route sets each.
  const served = path => {
    const set = state.routes.filter(route => route.headers && new RegExp(route.src).test(path));
    assert.ok(set.length <= 1, `${path} matches ${set.length} header routes`);
    const csp = set[0]?.headers['Content-Security-Policy'];
    return state.badFrame && /look-frame\.html$/.test(path) ? csp.replace("frame-ancestors 'self'", "frame-ancestors 'none'") : csp;
  };
  // Only Vercel's HTTP boundary is substituted; real filesystem assembly and
  // verification run unchanged, with deliberately unrelated old bundle bytes.
  const fetcher = async (url, options) => {
    if (url.startsWith('https://api.vercel.com/v9/projects/')) {
      assert.equal(options.headers.Authorization, 'Bearer fixture-token');
      return new Response(JSON.stringify(state.project));
    }
    const address = new URL(url);
    const data = address.origin === 'https://previous.vercel.app' ? state.previous : state.live;
    const path = address.pathname.endsWith('/') ? `${address.pathname.slice(1)}index.html` : address.pathname.slice(1);
    const csp = data === state.live ? served(address.pathname) : undefined;
    return data.has(path) ? new Response(data.get(path), { headers: csp === undefined ? {} : {
      'content-security-policy': state.badHeader ? "script-src *" : csp,
    } }) : new Response('missing', { status: 404 });
  };
  const stage = resolve(root, 'new-parent/stage');
  const prepare = (destination = stage) => preparePhone({ source, dist, stage: destination, secrets, fetcher, ancestor: async () => {
    if (!state.ancestor) throw new Error('source would roll back production');
  } });
  const serve = async (destination = stage) => {
    const dir = resolve(destination, '.vercel/output/static');
    state.live.clear();
    state.routes = JSON.parse(await readFile(resolve(destination, '.vercel/output/config.json'))).routes;
    for (const path of await readdir(dir, { recursive: true, withFileTypes: true })) {
      if (path.isFile()) {
        const absolute = resolve(path.parentPath, path.name);
        state.live.set(absolute.slice(dir.length + 1), await readFile(absolute));
      }
    }
  };
  return { root, dist, stage, state, fetcher, prepare, serve };
}

test('only the main candidate CI can deploy, and absent secrets name every missing key without values', () => {
  requireMain(ci);
  for (const [key, value] of Object.entries({ GITHUB_ACTIONS: 'false', GITHUB_REPOSITORY: 'fork/hide',
    GITHUB_REF: 'refs/heads/feature', GITHUB_EVENT_NAME: 'pull_request', GITHUB_WORKFLOW_REF: 'another-workflow' })) {
    assert.throws(() => requireMain({ ...ci, [key]: value }), /protected main/);
  }
  assert.deepEqual(credentials(secrets), secrets);
  for (const key of Object.keys(SECRETS)) assert.throws(() => credentials({ ...secrets, [key]: '' }), new RegExp(key));
  const command = execFileSync('node', ['scripts/phone-release.mjs', 'check'], { env: { ...process.env, ...ci, ...secrets }, encoding: 'utf8' });
  assert.equal(command, '');
  assert.throws(() => execFileSync('node', ['scripts/phone-release.mjs', 'check'], {
    env: { ...process.env, ...ci, ...Object.fromEntries(Object.keys(SECRETS).map(k => [k, ''])) }, stdio: 'pipe',
  }), error => error.status === 1 && Object.keys(SECRETS).every(key => error.stderr.includes(key)));
});

test('first production deploy needs an existing empty host project, and writes no token', async t => {
  const f = await fixture(t);
  f.state.project.targets = {};
  await f.prepare();
  const root = resolve(f.stage, '.vercel/output/static');
  assert.equal(await readFile(resolve(root, 'protocols.json'), 'utf8'), '[2]');
  const config = JSON.parse(await readFile(resolve(f.stage, '.vercel/output/config.json')));
  assert.equal(config.routes[0].headers['Content-Security-Policy'], `${policy}; frame-ancestors 'none'`);
  assert.deepEqual(JSON.parse(await readFile(resolve(f.stage, '.vercel/project.json'))), { projectId: 'prj_fixture', orgId: 'team_fixture' });
  await f.serve();
  await confirmPhone(f.stage, f.fetcher);
});

test('a new protocol preserves every old file and publishes the union, with exact source confirmation', async t => {
  const f = await fixture(t);
  await f.prepare();
  const root = resolve(f.stage, '.vercel/output/static');
  assert.equal(await readFile(resolve(root, 'p/1/assets/old.js'), 'utf8'), 'old bundle');
  assert.equal(await readFile(resolve(root, 'p/2/assets/new.js'), 'utf8'), 'new bundle');
  assert.equal(await readFile(resolve(root, 'protocols.json'), 'utf8'), '[1,2]');
  await f.serve();
  await confirmPhone(f.stage, f.fetcher);
  const manifest = JSON.parse(f.state.live.get('phone-site.json'));
  manifest.source = 'c'.repeat(40);
  f.state.live.set('phone-site.json', JSON.stringify(manifest));
  await assert.rejects(confirmPhone(f.stage, f.fetcher), /does not match this release/);
});

test('updating the same protocol keeps its previously referenced assets', async t => {
  const f = await fixture(t, 1);
  await f.prepare();
  assert.equal(await readFile(resolve(f.stage, '.vercel/output/static/protocols.json'), 'utf8'), '[1]');
  assert.equal(await readFile(resolve(f.stage, '.vercel/output/static/p/1/assets/old.js'), 'utf8'), 'old bundle');
});

test('an open loader can fetch its old lazy module and WASM after two deployments', async t => {
  const f = await fixture(t, 1);
  f.state.project.targets = {};
  await mkdir(resolve(f.dist, 'assets'));
  const oldWasm = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);
  const oldLoader = `export async function connect(read) {
    const response = await read('${ORIGIN}/assets/connect-old.js');
    if (!response.ok) throw new Error('The open loader lost its lazy module');
    const lazy = await import('data:text/javascript,' + encodeURIComponent(await response.text()));
    const wasm = await read('${ORIGIN}' + lazy.wasmPath);
    if (!wasm.ok) throw new Error('The open loader lost its WASM');
    return new Uint8Array(await wasm.arrayBuffer());
  }`;
  await writeFile(resolve(f.dist, 'assets/loader-old.js'), oldLoader);
  await writeFile(resolve(f.dist, 'assets/connect-old.js'), "export const wasmPath = '/assets/relay_bg-old.wasm';");
  await writeFile(resolve(f.dist, 'assets/relay_bg-old.wasm'), oldWasm);
  await f.prepare();
  await f.serve();
  // The tab loads its loader now, but connects only after the next deployment.
  const response = await f.fetcher(`${ORIGIN}/assets/loader-old.js`);
  const openTab = await import('data:text/javascript,' + encodeURIComponent(await response.text()));
  f.state.previous = new Map(f.state.live);
  f.state.project.targets = { production: { readyState: 'READY', url: 'previous.vercel.app' } };
  await rm(resolve(f.dist, 'assets'), { recursive: true });
  await mkdir(resolve(f.dist, 'assets'));
  await writeFile(resolve(f.dist, 'assets/loader-new.js'), 'new loader');
  await writeFile(resolve(f.dist, 'assets/connect-new.js'), 'new lazy module');
  await writeFile(resolve(f.dist, 'assets/relay_bg-new.wasm'), 'new WASM');
  const next = `${f.stage}-next`;
  await f.prepare(next);
  // Replace the served deployment completely: the HTTP fixture retains nothing.
  await f.serve(next);
  assert.deepEqual(Buffer.from(await openTab.connect(f.fetcher)), oldWasm);
  assert.equal(await (await f.fetcher(`${ORIGIN}/assets/loader-old.js`)).text(), oldLoader);
  assert.equal(await (await f.fetcher(`${ORIGIN}/assets/loader-new.js`)).text(), 'new loader');
  await confirmPhone(next, f.fetcher);
});

test('the public confirmation command reaches its files without deployment credentials', async t => {
  const f = await fixture(t);
  assert.throws(() => execFileSync('node', ['scripts/phone-release.mjs', 'confirm', f.stage], {
    env: { ...process.env, ...ci, ...Object.fromEntries(Object.keys(SECRETS).map(k => [k, ''])) }, stdio: 'pipe',
  }), error => error.status === 1 && error.stderr.includes('ENOENT') && !error.stderr.includes('secrets'));
});

for (const failure of ['missing inventory', 'missing asset', 'corrupt asset', 'missing root asset', 'corrupt root asset', 'protocol disagreement', 'rollback', 'analytics', 'git deploy', 'foreign project']) {
  test(`release stops before an upload on ${failure}`, async t => {
    const f = await fixture(t);
    if (failure === 'missing inventory') f.state.previous.delete('phone-site.json');
    if (failure === 'missing asset') f.state.previous.delete('p/1/assets/old.js');
    if (failure === 'corrupt asset') f.state.previous.set('p/1/assets/old.js', 'changed');
    if (failure === 'missing root asset') f.state.previous.delete('assets/relay_bg-old.wasm');
    if (failure === 'corrupt root asset') f.state.previous.set('assets/relay_bg-old.wasm', 'changed');
    if (failure === 'protocol disagreement') f.state.previous.set('protocols.json', '[1,2]');
    if (failure === 'rollback') f.state.ancestor = false;
    if (failure === 'analytics') f.state.project.webAnalytics = { enabledAt: 20, disabledAt: 10 };
    if (failure === 'git deploy') f.state.project.link = { type: 'github' };
    if (failure === 'foreign project') f.state.project.id = 'prj_wrong';
    await assert.rejects(f.prepare());
    assert.equal(f.state.live.size, 0);
  });
}

test('unsafe production CSP and a development relay fail before deployment', async t => {
  for (const replacement of ["script-src *", "connect-src 'self' ws://localhost:8787"]) {
    const f = await fixture(t);
    const html = replacement.startsWith('script') ? page.replace("script-src 'self' 'wasm-unsafe-eval'", replacement)
      : page.replace("connect-src 'self' wss://relay.withhide.dev", replacement);
    await writeFile(resolve(f.dist, 'index.html'), html);
    await assert.rejects(f.prepare(), /block external scripts/);
  }
});

test('a successful upload is insufficient if a retained file or production CSP is wrong', async t => {
  const f = await fixture(t);
  await f.prepare();
  await f.serve();
  f.state.badHeader = true;
  await assert.rejects(confirmPhone(f.stage, f.fetcher), /CSP header/);
  f.state.badHeader = false;
  f.state.live.delete('p/1/assets/old.js');
  await assert.rejects(confirmPhone(f.stage, f.fetcher), /404/);
});

test('unsafe inventory paths, excessive retained files and reused asset names are refused', async t => {
  for (const path of ['p/1/assets/old.js', 'assets/relay_bg-old.wasm']) {
    const f = await fixture(t, 1);
    await mkdir(resolve(f.dist, 'assets'), { recursive: true });
    await writeFile(resolve(f.dist, path), 'replacement');
    await assert.rejects(f.prepare(), /name reused/);
  }
  for (const path of ['../escape', 'p/1/../escape', 'p/1/%2e%2e/escape']) {
    const next = await fixture(t);
    const manifest = JSON.parse(next.state.previous.get('phone-site.json'));
    manifest.files[path] = manifest.files['p/1/index.html'];
    next.state.previous.set('phone-site.json', JSON.stringify(manifest));
    await assert.rejects(next.prepare(), /Invalid phone site file path/);
  }
  const next = await fixture(t);
  const manifest = JSON.parse(next.state.previous.get('phone-site.json'));
  for (let i = 0; i < 4096; i++) manifest.files[`assets/${i}.js`] = manifest.files['p/1/index.html'];
  next.state.previous.set('phone-site.json', JSON.stringify(manifest));
  await assert.rejects(next.prepare(), /Invalid phone site inventory/);
  // Root assets share the documented 16 MiB file and 256 MiB site limits.
  for (const [count, size, reason] of [[1, 16 * 1024 * 1024 + 1, /Invalid phone site entry/],
    [17, 16 * 1024 * 1024, /exceeds 256 MiB/]]) {
    const oversized = await fixture(t);
    const manifest = JSON.parse(oversized.state.previous.get('phone-site.json'));
    for (let i = 0; i < count; i++) manifest.files[`assets/large-${i}.wasm`] = { ...manifest.files['p/1/index.html'], size };
    oversized.state.previous.set('phone-site.json', JSON.stringify(manifest));
    await assert.rejects(oversized.prepare(), reason);
  }
});

test('the look frame keeps its own policy, is framed only by the app, and is confirmed by it', async t => {
  const f = await fixture(t);
  await writeFile(resolve(f.dist, 'p/2/look-frame.html'), frame());
  await f.prepare();
  const { routes } = JSON.parse(await readFile(resolve(f.stage, '.vercel/output/config.json')));
  const headers = path => routes.filter(route => route.headers && new RegExp(route.src).test(path)).map(route => route.headers['Content-Security-Policy']);
  assert.deepEqual(headers('/p/2/look-frame.html'), [`${framePolicy}; frame-ancestors 'self'`]);
  for (const path of ['/', '/p/2/', '/p/2/index.html', '/p/2/assets/new.js', '/assets/connect-old.js']) {
    assert.deepEqual(headers(path), [`${policy}; frame-ancestors 'none'`], path);
  }
  for (const path of ['/p/1/', '/p/1/index.html', '/p/1/assets/old.js', '/p/1/look-frame.htmlx']) {
    assert.deepEqual(headers(path), [`${oldPolicy}; frame-ancestors 'none'`], path);
  }
  await f.serve();
  await confirmPhone(f.stage, f.fetcher);
  f.state.badFrame = true;
  await assert.rejects(confirmPhone(f.stage, f.fetcher), /look frame CSP header/);
});

test('a look frame that could talk back or reach the app stops before deployment', async t => {
  for (const csp of [framePolicy.replace("connect-src 'none'", 'connect-src https:'), `${framePolicy}; img-src 'self'`, "script-src 'unsafe-inline'"]) {
    const f = await fixture(t);
    await writeFile(resolve(f.dist, 'p/2/look-frame.html'), frame(csp));
    await assert.rejects(f.prepare(), /look frame/);
    assert.equal(f.state.live.size, 0);
  }
});
