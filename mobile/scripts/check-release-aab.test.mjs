// check-release-aab.py against an unpacked bundle written here in the form the build writes it: the manifest and the XML
// resources as aapt2's compiled XmlNode protocol buffers, so the check's own reader is what is tested. A bundle as the
// project makes it passes; each test spoils one fact and requires the check that holds it to fail. The mini's run of
// `build-android.sh release` is what proves the reader against Gradle's real output.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const script = resolve(here, 'check-release-aab.py');
const expectFile = resolve(here, '../android-release-expect.json');
const want = JSON.parse(readFileSync(expectFile, 'utf8'));
const ANDROID = 'http://schemas.android.com/apk/res/android';
const PAGE = '<html>app</html>';
const VERSION = '1.0.0';
const CODE = '29909243';

// ---- aapt2's XmlNode, written the way aapt2 writes it --------------------------------------------------------------
const varint = (value) => {
  const out = [];
  do { let byte = value % 128; value = Math.floor(value / 128); if (value) byte |= 0x80; out.push(byte); } while (value);
  return Buffer.from(out);
};
const bytes = (number, body) => Buffer.concat([varint(number * 8 + 2), varint(body.length), body]);
const text = (number, value) => bytes(number, Buffer.from(value));
// SourcePosition {line, column}, field 3 of an XmlNode and field 4 of an XmlAttribute.
const position = (number = 3) => bytes(number, Buffer.concat([varint(8), varint(4), varint(16), varint(2)]));
/** `node('activity', {'android:name': 'x'}, [children])`; an `android:` key is in the Android namespace. */
const node = (name, attributes = {}, children = []) => ({ name, attributes, children });
function element({ name, attributes, children }) {
  const parts = [text(3, name)];
  for (const [key, value] of Object.entries(attributes)) {
    const android = key.startsWith('android:');
    // As aapt2 writes it: namespace, name, the raw value, a source position, a resource id and the compiled value.
    parts.push(bytes(4, Buffer.concat([android ? text(1, ANDROID) : Buffer.alloc(0), text(2, android ? key.slice(8) : key), text(3, String(value)),
      position(4), android ? Buffer.concat([varint(5 * 8), varint(0x0101021b)]) : Buffer.alloc(0), bytes(6, Buffer.from([0x3a, 0x02, 0x08, 0x01]))])));
  }
  for (const child of children) {
    parts.push(bytes(5, text(2, '\n    '))); // the whitespace between elements is a text node
    parts.push(bytes(5, Buffer.concat([bytes(1, element(child)), position()])));
  }
  return Buffer.concat(parts);
}
const compiled = (root) => Buffer.concat([bytes(1, element(root)), position()]);

// ---- a release bundle as the project makes it ----------------------------------------------------------------------
let root;
let aab;
let signers;
const write = (path, content) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, content); };

