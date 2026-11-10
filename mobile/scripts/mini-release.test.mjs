// mini-release.sh against stand-ins for Xcode: the secrets it reads, and what it leaves behind. It uses macOS's own
// `stat` and `df`, so it runs on a Mac; the mobile tests of the Linux CI lane skip it, and the Mac mini's own run
// of the command is what proves the real tools.
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const mobile = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const KEY_ID = 'ABCDE12345';
const ISSUER = '11111111-2222-3333-4444-555555555555';
const SECRET_LINE = 'c2VjcmV0LWtleS1ib2R5LW5vdC1mb3ItbG9ncw';

describe.skipIf(process.platform !== 'darwin')('the release command on the build machine', () => {
  let home;
  let bin;
  const base = () => join(home, 'hide-ios-build/branch');
  const keyPath = () => join(home, `.config/hide-ios/AuthKey_${KEY_ID}.p8`);
  const put = (path, text, mode = 0o600) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); chmodSync(path, mode); };
  const stub = (name, body) => put(join(bin, name), `#!/bin/bash\n${body}\n`, 0o755);

  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), 'mini-release-'));
    bin = join(home, 'bin');
    for (const file of ['release-env.json', 'release-expect.json', 'scripts/check-release-app.sh']) {
      const target = join(base(), 'release-inputs', file.split('/').pop());
      mkdirSync(dirname(target), { recursive: true });
      copyFileSync(join(mobile, file), target);
    }
    put(join(base(), 'release-inputs/bundle-manifest.json'), '{"schema":1,"files":{}}');
    put(join(base(), 'release-inputs/relay-source.txt'), `${'a'.repeat(64)}\n`);
    put(join(base(), 'ios/App/App/App.entitlements'), '<plist/>');
    stub('codesign', 'exit 0');
    put(keyPath(), `-----BEGIN PRIVATE KEY-----\n${SECRET_LINE}\n-----END PRIVATE KEY-----\n`);
    put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\nHIDE_IOS_ASC_ISSUER_ID=${ISSUER}\nHIDE_IOS_ASC_KEY_PATH=${keyPath()}\n`);
    stub('df', 'echo "Filesystem 1G-blocks Used Available Capacity iused ifree %iused Mounted"; echo "/dev/x 500 100 400 20% 1 1 1% /"');
    // Archive works; the signing export prints every argument it was given, as Xcode does with its own, and fails.
    stub('xcodebuild', 'printf "xcodebuild %s\\n" "$*"; case "$*" in *-exportArchive*) exit 65;; *) mkdir -p "${@: -1}" 2>/dev/null; exit 0;; esac');
  });
  afterEach(() => { rmSync(home, { recursive: true, force: true }); });

  const run = () => spawnSync('bash', [join(mobile, 'scripts/mini-release.sh'), 'release', 'branch', '1.2.5', '20261110120000'], {
    encoding: 'utf8', env: { PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`, HOME: home },
  });

  it('stops before it builds anything when a secret is missing, naming the key and no value', () => {
    put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\n`);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('HIDE_IOS_ASC_ISSUER_ID is missing');
    expect(result.stderr).toContain('HIDE_IOS_ASC_KEY_PATH is missing');
    expect(result.stdout + result.stderr).not.toContain(KEY_ID);
    expect(existsSync(join(base(), 'release'))).toBe(false);
  });

  it('refuses a value that is not shaped as the registry says, without printing it', () => {
    put(join(home, '.config/hide-ios/release.env'), `HIDE_IOS_ASC_KEY_ID=${KEY_ID}\nHIDE_IOS_ASC_ISSUER_ID=not-an-issuer\nHIDE_IOS_ASC_KEY_PATH=${keyPath()}\n`);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('HIDE_IOS_ASC_ISSUER_ID is not shaped');
    expect(result.stdout + result.stderr).not.toContain('not-an-issuer');
  });

  it('refuses a key file another account can read', () => {
    chmodSync(keyPath(), 0o644);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('other accounts can read');
  });

  it('keeps every secret out of its output and its log when the signing step fails, and removes what it made', () => {
    const result = run();
    const log = readFileSync(join(base(), 'release.log'), 'utf8');
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('release stopped at "export and sign"');
    for (const secret of [KEY_ID, ISSUER, keyPath(), SECRET_LINE]) {
      expect(result.stdout + result.stderr, secret).not.toContain(secret);
      expect(log, secret).not.toContain(secret);
    }
    expect(log).toContain('<HIDE_IOS_ASC_KEY_ID>');
    expect(log).toContain('<HIDE_IOS_ASC_ISSUER_ID>');
    expect(existsSync(join(base(), 'release'))).toBe(false);
  });

  it('deletes its own log when a secret reaches it anyway', () => {
    // A stand-in that prints the key's body, which the redaction does not know about, as a broken redaction would.
    stub('xcodebuild', `echo "${SECRET_LINE}"; exit 0`);
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('a secret value reached the log');
    expect(existsSync(join(base(), 'release.log'))).toBe(false);
  });
});
