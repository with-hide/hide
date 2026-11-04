// Every environment key the desktop host reads, in one enumerable registry
// (engineering practice env.md). Nothing else under src/ reads
// the process environment; `env.test.ts` scans for it.
//
// The daemon's own keys (HIDE_STATE_DIR, ...) are not read here: the `hide`
// CLI inherits this process's environment and `hided/src/env.rs` owns them.
// HOME (USERPROFILE on Windows) is read by both.

import os from "node:os";
import { HAS_LOGIN_SHELL } from "./cli";
import { isAbsolute } from "./wirePath";

export type EnvKey = {
  key: string;
  requirement: "optional";
  shape: string;
  fallback: string;
  note: string;
};

export const ENV_REGISTRY: readonly EnvKey[] = [
  {
    key: "HIDE_CLI_PATH",
    requirement: "optional",
    shape: "absolute path of an executable `hide` CLI",
    fallback: "this worktree's target/{debug,release}/hide when run unpackaged, the app's own bundled CLI when packaged, then PATH, the CLI that last attached, the login shell's PATH (packaged, macOS and Linux only), and each system's install folders (`cli.ts`, `wellKnownDirs`)",
    note: "Names the CLI the host asks for the daemon; when it is set but not executable the window says the CLI was not found instead of searching on",
  },
  {
    key: "HIDE_DESKTOP_USER_DATA_DIR",
    requirement: "optional",
    shape: "absolute directory path",
    fallback: "hide-desktop in Electron's appData folder: ~/Library/Application Support on macOS, %APPDATA% on Windows, ~/.config on Linux",
    note: "Isolates window state, the single-instance lock, web storage and the host log for a QA or e2e instance; without it the operator's desktop profile is used",
  },
  {
    key: "SHELL",
    requirement: "optional",
    shape: "absolute path of the login shell; read on macOS and Linux only",
    fallback: "the account's login shell in the user database (`os.userInfo().shell`); none on Windows, which has no login shell, and none where the account names no shell",
    note: "A packaged app launched from Finder asks this shell for the operator's PATH when neither PATH nor the CLI that last attached has `hide`; with no shell at all that step is skipped and `cli.login_path` records why",
  },
  {
    key: "HOME",
    requirement: "optional",
    shape: "absolute directory path; read on macOS and Linux only",
    fallback: "the account's home directory",
    note: "Names the `~/.local/bin` the CLI search tries and every `hide` child's PATH gains; Electron's own home path ignores it, which would let an isolated QA instance find the operator's CLI",
  },
  {
    key: "USERPROFILE",
    requirement: "optional",
    shape: "absolute directory path; read on Windows only, where it names the account's home as HOME does elsewhere",
    fallback: "the account's home directory",
    note: "Names the `~/.local/bin` the CLI search tries and every `hide` child's PATH gains on Windows, the folder Claude Code's installer puts `claude.exe` in",
  },
  {
    key: "LOCALAPPDATA",
    requirement: "optional",
    shape: "absolute directory path; read on Windows only",
    fallback: "none: the install folders below it are left out of the search and of every child's PATH, and `env.locations_missing` names the key",
    note: "Holds the per-account installs of Codex (`Programs\\OpenAI\\Codex\\bin`), Herdr (`Programs\\Herdr\\bin`) and Git for Windows (`Programs\\Git\\cmd`)",
  },
  {
    key: "APPDATA",
    requirement: "optional",
    shape: "absolute directory path; read on Windows only",
    fallback: "none: npm's global folder is left out of the search and of every child's PATH, and `env.locations_missing` names the key",
    note: "Holds `npm`, the global folder the Node.js installer gives npm, where `npm install -g` puts `claude` and `codex`",
  },
  {
    key: "ProgramFiles",
    requirement: "optional",
    shape: "absolute directory path; read on Windows only",
    fallback: "none: the machine-wide install folders are left out of the search and of every child's PATH, and `env.locations_missing` names the key",
    note: "Holds the machine-wide installs of Git for Windows (`Git\\cmd`), the GitHub CLI (`GitHub CLI`) and Node.js (`nodejs`), which npm's `claude` and `codex` commands run on",
  },
  {
    key: "SystemRoot",
    requirement: "optional",
    shape: "absolute directory path; read on Windows only",
    fallback: "none: a child started with PATH absent gets no system folders, and `env.locations_missing` names the key",
    note: "Names the Windows folder whose `System32`, `System32\\Wbem`, PowerShell and OpenSSH folders make the system's default Path, which a child gets when PATH is absent",
  },
  {
    key: "HIDE_DESKTOP_SYSTEM_LANGUAGE",
    requirement: "optional",
    shape: "a BCP 47 language tag such as ko-KR",
    fallback: "the operating system's primary language (`app.getPreferredSystemLanguages()`)",
    note: "Stands in for the system language when the operator has made no explicit choice, so a QA or e2e instance draws the menu and status page in a known language",
  },
  {
    key: "HIDE_UPDATE_FEED_URL",
    requirement: "optional",
    shape: "an http or https URL on this machine (127.0.0.1, localhost or [::1]) of a folder holding an update file (latest-mac.yml, latest.yml or latest-linux.yml) and the files it names",
    fallback: "the release `updater.ts` chooses from with-hide/hide's GitHub releases",
    note: "Points a packaged release build's updater at another feed, for the CI update test (PRD auto-update D-29); the build must still be one that updates itself. The update file's sha512 comes from that same feed, so setting it trusts whoever serves the feed with what the app installs; the host logs `update.feed_override` at launch whenever it is set",
  },
  {
    key: "PATH",
    requirement: "optional",
    shape: "directories separated by the system's PATH separator (`:`, `;` on Windows)",
    fallback: "empty: the search goes on to the CLI that last attached",
    note: "Searched for `hide` after the override and the worktree build; every `hide` child gets the system's own folders when PATH is absent and the install folders appended (`cli.ts`, `wellKnownDirs`) so a daemon started from Finder or the Start menu can find installed tools such as `git`, `gh`, the agent CLIs and Herdr",
  },
];

