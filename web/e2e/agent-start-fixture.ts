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
  const config = {
    kind, screen, root: fixture.root, resume: resumeFlags[kind] ?? "--resume",
    sessionPath: kind === "pi" || kind === "omp",
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
const pane = process.env.HIDE_PANE_ID;
if (!pane) throw new Error("the stand-in needs its pane identity");
const selected = args.indexOf(config.resume);
const fresh = "platform-" + config.kind + "-" + process.pid;
const session = selected < 0 ? (config.sessionPath ? path.join(config.root, fresh + ".jsonl") : fresh) : args[selected + 1];
process.stdin.setRawMode(true);
process.stdin.on("data", bytes => {
  if (!bytes.includes(0x3f)) return;
  // The test asks only after the engine has detected this process. The
  // engine reads resume arguments, and fresh detection needs no invented
  // session report. omp has no screen rules: its extension reports an
  // interrupted turn through Hide.
  if (config.kind === "omp") {
    for (const operation of ["session", "turn"]) {
      const report = execFileSync(config.hooks, ["omp", operation], {
        input: JSON.stringify({ version: 15, session_id: session, turn: "idle" }), encoding: "utf8", timeout: 10000,
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
