// What every engine fixture shares: the pane shell commands a spec types and
// the synthetic agent transcripts the fixture provider answers from.
//
// Each agent pane runs a tiny compiled `claude` that copies stdin to stdout.
// A copied /bin/cat would not do: macOS kills a relocated platform binary.
// Like a real TUI it reads the terminal raw, byte by byte with no line
// discipline echo and no extended input processing (which would swallow ^O
// and ^V), and it appends every byte it reads to `HIDE_E2E_INPUT_LOG`: that
// file is what the PTY received, which is how a test tells a click's mouse
// report from what the shell only sent (PRD S2 B20).
//
// The same binary is the daemon's Claude provider, because hided finds
// `claude` on the same PATH: `claude auth status` answers logged in, and a
// print-mode request (`--json-schema`, or a plain-JSON one carrying its schema in `--system-prompt`) answers with the label written after
// the last `HIDE_E2E_LABEL ` in its prompt, after the delay a
// `HIDE_E2E_DELAY_MS ` line before it asks for, and appends the answered
// task to `HIDE_E2E_PROVIDER_LOG` when that is set. `writeFixtureTranscript`
// writes that marker into a synthetic Claude transcript, so a pane's label
// comes from the core's own transcript read and analysis, never from a token
// (PRD labels-in-hided).

import fs from "node:fs";
import path from "node:path";
import { powershellCommand, powershellQuote } from "./windows-command";

/** What a helper here reads of a fixture: its root, where `home/` is the fixture HOME. */
export type FixtureRoot = { root: string };


export type PaneShell = "posix" | "cmd" | "powershell";

/** Match the ordinary pane shell selected by the engine fixture. */
export function panePrompt(shell: PaneShell): RegExp {
  // A narrow pane wraps the path onto physical rows; the prompt still ends at >.
  return shell === "powershell" ? /PS [^>]*>/ : /fixture %/;
}

/**
 * The family of the shell the engine fixture named for the panes. The shell
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
  return shell === "cmd" ? /cmd\.exe|Tab \d+/i : shell === "powershell" ? /powershell|Tab \d+/i : /zsh|Tab \d+/;
}

/** The line that makes a pane's shell print `lines`, one at a time. */
export function printLinesCommand(shell: PaneShell, prefix: string, count: number): string {
  if (shell === "powershell") return `1..${count} | ForEach-Object { Write-Output (${powershellQuote(prefix)} + $_) }`;
  if (shell === "cmd") return `for %n in (${Array.from({ length: count }, (_, i) => i + 1).join(" ")}) do @echo ${prefix}%n`;
  return `for n in ${Array.from({ length: count }, (_, i) => i + 1).join(" ")}; do printf '${prefix}%s\\n' "$n"; sleep 0.1; done`;
}

type PaneProgram = {
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
  if (shell === "powershell") return powershellCommand(argv);
  return argv.map(shell === "cmd" ? cmdQuote : posixQuote).join(" ");
}

/** Clears the pane and prints a fixture file through the shell it really runs. */
export function printFileCommand(shell: PaneShell, file: string): string {
  if (shell === "powershell") return `Clear-Host; Get-Content -LiteralPath ${powershellQuote(file)} -Encoding UTF8`;
  return shell === "cmd" ? `cls & type ${cmdQuote(file)}` : `clear; cat ${posixQuote(file)}`;
}

/** Prints the terminal dimensions read by a program in the fixture's actual shell. */
export function paneSizeCommand(shell: PaneShell, marker: string): string {
  if (shell === "posix") return `echo ${posixQuote(marker)} $(stty size)`;
  const script = `$s = $Host.UI.RawUI.WindowSize; ${powershellQuote(`${marker} `)} + $s.Height + ' ' + $s.Width`;
  return shell === "powershell" ? script : paneCommandLine(shell, ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script]);
}

