// Finding the `hide` CLI and reading its answers. The host meets the daemon
// only through `hide connect` (reuse or start, desktop PRD D-03) and
// `hide status --json` (attach-only), whose JSON contract `hided/src/cli.rs`
// owns.

import path from "node:path";
import type { DesktopEnv } from "./env";
import { executableFile, type ChildResult } from "./spawn";
import { isAbsolute } from "./wirePath";

export type CliSource = "env" | "worktree" | "bundled" | "path" | "remembered" | "login" | "well-known";

export type ResolvedCli = {
  found: { path: string; source: CliSource } | null;
  /** Every path looked at, in order and once each, for the log (B2). */
  tried: string[];
};

export type FileProbe = {
  isExecutable(file: string): boolean;
  mtimeMs(file: string): number | null;
};

export type CliSearch = {
  override: string | null;
  /** This worktree's root when run unpackaged; null when packaged. */
  worktreeRoot: string | null;
  /** The packaged app's own `Contents/Resources`, where `hide` and `hided` ship side by side; null when unpackaged. */
  bundledDir: string | null;
  /** This process's PATH: the terminal's for `pnpm dev`, launchd's bare one for an app opened from Finder. */
  searchPath: string;
  /** The last CLI that attached, read back from the profile. */
  remembered: string | null;
  /** Asks the login shell for its PATH; called only when every cheaper step missed, and null when unpackaged. */
  loginPath: (() => Promise<string | null>) | null;
  /** The install folders searched last (`wellKnownDirs`). */
  wellKnown: readonly string[];
};

/** The CLI's file name on this system. */
export const CLI_FILE = executableFile("hide");

/**
 * Whether this system has a login shell to ask for the operator's PATH.
 * Windows has none: an app started from the Start menu gets the account's
 * own PATH, so there is nothing a shell would add.
 */
export const HAS_LOGIN_SHELL = process.platform !== "win32";

/** Folders found from the system's own variables, and the registered keys whose absence left folders out. */
export type Folders = { dirs: string[]; missing: string[] };

/** The variables the install folders are found under, as the environment registry read them (`env.ts`). */
export type Locations = Pick<DesktopEnv, "home" | "localAppData" | "appData" | "programFiles" | "systemRoot">;

/**
 * The folders installers put the programs Hide runs in, searched last for
 * `hide` and appended to every child's PATH, so a daemon started from Finder
 * or the Start menu finds `git`, `gh` and the agent CLIs. On macOS and
 * Linux those are `~/.local/bin`, Homebrew's and `/usr/local/bin`. On Windows
 * each comes from that program's own installer, and a folder whose variable is
 * unset is left out and named rather than guessed:
 *
 * - `~\.local\bin`: Claude Code's installer puts `claude.exe` there. Hide's
 *   kit links `hide` there too, but without the `.exe` Windows needs, so the
 *   search does not find it yet.
 * - `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin`: the Codex installer's
 *   default folder (`install.ps1`'s visible bin folder).
 * - `%LOCALAPPDATA%\Programs\Git\cmd` and `%ProgramFiles%\Git\cmd`: Git for
 *   Windows, installed for the account or for the machine (`{userpf}` and
 *   `{commonpf}` in its installer), whose `cmd` folder is what it puts on PATH.
 * - `%APPDATA%\npm`: the global folder the Node.js installer gives npm, where
 *   `npm install -g` puts `claude` and `codex`.
 * - `%ProgramFiles%\GitHub CLI` and `%ProgramFiles%\nodejs`: the GitHub CLI's
 *   and Node.js's installers' folders; npm's commands run on `node`.
 */
export function wellKnownDirs(at: Locations): Folders {
  if (process.platform !== "win32") return { dirs: [path.join(at.home, ".local", "bin"), "/opt/homebrew/bin", "/usr/local/bin"], missing: [] };
  const missing: string[] = [];
  const under = (key: string, root: string | null, ...folders: string[][]): string[] => {
    if (root === null) {
      missing.push(key);
      return [];
    }
    return folders.map((names) => path.join(root, ...names));
  };
  const dirs = [
    path.join(at.home, ".local", "bin"),
    ...under("LOCALAPPDATA", at.localAppData, ["Programs", "OpenAI", "Codex", "bin"], ["Programs", "Git", "cmd"]),
    ...under("APPDATA", at.appData, ["npm"]),
    ...under("ProgramFiles", at.programFiles, ["Git", "cmd"], ["GitHub CLI"], ["nodejs"]),
  ];
  return { dirs, missing };
}

