// @vitest-environment jsdom
// A mission's page in the 보기 slot (PRD software-factory-conductor B43; PRD
// mission-plan-visibility B25-B30, B39; PRD mission-map-git-criteria
// B21-B35): what it shows in each state of the record and its events, what a
// pick lights and opens, and the one event each press sends.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionMap } from "./MissionMap";
import type { AgentRow, Mission, MissionCriterion, MissionEvent, MissionNode, SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  vi.useRealTimers();
  document.body.innerHTML = "";
});

function node(key: string, extra: Partial<MissionNode> = {}): MissionNode {
  return { key, title: `Node ${key}`, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, verdict: null, checks: null, ...extra };
}

function mission(extra: Partial<Mission> = {}): Mission {
  return { id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, checkers: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: Date.now() - 180_000, closed_at_unix_ms: null, running: 2, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
}

async function mount(missions: { items: Mission[]; failed?: string }, remote: unknown[] = [], sessions: unknown[] = [], more: Partial<SnapshotRest> = {}, agents: AgentRow[] = []) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents,
    rest: { ...more, missions, navigator: { devices: [{ id: "mac", kind: "local", label: "This Mac" }, { id: "mini", kind: "remote", label: "mini" }], workspaces: [] }, status: { remote }, overview_graph: { sessions } } as unknown as SnapshotRest,
  });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionMap missionId="mission-1" actions={actions} /></TooltipProvider>));
  const q = (selector: string) => container.querySelector<HTMLElement>(selector);
  const rerender = async (next: Mission) => { await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, missions: { items: [next] } } }))); };
  return { container, events, q, rerender, unmount: async () => { await act(async () => root.unmount()); } };
}

/** Types into a field as a person does, through React's own value tracking. */
async function type(field: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(target: Element, key: string) {
  await act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })); });
}

it("reads top down as the head, 내 차례 only when the operator has something to do, the map and the node panel, with no flow table, legend or tabs (B24)", async () => {
  const empty = await mount({ items: [mission({ status: "planning", started_at_unix_ms: null })] });
  try {
    expect(empty.q("[data-mission-planning]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-pill]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-turn]")).toBeNull();
  } finally { await empty.unmount(); }
  const nodes = [node("a", { status: "done" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { status: "review", depends_on: ["a"] }), node("d", { status: "held" }), node("e", { depends_on: ["b"] }), node("f", { status: "dropped" })];
  const running = await mount({ items: [mission({ nodes })] });
  try {
    expect(running.q("[data-mission-pill]")?.textContent).toBe("Running");
    expect(running.q('[data-mission-stat="time"]')?.textContent).toBe("3m so far");
    expect(running.q('[data-mission-stat="done"]')?.textContent).toBe("1 / 5 done");
    // Its cells are drawn as the cards: done grey, the rest in their state's colour (B26).
    expect(running.q("[data-mission-summary] [data-progress-cells]")?.getAttribute("data-progress-cells")).toBe("done hold working review waiting");
    expect(running.q("[data-mission-summary] [data-progress-cells] span")?.className).toContain("bg-muted-foreground/60");
    expect(running.q("[data-mission-turn]")).toBeNull();
    expect(running.container.querySelectorAll("[data-mission-node]")).toHaveLength(6);
    expect(running.container.querySelector("[data-mission-legend], [data-mission-flow], [data-mission-zoom], [role=tablist], [data-node-steps], [data-node-base]")).toBeNull();
    // Nothing picked: no panel, the map has the whole width (B30, B31).
    const parts = () => [...running.container.querySelectorAll<HTMLElement>("[data-mission-summary], [data-mission-turn], [data-mission-map], [data-mission-panel]")].map((part) => Object.keys(part.dataset)[0]);
    expect(parts()).toEqual(["missionSummary", "missionMap"]);
    await running.rerender(mission({ nodes, decision: "Rail or slot?" }));
    expect(parts()).toEqual(["missionSummary", "missionTurn", "missionMap"]);
    await act(async () => running.q('[data-mission-node="b"]')!.click());
    expect(parts()).toEqual(["missionSummary", "missionTurn", "missionMap", "missionPanel"]);
  } finally { await running.unmount(); }
});

const asked = (extra: Partial<Mission> = {}): Mission => mission({
  decision: "Which base for the screen PR?",
  question: { key: 3, why: "Two PRs touch the same file", options: ["Stack on the API PR", "Wait for the API PR to merge"], recommend: 1, node: "screen", asked_at_unix_ms: Date.now() - 120_000 },
  nodes: [node("api", { status: "working" }), node("screen", { title: "API screen", status: "held" }), node("other", { status: "held" })],
  ...extra,
});

it("shows the decision with why, the node it stopped, its wait and the choices, and sends a pressed choice once (B25, B26, B30)", async () => {
  const map = await mount({ items: [asked()] });
  try {
    const banner = map.q('[data-mission-banner="decision"]')!;
    expect(banner.querySelector("[data-mission-question]")?.textContent).toBe("Which base for the screen PR?");
    expect(banner.querySelector("[data-mission-banner-why]")?.textContent).toBe("Two PRs touch the same file");
    expect(banner.querySelector("[data-mission-banner-node]")?.textContent).toBe("API screen · awaiting decision");
    expect(banner.querySelector("[data-mission-banner-waited]")?.textContent).toBe("waiting 2m");
    expect([...banner.querySelectorAll("[data-mission-option]")].map((button) => button.textContent)).toEqual(["Stack on the API PRRecommended", "Wait for the API PR to merge"]);
    expect(banner.querySelector("[data-mission-recommended]")?.getAttribute("data-mission-option")).toBe("1");
    // The decision's question rides only the node it stopped (B16).
    expect(map.q('[data-mission-node="screen"]')?.getAttribute("aria-label")).toContain("Which base for the screen PR?");
    expect(map.q('[data-mission-node="other"]')?.getAttribute("aria-label")).not.toContain("Which base");
    await act(async () => (banner.querySelector('[data-mission-option="2"]') as HTMLElement).click());
    expect(map.events.filter((event) => event.kind === "mission_answer")).toEqual([{ schema_version: 2, kind: "mission_answer", payload: { mission: "mission-1", key: 3, option: 2 } }]);
    // Sent at once: the choice with a check, nothing left to press twice.
    expect(map.q("[data-mission-banner-sent]")?.textContent).toBe("Wait for the API PR to merge· Conductor is reading");
    expect(map.q("[data-mission-option]")).toBeNull();
    // A working Conductor reads it on its next turn.
    await map.rerender(asked({ answer: { key: 3, text: "Wait for the API PR to merge", option: 2, state: "waiting" } }));
    expect(map.q("[data-mission-banner-sent]")?.getAttribute("data-mission-banner-sent")).toBe("waiting");
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Sent · waiting for its turn");
    await act(async () => map.q("[data-mission-conversation]")!.click());
    expect(map.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "p1" } });
    // A new question asks again; the answer to the old one is not shown on it (B29).
    await map.rerender(asked({ decision: "Merge order?", question: { key: 4, why: null, recommend: null, node: null, asked_at_unix_ms: Date.now() } }));
    expect(map.q("[data-mission-banner-sent]")).toBeNull();
    expect(map.q("[data-mission-question]")?.textContent).toBe("Merge order?");
    expect(map.q("[data-mission-option]")).toBeNull();
    expect(map.q("[data-mission-write]")).not.toBeNull();
    // A cleared decision takes the banner away.
    await map.rerender(asked({ decision: null, question: null }));
    expect(map.q("[data-mission-banner]")).toBeNull();
  } finally { await map.unmount(); }
});

