// An isolated pinned Herdr server with two fake agent panes, for the click
// flow e2e (PRD B7, B8). Nothing here touches the operator's socket: every
// command runs with a private HERDR_SOCKET_PATH, session, config and state.
//
// Sidebar rows exist only for panes Herdr classifies as an agent, and the
// pinned Herdr classifies by the process it started, so each pane runs a
// tiny compiled `claude` that copies stdin to stdout. A copied /bin/cat
// would not do: macOS kills a relocated platform binary. Like a real TUI it
// reads the terminal raw, byte by byte with no line discipline echo and no
// extended input processing (which would swallow ^O and ^V), and it
// appends every byte it reads to `HIDE_E2E_INPUT_LOG`: that file is what the
// PTY received, which is how a test tells a click's mouse report from what
// the shell only sent (PRD S2 B20).
//
// The same binary is the daemon's Claude provider, because hided finds
// `claude` on the same PATH: `claude auth status` answers logged in, and a
// print-mode request (`--json-schema`, or a plain-JSON one carrying its schema in `--system-prompt`) answers with the label written after
// the last `HIDE_E2E_LABEL ` in its prompt, after the delay a
// `HIDE_E2E_DELAY_MS ` line before it asks for, and appends the answered
// task to `HIDE_E2E_PROVIDER_LOG` when that is set. `labelAgent` writes that marker
// into a synthetic Claude transcript, so a pane's label comes from the
// core's own transcript read and analysis, never from a token (PRD
// labels-in-hided).

import { execFileSync, spawn, spawnSync, type ChildProcess } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import net from "node:net";
import { setTimeout as pause } from "node:timers/promises";
import path from "node:path";
import { afterCleanup, ownUntilWorkerExit } from "./worker-owned";
import { endWindowsProcesses, fixtureExecutable, fixtureHomeEnv, fixtureToolPath, inheritedFixtureEnv, localEndpoint, windowsProcessTree, type WindowsProcess } from "./platform-fixture";
import { copyFixtureShim } from "./shims/build";

/** What `pane process-info` answers about a pane's shell, as far as the fixture reads it. */
type ShellProcessInfo = {
  shell_pid: number | null;
  foreground_process_group_id: number | null;
  foreground_processes: { pid: number }[];
};

/** What `workspace create` answers, as far as the fixture reads it. */
type CreatedWorkspace = { result: { workspace: { workspace_id: string }; root_pane: { pane_id: string } } };

/** Only the pinned schema fields needed for the initial agents' startup identity. */
type InitialAgent = {
  name: string;
  terminal_id: string;
  pane_id: string;
  agent?: string | null;
  agent_status: string;
  launch_pending?: boolean;
  interactive_ready?: boolean;
};

/**
 * A custom socket client separates registration from the CLI's interactive wait.
 * The fixture owns at most two requests, each with a deadline and response cap.
 * Destroying a connection cancels its read before server/root cleanup begins.
 */
function agentRequest(env: NodeJS.ProcessEnv, method: "agent.start" | "agent.get", params: Record<string, string>, pending: Set<PendingStart>, timeout = 30_000): Promise<{ result: { agent: InitialAgent } }> {
  if (pending.size >= 2) return Promise.reject(new Error("fixture agent request limit exceeded"));
  return new Promise((resolve, reject) => {
    const peer = net.createConnection(localEndpoint(env.HERDR_SOCKET_PATH!));
    const id = crypto.randomUUID();
    let buffer = Buffer.alloc(0);
    let settled = false;
    const finish = (error?: Error, answer?: { result: { agent: InitialAgent } }) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      peer.destroy();
      pending.delete(request);
      if (error) reject(error);
      else resolve(answer!);
    };
    const request = { stop: () => finish(new Error(`fixture stopped during ${method}`)) };
    pending.add(request);
    const timer = setTimeout(() => finish(new Error(`${method} timed out`)), timeout);
    peer.once("connect", () => peer.write(`${JSON.stringify({ id, method, params })}\n`));
    peer.once("error", (error) => finish(error));
    peer.once("close", () => finish(new Error(`Herdr closed ${method} before answering`)));
    peer.on("data", (chunk: Buffer) => {
      if (buffer.length + chunk.length > 64 * 1024) { finish(new Error(`${method} response exceeded 64 KiB`)); return; }
      buffer = Buffer.concat([buffer, chunk]);
      const end = buffer.indexOf(10);
      if (end < 0) return;
      try {
        const answer = JSON.parse(buffer.subarray(0, end).toString("utf8"));
        if (answer.id !== id) throw new Error(`${method} response id did not match`);
        if (answer.error) throw Object.assign(new Error(`${method} refused: ${JSON.stringify(answer.error)}`), { stderr: JSON.stringify(answer.error) });
        if (answer.result?.type !== (method === "agent.start" ? "agent_started" : "agent_info") || !answer.result.agent) throw new Error(`${method} returned an invalid agent answer`);
        finish(undefined, answer);
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

export type HerdrFixture = {
  bin: string;
  socket: string;
  env: NodeJS.ProcessEnv;
  /** The fixture root; `fixture/` under it is the first workspace's cwd. */
  root: string;
  workspace: string;
  tab: string;
  panes: [string, string];
  /** Per agent pane, the file its `claude` shim appends every byte the PTY delivered to. */
  inputLogs: [string, string];
  /** The controlled shim and system-tool PATH for the server, panes and daemon. */
  fixturePath: string;
  /** The family of the shell Herdr starts in a pane, read from the shell its config names. */
  shell: PaneShell;
  /** Runs a pinned-herdr CLI command against the private server and parses its JSON. */
  run: (args: string[]) => unknown;
  stop: () => void;
  /** Waits for both initial agents to own their terminals and accept input. */
  ready: () => Promise<void>;
  /** Cold restart this private server, changing its files only while it is stopped. */
  restart: (beforeStart?: () => void) => Promise<void>;
  /**
   * Runs `cleanup` once the server has stopped and its panes' processes are
   * gone, or now when that is already so. A folder a pane's shell started in
   * cannot be deleted before then on Windows.
   */
  afterStop: (cleanup: () => void) => void;
};


function pinnedHerdrVersion(): string {
  const manifest = path.resolve("..", "contracts/herdr-bundle.json");
  return (JSON.parse(fs.readFileSync(manifest, "utf8")) as { version: string }).version;
}

export function herdrBinary(): string {
  const candidates = [process.env.HIDE_E2E_HERDR_BIN, process.env.HERDR_BIN_PATH];
  for (const candidate of candidates) {
    if (candidate && fs.existsSync(candidate)) return candidate;
  }
  const lookup = spawnSync(process.platform === "win32" ? "where.exe" : "which", [fixtureExecutable("herdr")], { encoding: "utf8" });
  if (lookup.status === 0) {
    const onPath = lookup.stdout.trim().split(/\r?\n/).find((candidate) => fs.existsSync(candidate));
    if (onPath) return onPath;
  }
  throw new Error(
    "no herdr binary: set HIDE_E2E_HERDR_BIN or HERDR_BIN_PATH, or put the pinned herdr on PATH",
  );
}

/**
 * The agent kinds Herdr lists in panes whose cwd is `dir`: what an agent
 * start really left running, not what the start's answer said.
 */
export function agentsIn(fixture: HerdrFixture, dir: string): string[] {
  const real = fs.realpathSync(dir);
  const kinds: string[] = [];
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === "object") {
      const row = value as Record<string, unknown>;
      if (typeof row.pane_id === "string" && typeof row.agent === "string" && typeof row.cwd === "string" && fs.existsSync(row.cwd) && fs.realpathSync(row.cwd) === real) {
        kinds.push(row.agent);
      }
      Object.values(row).forEach(visit);
    }
  };
  visit(fixture.run(["agent", "list"]));
  return kinds;
}

/**
 * Whether Herdr itself has `pane` focused. The shell draws a focus the moment
 * it asks for one and Herdr applies it a little later, and two requests in
 * flight at once can be applied in either order, so a spec that asks for a
 * second focus waits for Herdr to hold the first.
 */
export function herdrHasFocus(fixture: HerdrFixture, pane: string): boolean {
  const listed = fixture.run(["pane", "list"]) as { result: { panes: { pane_id: string; focused: boolean }[] } };
  return listed.result.panes.find((row) => row.pane_id === pane)?.focused === true;
}

function herdr(env: NodeJS.ProcessEnv, bin: string, args: string[], timeout = 30_000): unknown {
  const out = execFileSync(bin, args, { env, encoding: "utf8", timeout });
  return JSON.parse(out) as unknown;
}

type PendingStart = { stop: () => void };

/** Stop the private Unix process group, including a CLI wrapper's children. */
export function endStartGroup(pid: number): void {
  try { process.kill(-pid, "SIGKILL"); } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ESRCH") throw error;
  }
  const deadline = Date.now() + 5_000;
  for (;;) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error(`fixture CLI process group ${pid} did not exit`);
    const listed = spawnSync("/bin/ps", ["-A", "-o", "pgid=,stat="], { encoding: "utf8", timeout: remaining });
    if (listed.error || listed.status !== 0) throw new Error(`cannot confirm fixture CLI group ${pid} exit: ${listed.error ?? listed.stderr}`);
    // A zombie has exited and cannot use the root; Node reaps it after this
    // synchronous teardown, or the OS does when the worker itself exits.
    const live = listed.stdout.trim().split(/\r?\n/).some((line) => {
      const [group, state] = line.trim().split(/\s+/);
      return Number(group) === pid && !state?.startsWith("Z");
    });
    if (!live) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Math.min(25, remaining));
  }
}

