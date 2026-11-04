// The device side of SSH e2e runs only in the declared private HOME.
// The isolated sshd must set HOME to HIDE_E2E_SSH_HOME and that folder must
// carry the fixture marker before a device can receive Hide's kit.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { e2eEngine } from "../../web/e2e/e2e-engine";
import { herdrBinary } from "../../web/e2e/herdr-fixture";
import { HIDE_CLI, seedKitRecord } from "./fixture";
const MARKER = ".hide-e2e-device-home";

/** The private HOME the isolated sshd gives its sessions, refused unless it is declared a test HOME. */
export function deviceHome(): string {
  const declared = process.env.HIDE_E2E_SSH_HOME;
  if (!declared || !path.isAbsolute(declared)) {
    throw new Error("HIDE_E2E_SSH_HOME must name the private HOME the isolated sshd sets (see desktop/e2e/device-home.ts)");
  }
  const home = fs.realpathSync(declared);
  assertTestHome(home);
  return home;
}

/** Refuses any folder that is the account's own HOME, by the environment or by the account database, or that is not marked as a test HOME. */
function assertTestHome(home: string): void {
  for (const own of [os.homedir(), os.userInfo().homedir]) {
    if (fs.existsSync(own) && home === fs.realpathSync(own)) throw new Error(`${home} is this account's own HOME`);
  }
  if (!fs.existsSync(path.join(home, MARKER))) throw new Error(`${home} has no ${MARKER}, so it is not declared a test HOME`);
}

/** The device's own state folder, where its node binds the socket a device pane's `hide` reaches the daemon through. */
export const deviceState = (home: string) => path.join(home, ".hide", "state");
/** The folder holding one `bridge-*` folder per daemon that has the device open. */
export const deviceBridges = (home: string) => path.join(deviceState(home), "workspace-bridges");

/** An SSH config and known_hosts under the run's local HOME, one host entry per alias for the isolated server. */
export function writeSshConfig(localHome: string, aliases: string[]): void {
  const ssh = path.join(localHome, ".ssh");
  fs.mkdirSync(ssh, { recursive: true });
  fs.copyFileSync(process.env.HIDE_E2E_SSH_KNOWN_HOSTS!, path.join(ssh, "known_hosts"));
  fs.writeFileSync(path.join(ssh, "config"), aliases.flatMap((alias) => [
    `Host ${alias}`, "  HostName 127.0.0.1", `  Port ${process.env.HIDE_E2E_SSH_PORT}`,
    `  User ${os.userInfo().username}`, `  IdentityFile ${process.env.HIDE_E2E_SSH_KEY}`, "  IdentityAgent none", "",
  ]).join("\n"), { mode: 0o600 });
}

/** Proves an SSH session receives the declared private HOME before installing anything. */
export function proveDeviceHome(localEnv: Record<string, string>, alias: string, home: string): string {
  const ssh = path.join(localEnv.HOME!, ".ssh");
  const answer = spawnSync("ssh", [
    "-F", path.join(ssh, "config"), "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
    "-o", `UserKnownHostsFile=${path.join(ssh, "known_hosts")}`, "-o", "GlobalKnownHostsFile=/dev/null",
    alias, 'printf "%s" "$HOME"',
  ], { env: localEnv, encoding: "utf8", timeout: 20_000 });
  if (answer.status !== 0) throw new Error(`the isolated sshd did not answer: ${answer.stderr}`);
  const remoteHome = answer.stdout;
  if (!remoteHome || fs.realpathSync(remoteHome) !== home) {
    throw new Error(`a session on the isolated sshd does not receive the declared private HOME; the kit would write that account's own files`);
  }
  return remoteHome;
}

export type AgentSettings = { hooks: Record<string, { hooks: { type: string; command: string; timeout?: number }[] }[]>; [key: string]: unknown };

export const claudeSettings = (home: string) => path.join(home, ".claude", "settings.json");
export const codexHooks = (home: string) => path.join(home, ".codex", "hooks.json");
export const readSettings = (file: string) => JSON.parse(fs.readFileSync(file, "utf8")) as AgentSettings;

/**
 * Puts the private HOME back to a device that has never met Hide: the kit's
 * folders gone, the agent files seeded by `seedAgentFiles`, and, on Herdr,
 * Herdr installed in ~/.local/bin; the core's own engine needs no Herdr there.
 */
export function resetDeviceHome(home: string): { claude: AgentSettings; codex: AgentSettings } {
  assertTestHome(home);
  for (const name of [".claude", ".codex", ".hide", ".local"]) fs.rmSync(path.join(home, name), { recursive: true, force: true });
  fs.mkdirSync(path.join(home, ".local", "bin"), { recursive: true });
  if (e2eEngine() === "herdr") fs.symlinkSync(herdrBinary(), path.join(home, ".local", "bin", "herdr"));
  return seedAgentFiles(home);
}