it("writes an answer in the banner: Enter sends it, Escape closes it and gives focus back, and the field stops at its cap (B27)", async () => {
  const map = await mount({ items: [asked()] });
  try {
    const write = map.q("[data-mission-write]")!;
    write.focus();
    await act(async () => write.click());
    const field = map.q("[data-mission-answer-input]") as HTMLInputElement;
    expect(document.activeElement).toBe(field);
    expect(field.maxLength).toBe(500);
    await type(field, "Stack it");
    expect(map.q("[data-mission-answer-left]")?.textContent).toBe("492 left");
    await press(field, "Escape");
    expect(map.q("[data-mission-answer-input]")).toBeNull();
    expect(document.activeElement).toBe(map.q("[data-mission-write]"));
    await act(async () => map.q("[data-mission-write]")!.click());
    const again = map.q("[data-mission-answer-input]") as HTMLInputElement;
    // What was written stays across a close; an empty answer sends nothing.
    expect(again.value).toBe("Stack it");
    await type(again, "   ");
    await press(again, "Enter");
    expect(map.events.some((event) => event.kind === "mission_answer")).toBe(false);
    await type(again, "  Stack it on the API PR  ");
    await press(again, "Enter");
    expect(map.events.filter((event) => event.kind === "mission_answer").map((event) => event.payload)).toEqual([{ mission: "mission-1", key: 3, text: "Stack it on the API PR" }]);
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Stack it on the API PR");
  } finally { await map.unmount(); }
});

it("keeps an answer Herdr may have typed as unconfirmed and types it again only when the operator sends it again, once (B26)", async () => {
  const unconfirmed = asked({ answer: { key: 3, text: "Stack on the API PR", option: 1, state: "unconfirmed" } });
  const map = await mount({ items: [unconfirmed] });
  try {
    const row = map.q('[data-mission-banner-sent="unconfirmed"]')!;
    expect(row.textContent).toContain("Stack on the API PR");
    expect(row.textContent).toContain("Delivery not confirmed");
    // Nothing else answers the question while it may already be in the conversation.
    expect(map.q("[data-mission-option]")).toBeNull();
    await act(async () => map.q("[data-mission-resend]")!.click());
    expect(map.events.filter((event) => event.kind === "mission_answer").map((event) => event.payload)).toEqual([{ mission: "mission-1", key: 3, resend: true }]);
    // Pressed, it reads as sending and offers no second press.
    expect(map.q("[data-mission-banner-sent]")?.getAttribute("data-mission-banner-sent")).toBe("sending");
    expect(map.q("[data-mission-resend]")).toBeNull();
    await map.rerender(asked({ answer: { key: 3, text: "Stack on the API PR", option: 1, state: "delivered" } }));
    expect(map.q("[data-mission-banner-sent]")?.getAttribute("data-mission-banner-sent")).toBe("delivered");
    // With no Conductor there, sending again waits.
    await map.rerender({ ...unconfirmed, conductor: { ...unconfirmed.conductor, present: false } });
    expect((map.q("[data-mission-resend]") as HTMLButtonElement).disabled).toBe(true);
  } finally { await map.unmount(); }
});

