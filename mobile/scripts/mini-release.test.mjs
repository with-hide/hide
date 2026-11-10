// mini-release.sh and mini-ios.sh's lock against stand-ins for the Apple tools, on any system: the stand-ins answer the
// few macOS commands the script calls (`stat -f`, `df -g`), so these run in the checks lane with the rest of mobile/.
// They prove the secrets, the work folder, the lock, the upload record and the time bounds; the Mac mini's own run of
// the command is what proves the real Xcode.
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const mobile = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const KEY_ID = 'ABCDE12345';
const ISSUER = '11111111-2222-3333-4444-555555555555';
const SECRET_LINE = 'c2VjcmV0LWtleS1ib2R5LW5vdC1mb3ItbG9ncw';
const COMMIT = 'a'.repeat(40);
const DELIVERY = '12345678-1234-1234-1234-123456789abc';
const TOKEN = 'eyJhbGciOiJFUzI1NiIsImtpZCI6IkFCQ0RFMTIzNDUifQ.eyJpc3MiOiJ4In0.c2lnbmF0dXJl';

describe('the release command on the build machine', () => {
  let home;
  let bin;
  let accountTmp;
  const base = () => join(home, 'hide-ios-build/branch');
  const keyPath = () => join(home, `.config/hide-ios/AuthKey_${KEY_ID}.p8`);
  const put = (path, text, mode = 0o600) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); chmodSync(path, mode); };
  const stub = (name, body) => put(join(bin, name), `#!/bin/bash\n${body}\n`, 0o755);

  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), 'mini-release-'));
    bin = join(home, 'bin');
    accountTmp = join(home, 'account-tmp');
    mkdirSync(accountTmp);
    mkdirSync(join(base(), 'release-inputs'), { recursive: true });
    for (const file of ['release-env.json', 'release-expect.json']) copyFileSync(join(mobile, file), join(base(), 'release-inputs', file));
    put(join(base(), 'release-inputs/check-release-app.sh'), '#!/bin/bash\nexit 0\n', 0o755); // its own tests are check-release-app.test.mjs
    put(join(base(), 'release-inputs/bundle-manifest.json'), '{"schema":1,"files":{}}');
    put(join(base(), 'ios/App/App/App.entitlements'), '<plist/>');
    put(keyPath(), `-----BEGIN PRIVATE KEY-----\n${SECRET_LINE}\n-----END PRIVATE KEY-----\n`);
    put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\nHIDE_IOS_ASC_ISSUER_ID=${ISSUER}\nHIDE_IOS_ASC_KEY_PATH=${keyPath()}\n`);

    // The macOS commands the script asks questions of.
    stub('stat', 'f=${@: -1}; case "$*" in *%Lp*) cat "$f.mode" 2>/dev/null || echo 600;; *%u*) id -u;; esac');
    stub('df', 'echo "Filesystem 1G-blocks Used Available Capacity iused ifree %iused Mounted"; echo "/dev/x 500 100 400 20% 1 1 1% /"');
    // The Apple tools. xcodebuild prints every argument as the real one does, and writes its distribution log where
    // $TMPDIR says, as the real one does.
    stub('xcodebuild', `
printf 'xcodebuild %s\\n' "$*"
echo "$TMPDIR" > "${home}/xcodebuild-tmpdir"
case "$*" in *-exportArchive*)
  echo "authenticating with ${TOKEN}"
  mkdir -p "$TMPDIR/App_x.xcdistributionlogs" && echo "${KEY_ID} ${ISSUER} ${TOKEN}" > "$TMPDIR/App_x.xcdistributionlogs/log"
  [ -n "$STUB_EXPORT_FAILS" ] && exit 65;;
esac
while [ $# -gt 0 ]; do
  case "$1" in -archivePath) mkdir -p "$2/Products/Applications/App.app";; -exportPath) mkdir -p "$2"; : > "$2/App.ipa";; esac; shift
done
[ -n "$STUB_XCODEBUILD_SECRET" ] && echo "${SECRET_LINE}"
[ -n "$STUB_XCODEBUILD_FAILS" ] && exit 1
exit 0`);
    stub('codesign', 'case "$*" in *-dvv*) echo "Authority=${STUB_AUTHORITY:-Apple Distribution: Hide (GH6F6ZA497)}" >&2;; esac; exit 0');
    stub('unzip', 'dest=${@: -1}; mkdir -p "$dest/Payload/App.app"; : > "$dest/Payload/App.app/Info.plist"; : > "$dest/Payload/App.app/embedded.mobileprovision"');
    stub('plutil', 'case "$1" in -convert) out=$3; echo "{}" > "$out";; esac; exit 0');
    stub('security', 'echo "<plist/>"');
    stub('xcrun', `