/**
 * The system's own folders, which a child gets when PATH is absent so setting
 * it for user tools does not lose system commands: launchd's PATH on macOS
 * and Linux, and on Windows the default system Path a fresh install has
 * (`System32`, the Windows folder, `Wbem`, Windows PowerShell and OpenSSH under
 * `%SystemRoot%`), left out and named when SystemRoot is unset.
 */
export function systemDirs(at: Locations): Folders {
  if (process.platform !== "win32") return { dirs: ["/usr/bin", "/bin", "/usr/sbin", "/sbin"], missing: [] };
  const root = at.systemRoot;
  if (root === null) return { dirs: [], missing: ["SystemRoot"] };
  const system32 = path.join(root, "System32");
  return { dirs: [system32, root, path.join(system32, "Wbem"), path.join(system32, "WindowsPowerShell", "v1.0"), path.join(system32, "OpenSSH")], missing: [] };
}

/** Finds worth remembering once they attach; an override, a worktree build or the app's own bundled CLI is not the operator's installed CLI. */
export const REMEMBERED_SOURCES: ReadonlySet<CliSource> = new Set(["path", "login", "well-known"]);

function inDirs(searchPath: string): string[] {
  return searchPath
    .split(path.delimiter)
    // A rooted entry without a drive (`\tools`) names a folder on whatever
    // drive is current, and would be remembered so; it is skipped, as
    // `hide_platform::host::find_program` skips it.
    .filter((dir) => dir && isAbsolute(dir))
    .map((dir) => path.join(dir, CLI_FILE));
}

/**
 * B2's order: the override, then (unpackaged) this worktree's newest build,
 * then (packaged) the CLI the app ships in its own Resources, then PATH, the
 * CLI that last attached, the login shell's PATH, and the usual install
 * directories. An app opened from Finder gets launchd's PATH, not the
 * operator's, so a packaged app normally stops at its bundled CLI and the
 * steps after PATH exist for an app whose bundle is missing it; the login
 * shell is the slow step, which the remembered path spares every launch
 * after the first. An override that is set but unusable ends the search: it
 * names what the operator asked for, and quietly using another binary would
 * hide the mistake.
 */
export async function resolveCli(input: CliSearch, probe: FileProbe): Promise<ResolvedCli> {
  const tried: string[] = [];
  const done = (file: string | null, source: CliSource): ResolvedCli => ({ found: file ? { path: file, source } : null, tried });
  if (input.override) {
    tried.push(input.override);
    return done(probe.isExecutable(input.override) ? input.override : null, "env");
  }
  if (input.worktreeRoot) {
    const builds = ["debug", "release"].map((profile) => path.join(input.worktreeRoot as string, "target", profile, CLI_FILE));
    tried.push(...builds);
    const newest = builds
      .filter((file) => probe.isExecutable(file))
      .map((file) => ({ file, mtime: probe.mtimeMs(file) ?? 0 }))
      .sort((a, b) => b.mtime - a.mtime)[0];
    if (newest) return done(newest.file, "worktree");
  }
  if (input.bundledDir) {
    const bundled = path.join(input.bundledDir, CLI_FILE);
    tried.push(bundled);
    if (probe.isExecutable(bundled)) return done(bundled, "bundled");
  }
  const first = (files: string[]): string | null => {
    for (const file of files) {
      if (tried.includes(file)) continue;
      tried.push(file);
      if (probe.isExecutable(file)) return file;
    }
    return null;
  };
  const steps: [CliSource, () => Promise<string[]>][] = [
    ["path", async () => inDirs(input.searchPath)],
    ["remembered", async () => (input.remembered ? [input.remembered] : [])],
    ["login", async () => (input.loginPath ? inDirs((await input.loginPath()) ?? "") : [])],
    ["well-known", async () => input.wellKnown.map((dir) => path.join(dir, CLI_FILE))],
  ];
  for (const [source, files] of steps) {
    const file = first(await files());
    if (file) return done(file, source);
  }
  return { found: null, tried };
}

