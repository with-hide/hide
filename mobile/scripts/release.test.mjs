import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { sha256 } from '../../scripts/phone-release.mjs';
import { ADDED_BY_CAPACITOR, buildManifest } from './bundle-manifest.mjs';
import { buildNumber, marketingVersion } from './release-version.mjs';

const mobile = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFileSync(join(mobile, path), 'utf8');

describe('the version and build number of a release', () => {
  it('takes the numbers of the repository version, whatever follows them', () => {
    expect(marketingVersion('1.2.5')).toBe('1.2.5');
    expect(marketingVersion('0.4.0-rc.1')).toBe('0.4.0');
    expect(marketingVersion('1.2.5-main.6d229801.dirty')).toBe('1.2.5');
    expect(() => marketingVersion('main')).toThrow(/three numbers/);
  });

  it('is the UTC time of the run, which rises from one run to the next', () => {
    const first = buildNumber(new Date('2026-11-10T09:05:03.999Z'));
    const second = buildNumber(new Date('2026-11-10T09:05:04.000Z'));
    expect(first).toBe('20261110090503');
    expect(Number(second)).toBeGreaterThan(Number(first));
    expect(Number(buildNumber(new Date('2027-01-01T00:00:00Z')))).toBeGreaterThan(Number(buildNumber(new Date('2026-12-31T23:59:59Z'))));
  });
});

describe('the secrets the release command reads', () => {
  const registry = JSON.parse(read('release-env.json'));

  it('are the keys of the example file, and no others', () => {
    const example = read('release.env.example').split('\n').filter((line) => /^[A-Z_]+=/.test(line)).map((line) => line.split('=')[0]);
    expect(example.sort()).toEqual(Object.keys(registry.keys).sort());
  });

  it('are the only HIDE_IOS_ASC_ names anywhere the release reads one', () => {
    const files = [...readdirSync(join(mobile, 'scripts')).filter((name) => !name.endsWith('.test.mjs')).map((name) => `scripts/${name}`), 'release.env.example'];
    const used = new Set(files.flatMap((file) => read(file).match(/HIDE_IOS_ASC_[A-Z_]+/g) ?? []));
    expect([...used].filter((name) => !(name in registry.keys))).toEqual([]);
  });

  it('give a required key no fallback and every key a note and a shape', () => {
    for (const [name, key] of Object.entries(registry.keys)) {
      expect(['required', 'optional'], name).toContain(key.requirement);
      expect(key.note.length, name).toBeGreaterThan(40);
      expect(() => new RegExp(key.shape), name).not.toThrow();
      expect(key, name).not.toHaveProperty('fallback');
    }
  });
});

describe('what the project builds for the store', () => {
  it('has Capacitor logging off, no remote page and no web view inspection in its configuration', () => {
    const config = JSON.parse(read('capacitor.config.json'));
    expect(config.loggingBehavior).toBe('none');
    expect(config.server).not.toHaveProperty('url');
    expect(config.ios.webContentsDebuggingEnabled).toBe(false);
  });

  it('names the same app and team as the list the signed app is held to', () => {
    const want = JSON.parse(read('release-expect.json'));
    const project = read('ios/App/App.xcodeproj/project.pbxproj');
    expect(project).toContain(`PRODUCT_BUNDLE_IDENTIFIER = ${want.bundleId};`);
    expect(project).toContain(`DEVELOPMENT_TEAM = ${want.teamId};`);
    expect(JSON.parse(read('capacitor.config.json')).appId).toBe(want.bundleId);
    expect(want.entitlements['application-identifier']).toBe(`${want.teamId}.${want.bundleId}`);
  });

  it('leaves the export compliance answer to the operator, and ships a privacy manifest in the app', () => {
    expect(read('ios/App/App/Info.plist')).not.toMatch(/ITSAppUsesNonExemptEncryption|ITSEncryptionExportComplianceCode/);
    expect(read('ios/App/App.xcodeproj/project.pbxproj')).toContain('PrivacyInfo.xcprivacy in Resources');
  });
});

describe('the web bundle manifest', () => {
  let root;
  const put = (dir, path, text) => { const file = join(root, dir, path); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, text); };
  const pins = new Map([[1, { 'p/1/index.html': { sha256: sha256(Buffer.from('old')), size: 3 } }]]);

  beforeEach(() => {
    root = mkdtempSync(join(tmpdir(), 'bundle-manifest-'));
    for (const dir of ['www', 'public']) { put(dir, 'index.html', 'loader'); put(dir, 'p/1/index.html', 'old'); put(dir, 'p/2/index.html', 'new'); }
    for (const file of ADDED_BY_CAPACITOR) put('public', file, 'cordova');
  });
  afterEach(() => { rmSync(root, { recursive: true, force: true }); });
  const build = () => buildManifest({ www: join(root, 'www'), publicDir: join(root, 'public'), pins });

  it('lists every file of the app with its digest, the Cordova files Capacitor adds included', async () => {
    const manifest = await build();
    expect(Object.keys(manifest.files)).toEqual(['cordova.js', 'cordova_plugins.js', 'index.html', 'p/1/index.html', 'p/2/index.html']);
    expect(manifest.files['p/2/index.html']).toBe(sha256(Buffer.from('new')));
  });

  it.each([
    ['a file in the app nobody assembled', () => put('public', 'extra.js', 'x'), 'never assembled: extra.js'],
    ['an assembled file that did not reach the app', () => rmSync(join(root, 'public/p/2/index.html')), 'missing from the app\'s public/: p/2/index.html'],
    ['a file that changed on the way', () => put('public', 'index.html', 'other'), 'differs from the assembled folder: index.html'],
    ['a retained file that is not the pinned one', () => { put('www', 'p/1/index.html', 'changed'); put('public', 'p/1/index.html', 'changed'); }, 'pinned file is not in the app as pinned: p/1/index.html'],
  ])('refuses %s', async (_name, change, message) => {
    change();
    await expect(build()).rejects.toThrow(message);
  });
});
