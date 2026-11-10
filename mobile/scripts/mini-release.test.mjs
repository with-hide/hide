// mini-release.sh against stand-ins for the Apple tools, on any system: the stand-ins answer the few macOS commands the
// script calls (`stat -f`, `df -g`, `getconf DARWIN_USER_TEMP_DIR`), so these run in the checks lane with the rest of
// mobile/. They prove the secrets, what a run owns and its cleanup on every way out, the machine lock, the upload record
// and the time bounds; the Mac mini's own run of the command is what proves the real Xcode.
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const mobile = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const script = join(mobile, 'scripts/mini-release.sh');
const KEY_ID = 'ABCDE12345';
const ISSUER = 'abcdef11-2222-3333-4444-555555555555';
const SECRET_LINE = 'c2VjcmV0LWtleS1ib2R5LW5vdC1mb3ItbG9ncw';
const COMMIT = 'a'.repeat(40);
const DELIVERY = '12345678-1234-1234-1234-123456789abc';
const TOKEN = 'eyJhbGciOiJFUzI1NiIsImtpZCI6IkFCQ0RFMTIzNDUifQ.eyJpc3MiOiJ4In0.c2lnbmF0dXJl';
const real = (name) => spawnSync('/bin/sh', ['-c', `command -v ${name}`], { encoding: 'utf8' }).stdout.trim();
// A signal only ever goes to a pid that parsed as a real one: 0 or a negative number would reach this process's group.
const realPid = (pid) => { if (!Number.isInteger(pid) || pid <= 1) throw new Error(`not a process id: ${pid}`); return pid; };
const alive = (pid) => { try { process.kill(realPid(pid), 0); return true; } catch (error) { if (error.code === 'ESRCH') return false; throw error; } };
// It polls on a timer: a loop on setImmediate keeps a core busy, which on a loaded machine starves the very processes it
// waits for, and the test runner's own messages with them.
const until = async (what, check, ms = 60000) => {
  for (const start = Date.now(); !check(); ) {
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((done) => setTimeout(done, 25));
  }
};

// Each case runs the whole script, which starts tens of processes; under a loaded machine (fifteen of these files at
// once take 30 s a case) the bound is the load's, not the case's.
describe('the release command on the build machine', { timeout: 120000 }, () => {
  let home;
  let bin;
  let accountTmp;
  const base = (slug = 'branch') => join(home, 'hide-ios-build', slug);
  const lockFile = () => join(home, 'hide-ios-build/.release.lock');
  const keyPath = () => join(home, `.config/hide-ios/AuthKey_${KEY_ID}.p8`);
  const put = (path, text, mode = 0o600) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); chmodSync(path, mode); };
  const stub = (name, body) => put(join(bin, name), `#!/bin/bash\n${body}\n`, 0o755);
  const branch = (slug) => {
    mkdirSync(join(base(slug), 'release-inputs'), { recursive: true });
    for (const file of ['release-env.json', 'release-expect.json']) copyFileSync(join(mobile, file), join(base(slug), 'release-inputs', file));
    put(join(base(slug), 'release-inputs/check-release-app.sh'), '#!/bin/bash\nexit 0\n', 0o755); // its own tests are check-release-app.test.mjs
    put(join(base(slug), 'release-inputs/bundle-manifest.json'), '{"schema":1,"files":{}}');
    put(join(base(slug), 'release-inputs/relay-source.txt'), `${'a'.repeat(64)}\n`);
    put(join(base(slug), 'ios/App/App/App.entitlements'), '<plist/>');
  };
  // Where the real export writes its distribution logs whatever $TMPDIR and the home say: confstr's temporary folder.
  const appleTmp = '"$(getconf DARWIN_USER_TEMP_DIR)"';

  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), 'mini-release-'));
    bin = join(home, 'bin');
    accountTmp = join(home, 'account-tmp');
    mkdirSync(accountTmp);
    mkdirSync(join(accountTmp, 'unrelated'));
    branch('branch');
    put(keyPath(), `-----BEGIN PRIVATE KEY-----\n${SECRET_LINE}\n-----END PRIVATE KEY-----\n`);
    put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\nHIDE_IOS_ASC_ISSUER_ID=${ISSUER}\nHIDE_IOS_ASC_KEY_PATH=${keyPath()}\n`);

    // The macOS commands the script asks questions of.
    stub('stat', 'f=${@: -1}; case "$*" in *%Lp*) cat "$f.mode" 2>/dev/null || echo 600;; *%u*) id -u;; esac');
    stub('df', 'echo "Filesystem 1G-blocks Used Available Capacity iused ifree %iused Mounted"; echo "/dev/x 500 100 ${STUB_FREE_GB:-400} 20% 1 1 1% /"');
    stub('getconf', `[ "$1" = DARWIN_USER_TEMP_DIR ] && echo "${accountTmp}/"`);
    // sed and grep record their arguments, which is what `ps` would show of them.
    for (const name of ['sed', 'grep']) stub(name, `printf '%s\\n' "$*" >> "${home}/argv.log"; exec ${real(name)} "$@"`);
    // The Apple tools. xcodebuild prints every argument as the real one does, writes its distribution log where confstr
    // says, as the real one does, and records the home and temporary folder it was given.
    stub('xcodebuild', `