case "$*" in
  *--upload-app*) [ -n "$STUB_UPLOAD_HANGS" ] && { sleep 60; exit 0; }; printf 'UPLOAD SUCCEEDED with no errors\\nDelivery UUID: ${DELIVERY}\\n';;
  *--validate-app*) echo "No errors validating archive";;
  *--build-status*) echo "BUILD-STATUS: VALID";;
esac
exit 0`);
  });
  afterEach(() => { rmSync(home, { recursive: true, force: true }); });

  const env = (extra = {}) => ({ PATH: `${bin}:/usr/bin:/bin`, HOME: home, TMPDIR: accountTmp, ...extra });
  const run = (extra = {}, args = ['release', 'branch', '1.2.5', '20261110120000', COMMIT, '0']) =>
    spawnSync('bash', [join(mobile, 'scripts/mini-release.sh'), ...args], { encoding: 'utf8', env: env(extra), timeout: 60000 });
  const everything = (r) => r.stdout + r.stderr;
  const log = () => readFileSync(join(base(), 'release.log'), 'utf8');
  const records = () => readFileSync(join(base(), 'uploads.log'), 'utf8').trim().split('\n');

  describe('the secrets', () => {
    it('stop it before it builds anything when one is missing, naming the key and no value', () => {
      put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\n`);
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_IOS_ASC_ISSUER_ID is missing');
      expect(result.stderr).toContain('HIDE_IOS_ASC_KEY_PATH is missing');
      expect(everything(result)).not.toContain(KEY_ID);
      expect(existsSync(join(base(), 'release'))).toBe(false);
      expect(existsSync(join(base(), 'release.lock'))).toBe(false);
    });

    it('are refused when not shaped as the registry says, without printing the value', () => {
      put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\nHIDE_IOS_ASC_ISSUER_ID=not-an-issuer\nHIDE_IOS_ASC_KEY_PATH=${keyPath()}\n`);
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_IOS_ASC_ISSUER_ID is not shaped');
      expect(everything(result)).not.toContain('not-an-issuer');
    });

    it('are refused when the key file can be read by another account', () => {
      writeFileSync(`${keyPath()}.mode`, '644');
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('other accounts can read');
    });

    it('do not make a key path that names no file appear in the output, whatever the last run left', () => {
      writeFileSync(join(base(), 'release.log'), 'a log from the last run\n');
      rmSync(keyPath());
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('names no file here');
      expect(everything(result)).not.toContain(keyPath());
    });

    it('stay out of the output, the log and the account\'s temporary folder when the signing step fails, and the work goes', () => {
      const result = run({ STUB_EXPORT_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "export and sign"');
      for (const secret of [KEY_ID, ISSUER, keyPath(), SECRET_LINE, TOKEN]) {
        expect(everything(result), secret).not.toContain(secret);
        expect(log(), secret).not.toContain(secret);
      }
      expect(log()).toContain('<HIDE_IOS_ASC_KEY_ID>');
      expect(log()).toContain('<token>');
      expect(readFileSync(join(home, 'xcodebuild-tmpdir'), 'utf8').trim()).toBe(join(base(), 'release/tmp'));
      expect(readdirSync(accountTmp)).toEqual([]);
      expect(existsSync(join(base(), 'release'))).toBe(false);
      expect(existsSync(join(base(), 'release.lock'))).toBe(false);
    });

    it('withhold the end of a failed step\'s log, and delete the log, when a secret reached it that redaction missed', () => {
      const result = run({ STUB_XCODEBUILD_SECRET: '1', STUB_XCODEBUILD_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(everything(result)).not.toContain(SECRET_LINE);
      expect(result.stderr).not.toContain('the end of its log');
      expect(result.stderr).toContain('a secret value reached the log');
      expect(existsSync(join(base(), 'release.log'))).toBe(false);
    });

    it('delete, unread, the distribution logs Xcode wrote in the account temporary folder, on success and on a failed step', () => {
      const writes = `case "$*" in *-exportArchive*) mkdir -p "${accountTmp}/XcodeDistPipeline.~~~abc" "${accountTmp}/App_2026.xcdistributionlogs";; esac`;
      const tail = 'while [ $# -gt 0 ]; do case "$1" in -archivePath) mkdir -p "$2/Products/Applications/App.app";; -exportPath) mkdir -p "$2"; : > "$2/App.ipa";; esac; shift; done';
      mkdirSync(join(accountTmp, 'unrelated'));
      stub('xcodebuild', `${writes}\n${tail}; exit 0`);
      let result = run({ HIDE_IOS_NO_UPLOAD: '1' });
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('removed xcodebuild distribution log XcodeDistPipeline.~~~abc');
      expect(readdirSync(accountTmp)).toEqual(['unrelated']);
      stub('xcodebuild', `${writes}\n${tail}; exit 65`);
      result = run();
      expect(result.status).toBe(1);
      expect(readdirSync(accountTmp)).toEqual(['unrelated']);
    });
  });

  describe('a run that reaches Apple', () => {
    it('stops before the upload when asked, having validated, and leaves no record, work or lock', () => {
      const result = run({ HIDE_IOS_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('stopped before the upload');
      expect(log()).toContain('No errors validating archive');
      expect(existsSync(join(base(), 'uploads.log'))).toBe(false);
      expect(existsSync(join(base(), 'release'))).toBe(false);
      expect(existsSync(join(base(), 'release.lock'))).toBe(false);
      expect(readdirSync(accountTmp)).toEqual([]);
    });

    it('records the upload as it starts and ends, and does not upload the same commit twice by accident', () => {
      const first = run();
      expect(first.status).toBe(0);
      expect(first.stdout).toContain(`delivery id ${DELIVERY}`);
      expect(records().map((line) => line.split(' ').slice(1))).toEqual([
        [COMMIT, '1.2.5', '20261110120000', '-', 'started'],
        [COMMIT, '1.2.5', '20261110120000', DELIVERY, 'uploaded'],
      ]);

      const second = run({}, ['release', 'branch', '1.2.5', '20261110120100', COMMIT, '0']);
      expect(second.status).toBe(1);
      expect(second.stderr).toContain('already has an upload record');
      expect(second.stderr).toContain(DELIVERY);
      expect(records()).toHaveLength(2);

      const asked = run({}, ['release', 'branch', '1.2.5', '20261110120200', COMMIT, '1']);
      expect(asked.status).toBe(0);
      expect(records()).toHaveLength(4);
    });

    it('prints the record for a caller that lost the connection, and says so when there is none', () => {
      const none = run({}, ['uploads', 'branch']);
      expect(none.stdout).toContain('no upload has started');
      run();
      const some = run({}, ['uploads', 'branch']);
      expect(some.stdout).toContain(DELIVERY);
      expect(some.stdout).not.toContain(KEY_ID);
    });

    it('ends an upload that outlives its time bound, and the record says it started', () => {
      const result = run({ STUB_UPLOAD_HANGS: '1', HIDE_IOS_TEST_STEP_LIMIT: '2' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "upload": it did not finish in 2 seconds');
      expect(result.stderr).toContain('may have gone without its delivery id');
      expect(records().at(-1)).toMatch(/ - started$/);
      expect(existsSync(join(base(), 'release'))).toBe(false);
      expect(existsSync(join(base(), 'release.lock'))).toBe(false);
    });

    it('refuses an app that is not signed with a distribution certificate', () => {
      const result = run({ STUB_AUTHORITY: 'Apple Development: Someone (ABCDE12345)' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('not signed with an Apple Distribution certificate');
      expect(existsSync(join(base(), 'uploads.log'))).toBe(false);
    });

    it('names the step that failed when a step outside the logged ones fails', () => {
      stub('unzip', 'exit 9');
      const result = run();
      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain('release stopped during "unpack the signed app"');
    });
  });

  describe('two commands of one branch', () => {
    it('keep the second release out while the first runs, naming it, and take over a lock whose command is gone', async () => {
      const holder = spawn('sleep', ['30']);
      mkdirSync(join(base(), 'release.lock'), { recursive: true });
      writeFileSync(join(base(), 'release.lock/pid'), String(holder.pid));
      const refused = run();
      expect(refused.status).toBe(1);
      expect(refused.stderr).toContain(`another release of this branch is running here (pid ${holder.pid}`);
      holder.kill();
      await new Promise((done) => holder.once('exit', done));
      expect(run({ HIDE_IOS_NO_UPLOAD: '1' }).status).toBe(0);
    });

    it('is not disturbed by a status check, which deletes nothing of the release', () => {
      put(join(base(), 'release/App.xcarchive/keep'), 'x');
      mkdirSync(join(base(), 'release.lock'), { recursive: true });
      writeFileSync(join(base(), 'release.lock/pid'), String(process.pid));
      const result = run({}, ['status', 'branch', DELIVERY]);
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('BUILD-STATUS: VALID');
      expect(existsSync(join(base(), 'release/App.xcarchive/keep'))).toBe(true);
      expect(existsSync(join(base(), 'release.lock/pid'))).toBe(true);
      expect(readdirSync(base()).filter((name) => name.startsWith('status-tmp'))).toEqual([]);
    });

    it('keeps a build or a clean out of the folder of a running release', () => {
      mkdirSync(join(base(), 'release.lock'), { recursive: true });
      writeFileSync(join(base(), 'release.lock/pid'), String(process.pid));
      for (const command of ['guard', 'clean']) {
        const result = spawnSync('bash', [join(mobile, 'scripts/mini-ios.sh'), command, 'branch'], { encoding: 'utf8', env: env() });
        expect(result.status, command).toBe(1);
        expect(result.stderr, command).toContain('a release of this branch is running');
      }
      expect(existsSync(base())).toBe(true);
    });
  });
});
