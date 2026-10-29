// Native spellings for the isolated e2e homes and tools. The home/state
// contract is hide-platform::host; Herdr accepts a filesystem socket path
// on every OS and maps that path to a named pipe on Windows.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { endWindowsTree, listWindowsTree, type WindowsProcess } from "./process-tree";
import { copyFixtureShim, fixtureShimPath } from "./shims/build";

export type { WindowsProcess };

export const fixtureExecutable = (name: string): string => `${name}${process.platform === "win32" ? ".exe" : ""}`;

/**
 * Where a client reaches the local stream Herdr names by `socket`: the path
 * itself on Unix, the pipe `\\.\pipe\<path>` on Windows (hide-platform's
 * `ipc` makes the same mapping, so a fixture standing in front of Herdr
 * listens and connects there).
 */
export const localEndpoint = (socket: string): string => (process.platform === "win32" ? `\\\\.\\pipe\\${socket}` : socket);

// `launcher.c` (built once, e2e/shims/build.ts) runs the Node script next to it
// that shares its name: `name.exe` runs `name.js`, interpreted by the program
// its first line (`#!<path>`) names, with the arguments it was given, and
// exits with that program's status.

/**
 * An executable `name` in `dir` that runs the Node `script`, the same way on
 * every system: Unix runs the file through its interpreter line, and Windows,
 * which cannot run a script by name, gets `name.exe`, one compiled launcher
 * copied beside `name.js`. Returns the executable's path. The script reads
 * its arguments from `process.argv.slice(2)`.
 */
export function fixtureProgram(dir: string, name: string, script: string): string {
  fs.mkdirSync(dir, { recursive: true });
  const body = `#!${process.execPath}\n${script}`;
  if (process.platform !== "win32") {
    const executable = path.join(dir, name);
    fs.writeFileSync(executable, body, { mode: 0o755 });
    return executable;
  }
  fs.writeFileSync(path.join(dir, `${name}.js`), body);
  const executable = path.join(dir, `${name}.exe`);
  copyFixtureShim("launcher", executable);
  return executable;
}

/** What the system says a running process has used: cumulative CPU seconds and resident memory. */
export function processUsage(pid: number): { cpuSeconds: number; rssKiB: number } {
  if (process.platform === "win32") {
    const [cpu, bytes] = powershell(`$p = Get-Process -Id ${pid}; "$($p.TotalProcessorTime.TotalSeconds) $($p.WorkingSet64)"`).trim().split(" ");
    return { cpuSeconds: Number(cpu), rssKiB: Number(bytes) / 1024 };
  }
  const [time, rss] = execFileSync("/bin/ps", ["-p", String(pid), "-o", "time=,rss="], { encoding: "utf8" }).trim().split(/\s+/);
  return { cpuSeconds: time!.split(":").map(Number).reduce((sum, part) => sum * 60 + part, 0), rssKiB: Number(rss) };
}

/** Windows environment keys are case-insensitive, including Path/PATH. */
export function inheritedFixtureEnv(): Record<string, string> {
  return Object.fromEntries(Object.entries(process.env).flatMap(([key, value]) => {
    const canonical = process.platform === "win32" ? key.toUpperCase() : key;
    if (value === undefined || ["HERDR_", "HIDE_", "ELECTRON_", "HCOORD_", "SASU_"].some((prefix) => key.toUpperCase().startsWith(prefix))) return [];
    return [[canonical, value]];
  }));
}

/** Move native home lookups and CLI credential discovery together. */
export function fixtureHomeEnv(home: string): Record<string, string> {
  // Disable-only marker: works on every OS and with the shipped build token.
  // It grants none of the privileged device fixture's service-control rights.
  fs.mkdirSync(path.join(home, ".hide"), { recursive: true });
  fs.writeFileSync(path.join(home, ".hide", "product-analytics-disabled"), "");
  const windows: Record<string, string> = process.platform === "win32" ? {
    USERPROFILE: home,
    APPDATA: path.join(home, "AppData", "Roaming"),
    LOCALAPPDATA: path.join(home, "AppData", "Local"),
  } : {};
  for (const dir of Object.values(windows)) fs.mkdirSync(dir, { recursive: true });
  return {
    HOME: home,
    ...windows,
    XDG_CONFIG_HOME: path.join(home, ".config"),
    XDG_STATE_HOME: path.join(home, ".local", "state"),
    CLAUDE_CONFIG_DIR: path.join(home, ".claude"),
    CODEX_HOME: path.join(home, ".codex"),
    // The background fetch runs `git fetch origin` for every registered repository; a fixture's
    // `https://github.com/example/...` origin is refused as a transport here and never reaches the network.
    GIT_ALLOW_PROTOCOL: "file",
  };
}

/**
 * The fixture HOME's login shell puts `toolPath` first, as an account's own
 * startup files put the folders it installs into first. A daemon starts the
 * programs it names (`gh`, `git`, Hide AI's agents) from that shell's PATH
 * before its own (`hide_platform::programs`), and macOS's `path_helper` puts
 * the system's folders, with a runner's own `gh` and `git`, ahead of the PATH
 * a shell was handed, so without this a shim on the fixture PATH loses to the
 * runner's program. Windows asks no login shell.
 */
