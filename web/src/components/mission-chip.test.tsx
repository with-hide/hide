// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeskMission } from "../desk";
import { initializeInterfaceI18n } from "../i18n/instance";
import type { Mission, MissionNode, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { DeskMissionChip, MissionChipOf } from "./mission-chip";
import { TooltipProvider } from "./ui/tooltip";

// Desk's mission chip, in a job card and in an answer, says what the session
// header's chip says (Doop frame C): the number, the state's word, `k/n`.

initializeInterfaceI18n("en");

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: key, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

function mission(id: string, extra: Partial<Mission> = {}): Mission {
  return { id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: { device_id: "mac", agent: "claude", session_id: "s-1" }, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const NODES = [node("a", { status: "done" }), node("b", { status: "working" })];
const STATES: { status: Mission["status"]; nodes: MissionNode[]; face: string }[] = [
  { status: "planning", nodes: [], face: "M1·Planning" },
  { status: "planning", nodes: NODES, face: "M1·1/2" },
  { status: "running", nodes: NODES, face: "M1·1/2" },
  { status: "merge_wait", nodes: NODES, face: "M1·Waiting to merge" },
  { status: "done", nodes: NODES, face: "M1·1/2" },
  { status: "cancelled", nodes: NODES, face: "M1·Cancelled" },
];

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function face(node: React.ReactNode): Promise<string | undefined> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return host.querySelector("[data-mission-chip]")?.textContent ?? undefined;
}

it("draws a carried mission's number, state word and count for every status, as the session header's chip does", async () => {
  for (const { status, nodes, face: expected } of STATES) {
    const carried: DeskMission = { id: "mission-1", title: "Desk", status, done: nodes.length ? 1 : 0, total: nodes.length, turn: null, delay: null };
    expect(await face(<DeskMissionChip mission={carried} onOpen={() => {}} />), `${status} with ${nodes.length} nodes`).toBe(expected);
  }
});

it("draws a mission the store holds in an answer with the same face as the card's chip", async () => {
  const saved = useShellStore.getState();
  try {
    for (const { status, nodes, face: expected } of STATES) {
      const held = mission("mission-1", { title: "Desk", status, nodes });
      useShellStore.setState({ rest: { missions: { items: [held] } } as unknown as SnapshotRest });
      expect(await face(<MissionChipOf id="mission-1" onOpen={() => {}} />), `${status} with ${nodes.length} nodes`).toBe(expected);
    }
  } finally {
    useShellStore.setState(saved, true);
  }
});

it("keeps the word of a mission whose id has no number", async () => {
  const carried: DeskMission = { id: "m-x", title: "Desk", status: "running", done: 1, total: 2, turn: null, delay: null };
  expect(await face(<DeskMissionChip mission={carried} onOpen={() => {}} />)).toBe("Mission1/2");
});
