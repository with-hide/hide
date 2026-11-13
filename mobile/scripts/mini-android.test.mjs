// mini-android.sh against stand-ins for adb and the emulator, on any system, so it runs in the checks lane with the rest
// of mobile/. It proves the one thing a person cannot watch for: a run killed before its cleanup (SIGKILL) leaves its
// emulator, the processes the emulator started and the adb server, and the next run ends them and leaves nothing. The
// Mac mini's own runs are what prove the real emulator.
import { spawn, spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const script = join(resolve(dirname(fileURLToPath(import.meta.url))), 'mini-android.sh');
// A signal only ever goes to a pid that parsed as a real one: 0 or a negative number would reach this process's group.
const realPid = (pid) => { if (!Number.isInteger(pid) || pid <= 1) throw new Error(`not a process id: ${pid}`); return pid; };
const alive = (pid) => { try { process.kill(realPid(pid), 0); return true; } catch (error) { if (error.code === 'ESRCH') return false; throw error; } };
const until = async (what, check, ms = 30000) => {
  for (const start = Date.now(); !check(); ) {
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((done) => setTimeout(done, 25));
  }
};

describe('the emulator command on the build machine', { timeout: 120000 }, () => {
  let home;
  let first = null;
  const put = (path, text, mode = 0o600) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, text); chmodSync(path, mode); };
  const read = (name) => (existsSync(join(home, name)) ? readFileSync(join(home, name), 'utf8') : '');
  const lines = (name) => read(name).split('\n').filter(Boolean);
  // `<emulator pid> <its child's pid>`, one line per emulator started.
  const emulators = () => lines('emulators.log').map((line) => line.split(' ').map(Number));
  const serverPid = () => Number(read('adb-server').trim());
  const lockFile = () => join(home, 'hide-android-build/.emulator-lock');
  const lockHeld = () => spawnSync('perl', ['-MFcntl=:flock', '-e', 'open(my $f, "<", $ARGV[0]) or die; flock($f, LOCK_EX | LOCK_NB) or exit 75', lockFile()]).status === 75;
  const env = (extra) => ({ PATH: '/usr/bin:/bin', HOME: home, ...extra });

  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), 'mini-android-'));
    const sdk = join(home, 'Library/Android/sdk');
    put(join(home, 'hide-android-build/branch/repo/mobile/android/app/build/outputs/apk/debug/app-debug.apk'), 'apk');
    // adb records every call, and whether it was handed the lock's descriptor; its server outlives the call in a process
    // group of its own, as the real one does, and names itself once it is there.
    put(join(sdk, 'platform-tools/adb'), `#!/bin/bash
printf '%s\\n' "$*" >> "${home}/adb.log"
[ -e /dev/fd/9 ] && printf 'adb %s\\n' "$*" >> "${home}/leaks.log"
case "$*" in
  start-server) perl -e 'setpgrp(0, 0); open(my $f, ">", "$ARGV[0].tmp") or die; print $f "$$\\n"; close $f; rename("$ARGV[0].tmp", $ARGV[0]) or die; exec "sleep", "300"' "${home}/adb-server" > /dev/null 2>&1 &
    n=0; while [ ! -e "${home}/adb-server" ] && [ $n -lt 400 ]; do sleep 0.05; n=$((n + 1)); done ;;
  kill-server) [ -s "${home}/adb-server" ] && kill "$(cat "${home}/adb-server")" 2> /dev/null; rm -f "${home}/adb-server" ;;
  *"emu kill"*) read -r pid _ < <(tail -n 1 "${home}/emulators.log"); kill -TERM -- "-$pid" 2> /dev/null ;;
  *"getprop sys.boot_completed"*) echo 1 ;;
  *pidof*) echo 4242 ;;
esac
exit 0
`, 0o755);
    // The emulator starts a child of its own, as the real one starts qemu, and runs until it is ended.
    put(join(sdk, 'emulator/emulator'), `#!/bin/bash
[ -e /dev/fd/9 ] && printf 'emulator\\n' >> "${home}/leaks.log"
sleep 300 &
printf '%s %s\\n' "$$" "$!" >> "${home}/emulators.log"
wait
`, 0o755);
  });

  afterEach(() => {
    // The killed run's own process group (its sleep) and whatever a failing case left.
    if (first?.pid) try { process.kill(-realPid(first.pid), 'SIGKILL'); } catch { /* already gone */ }
    for (const pid of [...emulators().flat(), serverPid()]) if (pid > 1 && alive(pid)) process.kill(pid, 'SIGKILL');
    first = null;
    rmSync(home, { recursive: true, force: true });
  });

  it('ends the emulator and the adb server a killed run left, then leaves nothing of its own', async () => {
    first = spawn('bash', [script, 'run', 'branch'], { env: env({ HIDE_ANDROID_SETTLE: '300' }), detached: true, stdio: 'ignore' });
    let ended = false;
    first.once('exit', () => { ended = true; });
    await until('the first run to launch the app on its emulator', () => (
      lines('adb.log').some((line) => line.includes('shell am start')) && emulators().length === 1 && serverPid() > 1
    ));
    const [left, leftChild] = emulators()[0];
    const leftServer = serverPid();

    // Killed where it waits for the app to settle: no cleanup of its own runs.
    process.kill(first.pid, 'SIGKILL');
    await until('the first run to end', () => ended);
    expect([left, leftChild, leftServer].map(alive)).toEqual([true, true, true]);
    const before = lines('adb.log').length;

    const next = spawnSync('bash', [script, 'run', 'branch'], { encoding: 'utf8', env: env({ HIDE_ANDROID_SETTLE: '0' }), timeout: 90000 });
    expect(next.status, next.stdout + next.stderr).toBe(0);

    // The next run ended the leftover adb server before it started its own, and its own last of all.
    const calls = lines('adb.log').slice(before);
    expect(calls[0]).toBe('kill-server');
    expect(calls[1]).toBe('start-server');
    expect(calls.at(-1)).toBe('kill-server');
    const everything = [left, leftChild, leftServer, ...emulators()[1]];
    expect(emulators()).toHaveLength(2);
    await until('every emulator and adb server to be gone', () => !everything.some(alive), 15000).catch((error) => {
      throw new Error(`${error.message}: ${spawnSync('ps', ['-o', 'pid,ppid,pgid,stat,command', '-p', everything.join(',')], { encoding: 'utf8' }).stdout} ${JSON.stringify(everything)} ${read('adb.log')}`);
    });
    // No adb or emulator was ever handed the lock, the lock is free and names no one, and no record is left.
    expect(read('leaks.log')).toBe('');
    expect(lockHeld()).toBe(false);
    expect(readFileSync(lockFile(), 'utf8')).toBe('');
    expect(existsSync(join(home, 'hide-android-build/.emulator-run'))).toBe(false);
  });
});
