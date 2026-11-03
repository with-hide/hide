// A real pane process reports its own native session and starts each Hide CLI
// as a descendant. No session, lineage or capability is injected by the test.
import { expect } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import type { HerdrFixture } from "../../web/e2e/herdr-fixture";
import { fixtureExecutable, fixtureToolPath, inheritedFixtureEnv } from "../../web/e2e/platform-fixture";
import { copyFixtureShim } from "../../web/e2e/shims/build";
import { configureSpawnProvider, writeSessionReporter } from "../../web/e2e/spawn-provider";
import { powershellQuote, windowsArgument } from "../../web/e2e/windows-command";

type SpawnProvider = { script: string; completed: string };
export type SpawnedAgent = { id: string; pane: string; machine: string; parent: string | null; origin: string | null; watch: string | null; session: string };
const shellQuote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

function powershellBinary(bin: string): string {
  // Reuse the controlled system-tool boundary, never the account's PATH.
  const tool = fixtureToolPath(bin).split(path.delimiter)
    .map((directory) => path.join(directory, "powershell.exe"))
    .find((candidate) => fs.existsSync(candidate));
  if (!tool) throw new Error("Windows spawn fixture needs the native Windows PowerShell executable");
  return tool;
}

export function installSpawnProvider(herdr: HerdrFixture, root: string): SpawnProvider {
  const provider = {
    script: path.join(root, process.platform === "win32" ? "command.ps1" : "command.sh"),
    completed: path.join(root, "command-completed"),
  };
  const runner = process.platform === "win32"
    ? [powershellBinary(path.join(herdr.root, "bin")), "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", provider.script]
    : ["/bin/sh", provider.script];
  configureSpawnProvider(herdr.root, { reporter: writeSessionReporter(herdr), completed: provider.completed, command: runner });
  copyFixtureShim("spawn-provider", path.join(herdr.root, "bin", fixtureExecutable("claude")));
  return provider;
}

/** The caller sends one trigger only after this script is ready. */
export function prepareSpawnCommand(provider: SpawnProvider, binary: string, args: string[], env: NodeJS.ProcessEnv, response: string): void {
  fs.rmSync(response, { force: true });
  fs.rmSync(provider.completed, { force: true });
  // Preserve Herdr's actual pane/session identity inherited by the provider.
  // Move every native home lookup, provider home and daemon setting together.
  const privateEnv = Object.entries(env).flatMap(([key, value]) => {
    const canonical = process.platform === "win32" ? key.toUpperCase() : key;
    const home = ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "PATH"];
    return value !== undefined && (home.includes(canonical) || canonical.startsWith("XDG_") || canonical.startsWith("HIDE_") || canonical === "HERDR_BIN_PATH")
      ? [[canonical, value] as const] : [];
  });
  if (process.platform !== "win32") {
    const argv = ["env", ...privateEnv.map(([key, value]) => `${key}=${value}`), binary, ...args];
    fs.writeFileSync(provider.script, `#!/bin/sh\nexec ${argv.map(shellQuote).join(" ")} > ${shellQuote(response)}\n`, { mode: 0o700 });
    return;
  }
  // ProcessStartInfo bypasses PowerShell 5's native argument rewriting. The
  // entire command is an argv string encoded once for the Windows process API.
  const script = `$ErrorActionPreference = 'Stop'
$fixtureStart = New-Object System.Diagnostics.ProcessStartInfo
$fixtureStart.FileName = ${powershellQuote(binary)}
$fixtureStart.Arguments = ${powershellQuote(args.map(windowsArgument).join(" "))}
$fixtureStart.UseShellExecute = $false
$fixtureStart.RedirectStandardOutput = $true
$fixtureStart.StandardOutputEncoding = New-Object System.Text.UTF8Encoding($false)
${privateEnv.map(([key, value]) => `$fixtureStart.EnvironmentVariables[${powershellQuote(key)}] = ${powershellQuote(value)}`).join("\n")}
$fixtureChild = New-Object System.Diagnostics.Process
$fixtureChild.StartInfo = $fixtureStart
try {
  if (-not $fixtureChild.Start()) { throw 'fixture CLI did not start' }
  $fixtureOutput = $fixtureChild.StandardOutput.ReadToEndAsync()
  if (-not $fixtureChild.WaitForExit(20000)) { throw 'fixture CLI exceeded its process bound' }
  [System.IO.File]::WriteAllText(${powershellQuote(response)}, $fixtureOutput.GetAwaiter().GetResult(), (New-Object System.Text.UTF8Encoding($false)))
  exit $fixtureChild.ExitCode
} finally {
  $fixtureChild.Dispose()
}
`;
  // Windows PowerShell 5 needs a BOM to read non-ASCII paths and arguments.
  fs.writeFileSync(provider.script, `\uFEFF${script}`);
}

