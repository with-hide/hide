// The production phone site is assembled in main CI, never by the host's builder.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdir, readFile, readdir, lstat, writeFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { pathToFileURL } from 'node:url';

export const ORIGIN = 'https://app.withhide.dev';
const MAX_FILES = 4096;
const MAX_FILE = 16 * 1024 * 1024;
const MAX_SITE = 256 * 1024 * 1024;
// Required server-only secrets, no defaults. Missing values stop before any upload.
export const SECRETS = {
  PHONE_VERCEL_TOKEN: /^\S{1,512}$/,
  PHONE_VERCEL_ORG_ID: /^[A-Za-z0-9_-]{1,128}$/,
  PHONE_VERCEL_PROJECT_ID: /^prj_[A-Za-z0-9]+$/,
};
export function credentials(env) {
  const invalid = Object.entries(SECRETS).filter(([key, shape]) => !shape.test(env[key] ?? ''));
  if (invalid.length) throw new Error(`Missing or invalid phone deployment secrets: ${invalid.map(([key]) => key).join(', ')}. Configure the phone-production environment; release stopped.`);
  return Object.fromEntries(Object.keys(SECRETS).map(key => [key, env[key]]));
}
export function requireMain(env) {
  if (env.GITHUB_ACTIONS !== 'true' || env.GITHUB_REPOSITORY !== 'with-hide/hide' ||
      env.GITHUB_REF !== 'refs/heads/main' || env.GITHUB_EVENT_NAME !== 'workflow_dispatch' ||
      env.GITHUB_WORKFLOW_REF !== 'with-hide/hide/.github/workflows/release-candidate.yml@refs/heads/main') {
    throw new Error('Phone production deployment requires release-candidate.yml on protected main.');
  }
}
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
function filePath(path) {
  if (typeof path !== 'string' || path.length > 240 ||
      !path.split('/').every(part => /^[A-Za-z0-9_-][A-Za-z0-9_.-]*$/.test(part))) {
    throw new Error('Invalid phone site file path');
  }
  return path;
}
function inventory(value) {
  if (value?.schema !== 1 || !/^[a-f0-9]{40}$/.test(value.source) ||
      !value.files || typeof value.files !== 'object' || Array.isArray(value.files) ||
      Object.keys(value.files).length > MAX_FILES) throw new Error('Invalid phone site inventory');
  let size = 0;
  for (const [path, entry] of Object.entries(value.files)) {
    filePath(path);
    if (!entry || !/^[a-f0-9]{64}$/.test(entry.sha256) || !Number.isSafeInteger(entry.size) || entry.size < 1 || entry.size > MAX_FILE) {
      throw new Error(`Invalid phone site entry: ${path}`);
    }
    size += entry.size;
  }
  if (size > MAX_SITE) throw new Error('Phone site exceeds 256 MiB; retain old protocols and review the host limit.');
  const numbers = [...new Set(Object.keys(value.files).filter(p => /^p\/[1-9]\d*\/index.html$/.test(p)).map(p => Number(p.split('/')[1])))].sort((a, b) => a - b);
  if (!numbers.length || numbers.some(n => !Number.isSafeInteger(n))) throw new Error('Phone site has no valid protocol bundles');
  for (const path of Object.keys(value.files).filter(p => p.startsWith('p/'))) {
    if (!/^p\/[1-9]\d*\//.test(path) || !numbers.includes(Number(path.split('/')[1]))) throw new Error('Phone protocol is missing its index');
  }
  return numbers;
}
async function get(url, fetcher, headers = {}) {
  const response = await fetcher(url, { headers, redirect: 'error', signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`Phone deployment read failed (${response.status}): ${new URL(url).pathname}`);
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > MAX_FILE) throw new Error('Phone deployment response exceeds 16 MiB');
    chunks.push(chunk);
  }
  return { bytes: Buffer.concat(chunks), headers: response.headers };
}
async function json(url, fetcher, headers) {
  return JSON.parse((await get(url, fetcher, headers)).bytes);
}
async function put(root, path, bytes) {
  const target = resolve(root, filePath(path));
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
}
async function localFiles(root, prefix = '') {
  const result = [];
  for (const entry of await readdir(resolve(root, prefix), { withFileTypes: true })) {
    const path = filePath(prefix + entry.name);
    if (entry.isDirectory()) result.push(...await localFiles(root, `${path}/`));
    else if (entry.isFile()) result.push(path);
    else throw new Error('Phone build contains a link or special file');
    if (result.length > MAX_FILES) throw new Error('Phone build exceeds file limit');
  }
  return result;
}
// The frame a phone draws an HTML file in (web/public/m/look-frame.html) keeps a policy of its own: it runs a page's
// scripts in a sandbox of their own origin, and may load what a generated page needs but never talk back. It is framed
// only by the app, so every other file keeps `frame-ancestors 'none'`.
const FRAME = /^p\/[1-9][0-9]*\/look-frame\.html$/;
const FRAME_ROUTE = '^/p/[1-9][0-9]*/look-frame\\.html$';
function framePolicy(html) {
  const policy = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)"/)?.[1];
  const directives = new Map(policy?.split(';').map(d => { const [key, ...values] = d.trim().split(/\s+/); return [key, values.join(' ')]; }));
  for (const key of ['default-src', 'connect-src', 'form-action', 'frame-src', 'object-src']) {
    if (directives.get(key) !== "'none'") throw new Error(`Phone look frame must keep ${key} 'none'.`);
  }
  if (/'self'/.test(policy)) throw new Error("Phone look frame must not reach the app's own origin.");
  return policy;
}
function contentPolicy(html) {
  const policy = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]+)">/)?.[1];
  const directives = new Map(policy?.split(';').map(d => { const [key, ...values] = d.trim().split(/\s+/); return [key, values.join(' ')]; }));
  if (directives.get('script-src') !== "'self' 'wasm-unsafe-eval'" ||
      directives.get('connect-src') !== "'self' wss://relay.withhide.dev" ||
      directives.get('default-src') !== "'none'") throw new Error('Phone build must block external scripts and use the production relay.');
  return policy;
}