/** Each agent runtime's file holding only another tool's hook and setting, which every kit step must leave as it is. */
export function seedAgentFiles(home: string): { claude: AgentSettings; codex: AgentSettings } {
  const claude: AgentSettings = { theme: "dark", hooks: { SessionStart: [{ hooks: [{ type: "command", command: "/usr/bin/true other-tool-claude", timeout: 5 }] }] } };
  const codex: AgentSettings = { hooks: { SessionStart: [{ hooks: [{ type: "command", command: "/usr/bin/true other-tool-codex" }] }] } };
  for (const [file, settings] of [[claudeSettings(home), claude], [codexHooks(home), codex]] as const) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, `${JSON.stringify(settings, null, 2)}\n`);
  }
  seedCodex(home);
  seedClaude(home);
  seedKitRecord(home);
  return { claude, codex };
}

/**
 * A `codex` in the account's `~/.local/bin` that has the shared daemon and
 * answers `codex features` and `codex app-server daemon` the way Codex 0.160
 * does, so a runner without Codex reads as a machine that has one. The kit
 * only reads the setting (`features list`) unless the operator turns the
 * shared server off; `features disable` writes `fake-daemon`, which a spec
 * checks never appears on its own. The daemon answers while
 * `fake-daemon-running` exists (`startCodexDaemon`), and `daemon stop` removes
 * it and answers `stopped`, or `notRunning` when it was not running. A
 * machine whose PATH has a real Codex runs that one instead, against this
 * HOME's `.codex`.
 */
function seedCodex(home: string): void {
  const bin = path.join(home, ".local", "bin");
  fs.mkdirSync(bin, { recursive: true });
  const script = [
    "#!/bin/sh",
    'state="${CODEX_HOME:-$HOME/.codex}/fake-daemon"',
    'running="${CODEX_HOME:-$HOME/.codex}/fake-daemon-running"',
    'case "$1 $2 $3" in',
    "  '--version  ') echo 'codex-cli 0.160.0' ;;",
    "  'features list ') echo \"daemon_auto_start    stable  $(cat \"$state\" 2>/dev/null || echo true)\" ;;",
    "  'features disable '*) echo false > \"$state\" ;;",
    "  'features enable '*) echo true > \"$state\" ;;",
    "  'app-server daemon version') [ -e \"$running\" ] || { echo 'Error: failed to connect' >&2; exit 1; }; echo '{\"status\":\"running\"}' ;;",
    "  'app-server daemon stop') [ -e \"$running\" ] || { echo '{\"status\":\"notRunning\"}'; exit 0; }; rm -f \"$running\"; echo '{\"status\":\"stopped\"}' ;;",
    "  *) exit 1 ;;",
    "esac",
    "",
  ].join("\n");
  fs.writeFileSync(path.join(bin, "codex"), script, { mode: 0o755 });
}

/** A `claude` in the account's `~/.local/bin`, so Claude Code reads as installed on a runner without it. */
function seedClaude(home: string): void {
  const bin = path.join(home, ".local", "bin");
  fs.mkdirSync(bin, { recursive: true });
  fs.writeFileSync(path.join(bin, "claude"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
}

/** Whether a Codex setting the kit must never write was written: the stand-in's `features disable` leaves this file. */
export const codexDaemonWritten = (home: string) => fs.existsSync(path.join(home, ".codex", "fake-daemon"));

/** Starts the stand-in's shared daemon: `app-server daemon version` answers until `daemon stop`. */
export const startCodexDaemon = (home: string) => fs.writeFileSync(path.join(home, ".codex", "fake-daemon-running"), "");

/** Whether the stand-in's shared daemon still answers. */
export const codexDaemonAnswers = (home: string) => fs.existsSync(path.join(home, ".codex", "fake-daemon-running"));

/** The stand-in's autostart setting, `true` until `features disable`. */
export const codexAutostart = (home: string) => (fs.existsSync(path.join(home, ".codex", "fake-daemon"))
  ? fs.readFileSync(path.join(home, ".codex", "fake-daemon"), "utf8").trim() === "true" : true);

/**
 * A daemon folder laid out as the app's Contents/Resources: this build's
 * binaries, every kit part a device gets, and what the package carries for
 * the engine the run uses: the pinned Herdr, or, on the core's own engine,
 * the pane holder (`web/e2e/bundled-app.ts`, `binaries`). The daemon reads
 * the device payload from beside its own executable.
 */
export function stageBuild(root: string): string {
  const debug = path.dirname(HIDE_CLI);
  const build = path.join(root, "build");
  fs.mkdirSync(build, { recursive: true });
  const place = (source: string, target: string) => {
    try { fs.linkSync(source, target); } catch { fs.copyFileSync(source, target); fs.chmodSync(target, 0o755); }
  };
  const own = e2eEngine() === "own";
  for (const name of ["hided", "hide", "hide-agent-hooks", ...(own ? ["hide-holder"] : [])]) {
    const binary = path.join(debug, name);
    if (!fs.existsSync(binary)) throw new Error(`${binary} is missing: run \`cargo build -p hided -p hide-agent-hooks${own ? " -p hide-holder" : ""}\``);
    place(binary, path.join(build, name));
  }
  if (own) return build;
  // A copy, so the device's Herdr server, which runs the linked pinned
  // binary, never runs the file the upload reads.
  fs.copyFileSync(herdrBinary(), path.join(build, "herdr"));
  fs.chmodSync(path.join(build, "herdr"), 0o755);
  return build;
}