it("turns the choices off with a reason when no Conductor or no device can read the answer, and says why an answer did not arrive (B28, B39)", async () => {
  const gone = await mount({ items: [asked({ conductor: { ...mission().conductor, present: false }, answer_failed: "conductor_gone" })] });
  try {
    const banner = gone.q('[data-mission-banner="decision"]')!;
    expect([...banner.querySelectorAll<HTMLButtonElement>("[data-mission-option], [data-mission-write]")].every((button) => button.disabled)).toBe(true);
    expect(banner.querySelector("[data-mission-conversation]")).toBeNull();
    expect(banner.querySelector("[data-mission-banner-reason]")?.textContent).toBe("Not sent · no Conductor");
    await act(async () => (banner.querySelector("[data-mission-takeover]") as HTMLElement).click());
    expect(String((gone.events.find((event) => event.kind === "agent_start_in_checkout")?.payload as { prompt: string }).prompt)).toContain("hide mission take mission-1");
  } finally { await gone.unmount(); }
  const offline = await mount({ items: [asked({ device_id: "mini", stale: true })] });
  try {
    expect([...offline.container.querySelectorAll<HTMLButtonElement>("[data-mission-option]")].every((button) => button.disabled)).toBe(true);
    expect(offline.q("[data-mission-banner-reason]")?.textContent).toBe("mini is not responding · answer once it is back");
    // The wait is not counted while the device is silent (B35).
    expect(offline.q("[data-mission-banner-waited]")).toBeNull();
  } finally { await offline.unmount(); }
  // An agent the doorbell does not ring takes its answer in the conversation only.
  const unsupported = await mount({ items: [asked({ conductor: { ...mission().conductor, agent_kind: "opencode", takes_answers: false } })] });
  try {
    expect([...unsupported.container.querySelectorAll<HTMLButtonElement>("[data-mission-option], [data-mission-write]")].every((button) => button.disabled)).toBe(true);
    expect(unsupported.q("[data-mission-conversation]")).not.toBeNull();
    expect(unsupported.q("[data-mission-banner-reason]")?.getAttribute("data-mission-banner-reason")).toBe("unsupported");
  } finally { await unsupported.unmount(); }
  const failed = await mount({ items: [asked({ answer_failed: "send_failed" })] });
  try {
    // A failed send goes back to before it was sent, with what to do.
    expect([...failed.container.querySelectorAll<HTMLButtonElement>("[data-mission-option]")].some((button) => button.disabled)).toBe(false);
    expect(failed.q("[data-mission-banner-reason]")?.textContent).toBe("Could not send · answer again");
  } finally { await failed.unmount(); }
});

it("answers a decision without choices by writing, as one from before choices is (B25)", async () => {
  const map = await mount({ items: [mission({ decision: "Pick the screen PR's approach", nodes: [node("a", { status: "held" })] })] });
  try {
    expect(map.q("[data-mission-option]")).toBeNull();
    expect(map.q("[data-mission-banner-node]")).toBeNull();
    expect(map.q('[data-mission-node="a"]')?.getAttribute("aria-label")).toContain("Pick the screen PR's approach");
    await act(async () => map.q("[data-mission-write]")!.click());
    const field = map.q("[data-mission-answer-input]") as HTMLInputElement;
    await type(field, "Stack it");
    await press(field, "Enter");
    expect(map.events.find((event) => event.kind === "mission_answer")?.payload).toEqual({ mission: "mission-1", key: 0, text: "Stack it" });
  } finally { await map.unmount(); }
});

