// Two facts about what is linked into the iOS and the Android app, checked without Xcode or Gradle so every pull
// request can run them.
//
// 1. The Capacitor version is written in places that must agree: this package, the plugin package `cap sync` manages
//    and the iOS app's own native package, and the library folder `cap sync android` points Gradle at. A mismatch builds
//    two bridges into one app.
// 2. Each app links Capacitor itself and no plugin. A plugin runs with the whole app's rights and is reachable from
//    every script the app serves, so adding one is a change to this list, made with the app's gate list
//    (`BridgeGate.appPlugins` on iOS, `BridgeGate.APP_PLUGINS` on Android) in the same pull request, never a side effect
//    of an install.
import { readFile, readdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const LINKED_PACKAGES = ['@capacitor/android', '@capacitor/cli', '@capacitor/core', '@capacitor/ios'];
export const LINKED_SWIFT_PRODUCTS = ['Capacitor', 'Cordova'];
// What the Android app links (mobile/android/app/build.gradle), beside its tests' own libraries: Capacitor's library,
// the AndroidX parts the shell calls and Google's code scanner.
export const LINKED_ANDROID_LIBRARIES = [
  "project(':capacitor-android')",
  'androidx.appcompat:appcompat',
  'androidx.webkit:webkit',
  'com.google.android.gms:play-services-code-scanner',
];
const pin = /capacitor-swift-pm\.git",\s*exact:\s*"([^"]+)"/;

/** @returns {Promise<string[]>} what is wrong, empty when nothing is */
export async function check(root = ROOT) {
  const problems = [];
  const manifest = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
  const capApp = await readFile(resolve(root, 'ios/App/CapApp-SPM/Package.swift'), 'utf8');
  const shell = await readFile(resolve(root, 'ios/App/HideShell/Package.swift'), 'utf8');

  const androidSettings = await readFile(resolve(root, 'android/capacitor.settings.gradle'), 'utf8');
  const found = {
    'package.json @capacitor/android': manifest.dependencies['@capacitor/android'],
    'package.json @capacitor/ios': manifest.dependencies['@capacitor/ios'],
    'package.json @capacitor/core': manifest.dependencies['@capacitor/core'],
    'package.json @capacitor/cli': manifest.dependencies['@capacitor/cli'],
    'CapApp-SPM': capApp.match(pin)?.[1],
    HideShell: shell.match(pin)?.[1],
    'capacitor.settings.gradle': androidSettings.match(/@capacitor\+android@([0-9][^_/']*)/)?.[1],
  };
  if (new Set(Object.values(found)).size !== 1 || Object.values(found).includes(undefined)) problems.push(`Capacitor versions disagree: ${JSON.stringify(found)}`);

  const dependencies = Object.keys(manifest.dependencies).sort();
  if (JSON.stringify(dependencies) !== JSON.stringify(LINKED_PACKAGES)) problems.push(`mobile/package.json links ${dependencies.join(', ')}; the app links only ${LINKED_PACKAGES.join(', ')}`);

  // The app's own local packages are the only other thing HideShell links: the push core the notification extension
  // shares (no Capacitor, no network, no plugin), named here so a new one is a change to this list.
  const LOCAL_PACKAGES = { 'CapApp-SPM': [], HideShell: ['../HidePushCore'] };
  for (const [name, text] of [['CapApp-SPM', capApp], ['HideShell', shell]]) {
    const packages = [...text.matchAll(/\.package\(url:\s*"([^"]+)"/g)].map((match) => match[1]);
    if (packages.length !== 1 || !packages[0].endsWith('/capacitor-swift-pm.git')) problems.push(`${name} depends on ${packages.join(', ') || 'nothing'}; the app depends on capacitor-swift-pm alone`);
    const local = [...text.matchAll(/\.package\(path:\s*"([^"]+)"/g)].map((match) => match[1]);
    if (JSON.stringify(local) !== JSON.stringify(LOCAL_PACKAGES[name])) problems.push(`${name} links the local packages ${local.join(', ') || 'none'}; it may link ${LOCAL_PACKAGES[name].join(', ') || 'none'}`);
    const products = [...new Set([...text.matchAll(/\.product\(name:\s*"([^"]+)",\s*package:\s*"capacitor-swift-pm"/g)].map((match) => match[1]))].sort();
    if (JSON.stringify(products) !== JSON.stringify(LINKED_SWIFT_PRODUCTS)) problems.push(`${name} links ${products.join(', ')}; the app links only ${LINKED_SWIFT_PRODUCTS.join(', ')}`);
  }

  // The plugins a page may call (BridgeGate.appPlugins) and the plugin classes this package links are one list: a
  // plugin added to either alone is either unreachable or, worse, reachable without anyone having decided it.
  const sources = resolve(root, 'ios/App/HideShell/Sources/HideShell');
  const gate = await readFile(resolve(sources, 'BridgeGate.swift'), 'utf8');
  const exposed = [...(gate.match(/appPlugins: Set<String> = \[([^\]]*)\]/)?.[1] ?? '').matchAll(/"([^"]+)"/g)].map((match) => match[1]).sort();
  const linked = ['Console'];
  for (const file of (await readdir(sources)).filter((name) => name.endsWith('.swift'))) {
    linked.push(...[...(await readFile(resolve(sources, file), 'utf8')).matchAll(/public let jsName = "([^"]+)"/g)].map((match) => match[1]));
  }
  if (JSON.stringify(exposed) !== JSON.stringify(linked.sort())) problems.push(`BridgeGate.appPlugins is ${exposed.join(', ')}; the plugin classes in HideShell are ${linked.join(', ')} (Console is Capacitor's own)`);
  problems.push(...await checkAndroid(root, androidSettings));
  return problems;
}

/**
 * The Android app: Gradle builds only the app and Capacitor's library (`cap sync` writes an npm plugin's project into
 * capacitor.settings.gradle and its dependency into app/capacitor.build.gradle, which the app does not apply), the app
 * links the fixed library list, and the plugins its gate lets the page call are the plugin classes it defines.
 */
async function checkAndroid(root, capacitorSettings) {
  const problems = [];
  const projects = [...capacitorSettings.matchAll(/include\s+'([^']+)'/g)].map((match) => match[1]);
  if (JSON.stringify(projects) !== JSON.stringify([':capacitor-android'])) problems.push(`capacitor.settings.gradle includes ${projects.join(', ') || 'nothing'}; the Android app builds only :capacitor-android, no npm plugin`);
  const settings = await readFile(resolve(root, 'android/settings.gradle'), 'utf8');
  const included = [...settings.matchAll(/include\s+'([^']+)'/g)].map((match) => match[1]);
  if (JSON.stringify(included) !== JSON.stringify([':app'])) problems.push(`android/settings.gradle includes ${included.join(', ')}; it includes :app and Capacitor's settings alone`);
  const app = await readFile(resolve(root, 'android/app/build.gradle'), 'utf8');
  if (/apply\s+from:\s*['"]capacitor\.build\.gradle['"]/.test(app)) problems.push('android/app/build.gradle applies capacitor.build.gradle, through which `cap sync` links npm plugins');
  const libraries = [...app.matchAll(/^\s*implementation\s+(project\('[^']+'\)|["']([^:"']+:[^:"']+)[:"'])/gm)].map((match) => match[2] ?? match[1]).sort();
  if (JSON.stringify(libraries) !== JSON.stringify([...LINKED_ANDROID_LIBRARIES].sort())) problems.push(`android/app/build.gradle links ${libraries.join(', ')}; the Android app links only ${LINKED_ANDROID_LIBRARIES.join(', ')}`);

  const sources = resolve(root, 'android/app/src/main/java/me/grab/hide/android');
  const gate = await readFile(resolve(sources, 'BridgeGate.java'), 'utf8');
  const exposed = [...(gate.match(/APP_PLUGINS = Set\.of\(([^)]*)\)/)?.[1] ?? '').matchAll(/"([^"]+)"/g)].map((match) => match[1]).sort();
  const defined = [];
  for (const file of (await readdir(sources)).filter((name) => name.endsWith('.java'))) {
    defined.push(...[...(await readFile(resolve(sources, file), 'utf8')).matchAll(/@CapacitorPlugin\(\s*name\s*=\s*"([^"]+)"/g)].map((match) => match[1]));
  }
  if (!exposed.length || JSON.stringify(exposed) !== JSON.stringify(defined.sort())) problems.push(`BridgeGate.APP_PLUGINS is ${exposed.join(', ') || 'unreadable'}; the plugin classes of the Android app are ${defined.join(', ')}`);
  return problems;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const problems = await check();
  for (const problem of problems) console.error(problem);
  if (problems.length) process.exitCode = 1;
}
