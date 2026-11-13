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
  await cp(join(real, 'ios/App/HideShell/Sources'), join(copy, 'ios/App/HideShell/Sources'), { recursive: true });
  for (const file of ['capacitor.settings.gradle', 'settings.gradle', 'app/build.gradle', 'app/src/main/java/me/grab/hide/android']) {
    await cp(join(real, 'android', file), join(copy, 'android', file), { recursive: true });
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

  it('refuses a Capacitor version that differs between the places it is written', async () => {
    await edit('ios/App/HideShell/Package.swift', 'exact: "8.6.0"', 'exact: "8.5.3"');
    expect(await check(copy)).toEqual([expect.stringContaining('versions disagree')]);
  });

  it('refuses an Android Capacitor library of another version', async () => {
    await edit('android/capacitor.settings.gradle', '@capacitor+android@8.6.0_', '@capacitor+android@8.5.3_');
    expect(await check(copy)).toEqual([expect.stringContaining('"capacitor.settings.gradle":"8.5.3"')]);
  });

  it('refuses a plugin added to the package, to the plugin package or to the native package', async () => {
    await edit('package.json', '"@capacitor/core": "8.6.0",', '"@capacitor/core": "8.6.0",\n    "@capacitor/keyboard": "8.0.0",');
    expect(await check(copy)).toEqual([expect.stringContaining('@capacitor/keyboard')]);
    await edit('ios/App/CapApp-SPM/Package.swift', '.product(name: "Cordova", package: "capacitor-swift-pm")', '.product(name: "Cordova", package: "capacitor-swift-pm"),\n                .product(name: "CapacitorKeyboard", package: "capacitor-swift-pm")');
    expect((await check(copy)).join('\n')).toContain('CapApp-SPM links Capacitor, CapacitorKeyboard, Cordova');
  });

  it('refuses a local package the app does not name', async () => {
    await edit('ios/App/HideShell/Package.swift', '.package(path: "../HidePushCore")', '.package(path: "../HidePushCore"),\n        .package(path: "../Other")');
    expect((await check(copy)).join('\n')).toContain('HideShell links the local packages ../HidePushCore, ../Other; it may link ../HidePushCore');
  });

  it('refuses a plugin class the gate does not list, and a name the gate lists that no class has', async () => {
    await edit('ios/App/HideShell/Sources/HideShell/HideScannerPlugin.swift', 'public let jsName = "HideScanner"', 'public let jsName = "HideScanner2"');
    expect((await check(copy)).join('\n')).toContain('BridgeGate.appPlugins is Console, HideApp, HideKeychain, HidePush, HideScanner;');
    await edit('ios/App/HideShell/Sources/HideShell/HideScannerPlugin.swift', 'public let jsName = "HideScanner2"', 'public let jsName = "HideScanner"');
    await edit('ios/App/HideShell/Sources/HideShell/BridgeGate.swift', '"HidePush"]', '"HidePush", "HideAnything"]');
    expect((await check(copy)).join('\n')).toContain('HideAnything');
  });

  it('refuses an Android plugin class the gate does not list, and a name the gate lists that no class has', async () => {
    const scanner = 'android/app/src/main/java/me/grab/hide/android/HideScannerPlugin.java';
    await edit(scanner, '@CapacitorPlugin(name = "HideScanner")', '@CapacitorPlugin(name = "HideScanner2")');
    expect(await check(copy)).toEqual(['BridgeGate.APP_PLUGINS is HideApp, HideKeychain, HideScanner; the plugin classes of the Android app are HideApp, HideKeychain, HideScanner2']);
    await edit(scanner, '@CapacitorPlugin(name = "HideScanner2")', '@CapacitorPlugin(name = "HideScanner")');
    await edit('android/app/src/main/java/me/grab/hide/android/BridgeGate.java', 'Set.of("HideKeychain", "HideScanner", "HideApp")', 'Set.of("HideKeychain", "HideScanner", "HideApp", "HidePush")');
    expect(await check(copy)).toEqual([expect.stringContaining('BridgeGate.APP_PLUGINS is HideApp, HideKeychain, HidePush, HideScanner;')]);
  });

  it('refuses an npm plugin that cap sync would link into the Android app', async () => {
    await edit('android/capacitor.settings.gradle', "include ':capacitor-android'", "include ':capacitor-android'\ninclude ':capacitor-keyboard'");
    expect(await check(copy)).toEqual([expect.stringContaining('capacitor.settings.gradle includes :capacitor-android, :capacitor-keyboard')]);
  });

  it('refuses an Android app that applies capacitor.build.gradle or links a library the list does not name', async () => {
    const app = 'android/app/build.gradle';
    await edit(app, "apply plugin: 'com.android.application'", "apply plugin: 'com.android.application'\napply from: 'capacitor.build.gradle'");
    expect(await check(copy)).toEqual([expect.stringContaining('applies capacitor.build.gradle')]);
    await edit(app, "apply from: 'capacitor.build.gradle'", '');
    await edit(app, '    implementation "androidx.webkit:webkit:$androidxWebkitVersion"', '    implementation "androidx.webkit:webkit:$androidxWebkitVersion"\n    implementation "com.example:tracker:1.0"');
    expect(await check(copy)).toEqual([expect.stringContaining('com.example:tracker')]);
  });
});