export function fixtureLoginShellPath(home: string, toolPath: string): void {
  if (process.platform === "win32") return;
  fs.mkdirSync(home, { recursive: true });
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  const posix = `PATH=${quote(toolPath)}:"$PATH"\nexport PATH\n`;
  for (const file of [".zprofile", ".bash_profile", ".profile"]) fs.writeFileSync(path.join(home, file), posix);
  const fish = path.join(home, ".config", "fish", "config.fish");
  fs.mkdirSync(path.dirname(fish), { recursive: true });
  fs.writeFileSync(fish, `set -gx PATH ${toolPath.split(path.delimiter).map(quote).join(" ")} $PATH\n`);
}

/** System tools only: do not append the account's agent-provider PATH. */
export function fixtureToolPath(bin: string): string {
  if (process.platform !== "win32") return [bin, "/usr/bin", "/bin", "/usr/sbin", "/sbin"].join(path.delimiter);
  const env = inheritedFixtureEnv();
  const system = env.SYSTEMROOT;
  const programs = env.PROGRAMFILES;
  if (!system || !path.isAbsolute(system) || !programs || !path.isAbsolute(programs)) throw new Error("Windows fixture needs absolute SystemRoot and ProgramFiles for its controlled system-tool PATH");
  return [bin, path.join(system, "System32"), system, path.join(system, "System32", "WindowsPowerShell", "v1.0"), path.join(programs, "Git", "cmd"), path.join(programs, "Git", "usr", "bin")].join(path.delimiter);
}

/** The Unix no-op has no native Windows counterpart; use the compiled shim. */
export function fixtureOpenCommand(root?: string, privateRoot?: string): string {
  if (process.platform !== "win32") return "/usr/bin/true";
  if (!root && !privateRoot) throw new Error("Windows fixture opener needs a private root");
  const command = path.join((root ?? privateRoot)!, "bin", fixtureExecutable("hide-open"));
  // Unit/private-home fixtures need no Herdr server. Give those homes the
  // same native no-op without falling back to an account's GUI opener.
  if (!root) {
    fs.mkdirSync(path.dirname(command), { recursive: true });
    copyFixtureShim("noop", command);
  }
  if (!fs.existsSync(command)) throw new Error(`Windows fixture opener is missing: ${command}`);
  return command;
}

const powershell = (script: string, env: NodeJS.ProcessEnv = {}): string =>
  execFileSync("powershell.exe", ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", `$ErrorActionPreference = 'Stop'\n${script}`], {
    env: { ...process.env, ...env },
    encoding: "utf8",
    timeout: 30_000,
    maxBuffer: 1024 * 1024,
    windowsHide: true,
  });

/**
 * What holds TCP connections on this Windows machine now: the total, the
 * count in each state, and the ten process names holding the most. Chromium's
 * `ERR_NO_BUFFER_SPACE` (WSAENOBUFS) on a loopback connect to a test's hided
 * left nothing saying who held the sockets; `windows-sockets-reporter.ts`
 * prints this when a test attempt fails.
 */
export function windowsTcpSummary(): string[] {
  return powershell(`
    $connections = @(Get-NetTCPConnection)
    $names = @{}
    foreach ($process in Get-Process) { $names[[int]$process.Id] = $process.ProcessName }
    "TCP connections: $($connections.Count)"
    "by state: " + (($connections | Group-Object State | Sort-Object Count -Descending | ForEach-Object { "$($_.Name) $($_.Count)" }) -join ', ')
    "by process: " + (($connections | ForEach-Object { $name = $names[[int]$_.OwningProcess]; if ($name) { $name } else { "pid $($_.OwningProcess)" } } |
      Group-Object | Sort-Object Count -Descending | Select-Object -First 10 | ForEach-Object { "$($_.Name) $($_.Count)" }) -join ', ')
  `).split(/\r?\n/).filter(Boolean);
}

/** `pid` and its live descendants, as the fixtures' `hide-processes` lists them (`listWindowsTree`). */
export function windowsProcessTree(pid: number): WindowsProcess[] {
  return listWindowsTree(fixtureShimPath("hide-processes"), pid);
}

/**
 * Ends every `vctip.exe` and returns their pids once each has exited. MSVC's
 * compiler and linker start that telemetry helper and leave it running after
 * a build, still naming as its parent a linker that has exited; Windows
 * reuses that pid, and a pane shell that gets it has the helper for a child,
 * so Herdr 0.9.1, which counts a shell's children by parent pid alone, refuses
 * `agent start` there for as long as the helper runs. Throws naming the
 * survivors after five seconds.
 */
export function endVctip(): number[] {
  const ended = powershell(`
    $found = @(Get-Process -Name vctip -ErrorAction SilentlyContinue)
    foreach ($process in $found) { Stop-Process -InputObject $process -Force -ErrorAction SilentlyContinue }
    if ($found.Count -gt 0) { Wait-Process -InputObject $found -Timeout 5 }
    ConvertTo-Json -Compress -InputObject @($found | ForEach-Object { $_.Id })
  `);
  return JSON.parse(ended) as number[];
}

/** Ends `owned` and what they started, and given `root` what runs from under it, with the fixtures' `hide-processes` (`endWindowsTree`). */
export function endWindowsProcesses(owned: WindowsProcess[], root?: string): void {
  endWindowsTree(fixtureShimPath("hide-processes"), owned, root);
}