function manifest(change = (m) => m) {
  const m = {
    manifest: { package: want.package, 'android:versionCode': CODE, 'android:versionName': VERSION, 'android:compileSdkVersion': '36' },
    sdk: { 'android:minSdkVersion': '28', 'android:targetSdkVersion': '36' },
    permissions: want.permissions.map((name) => node('uses-permission', { 'android:name': name })),
    declared: want.declaredPermissions.map((name) => node('permission', { 'android:name': name, 'android:protectionLevel': '0x00000002' })),
    application: {
      'android:allowBackup': 'false', 'android:dataExtractionRules': '@xml/data_extraction_rules', 'android:networkSecurityConfig': '@xml/network_security_config',
      'android:extractNativeLibs': 'false', 'android:label': '@string/app_name',
    },
    components: [
      node('activity', { 'android:exported': 'true', 'android:name': 'me.grab.hide.android.ShellActivity', 'android:launchMode': '2' }, [
        node('intent-filter', {}, [node('action', { 'android:name': 'android.intent.action.MAIN' })]),
        node('intent-filter', { 'android:autoVerify': 'true' }, [node('data', { 'android:host': 'app.withhide.dev', 'android:scheme': 'https' })]),
      ]),
      node('activity', { 'android:exported': 'false', 'android:name': 'com.google.mlkit.vision.codescanner.internal.GmsBarcodeScanningDelegateActivity' }),
      node('provider', { 'android:authorities': 'me.grab.hide.android.androidx-startup', 'android:exported': 'false', 'android:name': 'androidx.startup.InitializationProvider' }),
      node('service', { 'android:exported': 'false', 'android:name': 'com.google.android.datatransport.runtime.scheduling.jobscheduling.JobInfoSchedulerService', 'android:permission': 'android.permission.BIND_JOB_SERVICE' }),
      node('receiver', { 'android:exported': 'true', 'android:name': 'androidx.profileinstaller.ProfileInstallReceiver', 'android:permission': 'android.permission.DUMP' }, [
        node('intent-filter', {}, [node('action', { 'android:name': 'androidx.profileinstaller.action.INSTALL_PROFILE' })]),
      ]),
      // The notifications: the app's own message service and tap activity, and Firebase's receiver, which only Google
      // Play services (the holder of c2dm.permission.SEND) can reach.
      node('activity', { 'android:exported': 'false', 'android:name': 'me.grab.hide.android.NoticeOpenActivity' }),
      node('service', { 'android:exported': 'false', 'android:name': 'me.grab.hide.android.HidePushService' }, [
        node('intent-filter', {}, [node('action', { 'android:name': 'com.google.firebase.MESSAGING_EVENT' })]),
      ]),
      node('receiver', { 'android:exported': 'true', 'android:name': 'com.google.firebase.iid.FirebaseInstanceIdReceiver', 'android:permission': 'com.google.android.c2dm.permission.SEND' }, [
        node('intent-filter', {}, [node('action', { 'android:name': 'com.google.android.c2dm.intent.RECEIVE' })]),
      ]),
    ],
  };
  change(m);
  return compiled(node('manifest', m.manifest, [
    node('uses-sdk', m.sdk), ...m.permissions, ...m.declared, node('application', m.application, m.components),
  ]));
}
const securityConfig = (children = [node('base-config', { cleartextTrafficPermitted: 'false' })]) => compiled(node('network-security-config', {}, children));
const domains = want.excludedDomains;
const extractionRules = (cloud = domains, transfer = domains, extra = []) => compiled(node('data-extraction-rules', {}, [
  node('cloud-backup', {}, cloud.map((domain) => node('exclude', { domain, path: '.' }))),
  node('device-transfer', {}, [...transfer.map((domain) => node('exclude', { domain, path: '.' })), ...extra]),
]));
const capacitorConfig = (change = (c) => c) => JSON.stringify(change({ appId: 'me.grab.hide.ios', appName: 'Hide', webDir: 'www', loggingBehavior: 'none', server: { iosScheme: 'capacitor', hostname: 'localhost', allowNavigation: [] }, ios: { webContentsDebuggingEnabled: false } }));

function fixture() {
  write(join(aab, 'base/manifest/AndroidManifest.xml'), manifest());
  write(join(aab, 'base/res/xml/network_security_config.xml'), securityConfig());
  write(join(aab, 'base/res/xml/data_extraction_rules.xml'), extractionRules());
  write(join(aab, 'base/res/xml/config.xml'), compiled(node('widget')));
  write(join(aab, 'base/assets/capacitor.config.json'), capacitorConfig());
  write(join(aab, 'base/assets/public/index.html'), PAGE);
  write(join(aab, 'base/assets/public/p/5/index.html'), PAGE);
  write(join(aab, 'base/dex/classes.dex'), 'dex\n035 me/grab/hide/android/ShellActivity BridgeGate PairLink');
  write(join(aab, 'base/root/META-INF/x.version'), '1');
  write(join(aab, 'BUNDLE-METADATA/com.android.tools/x'), '1');
  write(join(aab, 'META-INF/UPLOAD.RSA'), 'signature');
  write(signers, `${want.uploadCertificateSha256}\n`);
  const sha = createHash('sha256').update(PAGE).digest('hex');
  write(join(root, 'manifest.json'), JSON.stringify({ schema: 1, files: { 'index.html': sha, 'p/5/index.html': sha } }));
}
const run = () => spawnSync('python3', ['-I', script, aab, signers, expectFile, join(root, 'manifest.json'), VERSION, CODE], { encoding: 'utf8' });

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'aab-check-'));
  aab = join(root, 'aab');
  signers = join(root, 'signers.txt');
  fixture();
});
afterEach(() => { rmSync(root, { recursive: true, force: true }); });