printf 'xcodebuild %s\\n' "$*"
# A helper that outlives the step in a process group of its own, as a tool's daemon would. It names itself only once it
# has left the step's group, and the step waits for that, so a loaded machine cannot end the step while it is still in it.
if [ "$1" = archive ] && [ -n "$STUB_LEAVES_DAEMON" ]; then
  perl -e 'setpgrp(0, 0); open(my $f, ">", "$ARGV[0].tmp") or die; print $f "$$\\n"; close $f; rename("$ARGV[0].tmp", $ARGV[0]) or die; exec "sleep", "300"' "${home}/daemon" > /dev/null 2>&1 &
  n=0; while [ ! -e "${home}/daemon" ] && [ $n -lt 1200 ]; do sleep 0.05; n=$((n + 1)); done; fi
case "$*" in *-exportArchive*)
  printf '%s %s %s\\n' "$CFFIXED_USER_HOME" "$TMPDIR" "$([ -d "$CFFIXED_USER_HOME/Library/Caches" ] && echo caches-ready)" > "${home}/export-env"
  echo "authenticating with ${TOKEN} as ${KEY_ID} for ${ISSUER.toUpperCase()}"
  for logs in ${appleTmp}App_x.xcdistributionlogs ${appleTmp}XcodeDistPipeline.~~~x; do mkdir -p "$logs" && echo "${KEY_ID} ${ISSUER} ${TOKEN}" > "$logs/log"; done
  # Every copy of the key's body that exists while the export runs, where the script writes: the build folders and the
  # account temporary folder.
  ${real('grep')} -rlF "${SECRET_LINE}" "${home}/hide-ios-build" "${accountTmp}" > "${home}/key-copies" 2> /dev/null || true
  [ -n "$STUB_EXPORT_HOLDS" ] && { bash -c 'sleep 300 & echo $! > "${home}/grandchild.tmp" && mv "${home}/grandchild.tmp" "${home}/grandchild"; wait' & n=0; while [ -d "${home}" ] && [ ! -e "${home}/go" ] && [ $n -lt 1200 ]; do sleep 0.05; n=$((n + 1)); done; }
  [ -n "$STUB_EXPORT_FAILS" ] && exit 65;;
esac
while [ $# -gt 0 ]; do
  case "$1" in -archivePath) mkdir -p "$2/Products/Applications/App.app";; -exportPath) mkdir -p "$2"; : > "$2/App.ipa";;
    -resultBundlePath) echo "$2" >> "${home}/result-bundles";; esac; shift
