// mini-android-release.sh against stand-ins for Gradle, keytool, jarsigner, unzip and curl, on any system: the stand-ins
// answer the macOS commands the script asks questions of (`stat -f`, `df -g`), so these run in the checks lane with the
// rest of mobile/. They prove the secrets (from the file only, through pipes only, never in an argument, an environment,
// the log or the output), the upload to Play through an edit that is deleted when it is not committed, what a run owns
// and its cleanup on a failure and an interruption, the machine lock and the upload record. The real openssl signs the
// service account's assertion. The Mac mini's own run of `build-android.sh release` is what proves the real tools.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, generateKeyPairSync } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const mobile = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const script = join(mobile, 'scripts/mini-android-release.sh');
const PASSWORD = 'Pw4ndr0idUploadSecretValue';
const TOKEN = 'ya29.c2VjcmV0LWFjY2Vzcy10b2tlbg-test';
const COMMIT = 'b'.repeat(40);
const CODE = '29909300';
const BUNDLE = 'signed bundle bytes';
const SHA = createHash('sha256').update(BUNDLE).digest('hex');
const { privateKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
const PRIVATE_KEY = privateKey.export({ type: 'pkcs8', format: 'pem' });
const KEY_LINE = PRIVATE_KEY.split('\n')[1];
const real = (name) => spawnSync('/bin/sh', ['-c', `command -v ${name}`], { encoding: 'utf8' }).stdout.trim();
const realPid = (pid) => { if (!Number.isInteger(pid) || pid <= 1) throw new Error(`not a process id: ${pid}`); return pid; };
const alive = (pid) => { try { process.kill(realPid(pid), 0); return true; } catch (error) { if (error.code === 'ESRCH') return false; throw error; } };
const until = async (what, check, ms = 60000) => {
  for (const start = Date.now(); !check(); ) {
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((done) => setTimeout(done, 25));
  }
};

describe('the Android release command on the build machine', { timeout: 120000 }, () => {
  let home;
  let bin;
  let java;
  const base = () => join(home, 'hide-android-build/branch');
  const config = (name) => join(home, '.config/hide-android', name);
  const put = (path, text, mode = 0o600) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); chmodSync(path, mode); };
  const stub = (path, body) => put(path, `#!/bin/bash\n${body}\n`, 0o755);
  // Every argument any stand-in was started with, and the environment each was given: what `ps` would show of them.
  const recordArgs = (name) => `printf '%s %s\\n' "${name}" "$*" >> "${home}/argv.log"; env >> "${home}/env.log"`;
  const secretsFile = (lines) => put(config('release.env'), lines.join('\n') + '\n');
  const goodSecrets = () => [
    `HIDE_ANDROID_UPLOAD_KEYSTORE=${config('upload.p12')}`, 'HIDE_ANDROID_UPLOAD_KEY_ALIAS=upload',
    `HIDE_ANDROID_UPLOAD_PASSWORD=${PASSWORD}`, `HIDE_ANDROID_PLAY_SERVICE_ACCOUNT=${config('play.json')}`,
  ];

  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), 'mini-android-release-'));
    bin = join(home, 'bin');
    java = join(home, 'jdk');
    const inputs = join(base(), 'release-inputs');
    for (const file of ['android-release-env.json', 'android-release-expect.json']) put(join(inputs, file), readFileSync(join(mobile, file)));
    // Its own tests are check-release-aab.test.mjs; here it records what it was handed and can be made to fail.
    stub(join(inputs, 'check-release-aab.py'), `${recordArgs('check')}; cat "$2" > "${home}/signers-seen"; [ -z "$STUB_CHECK_FAILS" ] || { echo "release check failed: the app is debuggable" >&2; exit 1; }; echo "release checks passed"`);
    put(join(inputs, 'bundle-manifest.json'), '{"schema":1,"files":{}}');
    put(config('upload.p12'), 'keystore bytes');
    put(config('play.json'), JSON.stringify({ type: 'service_account', client_email: 'uploader@hide.iam.gserviceaccount.com', private_key: PRIVATE_KEY, token_uri: 'https://oauth2.googleapis.com/token' }));
    secretsFile(goodSecrets());

    // Gradle builds the bundle unsigned, and is given no secret.
    stub(join(base(), 'repo/mobile/android/gradlew'), `${recordArgs('gradlew')}
project=$2
[ -n "$STUB_GRADLE_PRINTS" ] && echo "$STUB_GRADLE_PRINTS"
if [ -n "$STUB_GRADLE_HANGS" ]; then
  bash -c 'sleep 300 & echo $! > "${home}/grandchild.tmp" && mv "${home}/grandchild.tmp" "${home}/grandchild"; wait' & wait; fi
[ -n "$STUB_GRADLE_FAILS" ] && { echo "FAILURE: Build failed with an exception."; exit 1; }
mkdir -p "$project/app/build/outputs/bundle/release" && printf 'unsigned' > "$project/app/build/outputs/bundle/release/app-release.aab"`);
    // keytool and jarsigner take the password from the file -storepass:file names, and say whether it was the right one.
    const password = `pw=; prev=; for a in "$@"; do [ "$prev" = -storepass:file ] && pw=$(head -n 1 "$a"); prev=$a; done`;
    stub(join(java, 'bin/keytool'), `${recordArgs('keytool')}; ${password}
case "$1" in -list) [ "$pw" = "${PASSWORD}" ] || { echo "keytool error: password was incorrect"; exit 1; };;
  -printcert) echo "Signer #1:"; echo "	 SHA256: \${STUB_SIGNER:-12:F9:F3:EF}";; esac`);
    stub(join(java, 'bin/jarsigner'), `${recordArgs('jarsigner')}; ${password}
if [ "$1" = -verify ]; then echo "jar verified."; [ -n "$STUB_UNSIGNED_ENTRIES" ] && echo "This jar contains unsigned entries which have not been integrity-checked."; exit 0; fi
[ "$pw" = "${PASSWORD}" ] || { echo "jarsigner: password was incorrect"; exit 1; }
[ -n "$STUB_JARSIGNER_ECHOES" ] && echo "signing with $pw"
out=; prev=; for a in "$@"; do [ "$prev" = -signedjar ] && out=$a; prev=$a; done; printf '${BUNDLE}' > "$out"; echo "jar signed."`);
    // unzip unpacks the bundle; curl is Play: it reads the token from the header file it is handed.
    stub(join(bin, 'unzip'), 'mkdir -p "${@: -1}/base/manifest"');
    stub(join(bin, 'curl'), `${recordArgs('curl')}
url=\${@: -1}; method=GET; out=/dev/stdout; auth=; prev=
for a in "$@"; do case "$prev" in -X) method=$a;; -o) out=$a;; -H) case "$a" in @-) auth=$(cat);; @*) auth=$(cat "\${a#@}");; esac;; esac; prev=$a; done
printf '%s %s %s\\n' "$method" "\${url#https://androidpublisher.googleapis.com}" "$([ "$auth" = "Authorization: Bearer ${TOKEN}" ] && echo with-token || echo no-token)" >> "${home}/play.log"
answer() { printf '%s' "$1" > "$out"; }
case "$method $url" in
  "POST https://oauth2.googleapis.com/token") body=$(cat); case "$body" in eyJ*.*.*) ;; *) echo '{"error":"invalid_grant"}'; exit 22;; esac
    [ -n "$STUB_TOKEN_ECHOES" ] && { echo "{\\"error\\":\\"bad\\",\\"seen\\":\\"$body\\"}"; exit 22; }
    echo '{"access_token":"${TOKEN}","expires_in":3599,"token_type":"Bearer"}';;
  "POST "*/edits) answer '{"id":"edit-1","expiryTimeSeconds":"1794560000"}';;
  "POST "*/bundles?uploadType=media)
    if [ -n "$STUB_UPLOAD_HANGS" ]; then bash -c 'sleep 300 & echo $! > "${home}/grandchild.tmp" && mv "${home}/grandchild.tmp" "${home}/grandchild"; wait'; fi
    answer "{\\"versionCode\\":\${STUB_PLAY_CODE:-${CODE}},\\"sha1\\":\\"x\\",\\"sha256\\":\\"${SHA}\\"}";;
  "PUT "*/tracks/internal) answer '{"track":"internal"}';;
  "POST "*:commit) [ -n "$STUB_COMMIT_FAILS" ] && { answer '{"error":{"code":403,"message":"The caller does not have permission"}}'; exit 22; }; answer '{"id":"edit-1"}';;
  "DELETE "*) : ;;
  *) echo "unexpected $method $url" >&2; exit 2;;
esac`);
    stub(join(bin, 'stat'), 'f=${@: -1}; case "$*" in *%Lp*) cat "$f.mode" 2>/dev/null || echo 600;; *%u*) id -u;; esac');
    stub(join(bin, 'df'), 'echo "Filesystem 1G-blocks Used Available Capacity iused ifree %iused Mounted"; echo "/dev/x 500 100 ${STUB_FREE_GB:-400} 20% 1 1 1% /"');
    stub(join(bin, 'shasum'), `${real('shasum') || real('sha256sum')} ${real('shasum') ? '"$@"' : '"${@: -1}"'}`);
    // The check runs as `python3 -I <script> ...`: the stand-in runs the stand-in check.
    stub(join(bin, 'python3'), 'shift; exec bash "$@"');
    for (const name of ['sed', 'grep', 'jq', 'openssl']) stub(join(bin, name), `${recordArgs(name)}; exec ${real(name)} "$@"`);
  });

  let started = [];
  afterEach(async () => {
    for (const { child, seen } of started) {
      if (seen.exit !== null) continue;
      child.kill('SIGTERM');
      await until('a background run to end after TERM', () => seen.exit !== null, 15000).catch(() => child.kill('SIGKILL'));
    }
    started = [];
    const text = existsSync(join(home, 'grandchild')) ? readFileSync(join(home, 'grandchild'), 'utf8').trim() : '';
    if (/^[0-9]+$/.test(text) && Number(text) > 1) try { process.kill(Number(text), 'SIGKILL'); } catch { /* already gone */ }
    rmSync(home, { recursive: true, force: true });
  });

  const env = (extra = {}) => ({ PATH: `${bin}:/usr/bin:/bin`, HOME: home, TMPDIR: tmpdir(), JAVA_HOME: java, ANDROID_HOME: join(home, 'sdk'), ...extra });
  const args = (again = '0') => ['release', 'branch', '1.0.0', CODE, COMMIT, again];
  const run = (extra = {}, argv = args()) => spawnSync('bash', [script, ...argv], { encoding: 'utf8', env: env(extra), timeout: 100000 });
  const start = (extra = {}) => {
    const child = spawn('bash', [script, ...args()], { env: env(extra), stdio: ['ignore', 'pipe', 'pipe'] });
    const seen = { out: '', exit: null };
    child.stdout.on('data', (chunk) => { seen.out += chunk; });
    child.stderr.on('data', (chunk) => { seen.out += chunk; });
    child.once('exit', (code, signal) => { seen.exit = { code, signal }; });
    started.push({ child, seen });
    return { child, seen };
  };
  const pidIn = async (file) => {
    let text = '';
    await until(`${file} to hold a pid`, () => { text = existsSync(join(home, file)) ? readFileSync(join(home, file), 'utf8').trim() : ''; return /^[0-9]+$/.test(text); });
    return realPid(Number(text));
  };
  const everything = (r) => r.stdout + r.stderr;
  const read = (file) => (existsSync(join(home, file)) ? readFileSync(join(home, file), 'utf8') : '');
  const log = () => readFileSync(join(base(), 'release.log'), 'utf8');
  const records = () => read('hide-android-build/uploads.log').trim().split('\n').filter(Boolean);
  const lockHeld = () => spawnSync('bash', [script, 'busy', 'any', 'machine'], { encoding: 'utf8', env: env() }).status === 1;
  const secrets = [PASSWORD, TOKEN, KEY_LINE];
  // No secret in any argument or environment a program was started with, in the output or in the log.
  const noSecretAnywhere = (result) => {
    for (const secret of secrets) {
      expect(read('argv.log'), `argument: ${secret}`).not.toContain(secret);
      expect(read('env.log'), `environment: ${secret}`).not.toContain(secret);
      if (result) expect(everything(result), `output: ${secret}`).not.toContain(secret);
      if (existsSync(join(base(), 'release.log'))) expect(log(), `log: ${secret}`).not.toContain(secret);
    }
    expect(read('argv.log')).not.toMatch(/eyJ[A-Za-z0-9_-]{10,}\./); // the signed assertion
  };
  // What a run must leave behind once it ended, however it ended.
  const leftNothing = () => {
    expect(existsSync(join(base(), 'release')), 'work folder').toBe(false);
    expect(lockHeld(), 'machine lock').toBe(false);
    expect(readFileSync(join(home, 'hide-android-build/.release.lock'), 'utf8'), 'lock text').toBe('');
  };

  describe('the secrets', () => {
    it('stop it before it builds anything when one is missing, naming the key and no value', () => {
      secretsFile([`HIDE_ANDROID_UPLOAD_PASSWORD=${PASSWORD}`]);
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_ANDROID_UPLOAD_KEYSTORE is missing');
      expect(result.stderr).toContain('HIDE_ANDROID_PLAY_SERVICE_ACCOUNT is missing');
      expect(read('argv.log')).not.toContain('gradlew');
      noSecretAnywhere(result);
      leftNothing();
    });

    it('do not ask for the service account when nothing is uploaded', () => {
      secretsFile(goodSecrets().slice(0, 3));
      const result = run({ HIDE_ANDROID_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
    });

    it('are refused from the environment, where every program the run starts would see them', () => {
      const result = run({ HIDE_ANDROID_UPLOAD_PASSWORD: PASSWORD });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_ANDROID_UPLOAD_PASSWORD is set in the environment');
      expect(read('argv.log')).not.toContain('gradlew');
      expect(everything(result)).not.toContain(PASSWORD);
    });

    it('are refused when a file can be read by another account', () => {
      writeFileSync(`${config('upload.p12')}.mode`, '644');
      expect(run().stderr).toContain('HIDE_ANDROID_UPLOAD_KEYSTORE names a file other accounts can read');
      rmSync(`${config('upload.p12')}.mode`);
      writeFileSync(`${config('release.env')}.mode`, '640');
      expect(run().stderr).toContain('readable by it alone');
    });

    it('are refused when the password does not open the keystore, before anything is built', () => {
      secretsFile(goodSecrets().map((line) => line.replace(PASSWORD, 'AnotherPasswordOfLength24')));
      const result = run();
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('HIDE_ANDROID_UPLOAD_PASSWORD does not open HIDE_ANDROID_UPLOAD_KEYSTORE');
      expect(everything(result)).not.toContain('AnotherPasswordOfLength24');
      expect(read('argv.log')).not.toContain('gradlew');
    });

    it('are refused when the service account file is not one', () => {
      put(config('play.json'), JSON.stringify({ type: 'authorized_user', client_email: 'x' }));
      expect(run().stderr).toContain('HIDE_ANDROID_PLAY_SERVICE_ACCOUNT is not a service account key file');
    });

    it('reach keytool and jarsigner through a pipe only, and Gradle not at all', () => {
      const result = run({ HIDE_ANDROID_NO_UPLOAD: '1' });
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      // The stand-ins refuse a wrong password, so the signature proves jarsigner read it from the file it was handed.
      expect(read('argv.log')).toMatch(/^jarsigner .*-storepass:file \/dev\/fd\/[0-9]+ /m);
      expect(read('argv.log')).toMatch(/^gradlew -p .* bundleRelease -PhideVersionCode=29909300$/m);
      noSecretAnywhere(result);
    });

    it('are replaced in the log when a tool prints one, and the run goes on', () => {
      const result = run({ HIDE_ANDROID_NO_UPLOAD: '1', STUB_JARSIGNER_ECHOES: '1', STUB_GRADLE_PRINTS: 'cached ya29.anotherAccessTokenSeenByGradle' });
      expect(result.status).toBe(0);
      expect(log()).toContain('signing with <HIDE_ANDROID_UPLOAD_PASSWORD>');
      expect(log()).toContain('cached <token>');
      noSecretAnywhere(result);
    });

    it('withhold the end of a failed step\'s log, and delete the log, when a key reached it that redaction cannot replace', () => {
      const result = run({ STUB_GRADLE_PRINTS: '-----BEGIN PRIVATE KEY-----', STUB_GRADLE_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).not.toContain('the end of its log');
      expect(result.stderr).toContain('a secret value reached the log');
      expect(existsSync(join(base(), 'release.log'))).toBe(false);
      leftNothing();
    });
  });

  describe('a run', () => {
    it('signs and checks the bundle and stops before Play when asked, leaving no record, work or lock', () => {
      const result = run({ HIDE_ANDROID_NO_UPLOAD: '1' });
      expect(result.status).toBe(0);
      expect(result.stdout).toContain(`signed and checked 1.0.0 (${CODE}), sha256 ${SHA}`);
      expect(result.stdout).toContain('nothing was sent to Play');
      expect(read('signers-seen')).toBe('12:F9:F3:EF\n');
      expect(read('play.log')).toBe('');
      expect(records()).toEqual([]);
      leftNothing();
    });

    it('uploads to the internal track through one edit, the token in a header pipe, and records it', () => {
      const result = run();
      expect(result.stderr).toBe('');
      expect(result.status).toBe(0);
      expect(result.stdout).toContain(`uploaded: 1.0.0 (${CODE}) to the internal testing track`);
      expect(read('play.log').trim().split('\n')).toEqual([
        'POST https://oauth2.googleapis.com/token no-token',
        'POST /androidpublisher/v3/applications/me.grab.hide.android/edits with-token',
        'POST /upload/androidpublisher/v3/applications/me.grab.hide.android/edits/edit-1/bundles?uploadType=media with-token',
        'PUT /androidpublisher/v3/applications/me.grab.hide.android/edits/edit-1/tracks/internal with-token',
        'POST /androidpublisher/v3/applications/me.grab.hide.android/edits/edit-1:commit with-token',
      ]);
      expect(read('argv.log')).toContain('"status":"completed"');
      expect(records().map((line) => line.split(' ').slice(1).join(' '))).toEqual([`${COMMIT} 1.0.0 ${CODE} started branch`, `${COMMIT} 1.0.0 ${CODE} uploaded branch`]);
      noSecretAnywhere(result);
      leftNothing();
    });

    it('does not put a token Play echoed back into the output or the log', () => {
      const result = run({ STUB_TOKEN_ECHOES: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('Play refused the service account');
      expect(log()).toContain('<token>');
      expect(log()).not.toMatch(/eyJ[A-Za-z0-9_-]{10,}\./);
      noSecretAnywhere(result);
      leftNothing();
    });

    it('stops at a failed release check before anything reaches Play', () => {
      const result = run({ STUB_CHECK_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release check failed: the app is debuggable');
      expect(read('play.log')).toBe('');
      leftNothing();
    });

    it('refuses a bundle the signature does not wholly cover, and one signed by another key reaches the check', () => {
      expect(run({ STUB_UNSIGNED_ENTRIES: '1' }).stderr).toContain('entries the signature does not cover');
      run({ HIDE_ANDROID_NO_UPLOAD: '1', STUB_SIGNER: 'AB:CD' });
      expect(read('signers-seen')).toBe('AB:CD\n');
    });

    it('deletes the edit when Play refuses the commit, so nothing reaches a track, and may run again', () => {
      const result = run({ STUB_COMMIT_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "commit the edit"');
      expect(result.stderr).toContain('The caller does not have permission');
      expect(result.stderr).toContain('deleted the uncommitted Play edit edit-1');
      expect(read('play.log')).toContain('DELETE /androidpublisher/v3/applications/me.grab.hide.android/edits/edit-1 with-token');
      expect(records().map((line) => line.split(' ')[4])).toEqual(['started', 'deleted']);
      noSecretAnywhere(result);
      leftNothing();
      expect(run().status).toBe(0);
    });

    it('refuses a bundle Play took under another version code', () => {
      const result = run({ STUB_PLAY_CODE: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('another version code');
      expect(read('play.log')).toContain('DELETE');
      leftNothing();
    });

    it('cleans up after a failed Gradle build, showing the end of its log', () => {
      const result = run({ STUB_GRADLE_FAILS: '1' });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('release stopped at "bundle"');
      expect(result.stderr).toContain('Build failed with an exception');
      noSecretAnywhere(result);
      leftNothing();
    });

    it('ends the Gradle step with everything it started, and the run with its work, when it is interrupted', async () => {
      const { child, seen } = start({ STUB_GRADLE_HANGS: '1' });
      const grandchild = await pidIn('grandchild');
      expect(lockHeld()).toBe(true);
      child.kill('SIGTERM');
      await until('the run to end', () => seen.exit !== null);
      expect(seen.exit.code).toBe(143);
      await until('the grandchild to end', () => !alive(grandchild), 15000);
      leftNothing();
    });

    it('deletes the edit and ends the upload when it is interrupted mid-upload', async () => {
      const { child, seen } = start({ STUB_UPLOAD_HANGS: '1' });
      const grandchild = await pidIn('grandchild');
      child.kill('SIGTERM');
      await until('the run to end', () => seen.exit !== null);
      expect(seen.exit.code).toBe(143);
      expect(seen.out).toContain('deleted the uncommitted Play edit edit-1');
      expect(records().map((line) => line.split(' ')[4])).toEqual(['started', 'deleted']);
      leftNothing();
      if (alive(grandchild)) process.kill(grandchild, 'SIGKILL'); // the stand-in's own child: the real curl starts none
    });
  });

  describe('the machine', () => {
    it('runs one release at a time, naming the holder', async () => {
      const { seen } = start({ STUB_GRADLE_HANGS: '1' });
      await pidIn('grandchild');
      const second = run();
      expect(second.status).toBe(1);
      expect(second.stderr).toContain('another Android release is running on this machine (pid');
      expect(spawnSync('bash', [script, 'busy', 'branch', 'branch'], { encoding: 'utf8', env: env() }).status).toBe(1);
      expect(spawnSync('bash', [script, 'busy', 'other', 'branch'], { encoding: 'utf8', env: env() }).status).toBe(0);
      expect(seen.exit).toBe(null);
    });

    it('refuses a commit that already went, unless asked to upload it again', () => {
      expect(run().status).toBe(0);
      const again = run();
      expect(again.status).toBe(1);
      expect(again.stderr).toContain('this commit already has an upload record');
      expect(run({}, args('1')).status).toBe(0);
      expect(spawnSync('bash', [script, 'uploads', 'branch'], { encoding: 'utf8', env: env() }).stdout.trim().split('\n')).toHaveLength(4);
    });

    it('refuses a disk below the floor and a version code Play does not take', () => {
      expect(run({ STUB_FREE_GB: '1' }).stderr).toContain('needs at least 3 GB');
      expect(run({}, ['release', 'branch', '1.0.0', '2100000001', COMMIT, '0']).stderr).toContain('not a number Play takes');
    });
  });
});