const REMEMBERED_SCHEMA = 1;

export function rememberedCliPath(userData: string): string {
  return path.join(userData, "cli-path.json");
}

/** The remembered CLI path; null when none is stored; `{ unreadable }` when the file is not one this host wrote. */
export function parseRememberedCli(stored: unknown): string | null | { unreadable: string } {
  if (stored === null) return null;
  const value = typeof stored === "object" ? (stored as Record<string, unknown>) : null;
  if (value && typeof value.unreadable === "string") return { unreadable: value.unreadable };
  if (value?.schema !== REMEMBERED_SCHEMA || typeof value.path !== "string" || !isAbsolute(value.path)) return { unreadable: "unexpected shape" };
  return value.path;
}

export function rememberedCliValue(file: string): unknown {
  return { schema: REMEMBERED_SCHEMA, path: file };
}

export type FailureReason = "cli_missing" | "start_failed" | "no_response" | "other_build" | "state_refused" | "core_newer" | "update_failed";

/**
 * The core's machine and the two builds, as `hide connect` and a node's `/health` name them: for a core newer
 * than this app (PRD core-host-node-move B11) and for an update of the core that failed (B10).
 */
export type CoreBuilds = { machine: string; core: string; app: string };

function coreBuilds(value: Record<string, unknown> | null | undefined): CoreBuilds | null {
  if (!value || typeof value.machine !== "string" || typeof value.core !== "string" || typeof value.app !== "string") return null;
  return { machine: value.machine, core: value.core, app: value.app };
}

/**
 * What an attached node's `/health` says the window shows instead of the shell: a core newer than this app, or
 * this connection's update of the core that failed, with the builds to name. Null for anything else.
 */
export function healthFailure(health: unknown): { reason: "core_newer" | "update_failed"; builds: CoreBuilds } | null {
  if (!health || typeof health !== "object") return null;
  const value = health as Record<string, unknown>;
  const reason = value.core_link_reason;
  if (reason !== "core_newer" && reason !== "update_failed") return null;
  const builds = coreBuilds(value.builds as Record<string, unknown> | null | undefined);
  return builds ? { reason, builds } : null;
}

export type Attached = { url: string; origin: string; port: number; pid: number };

export type ConnectAnswer =
  | ({ kind: "attached" } & Attached)
  | { kind: "failed"; reason: Exclude<FailureReason, "cli_missing">; detail: string; file?: string; builds?: CoreBuilds };

export type StatusAnswer = ({ running: true } & Attached) | { running: false };