export async function preparePhone({ source, dist, stage, secrets, fetcher = fetch, ancestor }) {
  if (!/^[a-f0-9]{40}$/.test(source)) throw new Error('Invalid phone source SHA');
  // A fresh directory prevents a previous attempt's unlisted files reaching production.
  await mkdir(dirname(resolve(stage)), { recursive: true });
  await mkdir(stage);
  const project = await json(`https://api.vercel.com/v9/projects/${secrets.PHONE_VERCEL_PROJECT_ID}?teamId=${secrets.PHONE_VERCEL_ORG_ID}`, fetcher,
    { Authorization: `Bearer ${secrets.PHONE_VERCEL_TOKEN}` });
  if (project.id !== secrets.PHONE_VERCEL_PROJECT_ID || project.accountId !== secrets.PHONE_VERCEL_ORG_ID || project.link) {
    throw new Error('Phone project must match the configured IDs and have no Git deployment integration.');
  }
  for (const key of ['analytics', 'webAnalytics', 'speedInsights']) {
    const setting = project[key];
    if (setting?.enabledAt && !(setting.disabledAt >= setting.enabledAt)) throw new Error(`Disable ${key} on the phone project.`);
  }
  const files = {};
  const output = resolve(stage, '.vercel/output/static');
  const previous = project.targets?.production;
  if (previous) {
    if (previous.readyState !== 'READY' || !/^[a-z0-9-]+\.vercel\.app$/.test(previous.url)) throw new Error('Previous phone deployment is not ready.');
    const origin = `https://${previous.url}`;
    const manifest = await json(`${origin}/phone-site.json`, fetcher);
    const numbers = inventory(manifest);
    if (JSON.stringify(await json(`${origin}/protocols.json`, fetcher)) !== JSON.stringify(numbers)) throw new Error('Previous phone protocol inventory disagrees.');
    await ancestor(manifest.source, source);
    // Only a project with no production deployment bootstraps empty. A missing
    // or corrupt existing inventory is an error, never permission to erase it.
    // An open loader can request its root WASM or lazy module after a deploy.
    // Both asset trees share the same digest, collision and resource checks.
    for (const [path, entry] of Object.entries(manifest.files).filter(([p]) => p.startsWith('p/') || p.startsWith('assets/'))) {
      const { bytes } = await get(`${origin}/${path}`, fetcher);
      if (bytes.length !== entry.size || sha256(bytes) !== entry.sha256) throw new Error(`Previous phone file differs: ${path}`);
      await put(output, path, bytes);
      files[path] = entry;
    }
  }
  const protocols = JSON.parse(await readFile(resolve(dist, 'protocols.json'), 'utf8'));
  if (!Array.isArray(protocols) || protocols.length !== 1 || !Number.isSafeInteger(protocols[0]) || protocols[0] < 1) throw new Error('Build must contain exactly its phone protocol');
  const protocol = protocols[0];
  const policy = contentPolicy(await readFile(resolve(dist, 'index.html'), 'utf8'));
  if (contentPolicy(await readFile(resolve(dist, `p/${protocol}/index.html`), 'utf8')) !== policy) throw new Error('Phone pages disagree on CSP');
  let size = Object.values(files).reduce((sum, entry) => sum + entry.size, 0);
  for (const path of await localFiles(dist)) {
    if (path === 'protocols.json') continue;
    if (path === 'phone-site.json' || (path.startsWith('p/') && !path.startsWith(`p/${protocol}/`))) throw new Error('Unexpected phone build file');
    const stat = await lstat(resolve(dist, path));
    size += stat.size - (files[path]?.size ?? 0);
    if (stat.size > MAX_FILE || size > MAX_SITE) throw new Error('Phone site exceeds deployment size limit');
    const bytes = await readFile(resolve(dist, path));
    const entry = { sha256: sha256(bytes), size: bytes.length };
    // Keep old hashed assets even when this protocol's entry page is replaced.
    if (files[path] && files[path].sha256 !== entry.sha256 && path !== `p/${protocol}/index.html`) throw new Error(`Phone asset name reused for different bytes: ${path}`);
    await put(output, path, bytes);
    files[path] = entry;
  }
  const manifest = { schema: 1, source, files };
  const numbers = inventory(manifest);
  await put(output, 'protocols.json', JSON.stringify(numbers));
  await put(output, 'phone-site.json', JSON.stringify(manifest));
  await mkdir(resolve(stage, '.vercel'), { recursive: true });
  await writeFile(resolve(stage, '.vercel/project.json'), JSON.stringify({ projectId: project.id, orgId: project.accountId }));
  const frames = [];
  for (const path of Object.keys(files).filter(p => FRAME.test(p))) {
    const frame = framePolicy(await readFile(resolve(output, path), 'utf8'));
    frames.push({ src: `^/${path.replaceAll('.', '\\.')}$`, headers: { 'Content-Security-Policy': `${frame}; frame-ancestors 'self'`,
      'Referrer-Policy': 'no-referrer', 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-cache' }, continue: true });
  }
  await writeFile(resolve(stage, '.vercel/output/config.json'), JSON.stringify({ version: 3, routes: [
    { src: `^/(?!${FRAME_ROUTE.slice(2, -1)}$).*$`, headers: { 'Content-Security-Policy': `${policy}; frame-ancestors 'none'`, 'Referrer-Policy': 'no-referrer', 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-cache' }, continue: true },
    ...frames,
    { src: '/', dest: '/index.html' },
    { src: '/p/([1-9][0-9]*)/', dest: '/p/$1/index.html' },
    { handle: 'filesystem' },
  ] }));
  return manifest;
}

export async function confirmPhone(stage, fetcher = fetch) {
  const root = resolve(stage, '.vercel/output/static');
  const expected = JSON.parse(await readFile(resolve(root, 'phone-site.json'), 'utf8'));
  const numbers = inventory(expected);
  const actual = await json(`${ORIGIN}/phone-site.json`, fetcher);
  if (JSON.stringify(actual) !== JSON.stringify(expected) ||
      JSON.stringify(await json(`${ORIGIN}/protocols.json`, fetcher)) !== JSON.stringify(numbers)) throw new Error('Production phone site does not match this release.');
  for (const [path, entry] of Object.entries(expected.files)) {
    const { bytes, headers } = await get(`${ORIGIN}/${path}`, fetcher);
    if (sha256(bytes) !== entry.sha256 || bytes.length !== entry.size) throw new Error(`Production phone file differs: ${path}`);
    if (FRAME.test(path)) {
      if (headers.get('content-security-policy') !== `${framePolicy(bytes.toString())}; frame-ancestors 'self'`) throw new Error('Production look frame CSP header is missing or changed');
    } else if (path.endsWith('.html')) {
      const policy = contentPolicy(bytes.toString());
      if (headers.get('content-security-policy') !== `${policy}; frame-ancestors 'none'`) throw new Error('Production phone CSP header is missing or changed');
    }
  }
  for (const path of ['index.html', ...numbers.map(number => `p/${number}/index.html`)]) {
    const route = path === 'index.html' ? '/' : `/${path.slice(0, -'index.html'.length)}`;
    const { bytes } = await get(`${ORIGIN}${route}`, fetcher);
    if (sha256(bytes) !== expected.files[path].sha256) throw new Error(`Production phone route differs: ${route}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    requireMain(process.env);
    const [command, source, dist, stage] = process.argv.slice(2);
    if (command === 'check') credentials(process.env); // Fail before installing or building.
    else if (command === 'prepare') {
      const secrets = credentials(process.env);
      execFileSync('git', ['merge-base', '--is-ancestor', source, 'origin/main']);
      if (execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim() !== source) throw new Error('Phone build checkout differs from source');
      await preparePhone({ source, dist, stage, secrets, ancestor: (before, after) => execFileSync('git', ['merge-base', '--is-ancestor', before, after]) });
    } else if (command === 'confirm') await confirmPhone(source);
    else throw new Error('Usage: phone-release.mjs check | prepare <source> <build> <fresh-stage> | confirm <stage>');
  } catch (error) {
    console.error(`Phone release stopped: ${error.message}`);
    process.exitCode = 1;
  }
}