done
[ -n "$STUB_PRINTS_KEY" ] && cat "${keyPath()}"
[ -n "$STUB_XCODEBUILD_FAILS" ] && exit 1
exit 0`);
    stub('codesign', 'case "$*" in *-dvv*) echo "Authority=${STUB_AUTHORITY:-Apple Distribution: Hide (GH6F6ZA497)}" >&2;; esac; exit 0');
    stub('unzip', 'dest=${@: -1}; mkdir -p "$dest/Payload/App.app"; : > "$dest/Payload/App.app/Info.plist"; : > "$dest/Payload/App.app/embedded.mobileprovision"');
    stub('plutil', 'case "$1" in -convert) while [ $# -gt 0 ]; do [ "$1" = -o ] && echo "{}" > "$2"; shift; done;; esac; exit 0');
    stub('security', 'echo "<plist/>"');
    stub('xcrun', `
mkdir -p "$CFFIXED_USER_HOME/Library/Logs/ContentDelivery" && echo "${KEY_ID}" > "$CFFIXED_USER_HOME/Library/Logs/ContentDelivery/altool.txt"
case "$*" in
  *--upload-app*) [ -n "$STUB_UPLOAD_HANGS" ] && { bash -c 'sleep 300 & echo $! > "${home}/grandchild.tmp" && mv "${home}/grandchild.tmp" "${home}/grandchild"; wait' & sleep 60; exit 0; }
                  printf 'UPLOAD SUCCEEDED with no errors\\nDelivery UUID: ${DELIVERY}\\n';;
  *--validate-app*) echo "No errors validating archive";;
  *--build-status*) [ -n "$STUB_STATUS_FAILS" ] && { echo "Unable to reach the service"; exit 1; }; echo "BUILD-STATUS: VALID";;
