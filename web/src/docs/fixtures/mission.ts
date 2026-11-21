// A small mission for the docs' map: the Conductor's plan as four nodes, a finished look at the callers, an
// API change at work, the callers waiting on it, and a check at the end. Values are invented; the shape is the
// wire's (`Mission`, `MissionNode`).
import { withCoreWaits } from "../../gallery/missionWaits";
import type { Mission, MissionNode, MissionWorker, SnapshotRest } from "../../snapshot";
import { minutesAgo } from "./agent";
import { docsProject } from "./project";

export const MISSION_ID = "docs-mission";
const CONDUCTOR_PANE = "docs-conductor";

function node(key: string, title: string, extra: Partial<MissionNode>): MissionNode {
  return {
    key, title, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null,
    attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null,
    commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra,
    prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []),
  };
}

const opus: MissionWorker = { agent: "claude", model: "Opus", effort: "high", effort_auto: true };
const reviewer: MissionWorker = { agent: "codex", model: "Astra", effort: "medium", effort_auto: true };

export function missionFixture(): { mission: Mission; rest: SnapshotRest } {
  const nodes: MissionNode[] = [
    node("callers", "Find every caller of the old endpoint", { kind: "general", preset: "research", delivery: null, worker: opus, status: "done", started_at_unix_ms: minutesAgo(50), finished_at_unix_ms: minutesAgo(35), line: "14 callers in 6 files" }),
    node("api", "Add the v2 endpoint", { depends_on: ["callers"], worker: opus, status: "working", started_at_unix_ms: minutesAgo(30), pr: 412, pr_state: "pending", branch: "feat/orders-v2", stage: 1, pane_id: "docs-worker" }),
    node("move", "Move the callers to v2", { depends_on: ["api"], worker: opus }),
    node("check", "Check the whole change", { kind: "general", preset: "verify", delivery: null, depends_on: ["move"], worker: reviewer }),
  ];
  const mission: Mission = withCoreWaits({
    id: MISSION_ID, title: "Move the orders API to v2", workspace_id: "hide", project: "/projects/orders-app", checkout_path: "/projects/orders-app", device_id: "local",
    status: "running", scope: "pr", cap: 4, exempt: false, strength: "normal",
    conductor: { pane_id: CONDUCTOR_PANE, name: "Orders v2 - Conductor", agent_kind: "claude", session: { device_id: "local", agent: "claude", session_id: "docs-conductor" }, present: true, takes_answers: true },
    nodes, merge_order: ["api"], landing_order: ["api"], stack_waiting: 0, check: "pnpm test", github: "you", decision: null, question: null, answer: null, answer_failed: null,
    delay: null, report: null, created_at_unix_ms: minutesAgo(60), started_at_unix_ms: minutesAgo(50), closed_at_unix_ms: null, running: 1,
    stale: false, github_stale: false, github_read_at_unix_ms: minutesAgo(1), conductor_letters: null,
  });
  const { rest: base } = docsProject();
  const rest: SnapshotRest = {
    missions: { items: [mission] },
    mission_events: { mission: mission.id, events: [], dropped: 0 },
    navigator: base.navigator,
    overview_graph: { available: true, limit: 2000, retained_count: 0, truncated: false, retention_ms: 86400000, letters: [], sessions: [] },
    status: { remote: [] },
    ui_state: { conductor_settings: { concurrency: 4, default_scope: "pr", candidates: [], default_strength: "normal", clean_merged: true, map_flow: "down" } },
  };
  return { mission, rest };
}