describe('the release gate on the signed bundle', () => {
  it('passes a release bundle as the project makes it', () => {
    const result = run();
    expect(result.stderr).toBe('');
    expect(result.status).toBe(0);
    expect(result.stdout).toContain(`release checks passed for ${VERSION} (${CODE})`);
  });

  const putManifest = (change) => () => write(join(aab, 'base/manifest/AndroidManifest.xml'), manifest(change));
  const putSecurity = (children) => () => write(join(aab, 'base/res/xml/network_security_config.xml'), securityConfig(children));
  const putConfig = (change) => () => write(join(aab, 'base/assets/capacitor.config.json'), capacitorConfig(change));
  const base = node('base-config', { cleartextTrafficPermitted: 'false' });
  const component = (tag, attributes, children) => putManifest((m) => { m.components.push(node(tag, attributes, children)); });
  const spoil = {
    'another package': [putManifest((m) => { m.manifest.package = 'me.example.other'; }), 'package is'],
    'another version': [putManifest((m) => { m.manifest['android:versionName'] = '9.9.9'; }), 'version is'],
    'another version code': [putManifest((m) => { m.manifest['android:versionCode'] = '1'; }), 'version code is'],
    'an older minimum Android': [putManifest((m) => { m.sdk['android:minSdkVersion'] = '24'; }), 'minimum SDK'],
    'another target SDK': [putManifest((m) => { m.sdk['android:targetSdkVersion'] = '35'; }), 'target SDK'],
    'a feature module beside base': [() => write(join(aab, 'feature/manifest/AndroidManifest.xml'), manifest()), 'base module only'],
    'a debuggable app': [putManifest((m) => { m.application['android:debuggable'] = 'true'; }), 'not debuggable'],
    'a test-only app': [putManifest((m) => { m.application['android:testOnly'] = 'true'; }), 'test-only'],
    'cleartext allowed in the manifest': [putManifest((m) => { m.application['android:usesCleartextTraffic'] = 'true'; }), 'allows no cleartext'],
    'no network security configuration': [putManifest((m) => { delete m.application['android:networkSecurityConfig']; }), 'networkSecurityConfig'],
    'cleartext by default': [putSecurity([node('base-config', { cleartextTrafficPermitted: 'true' })]), 'refuses cleartext by default'],
    'no base configuration': [putSecurity([]), 'refuses cleartext by default'],
    'cleartext for one domain (the debug build\'s fixture exception)': [putSecurity([base, node('domain-config', { cleartextTrafficPermitted: 'true' }, [node('domain', { includeSubdomains: 'false' })])]), 'permits cleartext (domain-config)'],
    'debug overrides': [putSecurity([base, node('debug-overrides')]), 'debug overrides'],
    'trusted user certificates': [putSecurity([base, node('domain-config', {}, [node('trust-anchors', {}, [node('certificates', { src: 'user' })])])]), 'user certificates'],
    'a qualified copy of the configuration': [() => write(join(aab, 'base/res/xml-v24/network_security_config.xml'), securityConfig([node('base-config', { cleartextTrafficPermitted: 'true' })])), 'no qualified copies (xml-v24)'],
    'a configuration that cannot be read': [() => write(join(aab, 'base/res/xml/network_security_config.xml'), Buffer.from([0x0a, 0x7f, 0x01])), 'cannot be read as compiled XML'],
    'backup on': [putManifest((m) => { m.application['android:allowBackup'] = 'true'; }), 'backup is off'],
    'backup left to its default': [putManifest((m) => { delete m.application['android:allowBackup']; }), 'backup is off'],
    'no data extraction rules': [putManifest((m) => { delete m.application['android:dataExtractionRules']; }), 'dataExtractionRules'],
    'a domain left in the cloud backup': [() => write(join(aab, 'base/res/xml/data_extraction_rules.xml'), extractionRules(domains.filter((d) => d !== 'sharedpref'))), 'cloud-backup excludes every domain (missing: sharedpref)'],
    'an include in the device transfer': [() => write(join(aab, 'base/res/xml/data_extraction_rules.xml'), extractionRules(domains, domains, [node('include', { domain: 'file', path: 'pairing' })])), 'device-transfer includes nothing'],
    'an extra permission': [putManifest((m) => { m.permissions.push(node('uses-permission', { 'android:name': 'android.permission.CAMERA' })); }), 'asks for exactly the permissions'],
    'an extra permission for Android 6 and later': [putManifest((m) => { m.permissions.push(node('uses-permission-sdk-23', { 'android:name': 'com.google.android.gms.permission.AD_ID' })); }), 'asks for exactly the permissions'],
    'a permission missing': [putManifest((m) => { m.permissions.pop(); }), 'asks for exactly the permissions'],
    'another declared permission': [putManifest((m) => { m.declared.push(node('permission', { 'android:name': 'me.grab.hide.android.OTHER' })); }), 'declares exactly the permissions'],
    'an exported service': [component('service', { 'android:exported': 'true', 'android:name': 'me.example.Leak' }), 'exported components are exactly'],
    'a component exported by an intent filter alone': [component('receiver', { 'android:name': 'me.example.Implicit' }, [node('intent-filter')]), 'exported components are exactly'],
    'the receiver\'s permission dropped': [putManifest((m) => { delete m.components[4].attributes['android:permission']; }), 'exported components are exactly'],
    'the notification receiver open to every app': [putManifest((m) => { delete m.components[7].attributes['android:permission']; }), 'exported components are exactly'],
    'web view inspection on': [putConfig((c) => ({ ...c, android: { webContentsDebuggingEnabled: true } })), 'cannot be inspected'],
    'Capacitor logging on': [putConfig((c) => ({ ...c, loggingBehavior: 'debug' })), 'logging is none'],
    'a remote page': [putConfig((c) => ({ ...c, server: { url: 'http://192.168.0.2:5173' } })), 'server url'],
    'cleartext in Capacitor': [putConfig((c) => ({ ...c, server: { cleartext: true } })), 'server url'],
    'allowed navigation': [putConfig((c) => ({ ...c, server: { allowNavigation: ['example.test'] } })), 'allows no navigation'],
    'mixed content': [putConfig((c) => ({ ...c, android: { allowMixedContent: true } })), 'mixed content'],
    'the debug pairing hook in the code': [() => write(join(aab, 'base/dex/classes2.dex'), 'getBooleanExtra hide_debug_connect'), 'holds no debug hook (hide_debug_)'],
    'the debug test activity in the code': [() => write(join(aab, 'base/dex/classes.dex'), 'dex me/grab/hide/android/ShellTestActivity'), 'holds no debug hook (ShellTestActivity)'],
    'no code': [() => rmSync(join(aab, 'base/dex'), { recursive: true }), 'holds code'],
    'a native library': [() => write(join(aab, 'base/lib/arm64-v8a/libbarhopper_v3.so'), 'ELF'), 'native libraries are exactly'],
    'a web file changed after assembly': [() => write(join(aab, 'base/assets/public/p/5/index.html'), '<html>other</html>'), 'public/p/5/index.html differs'],
    'a web file nobody assembled': [() => write(join(aab, 'base/assets/public/extra.js'), 'x'), 'holds different files'],
    'no web bundle': [() => rmSync(join(aab, 'base/assets/public'), { recursive: true }), 'no assets/public/'],
    'another signing key': [() => write(signers, `${'AB:'.repeat(31)}AB\n`), 'signed by the upload key alone'],
    'a second signer beside the upload key': [() => write(signers, `${want.uploadCertificateSha256}\n${'CD:'.repeat(31)}CD\n`), 'signed by the upload key alone'],
  };
  for (const [what, [change, message]] of Object.entries(spoil)) {
    it(`fails on ${what}`, () => {
      change();
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain(message);
      expect(result.stdout).not.toContain('passed');
    });
  }

  it('stops when a fact is missing rather than passing it', () => {
    rmSync(signers);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('is missing or empty');
  });

  it('lists every failure at once', () => {
    putManifest((m) => { m.application['android:debuggable'] = 'true'; m.permissions.push(node('uses-permission', { 'android:name': 'android.permission.CAMERA' })); })();
    write(signers, `${'AB:'.repeat(31)}AB\n`);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('3 release check(s) failed');
  });
});
