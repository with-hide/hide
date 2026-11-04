// External CLI stand-ins for the start contract. The same program is run by
// a native launcher and by npm's script launchers, with real process ancestry.
import fs from "node:fs";
import path from "node:path";
import type { AgentFixture } from "./engine-fixture";
import { fixtureExecutable } from "./platform-fixture";

export const resumeFlags: Record<string, string> = { codex: "resume", opencode: "-s", pi: "--session", agy: "--conversation" };

export function startProgram(fixture: AgentFixture, kind: string): string {
  const definition = ({ claude: "claude-code", agy: "antigravity" } as Record<string, string>)[kind] ?? kind;
  const screen = kind === "omp" ? "omp fixture ready\r\n" : fs.readFileSync(path.resolve("..", "hide-agent-adapter", "tests", "fixtures", "screens", definition, "idle-empty-composer.ansi"), "utf8");
  const journey = JSON.parse(fs.readFileSync(path.resolve("..", "contracts", "agent-journey.json"), "utf8")) as { herdr_kind: string; herdr_source: string }[];
  const source = journey.find((row) => row.herdr_kind === kind)?.herdr_source;
  if (!source) throw new Error(`the journey declares no integration source for ${kind}`);
  const config = {
    kind, screen, root: fixture.root, resume: resumeFlags[kind] ?? "--resume",
    source, sessionPath: kind === "pi" || kind === "omp",
    herdr: fixture.engine === "herdr" ? fixture.bin : undefined,
    hooks: path.resolve("..", "target", "debug", fixtureExecutable("hide-agent-hooks")),
  };
  return `
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const config = ${JSON.stringify(config)};
const args = process.argv.slice(2);
// The kit reads an older CLI with no shared daemon or app-server.
if (config.kind === "codex" && args[0] === "features") process.exit(args[1] === "list" ? 0 : 2);
if (config.kind === "codex" && args[0] === "app-server") process.exit(2);
const pane = process.env.HIDE_PANE_ID || process.env.HERDR_PANE_ID;
if (!pane) throw new Error("the stand-in needs its pane identity");
const selected = args.indexOf(config.resume);
const fresh = "platform-" + config.kind + "-" + process.pid;
const session = selected < 0 ? (config.sessionPath ? path.join(config.root, fresh + ".jsonl") : fresh) : args[selected + 1];
process.stdin.setRawMode(true);
process.stdin.on("data", bytes => {
  if (!bytes.includes(0x3f)) return;
  // The test asks only after the engine has detected this process. Herdr's
  // native integration supplies its session; the own engine reads resume
  // arguments, and fresh detection needs no invented session report.
  if (config.herdr) {
    const identity = ["--source", config.source, "--agent", config.kind,
      config.sessionPath ? "--agent-session-path" : "--agent-session-id", session];
    // These integrations own Herdr's state and establish their authority
    // with the session, as the Pi/omp host fixture does. Session-only
    // reports cannot establish that authority on their own.
    if (["pi", "omp", "opencode", "agy"].includes(config.kind)) {
      execFileSync(config.herdr, ["pane", "report-agent", pane, ...identity, "--state", "idle", "--seq", "1"], { timeout: 10000 });
    }
    execFileSync(config.herdr, ["pane", "report-agent-session", pane, ...identity,
      "--session-start-source", "startup", "--seq", "2"], { timeout: 10000 });
  }
  // omp has no screen rules. Reproduce its extension's idle report after
  // an interrupted turn, rather than inventing a resting-screen rule.
  if (config.kind === "omp") {
    for (const operation of ["session", "turn"]) {
      const report = execFileSync(config.hooks, ["omp", operation], {
        input: JSON.stringify({ version: 14, session_id: session, turn: "idle" }), encoding: "utf8", timeout: 10000,
      });
      if (JSON.parse(report).reported !== true) throw new Error("the stand-in's " + operation + " was not reported");
    }
  }
});
process.stdout.write("\\x1b[2J\\x1b[H" + config.screen, () => {
  fs.writeFileSync(path.join(config.root, "terminal-ready-" + pane.replaceAll(":", "-")), "");
});
`;
}