it("offers Take over in 내 차례 when no Conductor holds an open mission, which starts an agent asked to take it", async () => {
  const gone = mission({ conductor: { ...mission().conductor, present: false }, nodes: [node("a")] });
  const map = await mount({ items: [gone] });
  try {
    expect(map.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toBe("takeover");
    expect(map.q('[data-mission-turn-item="takeover"]')?.textContent).toContain("No Conductor");
    await act(async () => map.q("[data-mission-takeover]")!.click());
    const start = map.events.find((event) => event.kind === "agent_start_in_checkout");
    expect(start?.payload).toMatchObject({ checkout_path: "/repo", provider: "claude" });
    expect(String((start?.payload as { prompt: string }).prompt)).toContain("hide mission take mission-1");
  } finally { await map.unmount(); }
  const done = await mount({ items: [{ ...gone, status: "done" }] });
  try {
    expect(done.q("[data-mission-turn]")).toBeNull();
  } finally { await done.unmount(); }
});

it("says the record failed instead of a number, and names a gone mission", async () => {
  const failed = await mount({ items: [], failed: "unreadable" });
  try {
    expect(failed.q('[data-mission-state="unreadable"]')?.textContent).toBe("The mission record could not be read");
    expect(failed.q("[data-mission-summary]")).toBeNull();
  } finally { await failed.unmount(); }
  const gone = await mount({ items: [] });
  try {
    expect(gone.q('[data-mission-state="gone"]')).not.toBeNull();
  } finally { await gone.unmount(); }
});

it("dims a mission whose device does not answer and says whose last values it shows", async () => {
  const map = await mount({ items: [mission({ device_id: "mini", stale: true, nodes: [node("a", { status: "working" })] })] });
  try {
    expect(map.q("[data-mission]")?.dataset.missionStale).toBe("true");
    expect(map.q("[data-mission-stale-note]")?.textContent).toBe("mini is not responding · last values");
    expect(map.q("[data-mission-values]")?.className).toContain("opacity-(--opacity-dimmed)");
  } finally { await map.unmount(); }
});

const line = (key: string, kind: MissionCriterion["kind"], extra: Partial<MissionCriterion> = {}): MissionCriterion => ({ key, kind, state: "open", ...extra });
const checker = (agent: string, state: MissionCriterion["state"] = "open", agent_id?: string) => ({ worker: { agent, model: null, effort: "high" }, state, ...(agent_id ? { agent_id } : {}) });
const card = (map: { q: (selector: string) => HTMLElement | null }, key: string) => map.q(`[data-mission-node="${key}"]`)!;

it("draws each card's state as its mark and border, colour only for what wants a look, with its line, result, landing tag and criteria (B26-B28)", async () => {
  const nodes = [
    node("core", { title: "제안 core: 기록·공책·점수·반영", status: "review", pr: 1703, pr_state: "mergeable", criteria: [line("c1", "own", { state: "pass" }), line("check", "check", { state: "pass" }), line("review", "review", { checkers: [checker("codex", "open", "agent-7")] }), line("ci", "ci")], criteria_met: 2, criteria_skipped: 0 }),
    node("engine", { status: "working", depends_on: ["core"], pr: 1704, line: "replay 19/31", reopened: 1, worker: { agent: "claude", model: "opus", effort: "xhigh", effort_auto: false }, pane_id: "w1:p2", criteria: [line("c1", "own", { state: "pass" }), line("c2", "own", { state: "skipped", skip: { reason: "only 31 sessions" } }), line("c3", "own"), line("ci", "ci")], criteria_met: 2, criteria_skipped: 1 }),
    node("tool", { status: "held", depends_on: ["core"] }),
    node("api", { status: "done", pr: 1650, pr_state: "mergeable", worker: { agent: "codex", model: "gpt-6", effort: "high", effort_auto: false } }),
    node("old", { status: "done", pr: 1600, pr_state: "merged" }),
    node("branch", { status: "done", delivery: "direct", branch: "feat/a-very-long-branch-name-for-the-screen-node" }),
    node("broken", { status: "failed", line: "CI failed twice" }),
    node("outside", { title: "바깥: isolated 앱 실행기 정리", kind: "external", delivery: null, external: { pr: 1232 }, status: "waiting" }),
    node("unread", { kind: "external", delivery: null, external: { branch: "tools/runner" }, status: "waiting", unread: true }),
    node("merged-out", { kind: "external", delivery: null, external: { pr: 1702 }, status: "done" }),
    node("closed-out", { kind: "external", delivery: null, external: { pr: 1605 }, status: "failed" }),
    node("v1", { status: "done", pr: 1500 }),
    node("next", { depends_on: ["engine", "outside"] }),
  ];
  // The engine's worker pane was started with another effort than its plan asked.
  const launched = [{ device_id: "mac", pane_id: "w1:p2", session_digest: "e", launch_model: "opus", launch_effort: "high" }];
  const map = await mount({ items: [mission({ nodes, decision: "Rail or slot?", question: { key: 1, why: null, recommend: null, node: "tool", asked_at_unix_ms: Date.now() } })] }, [], launched);
  try {
    expect(Object.fromEntries(nodes.map((n) => [n.key, card(map, n.key).dataset.nodeLook]))).toEqual({ core: "review", engine: "working", tool: "hold", api: "done", old: "landed", branch: "done", broken: "failed", outside: "external", unread: "external", "merged-out": "landed", "closed-out": "failed", v1: "done", next: "waiting" });
    // Each mark says its state on hover and to a screen reader, and the card's name starts with the same words.
    expect(card(map, "tool").querySelector("[data-node-mark]")?.getAttribute("aria-label")).toBe("Waiting for a decision");
    expect(card(map, "tool").getAttribute("aria-label")).toBe("Node tool · Waiting for a decision · Rail or slot?");
    expect(card(map, "old").querySelector("[data-node-mark]")?.getAttribute("aria-label")).toBe("In main");
    // Colour belongs to work, review, a decision and a failure; done is grey and in main has no surface.
    expect(card(map, "engine").className).toContain("border-mission-working");
    expect(card(map, "core").className).toContain("ring-mission-review/20");
    expect(card(map, "api").className).toContain("border-border");
    expect(card(map, "api").className).not.toMatch(/mission-done/);
    expect(card(map, "old").className).toContain("bg-transparent");
    expect(card(map, "outside").className).toContain("border-dotted");
    // A pull request's number is plain on a card; its state's colours are the panel's.
    expect(card(map, "core").querySelector("[data-node-pr]")?.textContent).toBe(`${"#"}1703`);
    expect(card(map, "core").querySelector("[data-pr-mark]")).toBeNull();
    expect(card(map, "core").querySelector("[data-node-criteria]")?.textContent).toBe("criteria 2/4");
    // n is the ✓ lines alone; the skipped one stands beside it.
    expect(card(map, "engine").querySelector("[data-node-criteria]")?.textContent).toBe("criteria 1/4⤳1");
    expect(card(map, "engine").getAttribute("aria-label")).toContain("criteria 1/4 · 1 skipped");
    expect(card(map, "engine").querySelector("[data-node-reopened]")?.textContent).toBe("↻1");
    expect(card(map, "engine").querySelector("[data-node-line]")?.textContent).toBe("replay 19/31");
    // The review card says what is left; the waiting one what it waits for, by number.
    expect(card(map, "core").querySelector("[data-node-line]")?.textContent).toBe("left: review, CI");
    expect(card(map, "next").querySelector("[data-node-line]")?.textContent).toBe(`waiting for ${"#"}1704 바깥: isolated 앱 실행기 정리`);
    expect(card(map, "api").querySelector("[data-node-landing]")?.textContent).toBe("Merge left");
    // A done branch the core reads and has not found in main is left to join; a PR whose state was never read says nothing.
    expect(card(map, "branch").querySelector("[data-node-landing]")?.textContent).toBe("Join left");
    expect(card(map, "v1").querySelector("[data-node-landing]")).toBeNull();
    expect(card(map, "old").querySelector("[data-node-landing]")).toBeNull();
    expect(card(map, "broken").querySelector('[data-node-line="attention"]')?.textContent).toBe("CI failed twice");
    expect(card(map, "outside").querySelector("[data-node-pr]")?.textContent).toBe(`${"#"}1232`);
    expect(card(map, "unread").querySelector("[data-node-line]")?.textContent).toBe("Could not check");
    // Who works a card leads its bottom row, as a checker's row names it: the launch, else the plan marked planned; an outside node has none.
    const who = (key: string) => {
      const row = card(map, key).querySelector<HTMLElement>("[data-node-worker]");
      return row ? [row.dataset.nodeWorker, row.querySelector("[data-run]")?.getAttribute("data-run"), row.querySelector("[data-run]")?.textContent, row.parentElement?.firstElementChild === row] : null;
    };
    expect([who("engine"), who("api"), who("outside")]).toEqual([["claude", "launch", "opus·high", true], ["codex", "plan", "gpt-6·high (planned)", true], null]);
    expect(card(map, "engine").getAttribute("aria-label")).toContain("Working · opus·high");
    // Its words give way first when the row runs short; the count keeps its width.
    expect(card(map, "engine").querySelector("[data-node-worker] [data-run]")?.className).toContain("truncate");
    expect(card(map, "engine").querySelector("[data-node-criteria]")?.className).toContain("shrink-0");
    // Outside is the node's kind: a dotted border and ↗ in every state, its state shown on them (B19).
    const outside = (key: string) => [card(map, key).className.includes("border-dotted"), card(map, key).querySelector("[data-node-outside]")?.textContent, card(map, key).querySelector("[data-node-line]")?.textContent];
    expect(["outside", "unread", "merged-out", "closed-out"].map(outside)).toEqual([[true, `↗${"#"}1232`, "waiting for main"], [true, "↗tools/runner", "Could not check"], [true, `↗${"#"}1702`, "In main"], [true, `↗${"#"}1605`, "Closed without merge"]]);
    expect(card(map, "closed-out").className).toContain("border-mission-failed");
    expect(card(map, "merged-out").getAttribute("aria-label")).toBe("Node merged-out · In main · Outside the mission");
    // A long title wraps to two lines and a long branch truncates inside the card (B34).
    expect(card(map, "core").querySelector("[title='제안 core: 기록·공책·점수·반영']")?.className).toContain("line-clamp-2");
    expect(card(map, "branch").querySelector("[data-node-branch] .truncate")).not.toBeNull();
  } finally { await map.unmount(); }
});

it("glues the other agents checking a node under its card, each with its face, the model and effort its pane was started with or the plan's marked planned, and the rule when several decide (B29)", async () => {
  const nodes = [
    node("core", { status: "review", criteria: [line("review", "review", { checkers: [{ ...checker("codex", "open", "agent-7"), pane_id: "w1:p4" }] })] }),
    node("all", { depends_on: ["core"], kind: "verify", delivery: null, criteria: [line("verify", "verify", { rule: "all", checkers: [checker("claude", "pass", "agent-8"), checker("codex", "pass", "agent-9"), checker("gemini")] })] }),
    node("alone", { status: "working", criteria: [line("c1", "own")] }),
  ];
  // The reviewer's pane was started with another effort than the plan asked; the launch is what shows.
  const map = await mount({ items: [mission({ nodes })] }, [], [{ device_id: "mac", pane_id: "w1:p4", session_digest: "r", launch_model: "gpt-6", launch_effort: "xhigh" }]);
  try {
    const rows = (key: string) => [...card(map, key).querySelectorAll<HTMLElement>("[data-node-checker]")].map((row) => [row.dataset.nodeChecker, row.dataset.checkerState, row.querySelector("[data-run]")?.getAttribute("data-run"), row.querySelector("[data-run]")?.getAttribute("title"), row.lastElementChild?.textContent]);
    expect(rows("core")).toEqual([["codex", "running", "launch", "gpt-6·xhigh", "reviewing"]]);
    expect(rows("all")).toEqual([["claude", "pass", "plan", "high (planned)", "✓ passed"], ["codex", "pass", "plan", "high (planned)", "✓ passed"], ["gemini", "waiting", "plan", "high (planned)", "verification to start"]]);
    expect(card(map, "all").querySelector("[data-node-check-rule]")?.textContent).toBe("all 3 pass · 2/3 passed");
    // A node checked by no other agent has no strip.
    expect(card(map, "alone").querySelector("[data-node-checkers]")).toBeNull();
  } finally { await map.unmount(); }
});

it("shows how long a node took and how long a decision held it from its events, and no number without them or while the device is gone (B28, B35)", async () => {
  // The page's clock moves by whole minutes.
  const now = Math.floor(Date.now() / 60_000) * 60_000;
  const at = (minutes: number) => now - minutes * 60_000;
  const nodes = [node("a", { status: "done", pr: 3, pr_state: "merged" }), node("b", { status: "held" }), node("c", { status: "working" })];
  const move = (minutes: number, key: string, status: MissionNode["status"]): MissionEvent => ({ at_unix_ms: at(minutes), kind: "node", node: key, status });
  const events = [move(90, "a", "working"), move(30, "a", "done"), move(40, "b", "working"), move(25, "b", "held")];
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    expect(card(map, "a").querySelector("[data-node-time]")?.textContent).toBe("1h");
    expect(card(map, "b").querySelector('[data-node-time="held"]')?.textContent).toBe("held 25m");
    // A node with no event that moved it draws no time.
    expect(card(map, "c").querySelector("[data-node-time]")).toBeNull();
  } finally { await map.unmount(); }
  const stale = await mount({ items: [mission({ nodes, device_id: "mini", stale: true })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    // The finished node's time stays; the running ones stop, and so does the head's.
    expect(card(stale, "a").querySelector("[data-node-time]")?.textContent).toBe("1h");
    expect(card(stale, "b").querySelector("[data-node-time]")).toBeNull();
    expect(stale.q('[data-mission-stat="time"]')).toBeNull();
  } finally { await stale.unmount(); }
});

it("lights a picked card and the cards one edge away and dims the rest, and the same card, an empty spot or Escape lets go (B30)", async () => {
  const nodes = [node("a"), node("b", { depends_on: ["a"] }), node("c", { depends_on: ["b"] }), node("d", { depends_on: ["a"] })];
  const map = await mount({ items: [mission({ nodes })] });
  try {
    const dimmed = () => [...map.container.querySelectorAll<HTMLElement>("[data-node-dimmed]")].map((el) => el.dataset.missionNode ?? "report");
    await act(async () => card(map, "b").click());
    expect(card(map, "b").getAttribute("aria-pressed")).toBe("true");
    expect(dimmed()).toEqual(["d"]);
    expect(map.q('[data-mission-report][class*="opacity-(--opacity-dimmed)"]')).not.toBeNull();
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("b");
    // Edges not touching the pick dim with the cards.
    expect([...map.container.querySelectorAll<SVGPathElement>("[data-edge-dimmed]")].map((edge) => edge.dataset.dependencyEdge).sort()).toEqual(["a>d", "c>#report", "d>#report"]);
    await act(async () => card(map, "b").click());
    expect(dimmed()).toEqual([]);
    expect(map.q("[data-mission-panel]")).toBeNull();
    await act(async () => card(map, "c").click());
    await act(async () => map.q("[data-mission-map]")!.click());
    expect(map.q("[data-mission-panel]")).toBeNull();
    await act(async () => card(map, "c").click());
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => { card(map, "c").dispatchEvent(escape); });
    expect(escape.defaultPrevented).toBe(true);
    expect(map.q("[data-mission-panel]")).toBeNull();
    // With nothing picked, Escape is the lane's.
    const again = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => { card(map, "c").dispatchEvent(again); });
    expect(again.defaultPrevented).toBe(false);
  } finally { await map.unmount(); }
});

