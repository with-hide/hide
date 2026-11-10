import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const script = resolve(here, 'check-release-app.sh');
const expectFile = resolve(here, '../release-expect.json');
const want = JSON.parse(readFileSync(expectFile, 'utf8'));

const PAGE = '<html>app</html>';
let root;
let facts;

const write = (path, content) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, typeof content === 'string' ? content : JSON.stringify(content)); };

/** A signed app as the build machine reads it, in the form check-release-app.sh takes: good until a test spoils one fact. */
function fixture() {
  const app = join(root, 'Payload/App.app');
  write(join(app, 'App'), 'binary text BridgeGate more binary text');
  write(join(app, 'PrivacyInfo.xcprivacy'), 'plist');
  write(join(app, 'capacitor.config.json'), { appId: want.bundleId, loggingBehavior: 'none', server: { iosScheme: 'capacitor', hostname: 'localhost', allowNavigation: [] }, ios: { webContentsDebuggingEnabled: false } });
  write(join(app, 'public/index.html'), PAGE);
  write(join(app, 'public/p/3/index.html'), PAGE);
  facts = join(root, 'facts');
  write(join(facts, 'info.json'), {
    CFBundleIdentifier: want.bundleId, CFBundleExecutable: 'App', CFBundleShortVersionString: '1.2.5', CFBundleVersion: '20261110120000',
    CAPACITOR_DEBUG: 'false', MinimumOSVersion: '15.0', DTSDKName: 'iphoneos26.5', UIDeviceFamily: [1],
  });
  write(join(facts, 'privacy.json'), { NSPrivacyTracking: false, NSPrivacyTrackingDomains: [], NSPrivacyCollectedDataTypes: [], NSPrivacyAccessedAPITypes: [] });
  write(join(facts, 'entitlements.json'), want.entitlements);
  write(join(facts, 'profile.json'), { TeamIdentifier: [want.teamId], Platform: ['iOS'], Entitlements: want.entitlements });
  const sha = createHash('sha256').update(PAGE).digest('hex');
  write(join(root, 'manifest.json'), { schema: 1, files: { 'index.html': sha, 'p/3/index.html': sha } });
  return app;
}

const run = (app, options = {}) => spawnSync('bash', [script, app, facts, options.expect ?? expectFile, join(root, 'manifest.json'), '1.2.5', '20261110120000'], { encoding: 'utf8' });
const edit = (name, change) => { const path = join(facts, name); const value = JSON.parse(readFileSync(path, 'utf8')); change(value); writeFileSync(path, JSON.stringify(value)); };

beforeEach(() => { root = mkdtempSync(join(tmpdir(), 'release-check-')); });
afterEach(() => { rmSync(root, { recursive: true, force: true }); });

