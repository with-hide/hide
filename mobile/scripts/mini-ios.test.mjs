// Exercise the simulator command's ownership without Xcode: the app stand-in holds the caller's output pipe open,
// just as a forwarded fixture connection keeps SSH open. Ending the command must close that pipe on every exit.
import { spawn } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./mini-ios.sh', import.meta.url));
let testHome;
let bin;
beforeEach(() => {
  testHome = mkdtempSync(join(tmpdir(), 'mini-ios-'));
  bin = join(testHome, 'bin');
  mkdirSync(bin);
  mkdirSync(join(testHome, 'hide-ios-build/branch'), { recursive: true });
  const tool = join(bin, 'xcrun');
  writeFileSync(tool, `#!/bin/bash
printf '%s\\n' "$*" >> "$STUB_EVENTS"
case "$1 $2" in
  'simctl list') echo 'hide-ios-branch (11111111-2222-3333-4444-555555555555) (Booted)' ;;
  'simctl launch')
    /bin/sleep 300 &
    echo $! > "$STUB_APP_PID"
    if [[ "$STUB_FAIL" == launch ]]; then echo 'launch failed' >&2; exit 37; fi ;;
  'simctl terminate')
    if [[ -f "$STUB_APP_PID" ]]; then
      kill -TERM "$(cat "$STUB_APP_PID")" 2>/dev/null || true
      rm "$STUB_APP_PID"
      exit 57 # Cleanup failure must not replace the original launch/capture failure.
    fi ;;
  'simctl io')
    if [[ "$STUB_FAIL" == capture ]]; then echo 'capture failed' >&2; exit 42; fi ;;
esac
exit 0
`);
  chmodSync(tool, 0o755);
});
afterEach(() => { rmSync(testHome, { recursive: true, force: true }); });

it.each([
  { fail: 'capture', keep: '1', status: 42 },
  { fail: 'launch', keep: '1', status: 37 },
  { fail: 'capture', keep: '0', status: 42 },
  { fail: '', keep: '1', status: 0 },
])('closes its fixture app and preserves status $status after $fail (keep=$keep)', async ({ fail, keep, status }) => {
  const pidFile = join(testHome, 'app.pid');
  const events = join(testHome, 'events');
  const child = spawn('bash', [script, 'run', 'branch'], {
    env: { PATH: `${bin}:/usr/bin:/bin`, HOME: testHome, HIDE_IOS_SETTLE: '0', HIDE_IOS_KEEP_SIMULATOR: keep,
      HIDE_IOS_RELAY_PORT: '43179', STUB_FAIL: fail, STUB_APP_PID: pidFile, STUB_EVENTS: events },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  const closed = new Promise((resolve, reject) => {
    child.once('close', (code) => resolve(code));
    child.once('error', reject);
  });
  let timer;
  try {
    const code = await Promise.race([closed, new Promise((_, reject) => {
      // Readiness bound: both the command and the inherited fixture output channel must close.
      timer = setTimeout(() => reject(new Error('the fixture app kept the command output channel open')), 30000);
    })]);
    expect(code).toBe(status);
    if (fail) expect(output).toContain(`${fail} failed`);
    expect(existsSync(pidFile)).toBe(false);
    // It found this simulator already booted; closing its app must not shut down that simulator.
    expect(readFileSync(events, 'utf8')).not.toMatch(/simctl (shutdown|delete)/);
  } finally {
    clearTimeout(timer);
    // A regression leaves a real stand-in process behind. End only the pid this case started, even when it failed.
    if (existsSync(pidFile)) {
      const pid = Number(readFileSync(pidFile, 'utf8').trim());
      if (Number.isInteger(pid) && pid > 1) try { process.kill(pid, 'SIGTERM'); } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
    child.kill('SIGTERM');
    await closed;
  }
}, 45000);