export type DesktopEnv = {
  cliPath: string | null;
  userDataDir: string | null;
  /** A language tag standing in for the system's; null on a normal launch. */
  systemLanguage: string | null;
  /** Another update feed than with-hide/hide's releases; null on a normal launch. */
  updateFeedUrl: string | null;
  /** The login shell; null on Windows, and where neither SHELL nor the user database names one. */
  shell: string | null;
  home: string;
  /** Windows' own folders; null when unset, and always null on macOS and Linux, which do not read them. */
  localAppData: string | null;
  appData: string | null;
  programFiles: string | null;
  systemRoot: string | null;
  path: string;
  /** What every `hide` child inherits unchanged; hided's own keys travel in it and `hided/src/env.rs` owns them. */
  inherited: Record<string, string | undefined>;
};

/** The account's login shell in the user database; null where it names none or the account has no entry there. */
function accountShell(): string | null {
  try {
    return os.userInfo().shell || null;
  } catch {
    return null;
  }
}

export type EnvProblem = { key: string; kind: string };

/** Validates every key at once; a problem names the key and the kind, never the value. */
export function loadEnv(source: Record<string, string | undefined>): DesktopEnv {
  const problems: EnvProblem[] = [];
  const absolute = (key: string): string | null => {
    const value = source[key];
    if (value === undefined || value === "") return null;
    if (!isAbsolute(value)) {
      problems.push({ key, kind: "not an absolute path" });
      return null;
    }
    return value;
  };
  const windows = (key: string): string | null => (process.platform === "win32" ? absolute(key) : null);
  const feed = (key: string): string | null => {
    const value = source[key];
    if (value === undefined || value === "") return null;
    let url: URL | null = null;
    try {
      url = new URL(value);
    } catch {
      // Reported below.
    }
    if (url?.protocol !== "http:" && url?.protocol !== "https:") {
      problems.push({ key, kind: "not an http or https URL" });
      return null;
    }
    // Only a feed on this machine: what it serves is installed unsigned on Windows and Linux.
    if (!["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)) {
      problems.push({ key, kind: "not a loopback address" });
      return null;
    }
    return value;
  };
  const env: DesktopEnv = {
    cliPath: absolute("HIDE_CLI_PATH"),
    userDataDir: absolute("HIDE_DESKTOP_USER_DATA_DIR"),
    systemLanguage: source.HIDE_DESKTOP_SYSTEM_LANGUAGE || null,
    updateFeedUrl: feed("HIDE_UPDATE_FEED_URL"),
    shell: HAS_LOGIN_SHELL ? (absolute("SHELL") ?? accountShell()) : null,
    home: (process.platform === "win32" ? absolute("USERPROFILE") : absolute("HOME")) ?? os.userInfo().homedir,
    localAppData: windows("LOCALAPPDATA"),
    appData: windows("APPDATA"),
    programFiles: windows("ProgramFiles"),
    systemRoot: windows("SystemRoot"),
    path: source.PATH ?? "",
    inherited: source,
  };
  if (problems.length > 0) {
    throw new Error(`desktop environment is not usable: ${problems.map((problem) => `${problem.key}: ${problem.kind}`).join("; ")}`);
  }
  return env;
}