function lastJsonLine(stdout: string): Record<string, unknown> | null {
  const line = stdout
    .split("\n")
    .map((row) => row.trim())
    .filter(Boolean)
    .at(-1);
  if (!line) return null;
  try {
    const value: unknown = JSON.parse(line);
    return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** A daemon URL is loopback HTTP with the token in its hash, or it is not one. */
function attached(value: Record<string, unknown>): Attached | null {
  const { url, port, pid } = value;
  if (typeof url !== "string" || typeof port !== "number" || typeof pid !== "number") return null;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" || parsed.hostname !== "127.0.0.1" || Number(parsed.port) !== port) return null;
  if (!new URLSearchParams(parsed.hash.slice(1)).get("token")) return null;
  return { url, origin: parsed.origin, port, pid };
}

function unreadable(result: ChildResult): string {
  const stderr = result.stderr.trim().split("\n").at(-1) ?? "";
  return `unreadable answer (exit ${result.code ?? result.signal ?? "none"})${stderr ? `: ${stderr}` : ""}`;
}

/**
 * The records a `hide` verb wrote on standard error, one JSON object with a
 * `kind` per line, for the host log: what `hide connect` did on its way (a
 * daemon of another build replaced) is lost with the child otherwise. Only flat values are kept.
 */
export function cliRecords(stderr: string): { event: string; fields: Record<string, string | number | boolean | null> }[] {
  const records: { event: string; fields: Record<string, string | number | boolean | null> }[] = [];
  for (const line of stderr.split("\n")) {
    let value: unknown;
    try {
      value = JSON.parse(line.trim());
    } catch {
      continue;
    }
    if (!value || typeof value !== "object" || typeof (value as { kind?: unknown }).kind !== "string") continue;
    const { kind, ...rest } = value as Record<string, unknown>;
    const fields = Object.fromEntries(
      Object.entries(rest).filter(
        (entry): entry is [string, string | number | boolean | null] =>
          entry[0] !== "component" && (entry[1] === null || ["string", "number", "boolean"].includes(typeof entry[1])),
      ),
    );
    records.push({ event: kind as string, fields });
  }
  return records;
}

export function parseConnect(result: ChildResult): ConnectAnswer {
  if (result.spawnError) return { kind: "failed", reason: "start_failed", detail: result.spawnError };
  if (result.timedOut) return { kind: "failed", reason: "no_response", detail: "hide connect timed out" };
  const value = lastJsonLine(result.stdout);
  if (value?.ok === true) {
    const daemon = attached(value);
    if (daemon) return { kind: "attached", ...daemon };
  }
  // The daemon refused its state folder (PRD core-host-node B2): the screen names the file.
  if (value?.ok === false && value.reason === "state_refused" && typeof value.file === "string") {
    return { kind: "failed", reason: "state_refused", detail: typeof value.detail === "string" ? value.detail : "", file: value.file };
  }
  // The core runs a newer build than this app, or this connection's update of it failed: the screen names its
  // machine and both builds.
  if (value?.ok === false && (value.reason === "core_newer" || value.reason === "update_failed")) {
    const builds = coreBuilds(value);
    if (builds) return { kind: "failed", reason: value.reason, detail: typeof value.detail === "string" ? value.detail : "", builds };
  }
  if (value?.ok === false && (value.reason === "start_failed" || value.reason === "no_response" || value.reason === "other_build")) {
    return { kind: "failed", reason: value.reason, detail: typeof value.detail === "string" ? value.detail : "" };
  }
  return { kind: "failed", reason: "start_failed", detail: unreadable(result) };
}

/**
 * Where `hide runtime prepare` says this package's runtime runs from (PRD
 * auto-update D-10): an absolute folder, or why there is none. A folder that
 * cannot be read from the answer is a failure, never the install folder.
 */
export function parseRuntimePrepare(result: ChildResult): { dir: string } | { failed: string } {
  if (result.spawnError) return { failed: result.spawnError };
  if (result.timedOut) return { failed: "hide runtime prepare timed out" };
  const value = lastJsonLine(result.stdout);
  if (value?.ok === true && typeof value.dir === "string" && isAbsolute(value.dir)) return { dir: value.dir };
  return { failed: unreadable(result) };
}

/** Null when the probe itself did not answer; the caller asks again later. */
export function parseStatus(result: ChildResult): StatusAnswer | null {
  if (result.spawnError || result.timedOut) return null;
  const value = lastJsonLine(result.stdout);
  if (value?.running === false) return { running: false };
  if (value?.running === true) {
    const daemon = attached(value);
    return daemon ? { running: true, ...daemon } : null;
  }
  return null;
}

const PATH_MARK = "__HIDE_LOGIN_PATH__";

/**
 * The login shell asked for its PATH, printed between two marks past any
 * rc-file output. It runs through `env` to set what shell-env sets: Oh My
 * Zsh's auto-update prompt and its tmux autostart can otherwise hold an
 * interactive shell open until the timeout.
 */
export function loginPathCommand(shell: string): { file: string; args: string[] } {
  return {
    file: "/usr/bin/env",
    args: [
      "DISABLE_AUTO_UPDATE=true",
      "ZSH_TMUX_AUTOSTARTED=true",
      "ZSH_TMUX_AUTOSTART=false",
      shell,
      "-ilc",
      `printf '\\n${PATH_MARK}%s${PATH_MARK}\\n' "$PATH"`,
    ],
  };
}

export function parseLoginPath(stdout: string): string | null {
  const match = new RegExp(`${PATH_MARK}(.*?)${PATH_MARK}`).exec(stdout);
  return match?.[1] ? match[1] : null;
}