esac
exit 0`);
  });
  // Every run a test started in the background, so a test that fails before it ends one still ends it here.
  let started = [];
  afterEach(async () => {
    for (const { child, seen } of started) {
      if (seen.exit !== null) continue;
      child.kill('SIGTERM'); // the script's own TERM handling ends its step's process group and cleans up
      await until('a background run to end after TERM', () => seen.exit !== null, 15000).catch(() => child.kill('SIGKILL'));
    }
    started = [];
    for (const file of ['grandchild', 'daemon']) {
      const text = existsSync(join(home, file)) ? readFileSync(join(home, file), 'utf8').trim() : '';
      if (/^[0-9]+$/.test(text) && Number(text) > 1) try { process.kill(Number(text), 'SIGKILL'); } catch { /* already gone */ }
    }
    rmSync(home, { recursive: true, force: true });
  });
  // The pid a stand-in wrote (atomically, by a rename), once it is there.
  const pidIn = async (file) => {
    let text = '';
    await until(`${file} to hold a pid`, () => { text = existsSync(join(home, file)) ? readFileSync(join(home, file), 'utf8').trim() : ''; return /^[0-9]+$/.test(text); });
    return realPid(Number(text));
  };

  const env = (extra = {}) => ({ PATH: `${bin}:/usr/bin:/bin`, HOME: home, TMPDIR: accountTmp, ...extra });
  const args = (slug = 'branch', build = '20261110120000', again = '0') => ['release', slug, '1.2.5', build, COMMIT, again];
  const run = (extra = {}, argv = args()) => spawnSync('bash', [script, ...argv], { encoding: 'utf8', env: env(extra), timeout: 100000 });
  // A run that keeps going in the background; `out` grows as it prints.
  const start = (extra = {}, argv = args()) => {
    const child = spawn('bash', [script, ...argv], { env: env(extra), stdio: ['ignore', 'pipe', 'pipe'] });
    const seen = { out: '', exit: null };
    child.stdout.on('data', (chunk) => { seen.out += chunk; });
    child.stderr.on('data', (chunk) => { seen.out += chunk; });
    child.once('exit', (code, signal) => { seen.exit = { code, signal }; });
    started.push({ child, seen });
    return { child, seen };
  };
  const everything = (r) => r.stdout + r.stderr;
  const log = () => readFileSync(join(base(), 'release.log'), 'utf8');
  const records = () => readFileSync(join(home, 'hide-ios-build/uploads.log'), 'utf8').trim().split('\n');
  const lockHeld = () => spawnSync('bash', [script, 'busy', 'any', 'machine'], { encoding: 'utf8', env: env() }).status === 1;
  // What a run must leave behind once it ended, however it ended.
  const leftNothing = () => {
    expect(existsSync(join(base(), 'release')), 'work folder').toBe(false);
    expect(lockHeld(), 'machine lock').toBe(false);
    expect(readFileSync(lockFile(), 'utf8'), 'lock text').toBe('');
    expect(readdirSync(accountTmp), 'account temporary folder').toEqual(['unrelated']);
  };

  describe('the secrets', () => {
    it('stop it before it builds anything when one is missing, naming the key and no value', () => {
      put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\n`);
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_IOS_ASC_ISSUER_ID is missing');
      expect(result.stderr).toContain('HIDE_IOS_ASC_KEY_PATH is missing');
      expect(everything(result)).not.toContain(KEY_ID);
      leftNothing();
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

    it.skipIf(process.getuid?.() === 0)('leave the key file to Apple\'s tools: the script never reads it, copies it or puts a value in an argument', () => {
      chmodSync(keyPath(), 0o000); // the stand-in stat still says 600: only a read by the script itself would fail
      const result = run({ HIDE_IOS_NO_UPLOAD: '1' });
      chmodSync(keyPath(), 0o600);
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      expect(readFileSync(join(home, 'key-copies'), 'utf8')).toBe('');
      const argv = readFileSync(join(home, 'argv.log'), 'utf8');
      for (const secret of [KEY_ID, ISSUER, keyPath()]) expect(argv, secret).not.toContain(secret);
    });

    it('stay out of the output and the log when the export fails, and the run leaves nothing', () => {
      const result = run({ STUB_EXPORT_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "export and sign"');
      for (const secret of [KEY_ID, ISSUER, ISSUER.toUpperCase(), keyPath(), SECRET_LINE, TOKEN]) {
        expect(everything(result), secret).not.toContain(secret);
        expect(log(), secret).not.toContain(secret);
      }
      expect(log()).toContain('<HIDE_IOS_ASC_KEY_ID>');
      expect(log()).toContain('<token>');
      // The export ran with a home and a temporary folder inside the work folder, the home's Library/Caches already there
      // (CFNetwork puts the URL cache in the account's cache folder otherwise).
      expect(readFileSync(join(home, 'export-env'), 'utf8').trim()).toBe(`${join(base(), 'release/home')} ${join(base(), 'release/tmp')} caches-ready`);
      leftNothing();
    });

    it('withhold the end of a failed step\'s log, and delete the log, when a key reached it that redaction cannot replace', () => {
      const result = run({ STUB_PRINTS_KEY: '1', STUB_XCODEBUILD_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(everything(result)).not.toContain(SECRET_LINE);
      expect(result.stderr).not.toContain('the end of its log');
      expect(result.stderr).toContain('a secret value reached the log');
      expect(existsSync(join(base(), 'release.log'))).toBe(false);
      leftNothing();
    });
  });

  describe('a run that reaches Apple', () => {
    it('stops before the upload when asked, having validated, and leaves no record, work, lock or Apple file', () => {
      const result = run({ HIDE_IOS_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('stopped before the upload');
      expect(log()).toContain('No errors validating archive');
      expect(existsSync(join(home, 'hide-ios-build/uploads.log'))).toBe(false);
      expect(log()).toContain('removed xcodebuild distribution log XcodeDistPipeline.~~~x');
      // Each xcodebuild step names its result bundle inside the work folder, and altool's log went to the private home.
      expect(readFileSync(join(home, 'result-bundles'), 'utf8').trim().split('\n')).toEqual([join(base(), 'release/archive.xcresult'), join(base(), 'release/export.xcresult')]);
      expect(existsSync(join(home, 'Library'))).toBe(false);
      leftNothing();
    });

    it('records the upload as it starts and ends, and does not upload the same commit twice, under any branch name', () => {
      branch('main');
      const first = run();
      expect(first.status).toBe(0);
      expect(first.stdout).toContain(`delivery id ${DELIVERY}`);
      expect(records().map((line) => line.split(' ').slice(1))).toEqual([
        [COMMIT, '1.2.5', '20261110120000', '-', 'started', 'branch'],
        [COMMIT, '1.2.5', '20261110120000', DELIVERY, 'uploaded', 'branch'],
      ]);

      for (const slug of ['branch', 'main']) {
        const again = run({}, args(slug, '20261110120100'));
        expect(again.status, slug).toBe(1);
        expect(again.stderr, slug).toContain('already has an upload record');
        expect(again.stderr, slug).toContain(DELIVERY);
      }
      expect(records()).toHaveLength(2);

      const asked = run({}, args('main', '20261110120200', '1'));
      expect(asked.status).toBe(0);
      expect(records()).toHaveLength(4);
    });

    it('prints the record for a caller that lost the connection, and says so when there is none', () => {
      const none = run({}, ['uploads', 'branch']);
      expect(none.stdout).toContain('no upload has started');
      run();
      const some = run({}, ['uploads', 'other']);
      expect(some.stdout).toContain(DELIVERY);
      expect(some.stdout).not.toContain(KEY_ID);
    });

    it('ends an upload that outlives its time bound with everything it started, and the record says it started', async () => {
      const result = run({ STUB_UPLOAD_HANGS: '1', HIDE_IOS_TEST_UPLOAD_LIMIT: '2' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "upload": it did not finish in 2 seconds');
      expect(result.stderr).toContain('may have gone without its delivery id');
      expect(records().at(-1)).toMatch(/ - started branch$/);
      const grandchild = await pidIn('grandchild');
      await until('the upload\'s grandchild to end', () => !alive(grandchild), 5000);
      leftNothing();
    });

    it('starts with 5 GB free and refuses below that, before it builds anything', () => {
      expect(run({ STUB_FREE_GB: '5', HIDE_IOS_NO_UPLOAD: '1' }).status).toBe(0);
      const low = run({ STUB_FREE_GB: '4', HIDE_IOS_NO_UPLOAD: '1' });
      expect(low.status).toBe(1);
      expect(low.stderr).toContain('4 GB free and a release build needs at least 5 GB');
      expect(low.stdout).not.toContain('== archive');
      leftNothing();
    });

    it('refuses an app that is not signed with a distribution certificate', () => {
      const result = run({ STUB_AUTHORITY: 'Apple Development: Someone (ABCDE12345)' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('not signed with an Apple Distribution certificate');
      expect(existsSync(join(home, 'hide-ios-build/uploads.log'))).toBe(false);
      leftNothing();
    });

    it('names the step that failed when a step outside the logged ones fails', () => {
      stub('unzip', 'exit 9');
      const result = run();
      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain('release stopped during "unpack the signed app"');
      leftNothing();
    });

    it('fails a state read whose altool call failed, instead of reporting success with no answer', () => {
      const result = run({ STUB_STATUS_FAILS: '1' }, ['status', 'branch', DELIVERY]);
      expect(result.status).toBe(1);
      expect(result.stdout).toContain('Unable to reach the service');
      expect(result.stderr).toContain('altool --build-status failed (status 1)');
      expect(readdirSync(base()).filter((name) => name.startsWith('status'))).toEqual([]);
    });

    it('reads a build\'s state without touching a running release, and leaves no scratch folder', () => {
      put(join(base(), 'release/App.xcarchive/keep'), 'x');
      const result = run({}, ['status', 'branch', DELIVERY]);
      expect(result.status).toBe(0);
      expect(result.stdout).toContain('BUILD-STATUS: VALID');
      expect(existsSync(join(base(), 'release/App.xcarchive/keep'))).toBe(true);
      expect(readdirSync(base()).filter((name) => name.startsWith('status'))).toEqual([]);
      expect(existsSync(join(home, 'Library'))).toBe(false);
    });
  });

  describe('every way out', () => {
    it('cleans up when its standard output closes mid-run, as a dropped ssh connection does', async () => {
      const { child, seen } = start({ STUB_EXPORT_HOLDS: '1' });
      await pidIn('grandchild'); // the export has started
      child.stdout.destroy();
      child.stderr.destroy();
      writeFileSync(join(home, 'go'), '');
      await until('the run to end', () => seen.exit !== null);
      expect(seen.exit).toEqual({ code: 1, signal: null }); // not ended by SIGPIPE before its cleanup
      leftNothing();
    });

    it('ends the export and everything it started on TERM, and cleans up', async () => {
      const { child, seen } = start({ STUB_EXPORT_HOLDS: '1' });
      const grandchild = await pidIn('grandchild');
      child.kill('SIGTERM');
      await until('the run to end', () => seen.exit !== null);
      expect(seen.exit).toEqual({ code: 143, signal: null });
      expect(seen.out).toContain('release did not finish (status 143)');
      expect(alive(grandchild)).toBe(false);
      expect(seen.out).not.toContain(KEY_ID);
      leftNothing();
    });

    it('holds the machine against a release of another branch while one runs, and lets it go when that one ends', async () => {
      branch('other');
      const { child, seen } = start({ STUB_EXPORT_HOLDS: '1', HIDE_IOS_NO_UPLOAD: '1' });
      await pidIn('grandchild'); // the export has started
      const refused = run({ HIDE_IOS_NO_UPLOAD: '1' }, args('other'));
      expect(refused.status).toBe(1);
      expect(refused.stderr).toContain(`another release is running on this machine (pid ${child.pid}, started `);
      expect(refused.stderr).toContain(', branch branch)');
      const busy = (slug, scope) => spawnSync('bash', [script, 'busy', slug, scope], { encoding: 'utf8', env: env() }).status;
      expect([busy('branch', 'branch'), busy('other', 'branch'), busy('other', 'machine')]).toEqual([1, 0, 1]);
      // The running export's own files are untouched by the refused run.
      expect(readdirSync(accountTmp).sort()).toEqual(['App_x.xcdistributionlogs', 'XcodeDistPipeline.~~~x', 'unrelated']);
      writeFileSync(join(home, 'go'), '');
      await until('the first run to end', () => seen.exit !== null);
      expect(seen.exit).toEqual({ code: 0, signal: null });
      expect(run({ HIDE_IOS_NO_UPLOAD: '1' }, args('other')).status).toBe(0);
    });

    it('lets the lock go with the run even when a tool leaves a helper of its own running', async () => {
      const result = run({ STUB_LEAVES_DAEMON: '1', HIDE_IOS_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      expect(alive(await pidIn('daemon'))).toBe(true); // it escaped the step's group, as a real daemon can
      leftNothing(); // and still holds no descriptor of the lock
    });

    it('names a lock it cannot take or read, and never reads that as a running release', () => {
      stub('perl', 'exit 2');
      mkdirSync(dirname(lockFile()), { recursive: true });
      writeFileSync(lockFile(), '');
      const release = run({ HIDE_IOS_NO_UPLOAD: '1' });
      expect(release.status).toBe(1);
      expect(release.stderr).toContain('could not be taken (perl status 2)');
      expect(release.stderr).not.toContain('another release is running');
      const busy = spawnSync('bash', [script, 'busy', 'branch', 'machine'], { encoding: 'utf8', env: env() });
      expect(busy.status).toBe(1);
      expect(busy.stderr).toContain('could not be read (perl status 2)');
      expect(busy.stderr).not.toContain('a release is running');
    });

    it('does not believe a lock whose text names a live process the kernel does not hold the lock for', () => {
      mkdirSync(dirname(lockFile()), { recursive: true });
      writeFileSync(lockFile(), `pid ${process.pid}, started ${new Date().toUTCString()}, branch branch\n`);
      expect(lockHeld()).toBe(false);
      const result = run({ HIDE_IOS_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      leftNothing();
    });
  });
});