/** Run one delivery command as the real native caller, observing its reply. */
export async function nativeSpawnCommand(herdr: HerdrFixture, provider: SpawnProvider, pane: string, binary: string, args: string[], env: NodeJS.ProcessEnv, response: string): Promise<SpawnedAgent> {
  prepareSpawnCommand(provider, binary, args, env, response);
  execFileSync(herdr.bin, ["pane", "send-text", pane, "!"], { env: herdr.env, timeout: 20_000 });
  await expect.poll(() => {
    if (!fs.existsSync(provider.completed)) return null;
    const completed = fs.readFileSync(provider.completed, "utf8").trim();
    if (!completed) return null;
    if (Number(completed) !== 0) throw new Error(`native spawn fixture command exited ${completed}`);
    try { return JSON.parse(fs.readFileSync(response, "utf8")) as unknown; } catch { return null; }
  }).not.toBeNull();
  const answer = JSON.parse(fs.readFileSync(response, "utf8")) as { ok: boolean; value: SpawnedAgent };
  expect(answer.ok, JSON.stringify(answer)).toBe(true);
  return answer.value;
}

/** A prompt can precede the shell becoming available to agent.start. */
export async function waitForSpawnShell(herdr: HerdrFixture, pane: string): Promise<void> {
  const deadline = Date.now() + 10_000;
  let shell: number | null = null;
  await expect.poll(() => {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error("fixture shell was unavailable before the single agent start");
    const answer = JSON.parse(execFileSync(herdr.bin, ["pane", "process-info", "--pane", pane], {
      env: herdr.env, encoding: "utf8", timeout: remaining,
    })) as {
      result: { process_info: { shell_pid: number | null; foreground_process_group_id: number | null; foreground_processes: { pid: number }[] } };
    };
    const info = answer.result.process_info;
    shell = info.shell_pid;
    return shell !== null && shell > 1 && info.foreground_process_group_id === shell
      && info.foreground_processes.every((process) => process.pid === shell);
  }).toBe(true);
  if (process.platform !== "win32") return;
  const remaining = deadline - Date.now();
  if (remaining <= 0 || !shell) throw new Error("fixture shell was unavailable before the single agent start");
  // The pinned Herdr reports cmd as foreground while a non-agent child is
  // running. Observe its actual children, as the shared Herdr fixture does.
  execFileSync(powershellBinary(path.join(herdr.root, "bin")), ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `
    $ErrorActionPreference = 'Stop'
    $fixtureUntil = [DateTime]::UtcNow.AddMilliseconds(${remaining})
    do {
      $fixtureProcesses = @(Get-CimInstance -ClassName Win32_Process -Filter 'ProcessId = ${shell} OR ParentProcessId = ${shell}' -Property ProcessId, ParentProcessId)
      if (@($fixtureProcesses | Where-Object { $_.ProcessId -eq ${shell} }).Count -ne 1) { throw 'fixture shell exited before agent start' }
      if (@($fixtureProcesses | Where-Object { $_.ParentProcessId -eq ${shell} }).Count -eq 0) { exit 0 }
      Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $fixtureUntil)
    throw 'fixture shell still has child processes before agent start'
  `], { env: { ...inheritedFixtureEnv(), ...herdr.env }, timeout: remaining });
}