/** The text that runs `program` in a pane's shell, ready for `pane run`. */
export function paneProgramLine(shell: PaneShell, program: PaneProgram): string {
  // Type only a short command, even when the preceding process has finished
  // before the shell restores its line editor. Long multiline arguments can
  // otherwise hit the terminal's canonical input limit. The private script
  // also keeps bytes independent of PowerShell 5's UTF-16 redirection.
  const script = `${program.status}.cjs`;
  fs.writeFileSync(script, `
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
const program = ${JSON.stringify(program)};
const out = fs.openSync(program.stdout ?? ${JSON.stringify(process.platform === "win32" ? "NUL" : "/dev/null")}, "w");
const err = fs.openSync(program.stderr, "w");
try {
  const [binary, ...args] = program.argv;
  const result = spawnSync(binary, args, { env: { ...process.env, ...program.env }, stdio: ["inherit", out, err], timeout: 20000 });
  if (result.error) fs.writeSync(err, String(result.error));
  fs.writeFileSync(program.status, String(result.status ?? 1));
} finally { fs.closeSync(out); fs.closeSync(err); }
`);
  return paneCommandLine(shell, [process.execPath, script]);
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
  work?: { missed:{kind:"problem"|"follow_up"|"ask";text:string;/** The open ask this line repeats, by its id; none when left out. */same_as?:string|null;/** The answer's sentence a new ask is asked in; its text when left out, which the marker carries. */quote?:string;/** An ask's kind of step; decision when left out. */ask_type?:"decision"|"human_only"|null;/** Whom an ask waits on; the operator when left out. */addressed_to?:"operator"|"parent"|null}[]; resolved:{id:string;issue_number:number|null;/** What settled it without an answer; empty when left out. */reason?:string}[]; artifacts:string[] };
  at?: number;
  /** Native cwd witness, when repository provenance is part of the journey. */
  cwd?: string;
  /** Addresses printed by a real native tool-result record in this turn. */
  toolOutputs?: string[];
};

/**
 * What every start and restart Hide makes of Claude carries by default (Settings > Agents, 전부 허용):
 * its full-access flag and the setting that skips the warning Claude Code stops that flag at. Hide
 * passes these as argv; `CLAUDE_FULL_ACCESS_LINE` is them as a process listing joins them.
 */
export const CLAUDE_FULL_ACCESS = ["--dangerously-skip-permissions", "--settings", '{"skipDangerousModePermissionPrompt":true}'];
export const CLAUDE_FULL_ACCESS_LINE = CLAUDE_FULL_ACCESS.join(" ");

/** Where the fixture's Claude transcripts live: the fixture HOME's, which
 * `startHided` links into the daemon's HOME. */
export function claudeProjects(fixture: FixtureRoot): string {
  return path.join(fixture.root, "home", ".claude", "projects");
}

/**
 * The daemon reads agent transcripts under its own HOME; the fixture writes
 * them under its own, before or after the daemon starts. One folder for both
 * keeps every label the fixture writes readable by a daemon on `home`.
 */
export function linkFixtureTranscripts(fixture: FixtureRoot, home: string): void {
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
/** What a provider answers for a missed line the fixture leaves unsaid. */
function askDefaults(line: { kind: string; text: string; same_as?: string | null }) {
  const ask = line.kind === "ask";
  return { quote: ask && !line.same_as ? line.text : "", ask_type: ask ? "decision" : null, addressed_to: ask ? "operator" : null };
}

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
    // The schema asks every line for `same_as`, `quote`, `ask_type` and `addressed_to` and every resolution for `reason`, as a provider answers them.
    // A new ask quotes its own text: the marker is the turn's answer, so the core finds the quote in it.
    work: label.work ? { ...label.work, missed: label.work.missed.map((line) => ({ same_as: null, ...askDefaults(line), ...line })), resolved: label.work.resolved.map((done) => ({ reason: "", ...done })) } : {missed:[],resolved:[],artifacts:[]},
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
export function continueFixtureTranscript(fixture: FixtureRoot, sessionId: string, label: FixtureLabel): void {
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