it("opens the picked node's panel: its head, where it is, what it made, its finish lines as one list, its connections that pick, its ways out and its folded events (B31-B33)", async () => {
  const plan = {
    pane_id: "p2", session: { device_id: "mac", agent: "codex", session_id: "worker-session" },
    steps: [{ key: "parse", title: "Command parser", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false }],
    review: "approved" as const, ask: null, progress: { current: "parse", next: null, done: 0, total: 1, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: Date.now(),
  };
  const nodes = [
    node("core", { title: "Core", pr: 1703, pr_state: "merged", status: "done" }),
    node("engine", {
      title: "조사 에이전트: Hide가 실행하는 명령 읽기", status: "working", depends_on: ["core"], pr: 1704, base_pr: 1703, pr_state: "mergeable", pr_url: "https://github.com/o/r/pull/1704", branch: "hide-ai-engine", plan, attempts: 2,
      worker: { agent: "claude", model: "opus", effort: "xhigh", effort_auto: false },
      criteria: [line("c1", "own", { text: "명령 읽기가 cat/rg 잡음을 걸러낸다", state: "pass" }), line("c2", "own", { text: "실제 세션 50개 replay", state: "skipped", skip: { reason: "세션이 31개뿐" } }), line("check", "check", { state: "fail" }), line("ci", "ci", { state: "pass", reruns: 1 }), line("review", "review", { checkers: [checker("codex")] })],
      criteria_met: 3, criteria_skipped: 1,
    }),
    node("verify", { title: "세 PR 함께 확인", kind: "verify", delivery: null, depends_on: ["engine"] }),
  ];
  const minute = Math.floor(Date.now() / 60_000) * 60_000;
  const events: MissionEvent[] = [{ at_unix_ms: minute - 60_000, kind: "node", node: "engine", status: "working" }, { at_unix_ms: minute - 30_000, kind: "pr_opened", node: "engine", pr: 1704 }];
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    await act(async () => card(map, "engine").click());
    const panel = map.q('[data-mission-detail="engine"]')!;
    expect([...panel.querySelectorAll<HTMLElement>("[data-panel-part]")].map((part) => part.dataset.panelPart)).toEqual(["head", "now", "result", "criteria", "links", "ways", "events"]);
    expect(panel.querySelector("[data-panel-title]")?.textContent).toBe("조사 에이전트: Hide가 실행하는 명령 읽기");
    // No launch recorded for its pane: the plan's model and effort, marked planned.
    expect(panel.querySelector("[data-panel-meta] [data-run]")?.getAttribute("title")).toBe("opus·xhigh (planned)");
    expect([panel.querySelector("[data-panel-meta] [data-run-words]")?.textContent, panel.querySelector("[data-panel-meta] [data-run-planned]")?.textContent]).toEqual(["opus·xhigh", "(planned)"]);
    expect(panel.querySelector("[data-panel-facts]")?.textContent).toBe("took 1m · Attempt 2");
    expect([...panel.querySelectorAll<HTMLElement>("[data-plan-row]")].map((row) => row.dataset.planRow)).toEqual(["parse"]);
    // What it made: the PR in its state, the base it stacks on, the CI line and the branch.
    expect(panel.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark")).toBe("mergeable");
    expect(panel.querySelector("[data-panel-base]")?.textContent).toBe(`on ${"#"}1703`);
    expect(panel.querySelector('[data-panel-part="result"]')?.textContent).toContain("✓rerun 1 time");
    expect(panel.querySelector("[data-panel-branch]")?.textContent).toBe("hide-ai-engine");
    // One list of ✓ ○ ⤳ ✕, a skipped line with its reason, the review by its checker's name.
    const lines = [...panel.querySelectorAll<HTMLElement>("[data-panel-line]")].map((row) => row.textContent);
    expect(lines).toEqual(["✓명령 읽기가 cat/rg 잡음을 걸러낸다", "⤳실제 세션 50개 replay세션이 31개뿐", "✕Check command passes", "✓CI passesrerun 1 time", "○codex review"]);
    // The CI result and every line name their state with one mark's word.
    const named = (scope: string) => [...panel.querySelectorAll(`${scope} [data-criterion-mark]`)].map((mark) => mark.getAttribute("aria-label"));
    expect(named('[data-panel-part="result"]')).toEqual(["passed"]);
    expect(named("[data-panel-line]")).toEqual(["passed", "skipped", "failed", "passed", "not yet"]);
    expect(panel.querySelector('[data-panel-part="criteria"] h4')?.textContent).toBe("Done when · 2/5");
    expect(panel.querySelector('[data-panel-links="before"]')?.textContent).toBe(`Waits onCore${"#"}1703`);
    // A connection picks its node.
    await act(async () => (panel.querySelector('[data-panel-link="verify"]') as HTMLElement).click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("verify");
    await act(async () => (map.q('[data-panel-link="engine"]') as HTMLElement).click());
    const back = map.q('[data-mission-detail="engine"]')!;
    expect(back.querySelector("[data-node-pr-card]")).not.toBeNull();
    expect(back.querySelector("[data-node-terminal]")).not.toBeNull();
    expect(back.querySelector("[data-node-record]")).not.toBeNull();
    expect(back.querySelector("[data-panel-events-toggle]")?.textContent).toBe("2 events");
    expect(back.querySelector("[data-panel-event]")).toBeNull();
    await act(async () => (back.querySelector("[data-panel-events-toggle]") as HTMLElement).click());
    expect([...back.querySelectorAll<HTMLElement>("[data-panel-event]")].map((row) => row.dataset.panelEvent)).toEqual(["node", "pr_opened"]);
    await act(async () => (back.querySelector("[data-panel-close]") as HTMLElement).click());
    expect(map.q("[data-mission-panel]")).toBeNull();
  } finally { await map.unmount(); }
});

it("puts a held node's question and choices in its panel, read only, and the worker's plan line on a working card (B31; PRD hide-plan B16)", async () => {
  const own = {
    pane_id: "p2", session: { device_id: "mac", agent: "codex", session_id: "worker-session" },
    steps: [{ key: "core", title: "Core events", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false }, { key: "web", title: "Web map", depends_on: ["core"], status: "planned" as const, line: null, output: "pr" as const, changed: false, change: null, waiting: true }],
    review: "approved" as const, ask: null, progress: { current: "core", next: "web", done: 0, total: 2, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: Date.now(),
  };
  const map = await mount({ items: [asked({ nodes: [node("api", { status: "working", line: "Conductor's line", plan: own }), node("screen", { title: "API screen", status: "held" })] })] });
  try {
    expect(card(map, "api").querySelector("[data-node-line]")?.textContent).toBe("Now: Core events (0/2)");
    await act(async () => card(map, "screen").click());
    const panel = map.q('[data-mission-detail="screen"]')!;
    expect(panel.querySelector("[data-panel-question]")?.textContent).toBe("Which base for the screen PR?");
    expect(panel.querySelector("[data-panel-options]")?.textContent).toBe("Stack on the API PRRecommendedWait for the API PR to merge");
    expect(panel.querySelector("button[data-mission-option]")).toBeNull();
  } finally { await map.unmount(); }
});

it("lists only what the operator can do in 내 차례: the decision whose node chip picks the node, a worker the core raised, an outside PR closed unmerged, the merge order (B18, B20, B25)", async () => {
  const raised = { pane_id: "p2", escalation: { cause: "child_blocked", letter_id: null, since_unix_ms: 1, human_notice: true }, request: { verb: "blocked", verb_since_unix_ms: 1, line: "Login expired", end: "blocked", request: null, later_by: null, pull_requests: [] } } as unknown as AgentRow;
  const nodes = [node("api", { status: "working", pane_id: "p2" }), node("outside", { kind: "external", delivery: null, external: { pr: 1232 }, status: "failed" })];
  const map = await mount({ items: [asked({ nodes: [...nodes, node("screen", { title: "API screen", status: "held" })] })] }, [], [], {}, [raised]);
  try {
    expect(map.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toBe("decision worker_blocked outside_closed");
    expect(map.q("[data-mission-turn] h3")?.textContent).toBe("Your turn 3");
    expect(map.q('[data-mission-turn-item="worker_blocked"]')?.textContent).toContain("worker stuck: Login expired");
    expect(map.q('[data-mission-turn-item="outside_closed"]')?.textContent).toContain("Closed without merge");
    // The node chip of an item picks its node on the map.
    await act(async () => map.q("[data-mission-banner-node]")!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("screen");
    await act(async () => map.q('[data-mission-turn-node="outside"]')!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("outside");
  } finally { await map.unmount(); }
  const prs = [node("ui", { title: "UI", status: "done", pr: 1705, pr_state: "mergeable", base_pr: 1703, depends_on: ["core"] }), node("core", { title: "Core", status: "done", pr: 1703, pr_state: "mergeable" })];
  const waiting = await mount({ items: [mission({ status: "merge_wait", nodes: prs, merge_order: ["core", "ui"] })] });
  try {
    expect(waiting.q('[data-mission-turn-item="merge"] [data-mission-landing-order]')?.getAttribute("data-mission-landing-order")).toBe("core ui");
    expect(waiting.q('[data-mission-turn-item="merge"]')?.textContent).toContain(`Merge orderCore${"#"}1703→UI${"#"}1705`);
    // The stack is the purple dashed edge, and nothing else marks it.
    expect(waiting.q('[data-dependency-edge="core>ui"]')?.getAttribute("class")).toContain("text-mission-review");
    expect(waiting.q('[data-dependency-edge="core>ui"]')?.hasAttribute("data-edge-dashed")).toBe(true);
  } finally { await waiting.unmount(); }
  // Waiting on its merges, a git mission tags the branch the core reads and has not found in main; a branch it cannot read says nothing.
  const branches = [node("notes", { title: "Notes", status: "done", delivery: "direct", branch: "release/notes" }), node("bump", { title: "Bump", status: "done", delivery: "direct", branch: "release/bump", unread: true })];
  const git = await mount({ items: [mission({ status: "merge_wait", nodes: branches })] });
  try {
    expect(card(git, "notes").querySelector("[data-node-landing]")?.textContent).toBe("Join left");
    expect(card(git, "bump").querySelector("[data-node-landing]")).toBeNull();
    expect(git.q("[data-mission-landing-order]")?.getAttribute("data-mission-landing-order")).toBe("notes");
  } finally { await git.unmount(); }
});

it("draws a defect a later node found as a red dashed edge back to the node it opened, until its line passes (B21)", async () => {
  const defect = (state: MissionCriterion["state"]) => line("d1", "defect", { text: "the filter resets on reload", found_by: "verify", state });
  const nodes = (state: MissionCriterion["state"]) => [node("api", { status: "working", reopened: 1, criteria: [defect(state)], criteria_met: 0 }), node("verify", { kind: "verify", delivery: null, depends_on: ["api"], status: "held" })];
  const map = await mount({ items: [mission({ nodes: nodes("open") })] });
  try {
    const edge = map.q('[data-dependency-edge="verify~api"]');
    expect(edge?.getAttribute("class")).toContain("text-mission-failed");
    expect(edge?.hasAttribute("data-edge-dashed")).toBe(true);
    await act(async () => card(map, "api").click());
    expect(map.q('[data-panel-line="d1"]')?.textContent).toBe("○the filter resets on reloadfound by Node verify");
    await map.rerender(mission({ nodes: nodes("pass") }));
    expect(map.q('[data-dependency-edge="verify~api"]')).toBeNull();
  } finally { await map.unmount(); }
});

it("shows a mission from before criteria with its old sentence and no count (B23)", async () => {
  const map = await mount({ items: [mission({ exempt: true, nodes: [node("a", { criteria: [line("c1", "own", { text: "the lens keeps its filter" })] }), node("b")] })] });
  try {
    expect(map.container.querySelector("[data-node-criteria]")).toBeNull();
    expect(card(map, "a").querySelector("[data-node-old-criteria]")?.textContent).toBe("the lens keeps its filter");
    await act(async () => card(map, "a").click());
    expect(map.q("[data-panel-old-criteria]")?.textContent).toBe("the lens keeps its filter");
    expect(map.q("[data-panel-line]")).toBeNull();
  } finally { await map.unmount(); }
});

it("folds a finished mission's report into one line under the head that opens to all of it, and rests a cancelled mission's unfinished cards (B24, B35)", async () => {
  const report = "Released 1.0.0.\nNotes published.";
  const done = await mount({ items: [mission({ status: "done", report, nodes: [node("a", { status: "done", pr: 1, pr_state: "merged" })], started_at_unix_ms: 0, closed_at_unix_ms: 3 * 3_600_000 })] });
  try {
    expect(done.q("[data-mission-pill]")?.textContent).toBe("Finished");
    expect(done.q('[data-mission-stat="time"]')?.textContent).toBe("took 3h");
    expect(done.q("[data-mission-report-text] p")?.textContent).toBe("Released 1.0.0.");
    await act(async () => done.q("[data-mission-report-toggle]")!.click());
    expect(done.q("[data-mission-report-text] p")?.textContent).toBe(report);
    expect(done.q("[data-mission-turn]")).toBeNull();
  } finally { await done.unmount(); }
  const gone = { ...mission().conductor, present: false };
  const cancelled = await mount({ items: [mission({ status: "cancelled", conductor: gone, decision: "Keep going?", nodes: [node("a", { status: "done", pr: 1503, pr_state: "merged" }), node("b", { status: "working", depends_on: ["a"] })] })] });
  try {
    expect([card(cancelled, "a").dataset.nodeLook, card(cancelled, "b").dataset.nodeLook]).toEqual(["landed", "cancelled"]);
    expect(card(cancelled, "b").className).toContain("border-dashed");
    expect(cancelled.container.querySelector(".text-mission-working")).toBeNull();
    expect(cancelled.q("[data-mission-turn]")).toBeNull();
  } finally { await cancelled.unmount(); }
});
