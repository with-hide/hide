// @vitest-environment jsdom
// A mission's detail in the 보기 slot (PRD software-factory-conductor B15-B21,
// B43; PRD mission-plan-visibility B12-B30, B39): what it shows in each state
// of the record and its events, and the one event each press sends.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MissionMap } from "./MissionMap";
import type { Mission, MissionEvent, MissionNode, SnapshotRest } from "./snapshot";
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

async function mount(missions: { items: Mission[]; failed?: string }, remote: unknown[] = [], sessions: unknown[] = [], more: Partial<SnapshotRest> = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  initializeInterfaceI18n("en");
  useShellStore.setState({
    agents: [],
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

it("shows one planning line before a plan, and a head of state, time, a cell per node, n / m done and a legend of the states with nodes in it (B12, B13)", async () => {
  const empty = await mount({ items: [mission({ status: "planning", started_at_unix_ms: null })] });
  try {
    expect(empty.q("[data-mission-planning]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-pill]")?.textContent).toBe("Planning");
    expect(empty.q("[data-mission-banner]")).toBeNull();
    expect(empty.q("[data-mission-flow]")).toBeNull();
  } finally { await empty.unmount(); }
  const nodes = [node("a", { status: "done" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { status: "review", depends_on: ["a"] }), node("d", { status: "held" }), node("e", { depends_on: ["b"] }), node("f", { status: "dropped" })];
  const running = await mount({ items: [mission({ nodes })] });
  try {
    expect(running.q("[data-mission-pill]")?.textContent).toBe("Running");
    expect(running.q('[data-mission-stat="time"]')?.textContent).toBe("3m so far");
    expect(running.q('[data-mission-stat="done"]')?.textContent).toBe("1 / 5 done");
    expect(running.q("[data-mission-summary] [data-progress-cells]")?.getAttribute("data-progress-cells")).toBe("done hold working review waiting");
    expect(running.q("[data-mission-legend]")?.getAttribute("data-mission-legend")).toBe("done:1 review:1 working:1 hold:1 waiting:1");
    expect(running.q("[data-mission-legend]")?.textContent).toBe("Done1Review1Working1Decision1Waiting1");
    // No PR count, no cap: the head says the state, the time and the cells (B13).
    expect(running.q("[data-mission-summary]")?.textContent).not.toContain("/4");
    expect(running.container.querySelectorAll("[data-mission-node]")).toHaveLength(6);
    expect(running.q("[data-mission-report]")).not.toBeNull();
    // One page: the head, the map, then the flow table.
    const order = [...running.container.querySelectorAll("[data-mission-summary], [data-mission-fit], [data-mission-flow]")].map((part) => (part as HTMLElement).dataset.missionSummary ? "head" : (part as HTMLElement).dataset.missionFit ? "map" : "flow");
    expect(order).toEqual(["head", "map", "flow"]);
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

it("offers Take over when no Conductor holds an open mission, which starts an agent asked to take it", async () => {
  const gone = mission({ conductor: { ...mission().conductor, present: false }, nodes: [node("a")] });
  const map = await mount({ items: [gone] });
  try {
    expect(map.q('[data-mission-banner="no-conductor"]')?.textContent).toContain("No Conductor");
    await act(async () => map.q("[data-mission-takeover]")!.click());
    const start = map.events.find((event) => event.kind === "agent_start_in_checkout");
    expect(start?.payload).toMatchObject({ checkout_path: "/repo", provider: "claude" });
    expect(String((start?.payload as { prompt: string }).prompt)).toContain("hide mission take mission-1");
  } finally { await map.unmount(); }
  const done = await mount({ items: [{ ...gone, status: "done" }] });
  try {
    expect(done.q("[data-mission-banner]")).toBeNull();
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

it("draws a card's state as its whole border, with its PR mark, the base of its stack, its steps, badges and the model its pane runs, and a research node with its conclusion and no steps (B14-B16, B22)", async () => {
  const nodes = [
    node("api", { pr: 1503, pr_state: "mergeable", stage: 3, status: "review", pane_id: "p9", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false } }),
    node("screen", { pr: 1504, base_pr: 1503, pr_state: "failed", stage: 2, status: "failed", depends_on: ["api"], line: "CI failed · attempt 2/2", attempts: 2 }),
    node("cause", { kind: "research", delivery: null, status: "done", line: "The cache was stale", worker: { agent: "opencode", effort_auto: false } }),
    node("together", { kind: "verify", delivery: null, status: "planned", depends_on: ["screen"], worker: { agent: "codex", model: "gpt-6-astra", effort_auto: true } }),
  ];
  const map = await mount({ items: [mission({ nodes })] }, [], [{ device_id: "mac", pane_id: "p9", session_digest: "d", launch_model: "sonnet", launch_effort: null }]);
  try {
    const screen = map.q('[data-mission-node="screen"]')!;
    // The border is the node's state; the PR's own state is only its mark's colour, and no card has a band.
    expect(screen.dataset.nodeTone).toBe("failed");
    expect(screen.className).toContain("border-mission-failed");
    expect(map.container.querySelector('[data-mission-node][class*="border-l-"]')).toBeNull();
    expect(map.q('[data-mission-node="api"]')?.className).toContain("ring-mission-review/20");
    expect(map.q('[data-mission-node="together"]')?.className).toContain("border-dashed");
    expect(screen.querySelector("[data-node-attempts]")?.textContent).toBe("↻2");
    expect(screen.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark")).toBe("failed");
    expect(screen.querySelector("[data-node-base]")?.textContent).toContain(`${"#"}1503`);
    expect(screen.querySelector("[data-node-steps]")?.getAttribute("data-node-steps")).toBe("2");
    // The worker line is the model the pane runs, never the agent's name; a planned node shows the plan's.
    expect(map.q('[data-mission-node="api"] [data-node-worker]')?.getAttribute("data-node-worker")).toBe("sonnet·high");
    expect(map.q('[data-mission-node="together"] [data-node-worker]')?.getAttribute("data-node-worker")).toBe("gpt-6-astra·auto");
    expect(map.q('[data-mission-node="cause"] [data-node-worker]')).toBeNull();
    const cause = map.q('[data-mission-node="cause"]')!;
    expect(cause.querySelector("[data-pr-mark]")).toBeNull();
    expect(cause.querySelector("[data-node-steps]")).toBeNull();
    expect(map.q('[data-mission-node="together"] [data-node-steps]')).toBeNull();
    expect(cause.querySelector("[data-node-conclusion=folded]")?.getAttribute("title")).toBe("The cache was stale");
    expect(cause.className).toContain("border-mission-done/60");
    // A planned node waiting on an earlier one reads its kind: the edge says what it waits for.
    expect(map.q('[data-mission-node="together"] [data-node-line]')?.textContent).toBe("Verify together");
    // Pressing a card opens its row in the flow table, not a popover (B20).
    await act(async () => screen.click());
    const detail = map.q('[data-flow-row="screen"] [data-mission-detail="screen"]')!;
    expect(detail.textContent).toContain("Attempts2");
    expect(detail.querySelector<HTMLButtonElement>("[data-node-terminal]")?.disabled).toBe(true);
    expect(detail.querySelector<HTMLButtonElement>("[data-node-pr-card]")?.disabled).toBe(true);
    expect(map.q('[data-flow-row="cause"] [data-node-pr-card]')).toBeNull();
  } finally { await map.unmount(); }
});

it("marks a held card with !, says every badge in the card's name, and draws a research or verify line as a result only once the node ended (B16, B22, B36)", async () => {
  const nodes = [
    node("base", { status: "held" }),
    node("retry", { status: "working", attempts: 3, change: "Split from api" }),
    node("reading", { kind: "research", delivery: null, status: "working", line: "Reading 3 of 5 logs" }),
    node("checking", { kind: "verify", delivery: null, status: "working", line: "4 of 6 so far" }),
    node("cause", { kind: "research", delivery: null, status: "done", line: "The cache was stale" }),
    node("broken", { status: "failed", pr: 1600, pr_state: "failed", line: "CI failed · attempt 2/2" }),
    node("queued", { status: "waiting", line: "Waits on the API merge" }),
  ];
  const map = await mount({ items: [mission({ nodes, decision: "Which base?" })] });
  try {
    const card = (key: string) => map.q(`[data-mission-node="${key}"]`)!;
    expect(card("base").querySelector("[data-node-held]")?.textContent).toBe("!");
    expect(card("base").getAttribute("aria-label")).toBe("Node base · Held · Which base?");
    expect(card("retry").querySelector("[data-node-held]")).toBeNull();
    expect(card("retry").getAttribute("aria-label")).toBe("Node retry · Working · Attempt 3 · Changed by a replan · Split from api");
    // A running research or verify node's line is where it is, said once, never a result.
    for (const key of ["reading", "checking"]) {
      expect(card(key).querySelector("[data-node-conclusion], [data-node-verify]")).toBeNull();
      expect(card(key).querySelectorAll("[data-node-now]")).toHaveLength(1);
    }
    expect(card("cause").querySelector("[data-node-conclusion=folded]")).not.toBeNull();
    // A done research node's row writes its conclusion out under its own label, not under Now.
    await act(async () => card("cause").click());
    const detail = map.q('[data-mission-detail="cause"]')!;
    expect(detail.querySelector("[data-flow-result=conclusion]")?.textContent).toBe("The cache was stale");
    expect(detail.textContent).toContain("ConclusionThe cache was stale");
    expect(detail.textContent).not.toContain("Now");
    await act(async () => card("reading").click());
    expect(map.q('[data-mission-detail="reading"]')?.textContent).toContain("NowReading 3 of 5 logs");
    expect(map.q('[data-mission-detail="reading"] [data-flow-result]')).toBeNull();
    // A failed or waiting node's row still says why, though its card shows its PR instead.
    await act(async () => card("broken").click());
    expect(map.q('[data-mission-detail="broken"]')?.textContent).toContain("NowCI failed · attempt 2/2");
    await act(async () => card("queued").click());
    expect(map.q('[data-mission-detail="queued"]')?.textContent).toContain("NowWaits on the API merge");
  } finally { await map.unmount(); }
  // A cancelled mission's work stopped: no held mark, no line of where it is.
  const cancelled = await mount({ items: [mission({ nodes: [node("base", { status: "held" }), node("run", { status: "working", line: "Writing tests" })], status: "cancelled", closed_at_unix_ms: Date.now(), decision: "Which base?" })] });
  try {
    expect(cancelled.q('[data-mission-node="base"] [data-node-held]')).toBeNull();
    expect(cancelled.q('[data-mission-node="base"]')?.getAttribute("aria-label")).toBe("Node base · Held");
    expect(cancelled.q('[data-mission-node="run"] [data-node-now]')).toBeNull();
    await act(async () => cancelled.q('[data-mission-node="run"]')!.click());
    expect(cancelled.q('[data-mission-detail="run"]')?.textContent).not.toContain("Writing tests");
  } finally { await cancelled.unmount(); }
});

it("puts the worker's own plan on the card's last line and folded in the node's detail, else the Conductor's line (PRD hide-plan B15, B16)", async () => {
  const own = {
    pane_id: "p2", session: { device_id: "mac", agent: "codex", session_id: "worker-session" },
    steps: [
      { key: "core", title: "Core events", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false },
      { key: "web", title: "Web map", depends_on: ["core"], status: "planned" as const, line: null, output: "pr" as const, changed: false, change: null, waiting: true },
    ],
    review: "approved" as const, ask: null, progress: { current: "core", next: "web", done: 0, total: 2, finished: false },
    created_at_unix_ms: 1, updated_at_unix_ms: Date.now() - 12 * 60_000,
  };
  const map = await mount({ items: [mission({ nodes: [
    node("a", { status: "working", branch: "gen-prd/a", stage: 1, line: "Conductor's line", plan: own }),
    node("b", { status: "working", branch: "gen-prd/b", stage: 1, line: "Conductor's line" }),
  ] })] });
  try {
    expect(map.q('[data-mission-node="a"] [data-node-now]')?.textContent).toBe("Now: Core events (0/2)");
    expect(map.q('[data-mission-node="b"] [data-node-now]')?.textContent).toBe("Conductor's line");
    await act(async () => map.q('[data-mission-node="a"]')!.click());
    const detail = map.q('[data-mission-detail="a"]')!;
    expect(detail.querySelector("[data-node-step-names]")?.textContent).toBe("Code → PR → Review → Merge");
    expect(detail.querySelector("[data-node-plan]")?.getAttribute("data-node-plan")).toBe("folded");
    expect(detail.querySelector("[data-node-plan-toggle]")?.textContent).toBe("Own plan· updated 12m ago");
    // Opening it lists the steps, never a map (PRD mission-plan-visibility B20, D-07).
    await act(async () => detail.querySelector<HTMLButtonElement>("[data-node-plan-toggle]")!.click());
    expect([...detail.querySelectorAll<HTMLElement>("[data-plan-row]")].map((step) => step.dataset.planRow)).toEqual(["core", "web"]);
    expect(detail.querySelector("[data-plan-graph]")).toBeNull();
  } finally { await map.unmount(); }
});

it("draws every node, done ones too, and outlines the ones at work", async () => {
  const nodes = Array.from({ length: 14 }, (_, index) => node(`n-${index}`, { status: index < 6 ? "done" : index < 9 ? "working" : "planned" }));
  const map = await mount({ items: [mission({ nodes })] });
  try {
    expect(map.container.querySelectorAll("[data-mission-node]")).toHaveLength(14);
    expect([...map.container.querySelectorAll<HTMLElement>("[data-node-moving]")].map((card) => card.dataset.missionNode)).toEqual(["n-6", "n-7", "n-8"]);
  } finally { await map.unmount(); }
});

/** A CSS length as the box style writes it. */
const px = (value: number) => `${value}px`;

it("shrinks a map wider than its slot to the slot's width, and 100% draws it at its own size", async () => {
  const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(1800);
  const height = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(600);
  const room = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(900);
  const map = await mount({ items: [mission({ nodes: [node("a", { status: "working" }), node("b")] })] });
  try {
    const fit = map.q("[data-mission-fit]")!;
    expect(fit.dataset.missionScale).toBe("0.50");
    // The box holds the shrunk size, so the slot does not scroll sideways.
    expect((fit.firstElementChild as HTMLElement).style.width).toBe(px(900));
    expect((fit.firstElementChild as HTMLElement).style.height).toBe(px(300));
    await act(async () => map.q('[data-mission-zoom-choice="actual"]')!.click());
    expect(map.q("[data-mission-fit]")?.dataset.missionScale).toBe("1.00");
    expect((map.q("[data-mission-fit]")!.firstElementChild as HTMLElement).style.width).toBe(px(1800));
  } finally {
    await map.unmount();
    width.mockRestore();
    height.mockRestore();
    room.mockRestore();
  }
});

it("lists a row per node in order on the mission's time axis, filters it, and a card pressed on the map opens its row whatever the filter hid (B17, B19, B20)", async () => {
  // An open mission's axis runs to the end of the current minute; the clock is held on a minute so the bars measure exactly.
  vi.useFakeTimers({ toFake: ["Date"], now: 1_800_000_000_000 });
  const start = Date.now() + 60_000 - 100 * 60_000;
  const at = (minutes: number) => start + minutes * 60_000;
  const nodes = [
    node("ui", { status: "working", depends_on: ["api"], started_at_unix_ms: at(50) }),
    node("api", { status: "done", started_at_unix_ms: at(0), finished_at_unix_ms: at(50), pr: 1503, pr_state: "merged" }),
    node("cause", { kind: "research", delivery: null, status: "done", started_at_unix_ms: at(0), finished_at_unix_ms: at(25), line: "The cache was stale" }),
    node("fix", { delivery: "direct", status: "planned", depends_on: ["cause"], branch: "fix/cache" }),
    node("check", { kind: "verify", delivery: null, status: "planned", depends_on: ["ui"] }),
  ];
  const map = await mount({ items: [mission({ nodes, started_at_unix_ms: start })] });
  try {
    const rows = () => [...map.container.querySelectorAll<HTMLElement>("[data-flow-row]")].map((row) => row.dataset.flowRow);
    expect(rows()).toEqual(["api", "cause", "ui", "fix", "check"]);
    // The kind in one word beside the title, a PR change saying none.
    expect(map.q('[data-flow-row="cause"] button')?.textContent).toContain("Research");
    expect(map.q('[data-flow-row="fix"] button')?.textContent).toContain("Direct");
    expect(map.q('[data-flow-row="check"] button')?.textContent).toContain("Verify");
    // One bar from start to finish, or to now while it runs, and none before it starts.
    expect(map.q('[data-flow-row="api"] [data-flow-bar]')?.getAttribute("data-flow-bar")).toBe("0.0 50.0");
    expect(map.q('[data-flow-row="ui"] [data-flow-bar]')?.getAttribute("data-flow-bar")).toBe("50.0 50.0");
    expect(map.q('[data-flow-row="fix"] [data-flow-bar]')).toBeNull();
    expect(map.q("[data-mission-flow-axis]")?.getAttribute("data-mission-flow-axis")?.split("|").at(-1)).toBe("now");
    // A mission recorded before its events opens a row to its times, with no list of events.
    await act(async () => map.q('[data-flow-row="api"] button')!.click());
    expect(map.q('[data-mission-detail="api"] [data-flow-times]')?.textContent).toMatch(/^Started .+Finished .+$/);
    await act(async () => map.q('[data-mission-flow-choice="now"]')!.click());
    expect(rows()).toEqual(["ui", "fix", "check"]);
    await act(async () => map.q('[data-mission-flow-choice="finished"]')!.click());
    expect(rows()).toEqual(["api", "cause"]);
    await act(async () => map.q('[data-mission-node="ui"]')!.click());
    expect(map.q("[data-mission-flow]")?.dataset.missionFlow).toBe("all");
    expect(map.q('[data-flow-row="ui"] button')?.getAttribute("aria-expanded")).toBe("true");
    expect(map.q('[data-mission-node="ui"]')?.getAttribute("aria-pressed")).toBe("true");
  } finally { await map.unmount(); }
});

it("puts a finished mission's report and results first, each result by its kind and no PR number on a mission that had none (B22, B23)", async () => {
  const nodes = [
    node("rules", { kind: "research", delivery: null, status: "done", line: "close 12 of 19" }),
    node("bump", { delivery: "direct", status: "done", branch: "release/1.0.0", depends_on: ["rules"] }),
    node("agents", { kind: "verify", delivery: null, status: "done", line: "7 of 7 pass", depends_on: ["bump"] }),
  ];
  const report = "Released 1.0.0.\nNotes published.\nSite deployed.\nFloor raised.";
  const map = await mount({ items: [mission({ status: "done", nodes, report, started_at_unix_ms: 0, closed_at_unix_ms: 3 * 3_600_000 })] });
  try {
    expect(map.q("[data-mission-pill]")?.textContent).toBe("Finished");
    expect(map.q('[data-mission-stat="time"]')?.textContent).toBe("took 3h");
    const order = [...map.container.querySelectorAll("[data-mission-summary], [data-mission-report-text], [data-mission-results], [data-mission-fit], [data-mission-flow]")].map((part) => Object.keys((part as HTMLElement).dataset)[0]);
    expect(order).toEqual(["missionSummary", "missionReportText", "missionResults", "missionFit", "missionFlow"]);
    // A long report shows its first lines until 펼치기.
    expect(map.q("[data-mission-report-text] p")?.textContent).toBe("Released 1.0.0.\nNotes published.\nSite deployed.");
    await act(async () => map.q("[data-mission-report-toggle]")!.click());
    expect(map.q("[data-mission-report-text] p")?.textContent).toBe(report);
    const results = [...map.container.querySelectorAll<HTMLElement>("[data-mission-result-row]")].map((row) => [...row.querySelectorAll("td")].map((cell) => cell.textContent));
    expect(results).toEqual([["Node rules", "Research", "Conclusion: close 12 of 19"], ["Node bump", "Direct", "release/1.0.0"], ["Node agents", "Verify", "7 of 7 pass"]]);
    expect(map.container.querySelector("[data-pr-mark], [data-node-pr]")).toBeNull();
    expect(map.container.textContent).not.toContain("#");
  } finally { await map.unmount(); }
});

it("draws a cancelled mission in grey, its unfinished nodes dashed grey, with no banner to act on (B24)", async () => {
  const gone = { ...mission().conductor, present: false };
  const nodes = [node("a", { status: "done", pr: 1503, pr_state: "merged" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { depends_on: ["b"] })];
  const map = await mount({ items: [mission({ status: "cancelled", conductor: gone, decision: "Keep going?", nodes, started_at_unix_ms: 0, closed_at_unix_ms: 3_600_000 })] });
  try {
    expect(map.q("[data-mission-pill]")?.textContent).toBe("Cancelled");
    expect(map.q("[data-mission-pill]")?.className).toContain("text-mission-cancelled");
    expect([...map.container.querySelectorAll<HTMLElement>("[data-mission-node]")].map((card) => card.dataset.nodeTone)).toEqual(["done", "cancelled", "cancelled"]);
    expect(map.q('[data-mission-node="b"]')?.className).toContain("border-dashed");
    // Its work stopped: the unfinished node rests, and no line from it is lit.
    expect(map.q('[data-mission-node="b"] [data-mark]')?.getAttribute("data-mark")).toBe("quiet");
    expect(map.container.querySelector(".text-mission-working")).toBeNull();
    expect(map.q("[data-mission-legend]")?.getAttribute("data-mission-legend")).toBe("done:1 cancelled:2");
    expect(map.q("[data-mission-banner]")).toBeNull();
    expect(map.q('[data-mission-result-row="a"] [data-pr-mark=merged]')).not.toBeNull();
  } finally { await map.unmount(); }
});

it("draws each node's bar from its events: worked, review, a failed attempt and a stop in their colours, its merges and decisions marked, the working end live, and the events in its row (B18, B20, B21)", async () => {
  const start = Date.now() - 60 * 60_000;
  const at = (minutes: number) => start + minutes * 60_000;
  const nodes = [
    node("a", { status: "done", started_at_unix_ms: at(10), finished_at_unix_ms: at(40), pr: 1, pr_state: "merged" }),
    node("b", { status: "working", started_at_unix_ms: at(20), attempts: 2 }),
    node("c", { status: "working", started_at_unix_ms: at(5) }),
    // Worked before its events were kept; only its end was recorded.
    node("d", { status: "done", started_at_unix_ms: at(2), finished_at_unix_ms: at(41) }),
  ];
  const events: MissionEvent[] = [
    { at_unix_ms: at(0), kind: "planned", count: 3 },
    { at_unix_ms: at(10), kind: "node", node: "a", status: "working" },
    { at_unix_ms: at(15), kind: "replanned", count: 2, line: "Split the API" },
    { at_unix_ms: at(20), kind: "node", node: "b", status: "working" },
    { at_unix_ms: at(25), kind: "node", node: "a", status: "review" },
    { at_unix_ms: at(30), kind: "node", node: "b", status: "failed", line: "CI failed" },
    { at_unix_ms: at(32), kind: "retried", node: "b", count: 2 },
    { at_unix_ms: at(33), kind: "node", node: "b", status: "working" },
    { at_unix_ms: at(35), kind: "asked", node: "b", line: "Keep the old flag?", options: ["Keep", "Drop"] },
    { at_unix_ms: at(40), kind: "node", node: "a", status: "done" },
    { at_unix_ms: at(40), kind: "pr_merged", node: "a", pr: 1 },
    { at_unix_ms: at(41), kind: "node", node: "d", status: "done" },
  ];
  const map = await mount({ items: [mission({ nodes, started_at_unix_ms: start })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 3 } });
  try {
    const track = (key: string) => map.q(`[data-flow-track="${key}"]`)!;
    const tones = (key: string) => [...track(key).querySelectorAll("[data-flow-segment]")].map((segment) => segment.getAttribute("data-flow-segment"));
    expect(tones("a")).toEqual(["worked", "review"]);
    expect(tones("b")).toEqual(["failed", "paused", "worked"]);
    expect(track("a").querySelector("[data-flow-live]")).toBeNull();
    expect(track("b").querySelector("[data-flow-live]")).not.toBeNull();
    expect([...track("a").querySelectorAll("[data-flow-mark]")].map((mark) => mark.textContent)).toEqual(["⑂"]);
    expect([...track("b").querySelectorAll("[data-flow-mark]")].map((mark) => mark.textContent)).toEqual(["!"]);
    expect(map.q("[data-flow-axis-marks]")?.getAttribute("data-flow-axis-marks")).toBe("replanned");
    // A node with no event that draws a stretch keeps the bar its times draw (B19).
    expect(track("c").getAttribute("data-flow-drawn")).toBe("times");
    expect(track("c").querySelector("[data-flow-bar]")).not.toBeNull();
    expect(track("d").getAttribute("data-flow-drawn")).toBe("times");
    expect(map.q("[data-mission-flow-dropped]")?.textContent).toBe("3 earlier events omitted");
    await act(async () => (map.q('[data-flow-row="a"] button') as HTMLElement).click());
    const detail = map.q('[data-mission-detail="a"]')!;
    expect([...detail.querySelectorAll("[data-flow-event]")].map((row) => row.getAttribute("data-flow-event"))).toEqual(["node", "node", "node", "pr_merged"]);
    expect(detail.textContent).toContain("PR #1 merged");
    expect(detail.querySelector("[data-flow-times]")).toBeNull();
    await act(async () => (map.q('[data-flow-row="b"] button') as HTMLElement).click());
    const b = map.q('[data-mission-detail="b"]')!;
    expect(b.textContent).toContain("FailedCI failed");
    expect(b.textContent).toContain("Attempt 2");
    expect(b.textContent).toContain("Decision askedKeep the old flag? / Keep / Drop");
  } finally { await map.unmount(); }
  // Another mission's events draw nothing here.
  const other = await mount({ items: [mission({ nodes, started_at_unix_ms: start })] }, [], [], { mission_events: { mission: "mission-2", events, dropped: 0 } });
  try {
    expect(other.q('[data-flow-track="a"]')?.getAttribute("data-flow-drawn")).toBe("times");
    expect(other.q("[data-mission-flow-dropped]")).toBeNull();
  } finally { await other.unmount(); }
});

it("shows a direct change's commit on main and a verify node's verdict as their results, and a cancel's time (B22, B24)", async () => {
  const nodes = [
    node("fix", { delivery: "direct", status: "done", branch: "fix/cache", commit: "abc1234def5678" }),
    node("check", { kind: "verify", delivery: null, status: "failed", verdict: "fail", checks: { passed: 3, total: 5 }, line: "2 failed" }),
    node("pass", { kind: "verify", delivery: null, status: "done", verdict: "pass", checks: null }),
    node("late", { status: "working", started_at_unix_ms: Date.now() - 20 * 60_000 }),
  ];
  const closed = Date.now() - 5 * 60_000;
  const events: MissionEvent[] = [{ at_unix_ms: Date.now() - 20 * 60_000, kind: "node", node: "late", status: "working" }, { at_unix_ms: closed, kind: "cancelled" }];
  const map = await mount({ items: [mission({ status: "cancelled", nodes, closed_at_unix_ms: closed })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    const result = (key: string) => map.q(`[data-mission-result-row="${key}"]`)?.textContent;
    expect(result("fix")).toContain("→ main abc1234");
    expect(result("check")).toContain("FAIL 3/5");
    expect(result("pass")).toContain("PASS");
    expect(map.q("[data-node-commit]")?.getAttribute("title")).toBe("abc1234def5678");
    expect(map.q("[data-mission-cancelled-at]")?.textContent).toMatch(/^Cancelled /);
    expect(map.q("[data-flow-axis-marks]")?.getAttribute("data-flow-axis-marks")).toBe("cancelled");
    // A node the cancel stopped is grey, not the green of work, and nothing on it blinks (B24).
    expect(map.q('[data-flow-track="late"] [data-flow-segment]')?.className).toContain("bg-mission-cancelled");
    expect(map.q('[data-flow-track="late"] [data-flow-live]')).toBeNull();
  } finally { await map.unmount(); }
});
