import { cp, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { check } from './check-versions.mjs';

const real = resolve(dirname(fileURLToPath(import.meta.url)), '..');
let copy;
beforeEach(async () => {
  copy = await mkdtemp(join(tmpdir(), 'mobile-'));
  await cp(join(real, 'package.json'), join(copy, 'package.json'));
  for (const file of ['App/CapApp-SPM/Package.swift', 'App/HideShell/Package.swift']) {
    await cp(join(real, 'ios', file), join(copy, 'ios', file), { recursive: true });
  }
});
afterEach(async () => { await rm(copy, { recursive: true, force: true }); });

async function edit(file, from, to) {
  const path = join(copy, file);
  const text = await readFile(path, 'utf8');
  expect(text).toContain(from);
  await writeFile(path, text.replace(from, to));
}

describe('what the app links', () => {
  it('is Capacitor at one version and no plugin, as committed', async () => {
    expect(await check(real)).toEqual([]);
  });

  it('refuses a Capacitor version that differs between the three places', async () => {
    await edit('ios/App/HideShell/Package.swift', 'exact: "8.6.0"', 'exact: "8.5.3"');
    expect(await check(copy)).toEqual([expect.stringContaining('versions disagree')]);
  });

  it('refuses a plugin added to the package, to the plugin package or to the native package', async () => {
    await edit('package.json', '"@capacitor/core": "8.6.0",', '"@capacitor/core": "8.6.0",\n    "@capacitor/keyboard": "8.0.0",');
    expect(await check(copy)).toEqual([expect.stringContaining('@capacitor/keyboard')]);
    await edit('ios/App/CapApp-SPM/Package.swift', '.product(name: "Cordova", package: "capacitor-swift-pm")', '.product(name: "Cordova", package: "capacitor-swift-pm"),\n                .product(name: "CapacitorKeyboard", package: "capacitor-swift-pm")');
    expect((await check(copy)).join('\n')).toContain('CapApp-SPM links Capacitor, CapacitorKeyboard, Cordova');
  });
});
