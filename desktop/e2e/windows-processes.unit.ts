// The Windows process ending every fixture teardown relies on, against real
// processes: a child its owner started after the owner was listed, left
// running when the owner was killed, is ended with the owner; an owned
// process that has ended is not named as a survivor, and one the end cannot
// open is, instead of being taken for ended (#3126). Windows only.
import { beforeAll, expect, test } from "vitest";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { endWindowsProcesses, windowsProcessTree } from "../../web/e2e/platform-fixture";
import { buildFixtureShims, fixtureBuildTimeout, fixtureShimPath } from "../../web/e2e/shims/build";

// vitest has no `globalSetup` of Playwright's, so this file builds the
// programs the teardown runs, the same way, before it uses one.
beforeAll(async () => {
  if (process.platform === "win32") await buildFixtureShims(fileURLToPath(new URL("..", import.meta.url)), ["hide-processes"]);
}, fixtureBuildTimeout(["hide-processes"]));

test("a child its owner started after the owner was listed is ended once the owner is gone", { timeout: 30_000 }, async (context) => {
  if (process.platform !== "win32") return context.skip();
  const owner = spawn(process.env.ComSpec ?? "cmd.exe", ["/d", "/c", "ping -n 120 127.0.0.1 >nul"], { windowsHide: true, stdio: "ignore" });
  const listed = windowsProcessTree(owner.pid!).filter((process) => process.id === owner.pid);
  expect(listed).toHaveLength(1);
  let child: { id: number; created: string } | undefined;
  try {
    await expect.poll(() => (child = windowsProcessTree(owner.pid!).find((process) => process.id !== owner.pid)), { timeout: 10_000 }).toBeDefined();
    const exited = new Promise((resolve) => owner.once("exit", resolve));
    owner.kill();
    await exited;
    endWindowsProcesses(listed);
    expect(windowsProcessTree(child!.id).filter((process) => process.created === child!.created)).toEqual([]);
  } finally {
    if (child && windowsProcessTree(child.id).some((process) => process.created === child!.created)) process.kill(child.id);
  }
});

/** A Node child that runs until its input ends. */
async function waitingChild() {
  const child = spawn(process.execPath, ["-e", "process.stdin.resume(); process.stdin.on('end', () => process.exit(0));"], { stdio: ["pipe", "ignore", "ignore"] });
  await once(child, "spawn");
  return child;
}

/**
 * Closes the access list of `pid`, a child this test started ("D:P": no
 * entry, so no one may open it, while its owner keeps the right to the
 * list), and runs `hide-processes end` on `owned` from a PowerShell whose
 * token holds no debug privilege, which would open every process whatever
 * its list says (an administrator's token may hold it turned on). Returns the
 * tool's exit status and what it wrote.
 */
function endClosed(pid: number, owned: string[]): { status: number; output: string } {
  const script = `
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ClosedProcess {
  [StructLayout(LayoutKind.Sequential, Pack = 4)] struct Privilege { public uint Count; public long Luid; public uint Attributes; }
  [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValueW(string system, string name, out long luid);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr token, bool disableAll, ref Privilege state, uint length, IntPtr previous, IntPtr returned);
  [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string text, uint revision, out IntPtr descriptor, IntPtr size);
  [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetSecurityDescriptorDacl(IntPtr descriptor, out bool present, out IntPtr dacl, out bool defaulted);
  [DllImport("advapi32.dll")] static extern uint SetSecurityInfo(IntPtr handle, int type, uint information, IntPtr owner, IntPtr group, IntPtr dacl, IntPtr sacl);
  [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
  public static void WithoutDebugPrivilege() {
    long luid;
    IntPtr token;
    if (!LookupPrivilegeValueW(null, "SeDebugPrivilege", out luid) || !OpenProcessToken(GetCurrentProcess(), 0x28, out token)) throw new System.ComponentModel.Win32Exception();
    // SE_PRIVILEGE_REMOVED; a token without it is left as it is.
    var removed = new Privilege { Count = 1, Luid = luid, Attributes = 4 };
    bool adjusted = AdjustTokenPrivileges(token, false, ref removed, 0, IntPtr.Zero, IntPtr.Zero);
    CloseHandle(token);
    if (!adjusted) throw new System.ComponentModel.Win32Exception();
  }
  public static void Close(uint pid) {
    IntPtr descriptor, dacl;
    bool present, defaulted;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW("D:P", 1, out descriptor, IntPtr.Zero) || !GetSecurityDescriptorDacl(descriptor, out present, out dacl, out defaulted)) throw new System.ComponentModel.Win32Exception();
    // WRITE_DAC, which the owner keeps.
    IntPtr process = OpenProcess(0x40000, false, pid);
    if (process == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
    // SE_KERNEL_OBJECT; DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION.
    uint status = SetSecurityInfo(process, 6, 0x80000004, IntPtr.Zero, IntPtr.Zero, dacl, IntPtr.Zero);
    CloseHandle(process);
    LocalFree(descriptor);
    if (status != 0) throw new System.ComponentModel.Win32Exception((int)status);
  }
}
'@
$ErrorActionPreference = 'Stop'
[ClosedProcess]::WithoutDebugPrivilege()
[ClosedProcess]::Close(${pid})
$said = & $env:HIDE_PROCESSES end - ${owned.join(" ")} 2>&1 | Out-String
"status=$LASTEXITCODE"
$said
`;
  const printed = execFileSync("pwsh", ["-NoProfile", "-Command", script], {
    encoding: "utf8",
    env: { ...process.env, HIDE_PROCESSES: fixtureShimPath("hide-processes") },
    timeout: 60_000, // wait-ok: PowerShell compiles the access helper, then the end runs
  });
  const match = /^status=(\d+)\r?\n([\s\S]*)$/m.exec(printed);
  if (!match) throw new Error(`no status from the closed end: ${printed}`);
  return { status: Number(match[1]), output: match[2]!.trim() };
}

test("an owned process that has ended is not named, and one that cannot be opened is", { timeout: 120_000 }, async (context) => { // wait-ok: PowerShell compiles the access helper on a cold runner
  if (process.platform !== "win32") return context.skip();
  // Ended before the end looks: its identity names nothing left.
  const ended = await waitingChild();
  const endedTree = windowsProcessTree(ended.pid!);
  expect(endedTree.map((item) => item.id)).toEqual([ended.pid]);
  const exited = once(ended, "exit");
  ended.stdin!.end();
  await exited;
  endWindowsProcesses(endedTree);

  // Running, and closed to everyone: the end can neither end it nor see it end.
  const closed = await waitingChild();
  try {
    const owned = windowsProcessTree(closed.pid!).map((item) => `${item.id}:${item.created}`);
    const answer = endClosed(closed.pid!, owned);
    expect(answer).toEqual({ status: 1, output: expect.stringContaining(`${closed.pid} (cannot be opened: error 5)`) });
    expect(closed.exitCode).toBeNull();
  } finally {
    // Node's own handle, opened when it started the child, keeps every right.
    const gone = once(closed, "exit");
    closed.kill();
    await gone;
  }
});