describe('the release gate on the signed app', () => {
  it('passes a release build as the project makes it, whatever other platforms its profile lists', () => {
    const app = fixture();
    edit('profile.json', (v) => { v.Platform = ['iOS', 'xrOS', 'visionOS']; });
    const result = run(app);
    expect(result.stderr).toBe('');
    expect(result.status).toBe(0);
    expect(result.stdout).toContain('release checks passed');
  });

  const spoil = {
    'a debug switch left on': [(app) => edit('info.json', (v) => { v.CAPACITOR_DEBUG = 'true'; }), 'CAPACITOR_DEBUG'],
    'an App Transport Security exception': [(app) => edit('info.json', (v) => { v.NSAppTransportSecurity = { NSAllowsArbitraryLoads: true }; }), 'App Transport Security'],
    'an export compliance answer given by the app': [(app) => edit('info.json', (v) => { v.ITSAppUsesNonExemptEncryption = false; }), 'export compliance question'],
    'an export compliance code given by the app': [(app) => edit('info.json', (v) => { v.ITSEncryptionExportComplianceCode = 'x'; }), 'export compliance question'],
    'another bundle id': [(app) => edit('info.json', (v) => { v.CFBundleIdentifier = 'me.example.other'; }), 'bundle id is'],
    'another version': [(app) => edit('info.json', (v) => { v.CFBundleShortVersionString = '9.9.9'; }), 'version is'],
    'an iPad build': [(app) => edit('info.json', (v) => { v.UIDeviceFamily = [1, 2]; }), 'iPhone only'],
    'an older minimum iOS': [(app) => edit('info.json', (v) => { v.MinimumOSVersion = '13.0'; }), 'minimum iOS'],
    'allowed navigation': [(app) => write(join(app, 'capacitor.config.json'), { loggingBehavior: 'none', server: { allowNavigation: ['https://example.test'] } }), 'allows no navigation'],
    'cleartext traffic': [(app) => write(join(app, 'capacitor.config.json'), { loggingBehavior: 'none', server: { cleartext: true } }), 'server url'],
    'an older SDK': [(app) => edit('info.json', (v) => { v.DTSDKName = 'iphoneos18.5'; }), 'SDK'],
    'another build number': [(app) => edit('info.json', (v) => { v.CFBundleVersion = '1'; }), 'build number'],
    'Capacitor logging on': [(app) => write(join(app, 'capacitor.config.json'), { loggingBehavior: 'debug' }), 'logging'],
    'a remote server url': [(app) => write(join(app, 'capacitor.config.json'), { loggingBehavior: 'none', server: { url: 'https://example.test' } }), 'server url'],
    'an inspectable web view': [(app) => write(join(app, 'capacitor.config.json'), { loggingBehavior: 'none', ios: { webContentsDebuggingEnabled: true } }), 'inspected'],
    'the debug pairing hook in the binary': [(app) => write(join(app, 'App'), 'BridgeGate HIDE_DEBUG_PAIR_LINK'), 'debug pairing hook'],
    'no bridge gate in the binary': [(app) => write(join(app, 'App'), 'binary only'), 'bridge gate'],
    'no privacy manifest': [(app) => rmSync(join(app, 'PrivacyInfo.xcprivacy')), 'PrivacyInfo.xcprivacy'],
    'tracking in the privacy manifest': [(app) => edit('privacy.json', (v) => { v.NSPrivacyTracking = true; }), 'no tracking'],
    'an API declared with no reason': [(app) => edit('privacy.json', (v) => { v.NSPrivacyAccessedAPITypes = [{ NSPrivacyAccessedAPIType: 'x', NSPrivacyAccessedAPITypeReasons: [] }]; }), 'reason'],
    'an entitlement nobody listed': [(app) => edit('entitlements.json', (v) => { v['aps-environment'] = 'production'; }), 'entitlements differ'],
    'a development signature': [(app) => edit('entitlements.json', (v) => { v['get-task-allow'] = true; }), 'entitlements differ'],
    'a profile with a device list': [(app) => edit('profile.json', (v) => { v.ProvisionedDevices = ['device']; }), 'App Store profile'],
    'a profile for another platform': [(app) => edit('profile.json', (v) => { v.Platform = ['macOS']; }), 'for iOS'],
    'a profile of another team': [(app) => edit('profile.json', (v) => { v.TeamIdentifier = ['XXXXXXXXXX']; }), 'team'],
    'a bundle file nobody assembled': [(app) => write(join(app, 'public/extra.js'), 'x'), 'public/'],
    'a bundle file that is missing': [(app) => rmSync(join(app, 'public/p/3/index.html')), 'public/'],
    'a bundle file that changed': [(app) => write(join(app, 'public/index.html'), '<html>other</html>'), 'differs from the bundle manifest'],
  };
  for (const [name, [change, message]] of Object.entries(spoil)) {
    it(`refuses ${name}`, () => {
      const app = fixture();
      change(app);
      const result = run(app);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain(message);
      expect(result.stdout).not.toContain('release checks passed');
    });
  }

  it('holds the entitlements to the list a node extends', () => {
    const app = fixture();
    const extended = { ...want, entitlements: { ...want.entitlements, 'aps-environment': 'production' } };
    write(join(root, 'extended.json'), extended);
    expect(run(app, { expect: join(root, 'extended.json') }).status).toBe(1);
    edit('entitlements.json', (v) => { v['aps-environment'] = 'production'; });
    expect(run(app, { expect: join(root, 'extended.json') }).status).toBe(0);
  });

  it('reports every failed check, not the first', () => {
    const app = fixture();
    edit('info.json', (v) => { v.CAPACITOR_DEBUG = 'true'; v.MinimumOSVersion = '12.0'; });
    write(join(app, 'App'), 'HIDE_DEBUG_PAIR_LINK');
    const result = run(app);
    expect(result.stderr.match(/release check failed/g).length).toBeGreaterThanOrEqual(4);
  });

  it('stops when a fact was never read', () => {
    const app = fixture();
    rmSync(join(facts, 'profile.json'));
    const result = run(app);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('profile.json is missing');
  });
});