/** The fixture owns at most two starts until close, or confirmed tree teardown. */
function herdrAsync(env: NodeJS.ProcessEnv, bin: string, args: string[], pending: Set<PendingStart>): Promise<unknown> {
  if (pending.size >= 2) return Promise.reject(new Error("fixture already owns two pending CLI starts"));
  return new Promise((resolve, reject) => {
    let stdout = "", stderr = "";
    let failure: Error | undefined;
    let confirmed = false;
    let windowsOwned: WindowsProcess[] | undefined;
    // execFile does not pass detached through to spawn. A private group is
    // essential: killing only a wrapper leaves its real CLI child alive.
    const child = spawn(bin, args, { env, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"] });
    const command: PendingStart = { stop: () => {
      if (confirmed) return;
      if (child.pid !== undefined) {
        if (process.platform === "win32") {
          if (windowsOwned === undefined) throw new Error(`fixture CLI ${child.pid} has no confirmed process identity`);
          endWindowsProcesses(windowsOwned);
        } else endStartGroup(child.pid);
      }
      confirmed = true;
      pending.delete(command);
    } };
    pending.add(command);
    const terminate = (error: Error) => {
      failure ??= error;
      try { command.stop(); } catch (cleanup) {
        failure = afterCleanup(failure, () => { throw cleanup; }) as Error;
        // A failed tree confirmation must still settle setup within its bound.
        // Keep ownership for teardown, which retains the root if it fails again.
        child.kill("SIGKILL");
        reject(Object.assign(failure, { stdout, stderr }));
      }
    };
    const timeout = setTimeout(() => terminate(new Error("fixture CLI timed out after 30000 ms")), 30_000);
    child.on("error", terminate);
    for (const [name, stream] of [["stdout", child.stdout], ["stderr", child.stderr]] as const) {
      stream.setEncoding("utf8");
      stream.on("data", (chunk: string) => {
        const previous = name === "stdout" ? stdout : stderr;
        if (Buffer.byteLength(previous) + Buffer.byteLength(chunk) > 1024 * 1024) {
          terminate(new Error(`fixture CLI ${name} exceeded 1048576 bytes`));
          return;
        }
        if (name === "stdout") stdout += chunk;
        else stderr += chunk;
      });
    }
    child.once("close", (code, signal) => {
      clearTimeout(timeout);
      if (!failure && (code !== 0 || signal)) terminate(new Error(`fixture CLI exited with status ${code}, signal ${signal}`));
      if (!failure || confirmed) pending.delete(command);
      if (failure) { reject(Object.assign(failure, { stdout, stderr, status: code, signal })); return; }
      try { resolve(JSON.parse(stdout) as unknown); } catch (error) { reject(error); }
    });
    // Capture identity while the CLI handle is owned, before it can be reaped
    // and its PID reused. The helper also follows descendants born later.
    if (process.platform === "win32" && child.pid !== undefined) {
      try {
        const identities = windowsProcessTree(child.pid);
        if (!identities.some((item) => item.id === child.pid)) throw new Error(`fixture CLI ${child.pid} identity was not observed`);
        windowsOwned = identities;
      } catch (error) { terminate(error instanceof Error ? error : new Error(String(error))); }
    }
  });
}

function isolatedEnv(root: string, socket: string): NodeJS.ProcessEnv {
  const env = inheritedFixtureEnv();
  const config = path.join(root, "herdr-config.toml");
  // The pinned Herdr defaults to PowerShell on Windows and ignores SHELL there.
  // cmd.exe honors PROMPT, preserving the same prompt contract as zsh.
  const shell = process.platform === "win32" ? env.COMSPEC : "/bin/zsh";
  if (!shell || !path.isAbsolute(shell) || !fs.existsSync(shell)) throw new Error("fixture shell is unavailable: Windows needs ComSpec; Unix needs /bin/zsh");
  fs.writeFileSync(config, `[update]\nversion_check = false\nmanifest_check = false\n[terminal]\ndefault_shell = ${JSON.stringify(shell)}\n`);
  for (const dir of ["xdg-config", "xdg-state", "home", "fixture", "bin"]) {
    fs.mkdirSync(path.join(root, dir), { recursive: true });
  }
  // Herdr starts the pane shell from SHELL, so the fixture names it: the
  // CI runner's login shell is bash, and only zsh reads this private
  // HOME's .zshrc. A fixed prompt keeps the workstation's user and host
  // name out of screenshots and is what the fixture waits for.
  fs.writeFileSync(path.join(root, "home", ".zshrc"), "PS1='fixture %# '\n");
  return {
    ...env,
    SHELL: shell,
    ...(process.platform === "win32" ? { PROMPT: "fixture % " } : {}),
    // Ubuntu's /etc/zsh/zshrc runs compinit before this HOME's .zshrc, and on
    // a Linux runner compinit stops at "insecure directories, continue [y] or
    // abort [n]?", so the prompt never comes. That file skips compinit when
    // this parameter is set. It rides the environment, not a .zshenv, because
    // specs write their own .zshenv to put the fake agent first on PATH and
    // would drop it. macOS has no such file.
    skip_global_compinit: "1",
    ...fixtureHomeEnv(path.join(root, "home")),
    HERDR_SESSION: `hide-e2e-${path.basename(root)}`,
    HERDR_SOCKET_PATH: socket,
    HERDR_CONFIG_PATH: config,
    XDG_CONFIG_HOME: path.join(root, "xdg-config"),
    XDG_STATE_HOME: path.join(root, "xdg-state"),
    HERDR_DISABLE_SOUND: "1",
    // Every pane the server starts, however it is made, tells a fixture program where this run's files are.
    HIDE_E2E_ROOT: root,
  };
}

function paneRead(env: NodeJS.ProcessEnv, bin: string, pane: string, timeout = 10_000): { text: string; failure: string | null } {
  const result = spawnSync(bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], {
    env,
    encoding: "utf8",
    timeout,
  });
  if (result.status === 0) return { text: result.stdout, failure: null };
  return { text: "", failure: `pane read exited ${result.status}: ${result.stderr.trim()}` };
}

function paneText(env: NodeJS.ProcessEnv, bin: string, pane: string, timeout?: number): string {
  return paneRead(env, bin, pane, timeout).text;
}

/**
 * `detail` is read once, when the wait fails, so the error names what the
 * predicate last saw; a bare "timed out" cannot say whether the shell
 * printed something else or nothing at all.
 */
async function waitFor(predicate: () => boolean, what: string, ms = 10_000, detail?: () => string): Promise<void> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for ${what}${detail ? `; last seen: ${detail()}` : ""}`);
}

export type PaneShell = "posix" | "cmd";

/**
 * The family of the shell `isolatedEnv` named for Herdr's panes. The shell
 * is read from that name, never from the host OS, so a command is written
 * for the shell the pane really runs.
 */
export function paneShellOf(env: NodeJS.ProcessEnv): PaneShell {
  const shell = env.SHELL ?? "";
  const name = shell.split(/[\\/]/).pop()!.toLowerCase();
  if (name === "cmd.exe") return "cmd";
  if (name === "zsh") return "posix";
  throw new Error(`the fixture cannot write commands for the pane shell ${JSON.stringify(shell)}`);
}

/** A tab named for its pane's shell, or numbered when none has named it yet; cmd.exe titles its console with its own path. */
export function paneShellTabName(shell: PaneShell): RegExp {
  return shell === "cmd" ? /cmd\.exe|Tab \d+/i : /zsh|Tab \d+/;
}

/** The line that makes a pane's shell print `lines`, one at a time. */
export function printLinesCommand(shell: PaneShell, prefix: string, count: number): string {
  if (shell === "cmd") return `for %n in (${Array.from({ length: count }, (_, i) => i + 1).join(" ")}) do @echo ${prefix}%n`;
  return `for n in ${Array.from({ length: count }, (_, i) => i + 1).join(" ")}; do printf '${prefix}%s\\n' "$n"; sleep 0.1; done`;
}

export type PaneProgram = {
  /** Variables set for this program, and for the rest of the pane's session under cmd.exe. */
  env?: Record<string, string>;
  argv: string[];
  stdout?: string;
  stderr: string;
  /** Written last, with the exit status, so its presence says the program has finished. */
  status: string;
};

const posixQuote = (value: string): string => `'${value.replaceAll("'", "'\\''")}'`;

function cmdQuote(value: string): string {
  if (/["%\r\n]/.test(value)) throw new Error(`cmd.exe cannot carry ${JSON.stringify(value)} in a pane command`);
  return `"${value}"`;
}

/** Runs these arguments through the shell the fixture actually selected. */
export function paneCommandLine(shell: PaneShell, argv: string[]): string {
  return argv.map(shell === "cmd" ? cmdQuote : posixQuote).join(" ");
}

/** Clears the pane and prints a fixture file through the shell it really runs. */
export function printFileCommand(shell: PaneShell, file: string): string {
  return shell === "cmd" ? `cls & type ${cmdQuote(file)}` : `clear; cat ${posixQuote(file)}`;
}

/** The text that runs `program` in a pane's shell, ready for `pane run`. */
export function paneProgramLine(shell: PaneShell, program: PaneProgram): string {
  const env = Object.entries(program.env ?? {});
  if (shell === "cmd") {
    const sets = env.map(([key, value]) => `set ${cmdQuote(`${key}=${value}`)} && `).join("");
    const run = paneCommandLine(shell, program.argv);
    // `call echo %^errorlevel%` reads the status when it runs, not when the line is parsed.
    return `${sets}${run} > ${cmdQuote(program.stdout ?? "nul")} 2> ${cmdQuote(program.stderr)} & >${cmdQuote(program.status)} call echo %^errorlevel%`;
  }
  const sets = env.map(([key, value]) => `${key}=${posixQuote(value)} `).join("");
  return `${sets}${paneCommandLine(shell, program.argv)} > ${posixQuote(program.stdout ?? "/dev/null")} 2> ${posixQuote(program.stderr)}; printf '%s' "$?" > ${posixQuote(program.status)}`;
}

/**
 * Runs `argv` in the shell of `pane`, as an agent in that tab would, and
 * returns its exit status once the shell has written it. Output goes to
 * `<root>/<name>.out` and `<name>.err`; `name` is unique per call.
 */
export async function runInPane(
  fixture: Pick<HerdrFixture, "bin" | "env" | "root" | "shell">,
  pane: string,
  name: string,
  program: { env?: Record<string, string>; argv: string[]; stdout?: boolean },
): Promise<{ status: number; stdout: string; stderr: string }> {
  const files = { stdout: path.join(fixture.root, `${name}.out`), stderr: path.join(fixture.root, `${name}.err`), status: path.join(fixture.root, `${name}.status`) };
  const line = paneProgramLine(fixture.shell, { env: program.env, argv: program.argv, stdout: program.stdout ? files.stdout : undefined, stderr: files.stderr, status: files.status });
  execFileSync(fixture.bin, ["pane", "run", pane, line], { env: fixture.env, timeout: 10_000 });
  const read = (file: string): string => (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : "");
  await waitFor(() => /^\d+/.test(read(files.status)), `${name} to finish in pane ${pane}`, 20_000, () => `${paneText(fixture.env, fixture.bin, pane).trimEnd().split("\n").slice(-6).join(" | ")}; stderr ${JSON.stringify(read(files.stderr))}`);
  return { status: Number.parseInt(read(files.status), 10), stdout: read(files.stdout), stderr: read(files.stderr) };
}


/** What the fixture provider answers for one transcript. */
export type FixtureLabel = {
  /** 8 to 30 characters: the core refuses a shorter title. */
  task: string;
  /** How long the provider takes to answer. */
  delayMs?: number;
  progress?: string;
  /** The label's one-line summary of the turn's request (`context_label.v20`), which the provider shim answers for the newest request the input owes a summary; none by default, when the request shows its own words. */
  request?: string;
  /** How the flow this label's replace closes ended (`context_label.v21`); none by default. */
  closedResult?: string;
  /** The reply the agent asks for; the label's line when given, with `question` its end. */
  reply?: string;
  question?: boolean;
  /** How the stopped turn ended (`context_label.v7`); a question by default when
   * `question`, else done. A turn start's answer carries none: the provider shim
   * leaves `end` out when the request's schema asks for none. */
  end?: "question" | "blocked" | "done" | "waiting" | "unfinished" | "limited";
  work?: { missed:{kind:"problem"|"follow_up"|"ask";text:string}[]; resolved:{id:string;issue_number:number|null}[]; artifacts:string[] };
  at?: number;
  /** Native cwd witness, when repository provenance is part of the journey. */
  cwd?: string;
  /** Addresses printed by a real native tool-result record in this turn. */
  toolOutputs?: string[];
};

/** Where the fixture's Claude transcripts live: the fixture HOME's, which
 * `startHided` links into the daemon's HOME. */
export function claudeProjects(fixture: Pick<HerdrFixture, "root">): string {
  return path.join(fixture.root, "home", ".claude", "projects");
}

/**
 * The daemon reads agent transcripts under its own HOME; the fixture writes
 * them under its own, before or after the daemon starts. One folder for both
 * keeps every label the fixture writes readable by a daemon on `home`.
 */
export function linkFixtureTranscripts(fixture: Pick<HerdrFixture, "root">, home: string): void {
  const own = claudeProjects(fixture);
  const daemon = path.join(home, ".claude", "projects");
  fs.mkdirSync(own, { recursive: true });
  if (path.resolve(daemon) === path.resolve(own) || fs.existsSync(daemon)) return;
  fs.mkdirSync(path.dirname(daemon), { recursive: true });
  fs.symlinkSync(own, daemon, process.platform === "win32" ? "junction" : "dir");
}

/**
 * A Claude transcript for `sessionId`: one operator turn and the agent's
 * answer, which carries the label the fixture provider returns. Written
 * before the session is declared, because the core reads a session when its
 * reference or state changes, not when its file grows.
 */
/** The text a transcript carries for the fixture provider to answer with. */
export function labelMarker(label: FixtureLabel): string {
  const answer = {
    goal: label.task,
    goal_update: "replace",
    closed_result: label.closedResult ?? "",
    request: label.request ?? "",
    // One line: the reply asked for when there is one, else the progress.
    line: label.reply ?? label.progress ?? "",
    end: label.end ?? (label.question ? "question" : "done"),
    work: label.work ?? {missed:[],resolved:[],artifacts:[]},
  };
  const delay = label.delayMs ? `HIDE_E2E_DELAY_MS ${label.delayMs}\n` : "";
  return `${delay}HIDE_E2E_LABEL ${JSON.stringify(answer)}`;
}

export function writeFixtureTranscript(projects: string, sessionId: string, label: FixtureLabel): string {
  const records = [
    { type: "user", sessionId, ...(label.cwd ? { cwd: label.cwd } : {}), timestamp: label.at ? new Date(label.at).toISOString() : "2026-10-01T09:00:00Z", origin: { kind: "human" }, message: { role: "user", content: `${label.task} 진행해줘` } },
    ...fixtureToolResults(sessionId, label),
    { type: "assistant", sessionId, timestamp: label.at ? new Date(label.at+1000).toISOString() : "2026-10-01T09:00:01Z", message: { role: "assistant", content: [{ type: "text", text: labelMarker(label) }] } },
  ];
  const dir = path.join(projects, "e2e");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `${sessionId}.jsonl`);
  fs.writeFileSync(file, records.map((record) => `${JSON.stringify(record)}\n`).join(""));
  return file;
}

/**
 * Appends a turn carrying `label` to the transcript of `sessionId`: the same
 * session, so a parent relationship declared for it still holds. The core
 * reads the new turn on the agent's next state change; the request carries
 * the label too, so the analysis at the turn's start answers it as well.
 */
export function continueFixtureTranscript(fixture: Pick<HerdrFixture, "root">, sessionId: string, label: FixtureLabel): void {
  const records = [
    { type: "user", sessionId, timestamp: label.at ? new Date(label.at).toISOString() : "2026-10-01T09:01:00Z", origin: { kind: "human" }, message: { role: "user", content: `${label.task} 이어서 진행해줘\n${labelMarker(label)}` } },
    ...fixtureToolResults(sessionId, label),
    { type: "assistant", sessionId, timestamp: label.at ? new Date(label.at+1000).toISOString() : "2026-10-01T09:01:01Z", message: { role: "assistant", content: [{ type: "text", text: labelMarker(label) }] } },
  ];
  fs.appendFileSync(path.join(claudeProjects(fixture), "e2e", `${sessionId}.jsonl`), records.map((record) => `${JSON.stringify(record)}\n`).join(""));
}

function fixtureToolResults(sessionId: string, label: FixtureLabel) {
  return label.toolOutputs?.length ? [
    { type: "assistant", sessionId, timestamp: new Date((label.at ?? Date.parse("2026-10-01T09:00:00Z")) + 400).toISOString(), message: { role: "assistant", content: [{ type: "tool_use", id: "fixture-gh", name: "Bash", input: { command: "gh pr create" } }] } },
    { type: "user", sessionId, timestamp: new Date((label.at ?? Date.parse("2026-10-01T09:00:00Z")) + 500).toISOString(), message: { role: "user", content: [{ type: "tool_result", tool_use_id: "fixture-gh", content: label.toolOutputs.join("\n") }] } },
  ] : [];
}

/**
 * The native session id the fixture gives `pane`'s agent. A native id is
 * letters, digits, `.`, `_` and `-`, and the core's transcript read refuses
 * any other, so the pane id's `:` becomes `-`.
 */
function fixtureSessionId(pane: string): string {
  return `fixture-${pane.replaceAll(":", "-")}`;
}

/** The native session id `pane`'s agent reports now. */
export function sessionOf(fixture: Pick<HerdrFixture, "run">, pane: string): string {
  type Agent = { pane_id: string; agent_session?: { value: string } };
  const listed = fixture.run(["agent", "list"]) as { result: { agents: Agent[] } };
  const value = listed.result.agents.find((agent) => agent.pane_id === pane)?.agent_session?.value;
  if (!value) throw new Error(`pane ${pane} reports no agent session`);
  return value;
}

/** The last session sequence each pane declared; a replacement must be newer. */
const sessionSeqs = new Map<string, number>();

/** Controlled fake-agent provenance. This publishes the explicit test session,
 * not a transcript-discovery claim; native isolation tests use raw CLI reports. */
function declareFixtureSession(env: NodeJS.ProcessEnv, bin: string, pane: string, kind: string, sessionId: string, replacing = false): void {
  const key = `${env.HERDR_SOCKET_PATH}|${pane}`;
  const seq = replacing ? (sessionSeqs.get(key) ?? 1) + 1 : 1;
  sessionSeqs.set(key, seq);
  execFileSync(bin, ["pane", "report-agent-session", pane, "--source", `herdr:${kind}`, "--agent", kind, "--agent-session-id", sessionId, "--seq", String(seq), ...(replacing ? ["--session-start-source", "clear"] : [])], { env, timeout: 30_000 });
}

/**
 * Declares `child` as spawned by `parent` the way hided writes it: the
 * parent's pane and the digest of each pane's session, which Hide compares with
 * the session each pane reports now (docs: docs/status-model.md#where-a-parent-comes-from).
 * Both panes must already hold an agent session; `herdr agent list` names it.
 */
export function declareParent(fixture: Pick<HerdrFixture, "bin" | "env">, child: string, parent: string): void {
  type Agent = { pane_id: string; agent_session?: { value: string } };
  const listed = JSON.parse(execFileSync(fixture.bin, ["agent", "list"], { env: fixture.env, encoding: "utf8", timeout: 30_000 })) as { result: { agents: Agent[] } };
  const session = (pane: string): string => {
    const value = listed.result.agents.find((agent) => agent.pane_id === pane)?.agent_session?.value;
    if (!value) throw new Error(`pane ${pane} reports no agent session, so its relationship could not be written`);
    return crypto.createHash("sha256").update(value, "utf8").digest("hex");
  };
  execFileSync(fixture.bin, ["pane", "report-metadata", child, "--source", "e2e-lineage", "--token", `parent_pane=${parent}`, "--token", `child_session=${session(child)}`, "--token", `parent_session=${session(parent)}`], { env: fixture.env, timeout: 30_000 });
}

export function setFixtureSession(fixture: HerdrFixture, pane: string, sessionId: string): void {
  declareFixtureSession(fixture.env, fixture.bin, pane, "claude", sessionId, true);
}

let labelSessions = 0;

/**
 * Gives `pane` a new Claude session whose transcript the core analyzes into
 * `label`. A new session, because the same session is read again only when
 * the agent's state changes.
 */
export function labelAgent(fixture: HerdrFixture, pane: string, label: FixtureLabel): string {
  const sessionId = `label-${process.pid}-${(labelSessions += 1)}`;
  writeFixtureTranscript(claudeProjects(fixture), sessionId, label);
  setFixtureSession(fixture, pane, sessionId);
  return sessionId;
}

/** Drive the fake Claude's actual screen detector while retaining its native
 * session identity. Lifecycle hook reports may be ignored after declaration.
 *
 * `idle` ends the turn: Herdr reports `done` when the pane's tab is not the
 * one its clients show, and `idle` when it is, so either answers it. */
export async function setFixtureLifecycle(fixture: HerdrFixture, pane: string, state: "working" | "blocked" | "idle"): Promise<string> {
  if (fixture.panes.includes(pane)) await fixture.ready();
  type Agent = { pane_id: string; agent_status: string; agent_session?: { source: string; agent: string; kind: string; value: string } };
  const current = () => (fixture.run(["agent", "list"]) as { result: { agents: Agent[] } }).result.agents.find((agent) => agent.pane_id === pane);
  const reference = current()?.agent_session;
  if (!reference || reference.source !== "herdr:claude" || reference.agent !== "claude") {
    throw new Error("lifecycle fixture requires its declared Claude native session");
  }
  // The existing raw-mode shim echoes these bytes, so pinned Herdr's
  // osc_title_working / bash_permission_prompt rules see a controlled TUI.
  // Leaving a permission prompt clears it off the screen, where Herdr would
  // still read it as blocked; nothing else touches the pane's text.
  const clear = reference && current()?.agent_status === "blocked" ? "\x1b[2J\x1b[H" : "";
  const screen = state === "working"
    ? `${clear}\x1b]0;\u280b Working\x07`
    : state === "idle"
      ? `${clear}\x1b]0;\u2733 Claude Code\x07`
      : "\x1b]0;Fixture\x07\x1b[2J\x1b[Hdo you want to proceed?\n"
        + "bash command\n❯ 1. Yes\n2. No\n";
  const reached = (status: string | undefined) => (state === "idle" ? status === "idle" || status === "done" : status === state);
  execFileSync(fixture.bin, ["pane", "send-text", pane, screen], { env: fixture.env, timeout: 30_000 });
  await waitFor(() => reached(current()?.agent_status), `native fixture state ${state}`, 10_000,
    () => JSON.stringify({ expected: state, observed: current()?.agent_status }));
  const observed = current()!;
  if (JSON.stringify(observed.agent_session) !== JSON.stringify(reference)) {
    throw new Error("lifecycle fixture changed its declared native session");
  }
  // Detection provenance is run evidence, never product source.
  const explanation = execFileSync(fixture.bin, ["agent", "explain", pane, "--json"], { env: fixture.env, encoding: "utf8", timeout: 30_000 });
  const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (evidence) {
    fs.mkdirSync(evidence, { recursive: true });
    fs.writeFileSync(path.join(evidence, `lifecycle-${pane.replaceAll(":", "-")}-${state}.json`),
      JSON.stringify({ expected: state, observed: observed.agent_status, nativeSessionUnchanged: true, explanation: JSON.parse(explanation) }));
  }
  return observed.agent_status;
}

/** A new tab in the fixture's workspace, which no agent runs in: Herdr's
 * clients look at it while a turn ends elsewhere, so that turn ends unseen. */
export function elsewhereTab(fixture: HerdrFixture): string {
  const created = fixture.run(["tab", "create", "--workspace", fixture.workspace, "--label", "elsewhere", "--env", `PATH=${fixture.fixturePath}`, "--no-focus"]) as {
    result: { tab: { tab_id: string } };
  };
  return created.result.tab.tab_id;
}

/**
 * A finished turn the operator has not seen: the pane works, then stops
 * while its tab is not the one Herdr's clients show, which Herdr reports as
 * `done`. `elsewhere` is another tab; focusing it moves Herdr's clients off
 * the pane's tab first.
 */
export async function finishFixtureTurn(fixture: HerdrFixture, pane: string, elsewhere: string): Promise<void> {
  fixture.run(["tab", "focus", elsewhere]);
  await setFixtureLifecycle(fixture, pane, "working");
  const status = await setFixtureLifecycle(fixture, pane, "idle");
  if (status !== "done") throw new Error(`pane ${pane} stopped as ${status}, not done: its tab is still shown`);
}

/**
 * Whether the pinned Herdr counts `pane`'s shell as available to `agent start`
 * (docs/ARCHITECTURE.md, Starting an agent), the one place the fixture decides
 * it. On macOS and Linux that is the shell alone in the terminal, its
 * foreground process group its own with no other member, which `pane
 * process-info` reports. On Windows it is no process naming the shell as its
 * parent, by pid alone, which `pane process-info` does not report (its
 * foreground is the shell unless an agent runs below it), so the fixture reads
 * the process table itself (`hide-children.exe`). A process older than the
 * shell that names it is a dead parent's pid given to the shell; Herdr refuses
 * that pane for as long as the process runs, so no wait reaches a start and
 * this throws, naming it (`unclaimedWorkspace` keeps such panes out of the
 * workspaces the fixture opens).
 */
function shellAvailability(env: NodeJS.ProcessEnv, bin: string, root: string, pane: string, timeout?: number): { available: boolean; info: ShellProcessInfo } {
  const info = (herdr(env, bin, ["pane", "process-info", "--pane", pane], timeout) as { result: { process_info: ShellProcessInfo } }).result.process_info;
  const shell = info.shell_pid;
  if (shell === null || shell <= 1) return { available: false, info };
  if (process.platform !== "win32") {
    return { available: info.foreground_process_group_id === shell && info.foreground_processes.every((process) => process.pid === shell), info };
  }
  const children = listShellChildren(root, shell);
  const older = children.filter(isOlderClaimant);
  if (older.length > 0) {
    throw new Error(`a process older than the shell of pane ${pane} names it as parent, so the pinned Herdr refuses agent start there for as long as it runs: ${older.join(", ")}`);
  }
  return { available: children.length === 0, info };
}

/**
 * `herdr agent start` for `pane`, sent only while `shellAvailability` says the
 * pinned Herdr counts the pane's shell as available. Herdr does not wait, so
 * the fixture does, as the product does (`agent_start::start_at_shell`): a
 * refusal as `agent_pane_busy` typed nothing, so it goes back to waiting,
 * within the same bound. Any other answer is the start's. `run` stays synchronous; the initial
 * pair uses the asynchronous form so both interactive waits can overlap.
 */
function startAgentAtShell(env: NodeJS.ProcessEnv, bin: string, root: string, args: string[], ms = 10_000, asynchronous?: (args: string[]) => Promise<unknown>): unknown {
  const pane = args[args.indexOf("--pane") + 1]!;
  const deadline = Date.now() + ms;
  let seen = "";
  const failed = (error: unknown) => {
    // Herdr prints a refusal on stderr; stdout is empty. A busy refusal typed nothing.
    const refusal = String((error as { stderr?: unknown }).stderr ?? error);
    if (!refusal.includes("agent_pane_busy")) {
      const result = error as { status?: number; signal?: string; stdout?: string; stderr?: string };
      throw new Error(`fixture agent start failed: ${JSON.stringify({ pane, readiness: seen, status: result.status ?? null, signal: result.signal ?? null, stdout: result.stdout ?? null, stderr: result.stderr ?? null })}; ${String(error)}`, { cause: error });
    }
    seen = `${seen}; busy answer: ${refusal.trim()}`;
  };
  const attempt = (): unknown => {
    for (;;) {
      const promptRead = paneRead(env, bin, pane, Math.max(1, deadline - Date.now()));
      const { available, info } = shellAvailability(env, bin, root, pane, Math.max(1, deadline - Date.now()));
      const shell = info.shell_pid;
      const prompt = promptRead.text.includes("fixture %");
      seen = JSON.stringify({ prompt, promptFailure: promptRead.failure, processInfo: info });
      // Foreground ownership can precede shell startup. Later split and resumed
      // panes need the same prompt barrier as the initial fixture panes.
      if (available && prompt) {
        if (asynchronous) return asynchronous(args).catch(async (error: unknown) => {
          failed(error);
          if (Date.now() >= deadline) throw new Error(`the shell of pane ${pane} never held the terminal alone before agent start; last process info ${seen}${shellChildren(root, shell)}`);
          // Poll the same shell-availability condition after a busy refusal.
          await new Promise((resolve) => setTimeout(resolve, 100));
          return attempt();
        });
        try {
          return herdr(env, bin, args);
        } catch (error) {
          failed(error);
        }
      }
      if (Date.now() >= deadline) {
        throw new Error(`the shell of pane ${pane} never held the terminal alone before agent start; last process info ${seen}${shellChildren(root, shell)}`);
      }
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
    }
  };
  return attempt();
}

/** The processes that name `shell` as their parent on Windows, `<pid> <started> <older|newer|unknown> <name>` each (`shims/hide-children.c`). */
function listShellChildren(root: string, shell: number): string[] {
  const listed = spawnSync(path.join(root, "bin", "hide-children.exe"), [String(shell)], { encoding: "utf8", timeout: 10_000, windowsHide: true });
  if (listed.error) throw listed.error;
  if (listed.status !== 0) throw new Error(`hide-children.exe ${shell} exited ${listed.status}`);
  return listed.stdout.split(/\r?\n/).filter(Boolean);
}

/** On Windows, what the shell started, for a failure message; nothing elsewhere. */
function shellChildren(root: string, shell: number | null): string {
  if (process.platform !== "win32" || shell === null) return "";
  let listed: string;
  try { listed = listShellChildren(root, shell).join(", ") || "none"; } catch (error) { listed = String(error); }
  return `; children of shell ${shell}: ${listed}`;
}

/**
 * A `listShellChildren` line for a process that started before the shell yet
 * names its pid as its parent: Windows gave the shell the pid of that
 * process's dead parent (csrss.exe names the smss.exe that started it at
 * boot). The pinned Herdr counts a shell's children by parent pid alone, so it
 * refuses `agent start` in that pane for as long as the process runs, and
 * csrss.exe runs until shutdown.
 */
function isOlderClaimant(child: string): boolean {
  return child.split(" ")[2] === "older";
}

/** Each older claimant of each pane's shell, `pane <id> shell <pid>: <child>`. */
function claimedShells(env: NodeJS.ProcessEnv, bin: string, root: string, panes: string[]): string[] {
  return panes.flatMap((pane) => {
    const shell = (herdr(env, bin, ["pane", "process-info", "--pane", pane]) as { result: { process_info: { shell_pid: number | null } } }).result.process_info.shell_pid;
    if (shell === null) throw new Error(`pane ${pane} reports no shell pid`);
    return listShellChildren(root, shell).filter(isOlderClaimant).map((child) => `pane ${pane} shell ${shell}: ${child}`);
  });
}

/**
 * The workspace `open` makes, or on Windows, when an older process claims one
 * of its panes' shells (`isOlderClaimant`), a second one: opened while the
 * claimed shells still hold their pids, so no new shell can be given one of
 * them, after which the claimed workspace closes. A replacement that is
 * claimed too fails with each process's pid, start time and name. Herdr sets
 * a pane's shell pid before it answers the create, so the check needs no wait.
 */
function unclaimedWorkspace<T>(env: NodeJS.ProcessEnv, bin: string, root: string, open: () => T, of: (opened: T) => { workspace: string; panes: string[] }): T {
  const opened = open();
  if (process.platform !== "win32") return opened;
  const claimed = claimedShells(env, bin, root, of(opened).panes);
  if (claimed.length === 0) return opened;
  const replaced = of(opened).workspace;
  console.log(`herdr fixture: replacing workspace ${replaced}, a process older than its shell names the shell as parent: ${claimed.join("; ")}`);
  const replacement = open();
  herdr(env, bin, ["workspace", "close", replaced]);
  const again = claimedShells(env, bin, root, of(replacement).panes);
  if (again.length > 0) {
    throw new Error(`a process older than the shell names it as parent in the replacement workspace too, so the pinned Herdr refuses agent start there: ${again.join("; ")}`);
  }
  return replacement;
}

/**
 * Hands the private server's session to a new server process of the same
 * pinned binary, the way an update does (`herdr server live-handoff`): pane
 * ids and the panes' processes stay, and every pane token is gone. The new
 * server derives its own sockets below the config folder, so the fixture it
 * ran under was started with `shortRoot`.
 */
export function liveHandoff(fixture: Pick<HerdrFixture, "bin" | "env">): void {
  const contract = JSON.parse(fs.readFileSync(path.resolve("..", "contracts/herdr-api.schema.json"), "utf8")) as { protocol: number };
  const args = ["server", "live-handoff", "--import-exe", fixture.bin, "--expected-protocol", String(contract.protocol), "--expected-version", pinnedHerdrVersion()];
  execFileSync(fixture.bin, args, { env: fixture.env, encoding: "utf8", timeout: 60_000 });
}

/**
 * `shortRoot` puts the fixture's folder directly under /tmp: a live handoff
 * binds sockets named from the config folder, and the macOS temp folder is
 * too long for a Unix socket path. Windows has no such limit and no handoff.
 */
export async function startHerdr({ agents = true, shortRoot = false, readiness = "interactive" }: { agents?: boolean; shortRoot?: boolean; readiness?: "registered" | "interactive" } = {}): Promise<HerdrFixture> {
  const bin = herdrBinary();
  const version = execFileSync(bin, ["--version"], { encoding: "utf8" }).trim().split(/\s+/)[1];
  const pinned = pinnedHerdrVersion();
  if (version !== pinned) {
    throw new Error(`herdr ${version} at ${bin} is not the pinned ${pinned}`);
  }
  const root = fs.realpathSync.native(shortRoot && process.platform !== "win32" ? fs.mkdtempSync("/tmp/hh-") : fs.mkdtempSync(path.join(os.tmpdir(), "hide-e2e-herdr-")));
  // Windows Herdr maps this path to its pipe and marker; Unix needs /tmp's
  // short spelling, including room for the derived -client.sock address.
  const socket = process.platform === "win32" ? path.join(root, "herdr.sock") : `/tmp/hide-e2e-${crypto.randomBytes(4).toString("hex")}.sock`;
  let env: NodeJS.ProcessEnv;
  let fixturePath: string;
  try {
    env = isolatedEnv(root, socket);
    const shim = path.join(root, "bin", fixtureExecutable("claude"));
    copyFixtureShim("claude-shim", shim);
    if (process.platform === "win32") {
      copyFixtureShim("noop", path.join(root, "bin", "hide-open.exe"));
      copyFixtureShim("hide-children", path.join(root, "bin", "hide-children.exe"));
    }
    // Keep host-installed providers out while retaining system tools.
    fixturePath = fixtureToolPath(path.join(root, "bin"));
  } catch (error) {
    throw afterCleanup(error, () => fs.rmSync(root, { recursive: true, force: true }));
  }
  // Workspaces created later inherit the server's PATH, not hided's PATH.
  // Keep every pane on the same fake agent binary, including new workspaces.
  env.PATH = fixturePath;

  let spawnFailed: Error | null = null;
  const launchServer = (): ChildProcess => {
    spawnFailed = null;
    const log = fs.openSync(path.join(root, "herdr-server.log"), "a");
    try {
      const child = spawn(bin, ["server"], { env, stdio: ["ignore", log, log] });
      child.once("error", (error) => { spawnFailed = error; });
      return child;
    } finally {
      fs.closeSync(log);
    }
  };
  let server = launchServer();
  let stopped = false;
  const afterStop: (() => void)[] = [];
  const pendingStarts = new Set<PendingStart>();
  const pendingRequests = new Set<PendingStart>();
  const reads = new AbortController();
  const stopServer = () => {
    reads.abort();
    for (const request of [...pendingRequests]) request.stop();
    // On Windows a pane's processes can outlive the server and keep the
    // root locked. Listed while the server still runs (so its pid is its
    // own), ended after it stops; a failure here keeps the root and is
    // thrown after the logs are copied.
    let failure: unknown;
    const startFailures: unknown[] = [];
    for (const command of [...pendingStarts]) {
      try { command.stop(); } catch (error) { startFailures.push(error); }
    }
    if (startFailures.length > 0) failure = new AggregateError(startFailures, `fixture CLI exit unconfirmed; preserve ${root}: ${startFailures.map(String).join("; ")}`);
    let owned: WindowsProcess[] = [];
    if (process.platform === "win32" && server.pid && server.exitCode === null && server.signalCode === null) {
      try { owned = windowsProcessTree(server.pid); } catch (error) { failure = failure === undefined ? error : afterCleanup(failure, () => { throw error; }); }
    }
    spawnSync(bin, ["server", "stop"], { env, timeout: 10_000 });
    if (server.exitCode === null) server.kill("SIGKILL");
    if (process.platform === "win32") {
      try { endWindowsProcesses(owned, root); } catch (error) { failure = failure === undefined ? error : afterCleanup(failure, () => { throw error; }); }
    }
    for (const file of [socket, socket.replace(/\.sock$/, "-client.sock")]) {
      fs.rmSync(file, { force: true });
    }
    // Run evidence for a reviewer, next to the screenshots: the server's log
    // and what each agent pane's PTY received.
    const keep = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (keep) {
      for (const name of ["herdr-server.log", "input-one.log", "input-two.log"]) {
        const file = path.join(root, name);
        if (fs.existsSync(file)) fs.copyFileSync(file, path.join(keep, `${path.basename(root)}-${name}`));
      }
      // The server's own session log, where a pane whose shell never printed
      // a prompt says what it ran and how that ended.
      const sessions = path.join(root, "xdg-config", "herdr", "sessions");
      if (fs.existsSync(sessions)) {
        for (const session of fs.readdirSync(sessions)) {
          const file = path.join(sessions, session, "herdr-server.log");
          if (fs.existsSync(file)) fs.copyFileSync(file, path.join(keep, `${path.basename(root)}-session.log`));
        }
      }
    }
    if (failure !== undefined) throw failure;
    fs.rmSync(root, { recursive: true, force: true });
  };
  const { stop } = ownUntilWorkerExit(() => {
    let failure: unknown;
    try { stopServer(); } catch (error) { failure = error; }
    // Whatever the server left, nothing more will end it: a later cleanup
    // runs now, and every queued one runs even when one fails.
    stopped = true;
    const failed: unknown[] = [];
    for (const cleanup of afterStop.splice(0)) {
      try { cleanup(); } catch (error) { failed.push(error); }
    }
    if (failure !== undefined) throw failed.length === 0 ? failure : afterCleanup(failure, () => { throw failed[0]; });
    if (failed.length > 0) throw failed[0];
  });
  try {
    await waitFor(() => {
      if (spawnFailed) throw spawnFailed;
      return fs.existsSync(socket);
    }, `herdr socket ${socket}`);
    const snapshot = herdr(env, bin, ["api", "snapshot"]) as {
      result?: { snapshot?: { workspaces?: unknown[] } };
    };
    const workspaces = snapshot.result?.snapshot?.workspaces ?? [];
    if (workspaces.length !== 0) throw new Error("private herdr server already has workspaces");

    const inputLogs: [string, string] = [path.join(root, "input-one.log"), path.join(root, "input-two.log")];
    // A focused workspace of two panes side by side.
    const openPanes = () => {
      const created = herdr(env, bin, [
        "workspace",
        "create",
        "--cwd",
        path.join(root, "fixture"),
        "--label",
        "e2e",
        "--env",
        `PATH=${fixturePath}`,
        "--env",
        `HIDE_E2E_INPUT_LOG=${inputLogs[0]}`,
        "--focus",
      ]) as { result: { workspace: { workspace_id: string }; tab: { tab_id: string }; root_pane: { pane_id: string } } };
      const first = created.result.root_pane.pane_id;
      const split = herdr(env, bin, [
        "pane",
        "split",
        first,
        "--direction",
        "right",
        "--env",
        `PATH=${fixturePath}`,
        "--env",
        `HIDE_E2E_INPUT_LOG=${inputLogs[1]}`,
        "--no-focus",
      ]) as { result: { pane: { pane_id: string } } };
      const second = split.result.pane.pane_id;
      return { workspace: created.result.workspace.workspace_id, tab: created.result.tab.tab_id, panes: [first, second] as [string, string] };
    };
    const opened = unclaimedWorkspace(env, bin, root, openPanes, (workspace) => workspace);
    // Each shell at its prompt and, with agents, available to agent start. A
    // printed prompt does not mean agent.start can use the shell yet. Keep the
    // prompt and process-state wait inside the same setup deadline.
    for (const pane of opened.panes) {
      const deadline = Date.now() + 10_000;
      let processInfo: ShellProcessInfo | null = null;
      await waitFor(
        () => {
          if (!paneText(env, bin, pane).includes("fixture %")) return false;
          if (!agents) return true;
          const remaining = deadline - Date.now();
          if (remaining <= 0) return false;
          const { available, info } = shellAvailability(env, bin, root, pane, remaining);
          processInfo = info;
          return available;
        },
        `a prompt with an available shell in pane ${pane}`,
        10_000,
        () => JSON.stringify({ pane: paneRead(env, bin, pane), processInfo }),
      );
    }
    const [first, second] = opened.panes;
    let initialAgents: InitialAgent[] = [];
    let interactive = !agents || readiness === "interactive";
    let readyWait: Promise<void> | undefined;
    const waitUntilInteractive = async () => {
      const deadline = Date.now() + 30_000;
      let seen: InitialAgent[] = [];
      while (Date.now() < deadline) {
        if (reads.signal.aborted) throw new Error("fixture stopped while waiting for interactive agents");
        seen = [];
        for (const expected of initialAgents) {
          const { result: { agent } } = await agentRequest(env, "agent.get", { target: expected.name }, pendingRequests, Math.max(1, deadline - Date.now()));
          if (agent.terminal_id !== expected.terminal_id || agent.pane_id !== expected.pane_id || agent.name !== expected.name || (agent.agent && agent.agent !== "claude")) {
            throw new Error(`fixture agent identity changed during startup: ${JSON.stringify({ expected, agent })}`);
          }
          if (agent.agent_status === "blocked" || (["idle", "done"].includes(agent.agent_status) && !agent.launch_pending && !agent.interactive_ready)) {
            throw new Error(`fixture agent did not become interactive: ${JSON.stringify(agent)}`);
          }
          seen.push(agent);
        }
        if (seen.every((agent) => agent.agent === "claude" && agent.interactive_ready === true && !agent.launch_pending && ["idle", "done"].includes(agent.agent_status))) return;
        await pause(100, undefined, { signal: reads.signal });
      }
      throw new Error(`timed out waiting for both fixture agents to become interactive; last seen: ${JSON.stringify(seen)}`);
    };
    const ready = (): Promise<void> => {
      if (stopped || reads.signal.aborted) return Promise.reject(new Error("cannot wait for agents in a stopped fixture"));
      if (interactive) return Promise.resolve();
      return readyWait ??= waitUntilInteractive().then(() => { interactive = true; });
    };
    if (agents) {
      const start = readiness === "interactive"
        ? (args: string[]) => herdrAsync(env, bin, args, pendingStarts)
        : (args: string[]) => agentRequest(env, "agent.start", { name: args[2]!, kind: "claude", pane_id: args[args.indexOf("--pane") + 1]! }, pendingRequests);
      // Exactly two starts may run together. Observe both exits before setup
      // cleanup, so a sibling cannot keep using the server or root afterwards.
      const starts = await Promise.allSettled([
        Promise.resolve().then(() => startAgentAtShell(env, bin, root, ["agent", "start", "one", "--kind", "claude", "--pane", first], 10_000, start)),
        Promise.resolve().then(() => startAgentAtShell(env, bin, root, ["agent", "start", "two", "--kind", "claude", "--pane", second], 10_000, start)),
      ]);
      const failures = starts.flatMap((start) => start.status === "rejected" ? [start.reason as unknown] : []);
      if (failures.length > 0) throw failures.length === 1 ? failures[0] : new AggregateError(failures, "both fixture agents failed to start");
      initialAgents = starts.map((start, index) => {
        const agent = (start as PromiseFulfilledResult<{ result: { agent: InitialAgent } }>).value.result.agent;
        if (!agent.terminal_id || agent.name !== ["one", "two"][index] || agent.pane_id !== opened.panes[index]) throw new Error(`fixture start did not register its agent: ${JSON.stringify(agent)}`);
        return agent;
      });
      // Distinct row labels, made by the core from each pane's transcript.
      for (const [pane, task] of [
        [first, "Agent one"],
        [second, "Agent two"],
      ]) {
        writeFixtureTranscript(path.join(root, "home", ".claude", "projects"), fixtureSessionId(pane!), { task: task! });
        declareFixtureSession(env, bin, pane!, "claude", fixtureSessionId(pane!));
      }
    }
    return {
      bin,
      socket,
      env,
      root,
      workspace: opened.workspace,
      tab: opened.tab,
      panes: [first, second],
      inputLogs,
      fixturePath,
      shell: paneShellOf(env),
      run: (args) => {
        const input = (args[0] === "pane" && ["send-text", "send-keys", "run"].includes(args[1]!)) || (args[0] === "agent" && ["prompt", "send-keys"].includes(args[1]!));
        const target = args.includes("--pane") ? args[args.indexOf("--pane") + 1] : args[2];
        if (!interactive && input && [first, second, "one", "two"].includes(target!)) {
          throw new Error("initial fixture agent input requires await fixture.ready() or interactive setup");
        }
        if (args[0] === "workspace" && args[1] === "create") {
          return unclaimedWorkspace(env, bin, root, () => herdr(env, bin, args) as CreatedWorkspace, (created) => ({
            workspace: created.result.workspace.workspace_id,
            panes: [created.result.root_pane.pane_id],
          }));
        }
        const result = args[0] === "agent" && args[1] === "start" && args.includes("--pane") ? startAgentAtShell(env, bin, root, args) : herdr(env, bin, args);
        if (args[0] === "agent" && args[1] === "start") {
          const pane = args[args.indexOf("--pane") + 1];
          const kind = args[args.indexOf("--kind") + 1];
          if (args.includes("--pane") && ["claude", "codex"].includes(kind!))
            declareFixtureSession(env, bin, pane!, kind!, fixtureSessionId(pane!));
        }
        return result;
      },
      stop,
      ready,
      restart: async (beforeStart) => {
        const owned = process.platform === "win32" && server.pid ? windowsProcessTree(server.pid) : [];
        const result = spawnSync(bin, ["server", "stop"], { env, timeout: 10_000 });
        if (result.status !== 0) throw new Error(`private server stop failed: ${result.stderr}`);
        await waitFor(() => server.exitCode !== null || server.signalCode !== null, "private server exit before cold restore");
        if (process.platform === "win32") endWindowsProcesses(owned, root);
        beforeStart?.();
        server = launchServer();
        await waitFor(() => {
          if (spawnFailed) throw spawnFailed;
          return fs.existsSync(socket);
        }, "private cold-restored server socket");
      },
      afterStop: (cleanup) => {
        if (stopped) cleanup();
        else afterStop.push(cleanup);
      },
    };
  } catch (error) {
    throw afterCleanup(error, stop);
  }
}

/**
 * A workspace of its own at `cwd` with one agent in it, recorded as spawned
 * by `parent` the way hided records it (`declareParent`) when one is named.
 * With `named`, the agent runs a session labelled so before the relationship
 * is written for it, since a later session is no longer that child's.
 * Returns the new pane.
 */
export async function spawnAgent(fixture: HerdrFixture, label: string, parent: string | null, cwd = path.join(fixture.root, label), named?: FixtureLabel): Promise<string> {
  fs.mkdirSync(cwd, { recursive: true });
  const created = fixture.run(["workspace", "create", "--cwd", cwd, "--label", label, "--env", `PATH=${fixture.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await waitFor(() => paneText(fixture.env, fixture.bin, pane).includes("fixture %"), `a prompt in pane ${pane}`, 20_000, () => JSON.stringify(paneRead(fixture.env, fixture.bin, pane)));
  fixture.run(["agent", "start", label, "--kind", "claude", "--pane", pane]);
  if (named) labelAgent(fixture, pane, named);
  if (parent) declareParent(fixture, pane, parent);
  return pane;
}
