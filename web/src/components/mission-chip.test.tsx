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

// Desk's mission chip, in a job card, in an answer and in the head of the
// session that conducts the mission, is two lines (Doop N2 가): the mark, the
// number and the name; then the cells, `k/n` or the state's word, and the
// operator's turn.

initializeInterfaceI18n("en");

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: key, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

function mission(id: string, extra: Partial<Mission> = {}): Mission {
  return { id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: { device_id: "mac", agent: "claude", session_id: "s-1" }, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

const NODES = [node("a", { status: "done" }), node("b", { status: "working" })];
const STATES: { status: Mission["status"]; nodes: MissionNode[]; text: string; cells: string | null }[] = [
  { status: "planning", nodes: [], text: "M1DeskPlanning", cells: null },
  { status: "planning", nodes: NODES, text: "M1DeskPlanning", cells: null },
  { status: "running", nodes: NODES, text: "M1Desk1/2", cells: "done working" },
  { status: "merge_wait", nodes: NODES, text: "M1Desk1/2Waiting to merge", cells: "done working" },
  { status: "done", nodes: NODES, text: "M1Desk1/2Finished", cells: "done working" },
  { status: "cancelled", nodes: NODES, text: "M1DeskCancelled", cells: null },
];

let mounted: ReturnType<typeof createRoot>[] = [];
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function chip(node: React.ReactNode): Promise<HTMLElement | null> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<TooltipProvider>{node}</TooltipProvider>));
  return host.querySelector<HTMLElement>("[data-mission-chip]");
}


const shape = (one: HTMLElement | null) => [one?.textContent, one?.querySelector("[data-progress-cells]")?.getAttribute("data-progress-cells") ?? null];

it("draws a carried mission in two lines for every status, its cells from the counts it carries", async () => {
  for (const { status, nodes, text, cells } of STATES) {
    const carried: DeskMission = { id: "mission-1", title: "Desk", status, done: nodes.length ? 1 : 0, total: nodes.length, turn: null, delay: null };
    const drawn = await chip(<DeskMissionChip mission={carried} onOpen={() => {}} />);
    // A planning mission with no nodes and a cancelled one draw no cells; the others draw the counts' cells (`done waiting`).
    expect(shape(drawn), `${status} with ${nodes.length} nodes`).toEqual([text, cells?.replace("working", "waiting") ?? null]);
    expect(drawn?.querySelectorAll("[data-mission-chip-line]")).toHaveLength(1);
  }
});

it("draws a mission the store holds with the nodes' own cells, the same words, and in an answer's row the same chip", async () => {
  const saved = useShellStore.getState();
  try {
    for (const { status, nodes, text, cells } of STATES) {
      const held = mission("mission-1", { title: "Desk", status, nodes });
      useShellStore.setState({ rest: { missions: { items: [held] } } as unknown as SnapshotRest });
      expect(shape(await chip(<MissionChipOf id="mission-1" onOpen={() => {}} />)), `${status} with ${nodes.length} nodes`).toEqual([text, cells]);
    }
    const carried: DeskMission = { id: "mission-1", title: "Desk", status: "cancelled", done: 1, total: 2, turn: null, delay: null };
    expect((await chip(<DeskMissionChip mission={carried} onOpen={() => {}} />))?.className).toContain("opacity-(--opacity-dimmed)");
  } finally {
    useShellStore.setState(saved, true);
  }
});

it("edges a chip in the warning colour and says `! Needs you` while the operator's turn waits, keeping the progress", async () => {
  const carried: DeskMission = { id: "mission-1", title: "Desk", status: "running", done: 1, total: 2, turn: "decision", delay: null };
  const drawn = await chip(<DeskMissionChip mission={carried} onOpen={() => {}} />);
  expect([drawn?.textContent, drawn?.className.includes("border-warning"), drawn?.dataset.missionChipTurn]).toEqual(["M1Desk1/2! Needs you", true, "decision"]);
});

it("keeps the name of a mission whose id has no number and cuts only the name short", async () => {
  const carried: DeskMission = { id: "m-x", title: "A very long mission name that runs on and on", status: "running", done: 1, total: 2, turn: null, delay: null };
  const drawn = await chip(<DeskMissionChip mission={carried} onOpen={() => {}} />);
  expect([drawn?.textContent, drawn?.querySelector(".truncate")?.textContent]).toEqual(["A very long mission name that runs on and on1/2", "A very long mission name that runs on and on"]);
});

it("ends the second line with a grey pause and the time left while a mission delay holds the mission", async () => {
  const carried: DeskMission = { id: "mission-1", title: "Desk", status: "running", done: 1, total: 2, turn: null, delay: "rate limit", delay_until_unix_ms: Date.now() + 12 * 60_000 };
  const drawn = await chip(<DeskMissionChip mission={carried} onOpen={() => {}} />);
  const held = drawn?.querySelector("[data-mission-chip-line] [data-mission-chip-delay]");
  expect([held?.textContent, held?.querySelector("svg") !== null, drawn?.querySelector("[data-mission-chip-line]")?.lastElementChild === held]).toEqual(["12m", true, true]);
});
