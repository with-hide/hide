// What the Hide AI specs share: the coordination record Hide leaves when it
// starts an agent, and a git that never reads the operator's configuration.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";

export const digest = (value: string) => createHash("sha256").update(value).digest("hex");

/** The operator's git configuration (signing, hooks) never reaches the scratch repository. */
export const git = (cwd: string, ...args: string[]) =>
  execFileSync("git", ["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args], {
    cwd,
    env: { ...process.env, GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1" },
    stdio: ["ignore", "ignore", "inherit"],
  });

/** The coordination record Hide leaves when it starts an agent for a card: a handoff whose origin is its own identity. */
export function ledgerOf(node: string, started: { pane: string; session: string; path: string; branch: string; task: { kind: string; target: string } | null }[]): string {
  const hide = {
    id: "agent-1", name: "hide:memory", machine: node, host_scope: "hide", native_machine: "hide", session: "hide:memory", instance: "hide:memory", pane: "hide:memory",
    parent: null, origin: null, project: null, ended: false, forked: false,
    actor: { pane_id: "hide:memory", name: "hide:memory", kind: "hide", device_id: node, session: digest("hide:memory") },
  };
  const agents = [hide];
  const spawns: object[] = [];
  started.forEach((one, index) => {
    const name = `hide-ai-${index}`;
    const id = `agent-${index + 2}`;
    agents.push({
      id, name, machine: node, host_scope: "local", native_machine: node, session: one.session, instance: one.pane, pane: one.pane,
      parent: null, origin: "agent-1", project: one.path, ended: false, forked: false,
      actor: { pane_id: one.pane, name, kind: "claude", device_id: node, session: digest(one.session) },
    } as never);
    spawns.push({
      id: `spawn-${index + 20}`, parent: "agent-1", mode: "handoff", machine: null, intent: `memory-promote-${index}`, name, kind: "claude", repo: path.dirname(one.path),
      branch: one.branch, requested_path: null, args: [], path: one.path, pane: one.pane, terminal: null, child: id, completed: true, auto_watch_id: null,
      ...(one.task ? { task: one.task } : {}),
    });
  });
  return JSON.stringify({ version: 1, next_id: 100, letters: [], watches: [], agents, spawns });
}

