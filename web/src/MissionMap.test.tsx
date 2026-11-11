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
import { PRESSED_MS } from "./missionAnswer";
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
  return { key, title: `Node ${key}`, kind: "change", depends_on: [], status: "planned", blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: [], criteria_met: 0, criteria_skipped: 0, steps: [], brief_bytes: null, external: null, unread: false, reopened: 0, started_at_unix_ms: null, finished_at_unix_ms: null, commit: null, landing: null, verdict: null, checks: null, attention: null, hide: null, ...extra, prs: extra.prs ?? (extra.pr != null ? [{ number: extra.pr, state: extra.pr_state ?? null, url: extra.pr_url ?? null }] : []) };
}

function mission(extra: Partial<Mission> = {}): Mission {
  return { id: "mission-1", title: "Fix the bug", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: Date.now() - 180_000, closed_at_unix_ms: null, running: 2, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra };
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
  const nodes = [node("a", { status: "done" }), node("b", { status: "working", depends_on: ["a"] }), node("c", { status: "working", depends_on: ["a"], attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }), node("d"), node("e", { depends_on: ["b"] }), node("f", { status: "dropped" })];
  const running = await mount({ items: [mission({ nodes })] });
  try {
    expect(running.q("[data-mission-pill]")?.textContent).toBe("Running");
    // The head starts with the mission's number, the same `M1` its list row and its Conductor's chip use.
    expect(running.q("[data-mission-summary]")?.firstElementChild?.getAttribute("data-mission-number")).toBe("M1");
    expect(running.q('[data-mission-stat="time"]')?.textContent).toBe("3m so far");
    expect(running.q('[data-mission-stat="done"]')?.textContent).toBe("1 / 5 done");
    // Its cells are drawn as the cards: done green, a node someone has to look at in the decision colour (PRD mission-node-simple B5).
    expect(running.q("[data-mission-summary] [data-progress-cells]")?.getAttribute("data-progress-cells")).toBe("done waiting working attention waiting");
    expect(running.q("[data-mission-summary] [data-progress-cells] span")?.className).toContain("bg-mission-done");
    expect(running.q("[data-mission-turn]")).toBeNull();
    expect(running.container.querySelectorAll("[data-mission-node]")).toHaveLength(6);
    expect(running.container.querySelector("[data-mission-legend], [data-mission-flow], [role=tablist], [data-node-steps], [data-node-base]")).toBeNull();
    // The head carries no zoom; the map's own box does.
    expect(running.q("[data-mission-summary] [data-mission-zoom]")).toBeNull();
    expect(running.q("[data-mission-map] [data-mission-zoom]")).not.toBeNull();
    // Nothing picked: no panel, the map has the whole width (B30, B31).
    const parts = () => [...running.container.querySelectorAll<HTMLElement>("[data-mission-summary], [data-mission-turn], [data-mission-map], [data-mission-panel]")].map((part) => Object.keys(part.dataset)[0]);
    expect(parts()).toEqual(["missionSummary", "missionMap"]);
    await running.rerender(mission({ nodes, decision: "Rail or slot?" }));
    expect(parts()).toEqual(["missionSummary", "missionTurn", "missionMap"]);
    await act(async () => running.q('[data-mission-node="b"]')!.click());
    expect(parts()).toEqual(["missionSummary", "missionTurn", "missionMap", "missionPanel"]);
  } finally { await running.unmount(); }
});

it("ends the head with where results go and one line of the options this mission applies, a changed value bright with its dot and nothing on it to press (PRD mission-destination B48, B53)", async () => {
  const changed = await mount({ items: [mission({ nodes: [node("a")], scope: "merge", strength: "thorough", destination: "release/1.2" })] });
  try {
    expect(changed.q("[data-mission-destination]")?.dataset.missionDestination).toBe("release/1.2");
    const line = changed.q("[data-mission-options]")!;
    expect([...line.querySelectorAll<HTMLElement>("[data-mission-head-option]")].map((option) => [option.textContent, option.dataset.missionHeadOptionChanged ?? null])).toEqual([
      ["Through the merge", "true"], ["Worktree cleanup", null], ["Thorough", "true"], ["4 at once", null],
    ]);
    expect(line.querySelectorAll("[data-mission-head-option-dot]")).toHaveLength(2);
    // Its accessible name says what changed, since the dot is only drawn.
    expect(line.getAttribute("aria-label")).toBe("This mission's options · Through the merge · Worktree cleanup · Thorough · 4 at once · changed for this mission: Through the merge, Thorough");
    // Nothing in the head changes it; its one control is the mission's ⋯ menu.
    expect([...line.closest("[data-mission-summary]")!.querySelectorAll<HTMLElement>("button, [role=button], select, input")].map((control) => control.dataset.missionMore)).toEqual(["mission-1"]);
  } finally { await changed.unmount(); }
  // A scope `pr` mission from before verification strength opens with who merges and names no strength; one with no code node names no destination.
  const before = await mount({ items: [mission({ nodes: [node("a", { kind: "research", delivery: null })], destination: "main" })] });
  try {
    expect(before.q("[data-mission-destination]")).toBeNull();
    expect([...before.container.querySelectorAll<HTMLElement>("[data-mission-head-option]")].map((option) => option.textContent)).toEqual(["Up to PRs · you merge them all", "Worktree cleanup", "4 at once"]);
    expect(before.q("[data-mission-head-option-changed]")).toBeNull();
  } finally { await before.unmount(); }
});

const asked = (extra: Partial<Mission> = {}): Mission => mission({
  decision: "Which base for the screen PR?",
  question: { key: 3, why: "Two PRs touch the same file", options: ["Stack on the API PR", "Wait for the API PR to merge"], recommend: 1, node: "screen", asked_at_unix_ms: Date.now() - 120_000 },
  nodes: [node("api", { status: "working" }), node("screen", { title: "API screen", status: "working" }), node("other", { status: "working" })],
  ...extra,
});

it("shows the decision with why, the node it stopped, its wait and the choices as radio rows, and sends a marked choice once on 답하기 (B25, B26, B30; PRD mission-human-gate B19)", async () => {
  const map = await mount({ items: [asked()] });
  try {
    const banner = map.q('[data-mission-banner="decision"]')!;
    expect(banner.querySelector("[data-mission-question]")?.textContent).toBe("Which base for the screen PR?");
    expect(banner.querySelector("[data-mission-banner-why]")?.textContent).toBe("Two PRs touch the same file");
    expect(banner.querySelector("[data-mission-banner-node]")?.textContent).toBe("API screen · awaiting decision");
    expect(banner.querySelector("[data-mission-banner-waited]")?.textContent).toBe("waiting 2m");
    expect([...banner.querySelectorAll("[data-mission-option]")].map((radio) => radio.closest("label")?.textContent)).toEqual(["Stack on the API PRRecommended", "Wait for the API PR to merge"]);
    expect(banner.querySelector("[data-mission-recommended]")?.getAttribute("data-mission-option")).toBe("1");
    // Nothing is marked yet, so 답하기 cannot send; a press on a row only marks it.
    const send = () => map.q("[data-mission-answer-send]") as HTMLButtonElement | null;
    expect(send()?.textContent).toBe("Answer");
    expect(send()?.disabled).toBe(true);
    await act(async () => (banner.querySelector('[data-mission-option="1"]') as HTMLElement).click());
    await act(async () => (banner.querySelector('[data-mission-option="2"]') as HTMLElement).click());
    expect(map.events.some((event) => event.kind === "mission_answer")).toBe(false);
    expect(banner.querySelector('[data-mission-option="2"]')?.getAttribute("aria-checked")).toBe("true");
    await act(async () => send()!.click());
    expect(map.events.filter((event) => event.kind === "mission_answer")).toEqual([{ schema_version: 2, kind: "mission_answer", payload: { mission: "mission-1", key: 3, option: 2 } }]);
    // Sent at once: the choice with a check, nothing left to press twice.
    expect(map.q("[data-mission-banner-sent]")?.textContent).toBe("Wait for the API PR to merge· Conductor is reading");
    expect(map.q("[data-mission-option]")).toBeNull();
    expect(send()).toBeNull();
    // A working Conductor reads it on its next turn.
    await map.rerender(asked({ answer: { key: 3, text: "Wait for the API PR to merge", option: 2, state: "waiting" } }));
    expect(map.q("[data-mission-banner-sent]")?.getAttribute("data-mission-banner-sent")).toBe("waiting");
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Sent · waiting for its turn");
    // The decision has no button of its own to the Conductor: the Conductor row above carries 터미널로 (PRD mission-tool B12).
    expect(map.q('[data-mission-banner="decision"] [data-mission-conversation]')).toBeNull();
    await act(async () => map.q("[data-mission-conductor-terminal]")!.click());
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

it("says what a waiting answer waits for and offers Send now only where pressing can lift it, once (B26, B28)", async () => {
  const answer = (hold?: "working" | "typing" | "recent" | "approval" | "other" | "forced") => asked({ answer: { key: 3, text: "Stack it", option: 1, state: "waiting", ...(hold ? { hold } : {}) } });
  const map = await mount({ items: [answer("working")] });
  try {
    const now = () => map.q("[data-mission-now]") as HTMLButtonElement | null;
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Sent · Conductor is working");
    await act(async () => now()!.click());
    expect(map.events.filter((event) => event.kind === "mission_answer").map((event) => event.payload)).toEqual([{ mission: "mission-1", key: 3, now: true }]);
    await map.rerender(answer("typing"));
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Sent · something is typed in its input");
    expect(now()).not.toBeNull();
    // The quiet period is not typing: its input is empty, and it still takes the press.
    await map.rerender(answer("recent"));
    expect(map.q("[data-mission-banner-sent]")?.textContent).toContain("Sent · there was activity a moment ago");
    expect(now()).not.toBeNull();
    // A menu of its own, a wait that ends on its own, an older core's answer and the press itself offer nothing to press.
    for (const [hold, words] of [["approval", "Sent · Conductor waits at a menu"], ["other", "Sent · waiting for its turn"], [undefined, "Sent · waiting for its turn"], ["forced", "Sending now"]] as const) {
      await map.rerender(answer(hold));
      expect(map.q("[data-mission-banner-sent]")?.textContent).toContain(words);
      expect(now()).toBeNull();
    }
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

it("draws a pressed choice as sent for five seconds and then the core's state again, when the core never answers for it (B26)", async () => {
  const map = await mount({ items: [asked()] });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    await act(async () => map.q('[data-mission-option="2"]')!.click());
    await act(async () => map.q("[data-mission-answer-send]")!.click());
    const sent = () => map.q("[data-mission-banner-sent]");
    expect(sent()?.getAttribute("data-mission-banner-sent")).toBe("sending");
    expect(sent()?.textContent).toContain("Wait for the API PR to merge");
    await act(async () => vi.advanceTimersByTime(PRESSED_MS - 1));
    expect(sent()?.getAttribute("data-mission-banner-sent")).toBe("sending");
    await act(async () => vi.advanceTimersByTime(1));
    expect(sent()).toBeNull();
    expect(map.q('[data-mission-option="2"]')).not.toBeNull();
  } finally {
    vi.useRealTimers();
    await map.unmount();
  }
});

it("leaves to 내 차례 the steps waiting on the operator, the decision's and a raised worker's, and lists a step waiting on the Conductor in progress, saying so (operator report on M41)", async () => {
  const raised = { pane_id: "p2", escalation: { cause: "child_blocked", letter_id: null, since_unix_ms: 1, human_notice: true }, request: { verb: "blocked", verb_since_unix_ms: 1, line: "Login expired", end: "blocked", request: null, later_by: null, pull_requests: [] } } as unknown as AgentRow;
  const nodes = [
    node("api", { status: "working", pane_id: "p2" }),
    node("screen", { title: "API screen", status: "working" }),
    node("waits", { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
    node("other", { status: "working" }),
  ];
  const map = await mount({ items: [asked({ nodes })] }, [], [], {}, [raised]);
  try {
    const row = (field: string) => [...map.container.querySelectorAll<HTMLElement>(`[data-mission-summary-row="${field}"] [data-mission-summary-node]`)].map((name) => name.dataset.missionSummaryNode);
    expect(map.q('[data-mission-summary-row="attention"]')).toBeNull();
    expect(row("working")).toEqual(["waits", "other"]);
    expect(map.q('[data-mission-summary-row="working"] dt')?.textContent).toBe("In progress 2");
    expect(map.q('[data-mission-summary-line="waits"]')?.textContent).toBe("Conductor · Worker ended with work left");
    expect(map.q('[data-mission-summary-line="waits"]')?.getAttribute("data-mission-summary-attention")).toBe("conductor");
    // The decision and the raised worker stand in 내 차례 above the map.
    expect(map.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toContain("worker_blocked");
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
    // The core takes it back to sending; once typed, the decision leaves the map with it.
    await map.rerender(asked({ answer: { key: 3, text: "Stack on the API PR", option: 1, state: "sending" } }));
    expect(map.q("[data-mission-banner-sent]")?.getAttribute("data-mission-banner-sent")).toBe("sending");
    // With no Conductor there, sending again waits.
    await map.rerender({ ...unconfirmed, conductor: { ...unconfirmed.conductor, present: false } });
    expect((map.q("[data-mission-resend]") as HTMLButtonElement).disabled).toBe(true);
  } finally { await map.unmount(); }
});

it("ends an open mission from its head's menu: asks a present Conductor once, cancels one no Conductor holds, after a confirm either way (UI_BEHAVIOR, A mission's map)", async () => {
  const openMenu = async (map: Awaited<ReturnType<typeof mount>>) => {
    await act(async () => map.q("[data-mission-more]")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    return document.querySelector<HTMLElement>('[role="menuitem"]')!;
  };
  const held = await mount({ items: [mission()] });
  try {
    const item = await openMenu(held);
    expect(item.textContent).toContain("End mission…");
    await act(async () => item.click());
    expect(document.querySelector("[data-mission-end-confirm]")?.getAttribute("data-mission-end-confirm")).toBe("ask");
    // Nothing is sent before the confirm.
    expect(held.events.some((event) => event.kind === "mission_end")).toBe(false);
    await act(async () => document.querySelector<HTMLElement>("[data-mission-end-action]")!.click());
    expect(held.events.filter((event) => event.kind === "mission_end")).toEqual([{ schema_version: 2, kind: "mission_end", payload: { mission: "mission-1" } }]);
    // Asked once: the head says so and the item stays off with its reason until the Conductor closes the mission.
    await held.rerender(mission({ end_asked: true }));
    expect(held.q("[data-mission-end-asked]")?.textContent).toBe("End asked · the Conductor is wrapping up");
    expect((await openMenu(held)).getAttribute("aria-disabled")).toBe("true");
  } finally { await held.unmount(); }
  const gone = await mount({ items: [mission({ end_asked: true, conductor: { ...mission().conductor, present: false } })] });
  try {
    // Once the Conductor is gone an earlier ask holds nothing: the press cancels the mission itself.
    expect(gone.q("[data-mission-end-asked]")).toBeNull();
    await act(async () => openMenu(gone).then((item) => item.click()));
    expect(document.querySelector("[data-mission-end-confirm]")?.getAttribute("data-mission-end-confirm")).toBe("cancel");
    expect(document.querySelector("[data-mission-end-confirm]")?.textContent).toContain("A cancelled mission does not reopen.");
    await act(async () => document.querySelector<HTMLElement>("[data-mission-end-keep]")!.click());
    expect(gone.events.some((event) => event.kind === "mission_end")).toBe(false);
  } finally { await gone.unmount(); }
  // A closed mission has nothing to end.
  const closed = await mount({ items: [mission({ status: "cancelled", closed_at_unix_ms: 2 })] });
  try {
    expect(closed.q("[data-mission-more]")).toBeNull();
  } finally { await closed.unmount(); }
});

it("turns the choices off with a reason when no Conductor or no device can read the answer, and says why an answer did not arrive (B28, B39)", async () => {
  const gone = await mount({ items: [asked({ conductor: { ...mission().conductor, present: false }, answer_failed: "conductor_gone" })] });
  try {
    const banner = gone.q('[data-mission-banner="decision"]')!;
    expect([...banner.querySelectorAll<HTMLButtonElement>("[data-mission-option], [data-mission-write]")].every((button) => button.disabled)).toBe(true);
    expect(banner.querySelector("[data-mission-conversation]")).toBeNull();
    expect(banner.querySelector("[data-mission-banner-reason]")?.textContent).toBe("Not sent · no Conductor");
    expect(banner.querySelector("[data-mission-takeover]")).toBeNull();
    await act(async () => gone.q("[data-mission-conductor] [data-mission-takeover]")!.click());
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
    expect(unsupported.q("[data-mission-conductor-terminal]")).not.toBeNull();
    expect(unsupported.q("[data-mission-banner-reason]")?.getAttribute("data-mission-banner-reason")).toBe("unsupported");
  } finally { await unsupported.unmount(); }
  const failed = await mount({ items: [asked({ answer_failed: "send_failed" })] });
  try {
    // A failed send goes back to before it was sent, with what to do.
    expect([...failed.container.querySelectorAll<HTMLButtonElement>("[data-mission-option]")].some((button) => button.disabled)).toBe(false);
    expect(failed.q("[data-mission-banner-reason]")?.textContent).toBe("Could not send · answer again");
  } finally { await failed.unmount(); }
  // An answer waiting on a Conductor session Hide cannot read says so instead of waiting for its turn.
  const unreadable = await mount({ items: [asked({ answer: { key: 3, text: "Wait for the API PR to merge", option: 2, state: "waiting" }, answer_failed: "session_unreadable" })] });
  try {
    expect(unreadable.q("[data-mission-banner-sent]")?.textContent).toContain("Wait for the API PR to merge");
    expect(unreadable.q("[data-mission-banner-reason]")?.textContent).toBe("· Waiting · Hide can't read the Conductor's session");
    expect(unreadable.q("[data-mission-option]")).toBeNull();
  } finally { await unreadable.unmount(); }
});

it("answers a decision without choices by writing, as one from before choices is (B25)", async () => {
  const map = await mount({ items: [mission({ decision: "Pick the screen PR's approach", nodes: [node("a", { status: "working" })] })] });
  try {
    expect(map.q("[data-mission-option]")).toBeNull();
    expect(map.q("[data-mission-banner-node]")).toBeNull();
    await act(async () => map.q("[data-mission-write]")!.click());
    const field = map.q("[data-mission-answer-input]") as HTMLInputElement;
    await type(field, "Stack it");
    await press(field, "Enter");
    expect(map.events.find((event) => event.kind === "mission_answer")?.payload).toEqual({ mission: "mission-1", key: 0, text: "Stack it" });
  } finally { await map.unmount(); }
});

it("offers Take over on the Conductor row when no Conductor holds an open mission, which starts an agent asked to take it (PRD mission-tool B13)", async () => {
  const gone = mission({ conductor: { ...mission().conductor, present: false }, nodes: [node("a")] });
  const map = await mount({ items: [gone] });
  try {
    expect(map.q("[data-mission-conductor]")?.getAttribute("data-mission-conductor")).toBe("gone");
    expect(map.q("[data-mission-conductor]")?.textContent).toContain("No Conductor");
    // The row carries it; 내 차례 does not say it twice.
    expect(map.q("[data-mission-turn]")).toBeNull();
    await act(async () => map.q("[data-mission-conductor] [data-mission-takeover]")!.click());
    const start = map.events.find((event) => event.kind === "agent_start_in_checkout");
    expect(start?.payload).toMatchObject({ checkout_path: "/repo", provider: "claude" });
    expect(String((start?.payload as { prompt: string }).prompt)).toContain("hide mission take mission-1");
  } finally { await map.unmount(); }
  const done = await mount({ items: [{ ...gone, status: "done" }] });
  try {
    expect(done.q("[data-mission-turn]")).toBeNull();
    expect(done.q("[data-mission-takeover]")).toBeNull();
  } finally { await done.unmount(); }
});

// Operator report on M41: 지휘 다시 열기 beside a Conductor at rest started a second Conductor; a raise for a live Conductor is handed back with Conductor에게 다시 맡기기 in 내 차례.
it("offers no 지휘 다시 열기 while a Conductor holds the mission, even once a raise it was asked first reached the operator", async () => {
  const conductor = [{ pane_id: "p1", mark: "quiet" } as AgentRow];
  const late = mission({ conductor: { ...mission().conductor, session: { device_id: "mac", agent: "claude", session_id: "s-1" } }, nodes: [node("a", { status: "working", pr: 7, attention: { reason: "pr_closed", pr: 7, to: "operator", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } })] });
  const map = await mount({ items: [late] }, [], [], {}, conductor);
  try {
    const row = map.q("[data-mission-conductor]")!;
    expect(row.dataset.missionConductor).toBe("resting");
    expect([...row.querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Go to terminal"]);
    expect(map.q("[data-mission-takeover]")).toBeNull();
    expect(map.q('[data-mission-turn-item="attention"] [data-attention-choice="leave"]')).not.toBeNull();
  } finally { await map.unmount(); }
});

it("names the Conductor under the head with its model and state, and 터미널로 brings its pane forward (PRD mission-tool B12)", async () => {
  const conductor = (mark: AgentRow["mark"]) => [{ pane_id: "p1", mark } as AgentRow];
  const launch = [{ device_id: "mac", pane_id: "p1", session_digest: "c", launch_model: "opus", launch_effort: "high" }];
  const map = await mount({ items: [mission({ nodes: [node("a")] })] }, [], launch, {}, conductor("going"));
  try {
    const row = map.q("[data-mission-conductor]")!;
    expect(row.dataset.missionConductor).toBe("working");
    expect(row.querySelector("[data-run]")?.textContent).toBe("opus·high");
    expect(row.querySelector("[data-mission-conductor-state]")?.textContent).toBe("Working");
    // One button, no record.
    expect([...row.querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Go to terminal"]);
    await act(async () => row.querySelector<HTMLElement>("[data-mission-conductor-terminal]")!.click());
    expect(map.events.find((event) => event.kind === "focus_pane")?.payload).toMatchObject({ pane_id: "p1" });
  } finally { await map.unmount(); }
  const words = async (extra: Partial<Mission>, agents: AgentRow[]) => {
    const one = await mount({ items: [mission({ nodes: [node("a")], ...extra })] }, [], [], {}, agents);
    try { return one.q("[data-mission-conductor]")?.dataset.missionConductor ?? null; } finally { await one.unmount(); }
  };
  expect(await words({ decision: "Which?" }, conductor("going"))).toBe("deciding");
  expect(await words({}, conductor("asleep"))).toBe("asleep");
  expect(await words({}, conductor("quiet"))).toBe("resting");
  // Held by the core but not listed: no state word is made up, the way to its terminal stays.
  expect(await words({}, [])).toBe("unknown");
  // A closed mission whose Conductor row is gone has nothing to bring forward.
  expect(await words({ status: "done" }, [])).toBeNull();
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

it("draws each card as its mark, its title with its worker at the row's right, its station line and `!` with who looks, Hide's mark when Hide changed it, and nothing else the panel carries (PRD mission-node-simple B1-B3, D-07; PRD mission-destination D-13, D-46, B24, B28, B30)", async () => {
  const nodes = [
    node("core", { title: "제안 core: 기록·공책·점수·반영", status: "working", pr: 1704, prs: [{ number: 1703, state: "merged", url: null }, { number: 1704, state: "mergeable", url: null }], criteria: [line("c1", "own", { state: "pass" }), line("ci", "ci")], criteria_met: 1, criteria_skipped: 0, worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false }, line: "replay 19/31", reopened: 1, change: "split in two" }),
    node("raised", { status: "working", pr: 1840, pr_state: "merged", attention: { reason: "merged_unproven", pr: 1840, to: "operator", since_unix_ms: 1, on_row: false, item: true, choices: ["leave", "done"] } }),
    node("asked", { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
    node("api", { status: "done", pr: 1650, pr_state: "merged", landing: { state: "landed", proof: { by: "merged" } }, hide: { action: "done", at_unix_ms: 1 }, criteria: [line("c1", "own", { state: "pass" })], criteria_met: 1, criteria_skipped: 0 }),
    node("branch", { status: "done", delivery: "direct", branch: "feat/a-very-long-branch-name-for-the-screen-node", landing: { state: "left", action: "join" } }),
    node("on-branch", { status: "working", branch: "feat/engine-own-layout", worker: { agent: "claude", model: "opus", effort: "xhigh", effort_auto: false } }),
    node("outside", { title: "바깥: isolated 앱 실행기 정리", kind: "external", delivery: null, external: { pr: 1232 }, outside: { base: "main", state: "pending" } }),
    node("closed-out", { kind: "external", delivery: null, external: { pr: 1605 }, outside: { base: "release/1.2", state: "closed" }, status: "working", attention: { reason: "pr_closed", pr: 1605, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
    node("merged-out", { kind: "external", delivery: null, external: { pr: 1702 }, outside: { base: "main", state: "merged" }, status: "done", landing: { state: "landed", proof: { by: "outside" } } }),
    node("branch-out", { kind: "external", delivery: null, external: { branch: "feat/outside-work" }, outside: { base: "develop" }, status: "working" }),
    node("next", { depends_on: ["core", "outside"] }),
    node("sleeping", { status: "working", agent_id: "agent-2", agent_runtime: "asleep" }),
    // Finished: one whose launch the core kept (its pane long gone), one with no launch kept and only the plan left.
    node("ran", { status: "done", agent_id: "agent-5", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false }, launched: { agent_id: "agent-5", agent: "codex", model: "Sonnet 5.5", effort: "low" } }),
    node("bound", { status: "working", agent_id: "agent-4", pane_id: "w1:p6", machine: "mac", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false } }),
    node("ran-unknown", { status: "done", agent_id: "agent-6", pane_id: "w1:p7", machine: "mac", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false } }),
    node("launched", { status: "working", agent_id: "agent-9", pane_id: "w1:p9", machine: "mac", launched: { agent_id: "agent-9", agent: "codex", model: "Opus 5.5", effort: "high" }, pr: 1910, pr_state: "pending", worker: { agent: "claude", model: "opus", effort: "medium", effort_auto: false }, attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
  ];
  // A launch the pane now shows for some session: a node names only the launch the core kept for its own worker, never its pane's.
  const launches = [{ device_id: "mac", pane_id: "w1:p6", session_digest: "other", launch_model: "Sonnet 5.5", launch_effort: "medium" }];
  const map = await mount({ items: [mission({ nodes, destination: "main" })] }, [], launches);
  try {
    expect(Object.fromEntries(nodes.map((n) => [n.key, card(map, n.key).dataset.nodeLook]))).toEqual({ core: "working", raised: "working", asked: "working", api: "done", branch: "done", outside: "external", "closed-out": "external", "merged-out": "done", "branch-out": "external", next: "waiting", sleeping: "working", ran: "done", bound: "working", "ran-unknown": "done", launched: "working", "on-branch": "working" });
    // One of three shapes, its word on hover and first in the card's name.
    expect(["next", "core", "api"].map((key) => card(map, key).querySelector("[data-node-mark]")?.getAttribute("aria-label"))).toEqual(["Waiting", "In progress", "Done"]);
    // A worker Hide put to sleep leaves its node in progress, the same ring; only its words name the sleep (B1).
    expect([card(map, "sleeping").dataset.nodeLook, card(map, "sleeping").querySelector("[data-node-mark]")?.getAttribute("aria-label")]).toEqual(["working", "In progress · Worker asleep"]);
    expect(card(map, "sleeping").querySelector("[data-node-mark] svg")?.getAttribute("class")).toContain("animate-spin");
    // The worker sits at the title row's right: its agent mark and model, the plan's marked planned, the launch's once kept (codex here, where the plan said claude); effort is the hover's (D-46).
    const worker = (key: string) => {
      const run = card(map, key).querySelector<HTMLElement>("[data-node-run]");
      return run ? [run.dataset.nodeRun, run.querySelector("[data-agent-mark]")?.getAttribute("data-agent-mark") ?? null, run.querySelector("[data-node-model]")?.textContent ?? null, run.querySelector("[data-run-planned]")?.textContent ?? null] : null;
    };
    expect(["core", "launched", "ran", "bound", "on-branch"].map(worker)).toEqual([
      ["plan", "claude", "opus", "(planned)"],
      ["launch", "codex", "Opus 5.5", null],
      ["launch", "codex", "Sonnet 5.5", null],
      ["plan", "claude", "opus", "(planned)"],
      ["plan", "claude", "opus", "(planned)"],
    ]);
    expect(card(map, "core").querySelector("[data-node-run]")?.parentElement).toBe(card(map, "core").querySelector("[data-node-mark]")?.parentElement);
    // A finished node with no launch kept, or a node with no worker, names no model and no mark.
    expect(["ran-unknown", "asked", "next", "outside"].map(worker)).toEqual([null, null, null, null]);
    // The last row is the station line: work, the pull request (the newest with +N), approval, and the branch it lands in (B24, B28).
    // An outside node has no work of the mission's: its pull request lit by the state GitHub answered, closed lighting nothing after it,
    // or its branch, then where that goes (D-14).
    const stations = (key: string) => card(map, key).querySelector<HTMLElement>("[data-node-row] [data-node-stations]")?.dataset.nodeStations ?? null;
    expect(Object.fromEntries(["core", "asked", "api", "branch", "outside", "closed-out", "merged-out", "branch-out"].map((key) => [key, stations(key)]))).toEqual({
      core: "work:done pr:current approve:todo land:todo",
      asked: "work:current pr:todo approve:todo land:todo",
      api: "work:done pr:done approve:done land:landed",
      branch: "work:done land:current",
      outside: "pr:current approve:todo land:todo",
      "closed-out": "pr:closed approve:todo land:todo",
      "merged-out": "pr:done approve:done land:landed",
      "branch-out": "branch:current land:todo",
    });
    // A base other than the destination ends in bold; the destination itself plainly.
    const last = (key: string) => [...card(map, key).querySelectorAll<HTMLElement>("[data-station-label]")].at(-1)!;
    expect(["outside", "closed-out", "branch-out"].map((key) => [last(key).textContent, last(key).className.includes("font-bold")])).toEqual([["main", false], ["release/1.2", true], ["develop", true]]);
    expect([...card(map, "core").querySelectorAll<HTMLElement>("[data-station-label]")].map((one) => one.textContent)).toEqual(["Work", `${"#"}1704 +1`, "Approval", "main"]);
    // `!` keeps the row's right end after the stations.
    expect((card(map, "asked").querySelector("[data-node-row]")!.lastElementChild as HTMLElement).dataset.nodeAttentionTag).toBe("worker_ended");
    // The stations are drawn for the eye; the card's name says them, each past `ahead` with its state, never only a colour (B28).
    expect(card(map, "core").getAttribute("aria-label")).toBe(`제안 core: 기록·공책·점수·반영 · In progress · opus·high (planned) · PR ${"#"}1703 · PR ${"#"}1704 · Where it goes: Work (done) → ${"#"}1704 +1 (now) → Approval → main`);
    expect(card(map, "launched").getAttribute("aria-label")).toMatch(new RegExp(`^Node launched · In progress · Opus 5\\.5·high · PR ${"#"}1910 · Where it goes: Work \\(done\\) → ${"#"}1910 \\(now\\) → Approval → main · Needs a look · Conductor · `));
    // The title is one line ending in an ellipsis with no native hover of its own: the whole of it is the card's hover and its name (B30).
    expect(card(map, "core").querySelector(".truncate")?.textContent).toBe("제안 core: 기록·공책·점수·반영");
    expect(card(map, "core").querySelector("[title='제안 core: 기록·공책·점수·반영']")).toBeNull();
    // The panel's head names the run as before: the launch kept, else nothing.
    await act(async () => card(map, "ran").click());
    expect([map.q('[data-mission-detail="ran"] [data-panel-meta] [data-run]')?.textContent, map.q('[data-mission-detail="ran"] [data-panel-meta] [data-agent-mark]')?.getAttribute("data-agent-mark")]).toEqual(["Sonnet 5.5·low", "codex"]);
    await act(async () => card(map, "ran-unknown").click());
    expect(map.q('[data-mission-detail="ran-unknown"] [data-panel-meta] [data-run], [data-mission-detail="ran-unknown"] [data-panel-meta] [data-agent-mark]')).toBeNull();
    // Its times, its finish lines, its line and its badges are the panel's (D-07).
    expect(card(map, "core").querySelector("[data-node-criteria], [data-node-line], [data-node-stage], [data-node-time], [data-node-reopened], [data-node-changed], [data-node-landing], [data-node-pr]")).toBeNull();
    // The operator's turn wears 내 차례's word and mark in the decision colour's border; one waiting on the Conductor names it quietly in its own border (B2).
    expect([card(map, "raised").querySelector("[data-node-attention-tag]")?.textContent, card(map, "asked").querySelector("[data-node-attention-tag]")?.textContent]).toEqual(["Needs you", "Conductor"]);
    expect(card(map, "raised").dataset.nodeAttention).toBe("operator");
    expect(card(map, "raised").className).toContain("border-mission-hold");
    expect(card(map, "raised").querySelector("[data-node-turn]")).not.toBeNull();
    expect(card(map, "asked").className).not.toContain("border-mission-hold");
    expect(card(map, "asked").querySelector("[data-node-turn]")).toBeNull();
    expect(card(map, "raised").getAttribute("aria-label")).toBe(`Node raised · In progress · PR ${"#"}1840 · Where it goes: Work (done) → ${"#"}1840 (done) → Approval (done) → main (in) · Needs a look · You · PR ${"#"}1840 merged with a finish line not passed`);
    // Hide's mark on the card it changed last, what it did and why in the name (B3).
    expect(card(map, "api").querySelector("[data-node-hide]")).not.toBeNull();
    expect(card(map, "api").getAttribute("aria-label")).toContain("Hide · Marked done · merged + criteria 1/1");
    expect(card(map, "core").querySelector("[data-node-hide]")).toBeNull();
    // Finished work: a green disc on a faint green surface inside a plain border.
    expect(card(map, "api").className).toContain("bg-mission-done/10");
    expect(card(map, "api").className).not.toMatch(/border-mission-done/);
    // Outside is the node's kind: a dotted border and `↗` on its pull request in every state; one closed unmerged waits on the Conductor (B12, B19).
    expect(["outside", "closed-out", "merged-out"].map((key) => [card(map, key).className.includes("border-dotted"), card(map, key).querySelector("[data-node-outside]")?.textContent])).toEqual([[true, `↗${"#"}1232`], [true, `↗${"#"}1605`], [true, `↗${"#"}1702`]]);
    // One that watches a branch starts at it, after `↗`.
    expect(card(map, "branch-out").querySelector("[data-node-outside]")?.textContent).toBe("↗ feat/outside-work");
    expect(card(map, "closed-out").querySelector("[data-node-attention-tag]")?.getAttribute("data-node-attention-tag")).toBe("pr_closed");
    expect(card(map, "merged-out").getAttribute("aria-label")).toBe(`Node merged-out · Done · Outside the mission · Where it goes: ↗${"#"}1702 (done) → Approval (done) → main (in)`);
  } finally { await map.unmount(); }
});

it("draws `👤 머지는 내가` as the card's second-line strip only where the node's plan gives the operator a merge Settings give the Conductor, in its state's tint, beside the gate's strip when both (PRD mission-destination D-35, D-45, B20, B21)", async () => {
  const mine = (state: "ahead" | "due" | "merged") => ({ by: "operator" as const, why: "DB change", state, prs: state === "merged" ? [] : [31] });
  const nodes = [
    node("due", { status: "done", pr: 31, pr_state: "mergeable", merge: mine("due") }),
    node("ahead", { status: "working", pr: 32, pr_state: "pending", merge: { ...mine("ahead"), prs: [32] } }),
    node("merged", { status: "done", pr: 33, pr_state: "merged", merge: mine("merged") }),
    node("conductor", { status: "done", pr: 34, pr_state: "mergeable", merge: { by: "conductor", state: "ahead", prs: [34] } }),
    node("both", { status: "working", pr: 35, pr_state: "pending", merge: { ...mine("ahead"), prs: [35] }, gate: { state: "ahead", look: "the screens", ask: null, since_unix_ms: null, long_wait_at_unix_ms: null, via: null, said: null } }),
  ];
  const map = await mount({ items: [mission({ scope: "merge", destination: "main", nodes })] });
  try {
    const strip = (key: string) => {
      const one = card(map, key).querySelector<HTMLElement>("[data-node-merge-strip]");
      return one ? [one.dataset.nodeMergeStrip, one.textContent] : null;
    };
    expect(["due", "ahead", "merged", "conductor"].map(strip)).toEqual([["due", "Yours to merge: PR #31"], ["ahead", "You merge it"], ["merged", "Merged"], null]);
    // Yellow is only what the operator does now; ahead is the faint orange of a planned human step (D-21).
    expect(card(map, "due").querySelector("[data-node-merge-strip]")?.className).toContain("bg-mission-hold/20");
    expect(card(map, "ahead").querySelector("[data-node-merge-strip]")?.className).not.toContain("bg-mission-hold/20");
    // It sits under the title, above the station line, and its words are in the card's name.
    const parts = [...card(map, "due").children].map((one) => (one as HTMLElement).dataset);
    expect(parts.findIndex((one) => one.nodeMergeStrip) < parts.findIndex((one) => one.nodeRow)).toBe(true);
    expect(card(map, "due").getAttribute("aria-label")).toContain("main · You merge it (yours now) · Yours to merge: PR #31");
    // With the gate's strip, both on one line.
    const both = card(map, "both").querySelector("[data-node-human-strip]")!;
    expect([both.querySelector("[data-node-gate]") !== null, both.querySelector<HTMLElement>("[data-node-merge-strip]")?.dataset.nodeMergeStrip]).toEqual([true, "ahead"]);
  } finally { await map.unmount(); }
  // A scope `pr` mission leaves every merge to the operator: no strip, only the faint orange 👤 on the destination station (D-35).
  const scoped = await mount({ items: [mission({ scope: "pr", destination: "main", nodes: [node("pr", { status: "working", pr: 36, pr_state: "pending", merge: { by: "operator", state: "ahead", prs: [36] } })] })] });
  try {
    expect(card(scoped, "pr").querySelector("[data-node-merge-strip], [data-node-human-strip]")).toBeNull();
    const last = card(scoped, "pr").querySelector<HTMLElement>('[data-station="land"]')!;
    expect([last.dataset.stationState, last.dataset.stationPlace]).toEqual(["ahead", "destination"]);
  } finally { await scoped.unmount(); }
});

it("shows on a card's hover its title, its worker in full, where it is, where its result goes with who merges and its worktree branch, and the Conductor's last line, with nothing to press (PRD mission-destination D-19, D-20, B32, B33)", async () => {
  const nodes = [
    node("core", { title: "A long title that the card cuts", status: "working", change: "split in two", branch: "feat/core-split", line: "replay 19/31", pr: 41, pr_state: "pending", prs: [{ number: 40, state: "closed", url: null }, { number: 41, state: "pending", url: null, base: "release/1.2" }], worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false }, merge: { by: "operator", why: "DB change", state: "ahead", prs: [41] } }),
    node("research", { kind: "research", delivery: null, status: "done", line: "the cause is the reopen order" }),
  ];
  const map = await mount({ items: [mission({ scope: "merge", destination: "main", nodes })] });
  try {
    await act(async () => card(map, "core").focus());
    const hover = document.querySelector<HTMLElement>('[data-node-hover="core"]')!;
    expect(hover.querySelector("[data-node-hover-title]")?.textContent).toBe("A long title that the card cuts");
    expect(hover.querySelector("[data-node-hover-change]")?.textContent).toBe("split in two");
    expect(hover.querySelector('[data-node-hover-part="worker"]')?.textContent).toContain("Claude Code · opus·high (planned)");
    // Where it goes in words, a branch other than the destination named as it is; every pull request it holds; who merges, and why the plan changed Settings' choice.
    expect(hover.querySelector("[data-node-hover-path]")?.textContent).toBe("Work (done) → #41 +1 (now) → Approval → release/1.2 · You merge it (yours later)");
    expect(hover.querySelector("[data-node-hover-prs]")?.textContent).toBe("Pull requests #40, #41");
    expect(hover.querySelector("[data-node-hover-merge]")?.textContent).toBe("You merge");
    expect(hover.querySelector("[data-node-hover-override]")?.textContent).toBe("Settings have the Conductor merge; this node is yours · DB changeTo change it, tell the Conductor");
    expect(hover.querySelector<HTMLElement>("[data-node-hover-branch]")?.dataset.nodeHoverBranch).toBe("feat/core-split");
    expect(hover.querySelector('[data-node-hover-part="line"]')?.textContent).toContain("replay 19/31");
    expect(hover.querySelectorAll("button, a, input")).toHaveLength(0);
  } finally { await map.unmount(); }
  // A node that changes no code has no station part; its card ends in its conclusion.
  const plain = await mount({ items: [mission({ scope: "merge", destination: "main", nodes })] });
  try {
    await act(async () => card(plain, "research").focus());
    const hover = document.querySelector<HTMLElement>('[data-node-hover="research"]')!;
    expect([hover.querySelector('[data-node-hover-part="where"]'), hover.querySelector('[data-node-hover-part="line"]')?.textContent]).toEqual([null, expect.stringContaining("the cause is the reopen order")]);
    expect(card(plain, "research").querySelector("[data-node-stations]")).toBeNull();
  } finally { await plain.unmount(); }
});

it("opens the options line's detail on focus: each value, where it comes from and why when the core kept a reason, and how to change it (PRD mission-destination D-50, B53)", async () => {
  const events = [{ at_unix_ms: 1, kind: "strength", strength: "thorough", line: "touches the session store" }];
  const map = await mount({ items: [mission({ nodes: [node("a")], scope: "pr", strength: "thorough", cap: 2 })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } } as unknown as Partial<SnapshotRest>);
  try {
    await act(async () => map.q("[data-mission-options]")!.focus());
    const detail = document.querySelector<HTMLElement>("[data-mission-options-detail]")!;
    const row = (key: string) => [detail.querySelector(`[data-mission-head-option-value="${key}"]`)?.textContent, detail.querySelector<HTMLElement>(`[data-mission-head-option-value="${key}"] + [data-mission-head-option-source]`)?.dataset.missionHeadOptionSource];
    expect(["merge", "worktree", "strength", "cap"].map(row)).toEqual([
      ["Up to ready PRs · you merge them all", "settings"],
      ["Cleaned once merged · branches kept", "settings"],
      ["Thorough · touches the session store", "mission"],
      ["At most 2", "mission"],
    ]);
    // A changed value with no reason on the wire names none.
    expect(detail.querySelector('[data-mission-head-option-why="cap"]')).toBeNull();
    expect(detail.querySelector("[data-mission-options-foot]")?.textContent).toBe("To change, tell the Conductor · Defaults in Settings › Mission");
  } finally { await map.unmount(); }
});

it("tethers each agent checking a node to its card's right side as a satellite, its ring the check's state, its name the agent, the model and effort its pane was started with or the plan's marked planned, and the state (PRD mission-tool B17)", async () => {
  const nodes = [
    node("core", { status: "working", criteria: [line("review", "review", { checkers: [{ ...checker("codex", "open", "agent-7"), pane_id: "w1:p4" }] })] }),
    node("all", { depends_on: ["core"], kind: "verify", delivery: null, criteria: [line("verify", "verify", { rule: "all", checkers: [checker("claude", "pass", "agent-8"), { ...checker("codex", "fail", "agent-9"), summary: "The filter resets on reload", evidence: "Opened /lens, set a filter, reloaded: the filter was gone. Steps 1-4 of the plan pass." }, checker("gemini")] })] }),
    node("alone", { status: "working", criteria: [line("c1", "own")] }),
  ];
  // The reviewer's pane was started with another effort than the plan asked; the launch is what shows.
  const map = await mount({ items: [mission({ nodes })] }, [], [{ device_id: "mac", pane_id: "w1:p4", session_digest: "r", launch_model: "gpt-6", launch_effort: "xhigh" }]);
  try {
    const box = (key: string) => map.q(`[data-dependency-node="${key}"]`)!;
    const satellites = (key: string) => [...box(key).querySelectorAll<HTMLElement>("[data-node-satellite]")].map((one) => [one.dataset.nodeSatellite, one.dataset.checkerState, one.getAttribute("aria-label"), one.className.match(/border-(dashed|mission-\w+)/g)?.join(" ")]);
    expect(satellites("core")).toEqual([["codex", "running", "Codex · gpt-6·xhigh · reviewing", "border-mission-review"]]);
    // Several stack; an agent Hide does not know is named by its id.
    expect(satellites("all")).toEqual([
      ["claude", "pass", "Claude Code · high (planned) · passed", "border-mission-done"],
      ["codex", "fail", "Codex · high (planned) · failed", "border-mission-failed"],
      ["gemini", "waiting", "gemini · high (planned) · verification to start", "border-dashed border-mission-waiting"],
    ]);
    // They stand beside the card, not in it, and the card draws no checker rows.
    expect(card(map, "all").querySelector("[data-node-satellite], [data-node-checker], [data-node-checkers]")).toBeNull();
    expect(card(map, "all").nextElementSibling?.hasAttribute("data-node-satellites")).toBe(true);
    // A node checked by no other agent has none, and its box is the card alone.
    expect(box("alone").querySelector("[data-node-satellites]")).toBeNull();
    expect(box("alone").firstElementChild).toBe(card(map, "alone"));
    // A satellite is reached by the keyboard and opens its answer: why it rejected, the full text folded, and the ways on.
    const rejecter = box("all").querySelectorAll<HTMLElement>("[data-node-satellite]")[1]!;
    expect(rejecter.tagName).toBe("BUTTON");
    await act(async () => rejecter.click());
    const popover = document.querySelector<HTMLElement>('[data-checker-popover="codex"]')!;
    expect(popover.querySelector("[data-checker-popover-state]")?.textContent).toBe("Rejected");
    expect(popover.querySelector("[data-verdict-summary]")?.textContent).toBe("The filter resets on reload");
    expect(popover.querySelector("[data-verdict-raw]")).toBeNull();
    await act(async () => popover.querySelector<HTMLElement>("[data-verdict-raw-toggle]")!.click());
    expect(popover.querySelector('[data-verdict-raw="open"]')?.textContent).toContain("the filter was gone");
    // Its pane is not among the agents Hide knows, so neither its terminal nor its record can open.
    expect(popover.querySelector<HTMLButtonElement>("[data-checker-terminal]")?.disabled).toBe(true);
    expect(popover.querySelector<HTMLButtonElement>("[data-checker-record]")?.disabled).toBe(true);
    // 노드 상세 picks its node.
    await act(async () => popover.querySelector<HTMLElement>("[data-checker-node]")!.click());
    expect(map.q('[data-mission-panel="all"]')).not.toBeNull();
    expect(document.querySelector("[data-checker-popover]")).toBeNull();
  } finally { await map.unmount(); }
});

it("names each role: a reviewer and a verifier satellite carry their role's name as hover and description, and the card's hover names its worker by the worker's role", async () => {
  const nodes = [
    node("core", { status: "working", worker: { agent: "claude", model: "opus", effort: "high", effort_auto: false }, criteria: [line("review", "review", { checkers: [checker("codex", "open", "agent-7")] }), line("verify", "verify", { checkers: [checker("claude", "open", "agent-8")] })] }),
  ];
  const map = await mount({ items: [mission({ nodes })] });
  try {
    const marks = [...map.q('[data-dependency-node="core"]')!.querySelectorAll<HTMLElement>("[data-node-satellite]")];
    expect(marks.map((one) => one.getAttribute("aria-description"))).toEqual([
      "Code review",
      "Verification",
    ]);
    await act(async () => marks[1]!.focus());
    expect(document.querySelector('[data-slot="tooltip-content"]')?.textContent).toContain("Verification");
    // The worker's mark is part of the card, whose hover names the worker's part by its role.
    expect(card(map, "core").querySelector("[data-node-worker-mark]")).not.toBeNull();
    await act(async () => card(map, "core").focus());
    expect(document.querySelector('[data-node-hover="core"] [data-node-hover-part="worker"] > span')?.textContent).toBe("Implementation");
  } finally { await map.unmount(); }
});

it("shows on the card and in the panel how long a node took from its events, and no number without them or while the device is gone (B4, B28, B35)", async () => {
  // The page reads its own clock, so the test holds it.
  const now = Math.floor(Date.now() / 60_000) * 60_000;
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(now);
  const at = (minutes: number) => now - minutes * 60_000;
  const nodes = [node("a", { status: "done", pr: 3, pr_state: "merged" }), node("b", { status: "working" }), node("c", { status: "working" })];
  const move = (minutes: number, key: string, status: MissionNode["status"]): MissionEvent => ({ at_unix_ms: at(minutes), kind: "node", node: key, status });
  const events = [move(90, "a", "working"), move(30, "a", "done"), move(40, "b", "working")];
  const facts = async (map: Awaited<ReturnType<typeof mount>>, key: string) => {
    await act(async () => card(map, key).click());
    return map.q("[data-panel-fact]")?.textContent ?? null;
  };
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    // The card says it on its run line, as the panel does (operator decision 2026-11-02).
    expect(["a", "b", "c"].map((key) => card(map, key).querySelector("[data-node-took]")?.textContent ?? null)).toEqual(["1h", "40m", null]);
    // With no worker the line holds the time alone.
    expect(card(map, "a").querySelector("[data-node-took]")?.closest("[data-node-run]")?.getAttribute("data-node-run")).toBe("none");
    expect(await facts(map, "a")).toBe("took 1h");
    expect(await facts(map, "b")).toBe("took 40m");
    // A node with no event that moved it draws no time.
    expect(await facts(map, "c")).toBeNull();
  } finally { await map.unmount(); }
  const stale = await mount({ items: [mission({ nodes, device_id: "mini", stale: true })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    // The finished node's time stays; the running ones stop, and so does the head's.
    expect(await facts(stale, "a")).toBe("took 1h");
    expect(await facts(stale, "b")).toBeNull();
    expect(["a", "b"].map((key) => card(stale, key).querySelector("[data-node-took]")?.textContent ?? null)).toEqual(["1h", null]);
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
    steps: [{ key: "parse", title: "Command parser", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null }],
    review: "approved" as const, ask: null, progress: { current: "parse", next: null, done: 0, total: 1, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: Date.now(),
  };
  const nodes = [
    node("core", { title: "Core", pr: 1703, pr_state: "merged", status: "done" }),
    node("engine", {
      title: "조사 에이전트: Hide가 실행하는 명령 읽기", status: "working", depends_on: ["core"], pr: 1704, base_pr: 1703, pr_state: "mergeable", pr_url: "https://github.com/o/r/pull/1704", branch: "hide-ai-engine", plan, attempts: 2,
      worker: { agent: "claude", model: "opus", effort: "xhigh", effort_auto: false },
      criteria: [line("c1", "own", { text: "명령 읽기가 cat/rg 잡음을 걸러낸다", state: "pass" }), line("c2", "own", { text: "실제 세션 50개 replay", state: "skipped", skip: { reason: "세션이 31개뿐" } }), line("check", "check", { state: "fail", summary: "3 tests fail", evidence: "cargo nextest: 3 failed, 120 passed" }), line("ci", "ci", { state: "pass", reruns: 1 }), line("review", "review", { checkers: [checker("codex")] })],
      criteria_met: 3, criteria_skipped: 1,
    }),
    node("verify", { title: "세 PR 함께 확인", kind: "verify", delivery: null, depends_on: ["engine"] }),
  ];
  // Held, as in the B28 test: the panel reads the clock again when it draws.
  const minute = Math.floor(Date.now() / 60_000) * 60_000;
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(minute);
  const events: MissionEvent[] = [{ at_unix_ms: minute - 60_000, kind: "node", node: "engine", status: "working" }, { at_unix_ms: minute - 30_000, kind: "pr_opened", node: "engine", pr: 1704 }];
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    await act(async () => card(map, "engine").click());
    const panel = map.q('[data-mission-detail="engine"]')!;
    expect([...panel.querySelectorAll<HTMLElement>("[data-panel-part]")].map((part) => part.dataset.panelPart)).toEqual(["head", "now", "plan", "result", "criteria", "links", "events"]);
    // The head holds the ways to the worker's terminal and record, beside ✕, so they stay in reach while the panel scrolls.
    const head = panel.querySelector('[data-panel-part="head"]')!;
    expect(head.className).toContain("sticky");
    expect([head.querySelector("[data-node-terminal]")?.textContent, head.querySelector("[data-node-terminal]")?.getAttribute("aria-label"), head.querySelector("[data-node-record]")?.textContent]).toEqual(["Terminal", "Go to terminal", "Record"]);
    // Its current pull request is the head's first way: its mark in its state, opening its PR card; a change says no kind.
    const prCard = head.querySelector<HTMLElement>("[data-node-pr-card]")!;
    expect([prCard.dataset.nodePrCard, prCard.getAttribute("aria-label"), prCard.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark"), prCard.textContent]).toEqual(["1704", `PR card · ${"#"}1704`, "mergeable", `${"#"}1704`]);
    expect(head.querySelector("[data-panel-kind]")).toBeNull();
    expect(panel.querySelector("[data-panel-title]")?.textContent).toBe("조사 에이전트: Hide가 실행하는 명령 읽기");
    // No launch recorded for its pane: the plan's model and effort, marked planned.
    expect(panel.querySelector("[data-panel-meta] [data-run]")?.getAttribute("title")).toBe("opus·xhigh (planned)");
    expect([panel.querySelector("[data-panel-meta] [data-run-words]")?.textContent, panel.querySelector("[data-panel-meta] [data-run-planned]")?.textContent]).toEqual(["opus·xhigh", "(planned)"]);
    // One line of facts: no state or kind word, which the mark already says, and the branch at its end.
    expect([...panel.querySelectorAll("[data-panel-fact]")].map((fact) => fact.textContent)).toEqual(["took 1m", "Attempt 2"]);
    expect(panel.querySelector("[data-panel-meta]")?.textContent).not.toContain("Working");
    expect(panel.querySelector("[data-panel-meta] [data-panel-branch]")?.textContent).toBe("hide-ai-engine");
    // The worker's plan: its count and checklist, no `Now:` line, and `Larger ›` opening its map as the pane chip does.
    expect(panel.querySelector('[data-panel-plan="open"] [data-panel-plan-count]')?.textContent).toBe("0/1");
    expect([...panel.querySelectorAll<HTMLElement>("[data-plan-row]")].map((row) => row.dataset.planRow)).toEqual(["parse"]);
    expect(panel.textContent).not.toContain("Now:");
    await act(async () => panel.querySelector<HTMLElement>("[data-panel-plan-large]")!.click());
    expect(map.events.filter((event) => event.kind === "detail_open").map((event) => event.payload)).toEqual([{ item: { kind: "plan", session: plan.session } }]);
    // What it made: no second mark of the head's PR, the base it stacks on and the CI line.
    expect(panel.querySelector('[data-panel-part="result"] [data-pr-mark]')).toBeNull();
    expect(panel.querySelector("[data-panel-base]")?.textContent).toBe(`on ${"#"}1703`);
    expect(panel.querySelector('[data-panel-part="result"]')?.textContent).toContain("✓rerun 1 time");
    // One list of ✓ ○ ⤳ ✕, each with a tag of who decides it, on one line with the whole text on hover; the review ends in its checker's chip.
    const lines = [...panel.querySelectorAll<HTMLElement>("[data-panel-line]")].map((row) => row.textContent);
    expect(lines).toEqual(["✓Work명령 읽기가 cat/rg 잡음을 걸러낸다", "⤳Work실제 세션 50개 replay", "✕CheckCheck command passes", "✓CICI passesrerun 1 time", "○Reviewcodex reviewreview to start"]);
    expect(panel.querySelector('[data-panel-line="check"] [title]')?.getAttribute("title")).toBe("Check command passes\n\ncargo nextest: 3 failed, 120 passed");
    expect(panel.querySelector("[data-criteria-group]")).toBeNull();
    // A skipped line carries its reason and a failed one its short reason, the full text folded.
    expect([...panel.querySelectorAll<HTMLElement>("[data-panel-verdict]")].map((verdict) => [verdict.dataset.panelVerdict, verdict.textContent])).toEqual([
      ["skipped", "Skipped세션이 31개뿐"],
      ["fail", "Failed3 tests failFull text"],
    ]);
    // The review names its checker by a chip that opens its answer.
    const chip = panel.querySelector<HTMLElement>('[data-panel-checker="codex"]')!;
    expect(chip.dataset.checkerState).toBe("waiting");
    await act(async () => chip.click());
    expect(document.querySelector('[data-checker-popover="codex"]')).not.toBeNull();
    expect(document.querySelector("[data-checker-node]")).toBeNull();
    await act(async () => chip.click());
    // The CI result and every line name their state with one mark's word.
    const named = (scope: string) => [...panel.querySelectorAll(`${scope} [data-criterion-mark]`)].map((mark) => mark.getAttribute("aria-label"));
    expect(named('[data-panel-part="result"]')).toEqual(["passed"]);
    expect(named("[data-panel-line]")).toEqual(["passed", "skipped", "failed", "passed", "not yet"]);
    expect(panel.querySelector('[data-panel-part="criteria"] h4')?.textContent).toBe("Done when 2/5");
    // The connections are one line of flow: before → this node → after, each side named for a screen reader.
    expect(panel.querySelector("[data-panel-flow]")?.textContent).toBe(`Core${"#"}1703This node세 PR 함께 확인`);
    expect(panel.querySelector('[data-panel-link="core"]')?.getAttribute("aria-label")).toBe("Waits on · Core");
    expect(panel.querySelector('[data-panel-link="verify"]')?.getAttribute("aria-label")).toBe("Waited on by · 세 PR 함께 확인");
    // A connection picks its node.
    await act(async () => (panel.querySelector('[data-panel-link="verify"]') as HTMLElement).click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("verify");
    // A kind other than a change leads the facts line.
    expect(map.q('[data-mission-detail="verify"] [data-panel-meta] [data-panel-kind]')?.textContent).toBe("Verify");
    await act(async () => (map.q('[data-panel-link="engine"]') as HTMLElement).click());
    const back = map.q('[data-mission-detail="engine"]')!;
    expect(back.querySelector('[data-panel-part="head"] [data-node-pr-card]')).not.toBeNull();
    expect(back.querySelector('[data-panel-part="ways"]')).toBeNull();
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

it("draws a worker's plan as one bar of cells on its card with the step being worked under it, and keeps a finished node's plan folded in its panel with its map one press away (operator decision 2026-11-02)", async () => {
  const step = (key: string, status: "done" | "working" | "planned") => ({ key, title: `Step ${key}`, depends_on: [], status, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null });
  const plan = (session_id: string, done: number, total: number) => ({
    pane_id: "", session: { device_id: "mac", agent: "codex", session_id },
    steps: Array.from({ length: total }, (_, index) => step(`s${index}`, index < done ? "done" : index === done ? "working" : "planned")),
    review: "approved" as const, ask: null, progress: { current: done < total ? `s${done}` : null, next: null, done, total, finished: done === total }, created_at_unix_ms: 1, updated_at_unix_ms: 2,
  });
  const worker = { agent: "codex", model: "gpt-5.5", effort: "high", effort_auto: false };
  const nodes = [
    node("live", { status: "working", worker, plan: plan("live-session", 3, 5), change: "the operator asked for WASM" }),
    node("over", { status: "done", worker, plan: plan("over-session", 4, 4), finished_at_unix_ms: 60, launched: { agent_id: "agent-2", agent: "codex", model: "gpt-5.5", effort: "high" }, kind: "research", delivery: null, pr: 1602, pr_state: "merged", prs: [{ number: 1601, state: "closed", url: null }, { number: 1602, state: "merged", url: null }] }),
    node("none", { status: "working", worker }),
  ];
  const map = await mount({ items: [mission({ nodes })] });
  try {
    // One bar of cells under the title and no number on the card; the count is in its name.
    expect(["live", "over", "none"].map((key) => card(map, key).querySelector("[data-progress-cells]")?.getAttribute("data-progress-cells") ?? null)).toEqual(["done done done working plan", "done done done done", null]);
    expect(card(map, "live").querySelector("[data-node-plan-count]")).toBeNull();
    expect(["live", "over"].map((key) => card(map, key).querySelector("[data-node-steps-now]")?.textContent ?? null)).toEqual(["Now Step s3", null]);
    expect(card(map, "live").getAttribute("aria-label")).toContain("Task steps 3/5");
    await act(async () => card(map, "live").click());
    const live = map.q('[data-mission-detail="live"]')!;
    expect(live.querySelector("[data-panel-plan]")?.getAttribute("data-panel-plan")).toBe("open");
    expect(live.querySelector("[data-panel-plan-toggle]")).toBeNull();
    // A replan is one mark in the facts line; its reason is its hover and its name.
    expect(live.querySelector("[data-panel-replan]")?.getAttribute("aria-label")).toBe("Changed by a replan · the operator asked for WASM");
    expect(live.querySelector("[data-panel-meta]")?.textContent).toContain("Replanned");
    await act(async () => card(map, "over").click());
    const over = map.q('[data-mission-detail="over"]')!;
    // Its current pull request is the head's; the earlier one stays in what it made. Research says its kind first.
    expect([over.querySelector('[data-panel-part="head"] [data-node-pr-card]')?.getAttribute("data-node-pr-card"), over.querySelector("[data-panel-prs]")?.getAttribute("data-panel-prs")]).toEqual(["1602", "1601"]);
    expect(over.querySelector("[data-panel-meta]")?.firstElementChild?.textContent).toBe("Research");
    // Finished: the plan its worker left, folded under its count.
    expect([over.querySelector("[data-panel-plan]")?.getAttribute("data-panel-plan"), over.querySelector("[data-panel-plan-count]")?.textContent, over.querySelector("[data-plan-row]")]).toEqual(["folded", "4/4", null]);
    const toggle = over.querySelector<HTMLElement>("[data-panel-plan-toggle]")!;
    expect([toggle.textContent, toggle.getAttribute("aria-expanded")]).toEqual(["Expand", "false"]);
    await act(async () => toggle.click());
    expect([...over.querySelectorAll<HTMLElement>("[data-plan-row]")].map((row) => row.dataset.stepStatus)).toEqual(["done", "done", "done", "done"]);
    await act(async () => over.querySelector<HTMLElement>("[data-panel-plan-large]")!.click());
    expect(map.events.filter((event) => event.kind === "detail_open").map((event) => event.payload)).toEqual([{ item: { kind: "plan", session: plan("over-session", 4, 4).session } }]);
  } finally { await map.unmount(); }
});

it("shows the steps the Conductor planned before a worker starts, marks a worker's step that differs from them, and folds the brief until it is opened", async () => {
  const step = (key: string, title: string, status: "done" | "working" | "planned") => ({ key, title, depends_on: [], status, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null });
  const plan = {
    pane_id: "", session: { device_id: "mac", agent: "codex", session_id: "fix-session" },
    steps: [step("a", "Find the cause", "done"), step("b", "Keep it in the core", "working"), step("c", "Open the PR", "planned")],
    review: "approved" as const, ask: null, progress: { current: "b", next: "c", done: 1, total: 3, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: 2,
  };
  const planned = ["Find the cause", "Keep it in the URL", "Open the PR"];
  const nodes = [
    node("ahead", { steps: ["Reproduce it", "Fix it", "Open the PR"] }),
    node("fix", { status: "working", steps: planned, plan, brief_bytes: 2458, worker: { agent: "codex", model: "gpt-5.5", effort: "high", effort_auto: false } }),
  ];
  const text = "Node fix: keep the filter after a reload.\n\nFinish lines: c1, check.";
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_brief: { mission: "mission-1", node: "fix", text } });
  try {
    // Before a worker starts: dashed cells, the count and that nothing began, and the Conductor's steps in the panel.
    const ahead = card(map, "ahead");
    expect(ahead.querySelector("[data-progress-cells]")?.getAttribute("data-progress-cells")).toBe("plan plan plan");
    expect(ahead.querySelector("[data-node-steps-waiting]")?.textContent).toBe("3 planned steps · Not started");
    expect(ahead.getAttribute("aria-label")).toContain("3 planned steps");
    await act(async () => ahead.click());
    const before = map.q('[data-mission-detail="ahead"]')!;
    expect(before.querySelector("[data-panel-plan]")?.getAttribute("data-panel-plan")).toBe("planned");
    expect([...before.querySelectorAll("[data-plan-row]")].map((row) => row.textContent)).toEqual(["Reproduce it", "Fix it", "Open the PR"]);
    expect(before.querySelector('[data-panel-part="plan"] h4')?.textContent).toBe("How it goes");
    // No worker, no brief: nothing to fold.
    expect(before.querySelector("[data-panel-part=\"brief\"]")).toBeNull();
    // The worker's plan is the one list once it exists: a step it changed is marked, one it kept is not.
    expect(card(map, "fix").querySelector("[data-node-steps]")?.getAttribute("data-node-steps")).toBe("worker");
    await act(async () => card(map, "fix").click());
    const fix = map.q('[data-mission-detail="fix"]')!;
    expect([...fix.querySelectorAll("[data-plan-row]")].map((row) => [row.getAttribute("data-plan-row"), row.querySelector("[data-step-differs]") !== null])).toEqual([["a", false], ["b", true], ["c", false]]);
    // The brief is the panel's last part, folded with its size; opening asks the core once and shows the text.
    const brief = fix.querySelector('[data-panel-part="brief"]')!;
    expect(fix.lastElementChild).toBe(brief);
    expect([brief.getAttribute("data-panel-brief"), brief.querySelector("[data-panel-brief-toggle]")?.textContent, brief.querySelector("[data-panel-brief-text]")]).toEqual(["folded", "Instructions· 2.4 KB", null]);
    await act(async () => brief.querySelector<HTMLElement>("[data-panel-brief-toggle]")!.click());
    expect(map.events.filter((event) => event.kind === "mission_brief_read").map((event) => event.payload)).toEqual([{ mission: "mission-1", node: "fix" }]);
    expect(brief.querySelector("[data-panel-brief-text]")?.textContent).toBe(text);
    // Another node's text is never drawn under this node.
    await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, mission_brief: { mission: "mission-1", node: "other", text: "not this one" } } })));
    expect(brief.querySelector("[data-panel-brief-text]")).toBeNull();
  } finally { await map.unmount(); }
});

it("puts the question the decision asks of a node and its choices in its panel, read only, and the worker's plan line in the summary (B31; PRD hide-plan B16; PRD mission-tool B19)", async () => {
  const own = {
    pane_id: "p2", session: { device_id: "mac", agent: "codex", session_id: "worker-session" },
    steps: [{ key: "core", title: "Core events", depends_on: [], status: "working" as const, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null }, { key: "web", title: "Web map", depends_on: ["core"], status: "planned" as const, line: null, output: "pr" as const, changed: false, change: null, waiting: true, delegate: false, evidence: null, delegation: null }],
    review: "approved" as const, ask: null, progress: { current: "core", next: "web", done: 0, total: 2, finished: false }, created_at_unix_ms: 1, updated_at_unix_ms: Date.now(),
  };
  const map = await mount({ items: [asked({ nodes: [node("api", { status: "working", line: "Conductor's line", plan: own }), node("screen", { title: "API screen", status: "working", line: "Asked which base to use" }), node("old", { title: "Old screen", status: "dropped", line: "Replaced by the API screen" })] })] });
  try {
    // A working node's own plan step is where the summary says it is (PRD mission-tool B19).
    expect(map.q('[data-mission-summary-line="api"]')?.textContent).toBe("Now: Core events (0/2)");
    // Beside the worker's plan the Conductor's line is left to the record: the plan says where the node is.
    await act(async () => card(map, "api").click());
    const api = map.q('[data-mission-detail="api"]')!;
    expect(api.querySelector("[data-panel-plan]")).not.toBeNull();
    expect(api.querySelector("[data-panel-conductor-line]")).toBeNull();
    await act(async () => card(map, "screen").click());
    const panel = map.q('[data-mission-detail="screen"]')!;
    expect(panel.querySelector("[data-panel-question]")?.textContent).toBe("Which base for the screen PR?");
    expect(panel.querySelector("[data-panel-options]")?.textContent).toBe("Stack on the API PRRecommendedWait for the API PR to merge");
    expect(panel.querySelector("button[data-mission-option]")).toBeNull();
    // The Conductor's last line stays beside the question, and on a dropped node (PRD mission-node-simple B4).
    expect(panel.querySelector("[data-panel-conductor-line]")?.textContent).toBe("Asked which base to use");
    await act(async () => card(map, "old").click());
    expect(map.q('[data-mission-detail="old"] [data-panel-conductor-line]')?.textContent).toBe("Replaced by the API screen");
  } finally { await map.unmount(); }
});

// Operator report on M41: a raise in 내 차례 said only a reason and the plan's long sentences; its card now says what happened, why it reached the operator, and on 자세히 the worker's one line on each failed finish line.
it("draws the raises its Conductor has not settled as one safety net card in 내 차례, a row per held node with its reason and age, and no card per node", async () => {
  const now = Date.now();
  const held = ["a", "b", "c", "d", "e"].map((key, index) => ({ key, title: `Node ${key}`, reason: "worker_ended" as const, pr: null, since_unix_ms: now - (25 - index) * 60_000 }));
  const nodes = held.map((one) => node(one.key, { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: one.since_unix_ms, on_row: false, item: false, choices: ["leave", "drop"] } }));
  const map = await mount({ items: [mission({ nodes, safety_net: { conductor_present: true, since_unix_ms: now - 25 * 60_000, nodes: held } })] });
  try {
    expect(map.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toBe("safety_net");
    expect(map.q('[data-mission-turn-item="attention"]')).toBeNull();
    const net = map.q('[data-mission-turn-item="safety_net"]')!;
    expect(net.getAttribute("data-mission-safety-net")).toBe("present");
    expect(net.querySelector("[data-mission-safety-net-head]")?.textContent).toBe("5 stuck items unchanged for 25m");
    expect(net.textContent).toContain("The Conductor has not settled them yet");
    expect([...net.querySelectorAll("[data-mission-held-node]")].map((row) => [row.getAttribute("data-mission-held-node"), row.textContent])).toEqual(
      held.map((one, index) => [one.key, `${one.title}Worker ended with work leftfor ${25 - index}m`]),
    );
    // A held row's node chip picks the node on the map.
    await act(async () => net.querySelector<HTMLButtonElement>('[data-mission-turn-node="c"]')!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("c");
    // The one way out is the Conductor's pane.
    await act(async () => net.querySelector<HTMLButtonElement>("[data-mission-conversation]")!.click());
    expect(map.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "p1" } });
  } finally { await map.unmount(); }
  // With no Conductor the card says so, holds 지휘 다시 열기, and the Conductor row steps aside so the screen says it once.
  const gone = await mount({ items: [mission({ nodes, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: false, takes_answers: false }, safety_net: { conductor_present: false, since_unix_ms: now - 25 * 60_000, nodes: held } })] });
  try {
    expect(gone.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toBe("takeover");
    const net = gone.q('[data-mission-turn-item="safety_net"]')!;
    expect(net.querySelector("[data-mission-safety-net-head]")?.textContent).toBe("No Conductor conducts this mission · 5 stuck");
    expect(net.querySelector("[data-mission-takeover]")?.textContent).toBe("Resume command");
    expect(gone.q("[data-mission-conductor]")).toBeNull();
    expect(gone.container.querySelectorAll("[data-mission-takeover]")).toHaveLength(1);
  } finally { await gone.unmount(); }
});

it("lists beside the map what the Conductor decided on its own: each skipped line with its node, why, who and when, open or closed", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-11-11T00:00:00Z"));
  const now = Date.now();
  const skips = [
    { node: "api", title: "Node api", line: "check", kind: "check" as const, reason: "no check command in this repo", by: "conductor", at_unix_ms: now - 5 * 60_000 },
    { node: "api", title: "Node api", line: "c2", kind: "own" as const, text: "The OS table matches", reason: "covered by c1", by: "conductor", at_unix_ms: now - 3 * 60_000 },
  ];
  for (const status of ["running", "done"] as const) {
    const map = await mount({ items: [mission({ status, closed_at_unix_ms: status === "done" ? now : null, nodes: [node("api", { status: "working" })], skipped_alone: skips })] });
    try {
      const row = map.q('[data-mission-summary-row="decided"]')!;
      expect(row.querySelector("dt span")?.textContent).toBe("Decided by the Conductor 2");
      const toggle = row.querySelector<HTMLButtonElement>("button[aria-expanded]")!;
      expect(toggle.getAttribute("aria-expanded")).toBe("false");
      expect(row.querySelectorAll("[data-mission-skip]")).toHaveLength(0);
      await act(async () => toggle.click());
      expect(toggle.getAttribute("aria-expanded")).toBe("true");
      expect([...row.querySelectorAll("[data-mission-skip]")].map((one) => one.textContent)).toEqual([
        "Node apicheck skipped · no check command in this repoconductor · 5m",
        "Node apiThe OS table matches skipped · covered by c1conductor · 3m",
      ]);
      await act(async () => toggle.click());
      expect(toggle.getAttribute("aria-expanded")).toBe("false");
      expect(row.querySelectorAll("[data-mission-skip]")).toHaveLength(0);
      // Nothing it lists is the operator's turn.
      expect(map.q("[data-mission-turn]")).toBeNull();
    } finally { await map.unmount(); }
  }
});

it("lists only what the operator can do in 내 차례: the decision whose node chip picks the node, a node raised to the operator with its reason's choices, a worker the core raised, the merge order (B18, B20, B25; PRD mission-node-simple B14-B16)", async () => {
  const raised = { pane_id: "p2", escalation: { cause: "child_blocked", letter_id: null, since_unix_ms: 1, human_notice: true }, request: { verb: "blocked", verb_since_unix_ms: 1, line: "Login expired", end: "blocked", request: null, later_by: null, pull_requests: [] } } as unknown as AgentRow;
  const nodes = [
    node("api", { status: "working", pane_id: "p2" }),
    node("outside", { kind: "external", delivery: null, external: { pr: 1232 }, status: "working", attention: { reason: "pr_closed", pr: 1232, to: "operator", since_unix_ms: 1, on_row: false, item: true, choices: ["leave", "drop"] } }),
    // What waits on the Conductor stays on its card.
    node("waits", { status: "working", attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
  ];
  const map = await mount({ items: [asked({ nodes: [...nodes, node("screen", { title: "API screen", status: "working" })] })] }, [], [], {}, [raised]);
  try {
    expect(map.q("[data-mission-turn]")?.getAttribute("data-mission-turn")).toBe("decision attention worker_blocked");
    expect(map.q("[data-mission-turn] h3")?.textContent).toBe("Your turn 3");
    expect(map.q('[data-mission-turn-item="worker_blocked"]')?.textContent).toContain("worker stuck: Login expired");
    const item = map.q('[data-mission-turn-item="attention"]')!;
    expect(item.textContent).toContain(`PR ${"#"}1232 closed without a merge`);
    // Beside its choices the pull request it is about, with its state (mission-38 flaky-phone).
    expect(item.querySelector("[data-attention-pr]")?.getAttribute("data-attention-pr")).toBe("1232");
    // Its choices say what they do, handing it to the Conductor first; 이 단계 빼기 never goes on one press.
    expect([...item.querySelectorAll<HTMLButtonElement>("[data-attention-choice]")].map((button) => button.dataset.attentionChoice)).toEqual(["leave", "drop"]);
    expect(item.querySelector('[data-attention-choice="leave"]')?.textContent).toBe("Hand back to the Conductor");
    const drop = item.querySelector<HTMLButtonElement>('[data-attention-choice="drop"]')!;
    expect(drop.textContent).toBe("Remove this step");
    await act(async () => drop.click());
    expect(map.events.filter((event) => event.kind === "mission_attention")).toEqual([]);
    expect(drop.textContent).toBe("Confirm removal");
    expect(drop.dataset.attentionConfirm).toBe("true");
    // The second press sends it, once, as one event (B15, B16).
    await act(async () => drop.click());
    await act(async () => drop.click());
    expect(map.events.filter((event) => event.kind === "mission_attention").map((event) => event.payload)).toEqual([{ mission: "mission-1", node: "outside", action: "drop", since_unix_ms: expect.any(Number) }]);
    expect(drop.disabled).toBe(true);
    // The node chip of an item picks its node on the map.
    await act(async () => map.q("[data-mission-banner-node]")!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("screen");
    await act(async () => map.q('[data-mission-turn-node="outside"]')!.click());
    expect(map.q("[data-mission-panel]")?.getAttribute("data-mission-panel")).toBe("outside");
  } finally { await map.unmount(); }
  const prs = [node("ui", { title: "UI", status: "done", pr: 1705, pr_state: "mergeable", base_pr: 1703, depends_on: ["core"], landing: { state: "left", action: "merge" } }), node("core", { title: "Core", status: "done", pr: 1703, pr_state: "mergeable", landing: { state: "left", action: "merge" } })];
  const waiting = await mount({ items: [mission({ status: "merge_wait", nodes: prs, merge_order: ["core", "ui"], landing_order: ["core", "ui"] })] });
  try {
    expect(waiting.q('[data-mission-turn-item="merge"] [data-mission-landing-order]')?.getAttribute("data-mission-landing-order")).toBe("core ui");
    expect(waiting.q('[data-mission-turn-item="merge"]')?.textContent).toContain(`Merge orderCore${"#"}1703→UI${"#"}1705`);
    // The stack is the purple dashed edge, and nothing else marks it.
    expect(waiting.q('[data-dependency-edge="core>ui"]')?.getAttribute("class")).toContain("text-mission-review");
    expect(waiting.q('[data-dependency-edge="core>ui"]')?.hasAttribute("data-edge-dashed")).toBe(true);
  } finally { await waiting.unmount(); }
  // Waiting on its merges, a git mission tags the branch the core reads and has not found in main; a branch it cannot read says nothing.
  const branches = [node("notes", { title: "Notes", status: "done", delivery: "direct", branch: "release/notes", landing: { state: "left", action: "join" } }), node("bump", { title: "Bump", status: "done", delivery: "direct", branch: "release/bump", unread: true, landing: { state: "unread", action: "join" } })];
  const git = await mount({ items: [mission({ status: "merge_wait", nodes: branches, landing_order: ["notes"] })] });
  try {
    expect(git.q("[data-mission-landing-order]")?.getAttribute("data-mission-landing-order")).toBe("notes");
  } finally { await git.unmount(); }
});

it("puts in a node's panel what waits on whom with its choices, the worker's life, the Conductor's line dimmed once Hide's facts passed it, Hide's record, and 다시 진행 on finished work (PRD mission-node-simple B4, B15-B17)", async () => {
  const nodes = [
    node("ended", { title: "Holder record", status: "working", line: "fixing the review", agent_runtime: "ended", worker: { agent: "claude", model: "opus", effort: null, effort_auto: false }, attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 50, on_row: false, item: false, choices: ["leave", "drop"] } }),
    node("done", { title: "Holder life", status: "done", pr: 1918, pr_state: "merged", line: "waiting on CI", finished_at_unix_ms: 60, hide: { action: "done", at_unix_ms: 60 } }),
  ];
  const events: MissionEvent[] = [
    { at_unix_ms: 10, kind: "node", node: "ended", status: "working", line: "fixing the review" },
    { at_unix_ms: 20, kind: "node", node: "done", status: "working", line: "waiting on CI" },
    { at_unix_ms: 50, kind: "raised", node: "ended", reason: "worker_ended" },
    { at_unix_ms: 60, kind: "hide", node: "done", hide: "done" },
  ];
  const map = await mount({ items: [mission({ nodes })] }, [], [], { mission_events: { mission: "mission-1", events, dropped: 0 } });
  try {
    await act(async () => card(map, "ended").click());
    const panel = map.q('[data-mission-detail="ended"]')!;
    // No connections and nothing to resume or open as a card: neither part is drawn.
    expect([...panel.querySelectorAll<HTMLElement>("[data-panel-part]")].map((part) => part.dataset.panelPart)).toEqual(["head", "attention", "now", "hide", "events"]);
    expect(panel.querySelector("[data-panel-attention]")?.textContent).toBe("! Conductor · Worker ended with work left");
    expect(panel.querySelector("[data-panel-worker-life]")?.textContent).toBe("Worker ended");
    // The operator may settle it before the Conductor does (B17); a press away from 빼기 확인 puts it back.
    const drop = panel.querySelector<HTMLButtonElement>('[data-attention-choice="drop"]')!;
    await act(async () => drop.click());
    await act(async () => { drop.focus(); drop.blur(); });
    expect(drop.textContent).toBe("Remove this step");
    expect(map.events.filter((event) => event.kind === "mission_attention")).toEqual([]);
    await act(async () => drop.click());
    await act(async () => drop.click());
    expect(map.events.filter((event) => event.kind === "mission_attention").map((event) => event.payload)).toEqual([{ mission: "mission-1", node: "ended", action: "drop", since_unix_ms: expect.any(Number) }]);
    // The Conductor's line stays readable, dimmed, once Hide raised the node after it.
    expect(panel.querySelector("[data-panel-line-behind]")?.textContent).toBe("fixing the review");
    expect([...panel.querySelectorAll<HTMLElement>("[data-panel-hide-event]")].map((row) => row.textContent)).toEqual([expect.stringContaining("! Conductor · Worker ended with work left")]);
    await act(async () => card(map, "done").click());
    const done = map.q('[data-mission-detail="done"]')!;
    // The mission deletes no file; the panel has no worktree row (D-14).
    expect(done.querySelector("[data-panel-worktree]")).toBeNull();
    // Finished work keeps the Conductor's last line, dimmed since Hide finished it after (B4).
    expect(done.querySelector("[data-panel-line-behind]")?.textContent).toBe("waiting on CI");
    expect([...done.querySelectorAll<HTMLElement>("[data-panel-hide-event]")].map((row) => row.textContent?.replace(/^.*?Hide/, "Hide"))).toEqual(["Hide · Marked done"]);
    await act(async () => done.querySelector<HTMLElement>("[data-node-resume]")!.click());
    await act(async () => done.querySelector<HTMLElement>("[data-node-resume]")!.click());
    expect(map.events.filter((event) => event.kind === "mission_attention").map((event) => event.payload)).toEqual([{ mission: "mission-1", node: "ended", action: "drop", since_unix_ms: expect.any(Number) }, { mission: "mission-1", node: "done", action: "resume" }]);
  } finally { await map.unmount(); }
});

it("draws a defect a later node found as a red dashed edge back to the node it opened, until its line passes (B21)", async () => {
  const defect = (state: MissionCriterion["state"]) => line("d1", "defect", { text: "the filter resets on reload", found_by: "verify", state });
  const nodes = (state: MissionCriterion["state"]) => [node("api", { status: "working", reopened: 1, criteria: [defect(state)], criteria_met: 0 }), node("verify", { kind: "verify", delivery: null, depends_on: ["api"], status: "working" })];
  const map = await mount({ items: [mission({ nodes: nodes("open") })] });
  try {
    const edge = map.q('[data-dependency-edge="verify~api"]');
    expect(edge?.getAttribute("class")).toContain("text-mission-failed");
    expect(edge?.hasAttribute("data-edge-dashed")).toBe(true);
    await act(async () => card(map, "api").click());
    expect(map.q('[data-panel-line="d1"]')?.textContent).toBe("○Workthe filter resets on reloadfound by Node verify");
    await map.rerender(mission({ nodes: nodes("pass") }));
    expect(map.q('[data-dependency-edge="verify~api"]')).toBeNull();
  } finally { await map.unmount(); }
});

it("shows a mission from before criteria with its old sentence and no count (B23)", async () => {
  const map = await mount({ items: [mission({ exempt: true, nodes: [node("a", { criteria: [line("c1", "own", { text: "the lens keeps its filter" })] }), node("b")] })] });
  try {
    await act(async () => card(map, "a").click());
    expect(map.q("[data-panel-old-criteria]")?.textContent).toBe("the lens keeps its filter");
    expect(map.q("[data-panel-line]")).toBeNull();
  } finally { await map.unmount(); }
});

it("opens a closed mission again from its map's head with one event, and offers it on no open or cancelled mission (PRD mission-node-simple B18)", async () => {
  for (const status of ["done", "merge_wait"] as const) {
    const closed = await mount({ items: [mission({ status, nodes: [node("a", { status: "done", pr: 1, pr_state: "merged" })], closed_at_unix_ms: 2 })] });
    try {
      await act(async () => closed.q("[data-mission-reopen]")!.click());
      expect(closed.events.filter((event) => event.kind === "mission_reopen").map((event) => event.payload)).toEqual([{ mission: "mission-1" }]);
    } finally { await closed.unmount(); }
  }
  const stale = await mount({ items: [mission({ status: "done", stale: true, nodes: [node("a", { status: "done" })] })] });
  try {
    expect((stale.q("[data-mission-reopen]") as HTMLButtonElement | null)?.disabled).toBe(true);
  } finally { await stale.unmount(); }
  for (const status of ["running", "cancelled"] as const) {
    const other = await mount({ items: [mission({ status, nodes: [node("a")] })] });
    try {
      expect(other.q("[data-mission-reopen]")).toBeNull();
    } finally { await other.unmount(); }
  }
});

it("folds a finished mission's report into its first words without Markdown marks, opens it drawn as Markdown, and rests a cancelled mission's unfinished cards (B24, B35)", async () => {
  const report = "# Released 1.0.0\n\n## PRs\n- [x] Notes published.";
  const done = await mount({ items: [mission({ status: "done", report, nodes: [node("a", { status: "done", pr: 1, pr_state: "merged" })], started_at_unix_ms: 0, closed_at_unix_ms: 3 * 3_600_000 })] });
  try {
    expect(done.q("[data-mission-pill]")?.textContent).toBe("Finished");
    expect(done.q('[data-mission-stat="time"]')?.textContent).toBe("took 3h");
    expect(done.q("[data-mission-report-text] p")?.textContent).toBe("Released 1.0.0");
    await act(async () => done.q("[data-mission-report-toggle]")!.click());
    expect(done.q('[data-mission-report-text="open"] [data-mission-report-body] [data-markdown-text]')).not.toBeNull();
    expect(done.q("[data-mission-report-text] p")).toBeNull();
    expect(done.q("[data-mission-turn]")).toBeNull();
  } finally { await done.unmount(); }
  const gone = { ...mission().conductor, present: false };
  const cancelled = await mount({ items: [mission({ status: "cancelled", conductor: gone, decision: "Keep going?", nodes: [node("a", { status: "done", pr: 1503, pr_state: "merged" }), node("b", { status: "working", depends_on: ["a"] })] })] });
  try {
    expect([card(cancelled, "a").dataset.nodeLook, card(cancelled, "b").dataset.nodeLook]).toEqual(["done", "cancelled"]);
    expect(card(cancelled, "b").className).toContain("border-dashed");
    expect(cancelled.container.querySelector(".text-mission-working")).toBeNull();
    expect(cancelled.q("[data-mission-turn]")).toBeNull();
  } finally { await cancelled.unmount(); }
});

it("leads a finished mission with its result tiles, its pull requests and its report's line, then its Conductor and the map, the summary beside it (PRD mission-tool B18-B21)", async () => {
  const review = (state: MissionCriterion["state"]) => line("review", "review", { state, checkers: [checker("codex", state, "agent-9")] });
  const nodes = [
    node("api", { title: "API added", status: "done", agent_id: "w1", pr: 1871, pr_state: "merged", landing: { state: "landed", proof: { by: "merged" } }, criteria: [line("c1", "own", { state: "pass" }), review("pass")], criteria_met: 2 }),
    node("screen", { title: "Screen on the API", status: "done", agent_id: "w2", depends_on: ["api"], pr: 1872, pr_state: "merged", landing: { state: "landed", proof: { by: "merged" } }, criteria: [line("c1", "own", { state: "skipped", skip: { reason: "n/a" } }), review("pass")], criteria_met: 2, criteria_skipped: 1 }),
  ];
  const conductor = [{ pane_id: "p1", mark: "asleep" } as AgentRow];
  const done = await mount({ items: [mission({ status: "done", report: "# Fix the bug\n\nBoth PRs merged after one review round.", nodes, started_at_unix_ms: 0, closed_at_unix_ms: 3_600_000 })] }, [], [], {}, conductor);
  try {
    const parts = [...done.container.querySelectorAll<HTMLElement>("[data-mission-summary], [data-mission-results], [data-mission-pulls], [data-mission-report-text], [data-mission-conductor], [data-mission-turn], [data-mission-map], [data-mission-side]")].map((part) => Object.keys(part.dataset)[0]);
    expect(parts).toEqual(["missionSummary", "missionResults", "missionPulls", "missionReportText", "missionConductor", "missionMap", "missionSide"]);
    const tile = (key: string) => [done.q(`[data-mission-tile="${key}"] [data-mission-tile-value]`)?.textContent, done.q(`[data-mission-tile="${key}"] dt`)?.textContent];
    expect(["merged", "criteria", "agents", "landed"].map(tile)).toEqual([["2", "PRs merged"], ["3 / 4", "Finish lines met · ⤳1 skipped"], ["2", "Workers · 2 checking"], ["2 / 2", "In main"]]);
    // Each pull request is its mark, its node's title and its state's word; pressing one picks its node.
    const pull = done.q('[data-mission-pull="1871"]')!;
    expect([pull.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark"), pull.textContent]).toEqual(["merged", `${"#"}1871API addedMerged`]);
    // The report's first line repeats the title, so the next one stands for it (B21).
    expect(done.q("[data-mission-report-text] p")?.textContent).toBe("Both PRs merged after one review round.");
    expect(done.q("[data-mission-conductor]")?.dataset.missionConductor).toBe("asleep");
    // Nothing picked: the summary says the time, the result pull requests and who worked (B19).
    const row = (field: string) => done.q(`[data-mission-summary-row="${field}"] dd`);
    expect(row("took")?.textContent).toMatch(/^1h · /);
    expect([...row("result")!.querySelectorAll("[data-pr-mark]")].map((mark) => mark.getAttribute("data-pr-mark"))).toEqual(["merged", "merged"]);
    expect(row("agents")?.textContent).toBe("2 + 2 checking");
    await act(async () => pull.click());
    expect(done.q('[data-mission-panel="api"] [data-mission-detail="api"]')).not.toBeNull();
    expect(done.q("[data-mission-side]")).toBeNull();
  } finally { await done.unmount(); }
  // A tile the record holds nothing for is not drawn, nor a list without pull requests.
  const bare = await mount({ items: [mission({ status: "cancelled", exempt: true, nodes: [node("r", { kind: "research", delivery: null, status: "done" })] })] });
  try {
    expect(bare.q("[data-mission-results]")).toBeNull();
    expect(bare.q("[data-mission-pulls]")).toBeNull();
  } finally { await bare.unmount(); }
});

it("sums up an open mission beside the map until a node is picked: in progress (a step waiting on the Conductor first) and done with their counts and names, its pull requests and the workers at work of the most allowed (PRD mission-tool B19, B22; PRD mission-node-simple B5)", async () => {
  const nodes = [
    node("api", { title: "API", status: "done", pr: 1901, pr_state: "merged" }),
    node("screen", { title: "Screen", status: "working", depends_on: ["api"], line: "moving tabs", stage: 1 }),
    node("docs", { title: "Docs", status: "working", depends_on: ["api"], attention: { reason: "worker_ended", pr: null, to: "conductor", since_unix_ms: 1, on_row: false, item: false, choices: ["leave", "drop"] } }),
    node("demo", { title: "Demo", depends_on: ["screen"] }),
  ];
  const map = await mount({ items: [mission({ nodes, running: 2, cap: 3, decision: "Which order?" })] });
  try {
    // Open: the head, the Conductor row, 내 차례, then the map with the summary beside it (B22).
    const parts = [...map.container.querySelectorAll<HTMLElement>("[data-mission-summary], [data-mission-results], [data-mission-conductor], [data-mission-turn], [data-mission-map], [data-mission-side]")].map((part) => Object.keys(part.dataset)[0]);
    expect(parts).toEqual(["missionSummary", "missionConductor", "missionTurn", "missionMap", "missionSide"]);
    const row = (field: string) => map.q(`[data-mission-summary-row="${field}"] dd`);
    const name = (field: string) => map.q(`[data-mission-summary-row="${field}"] dt`)?.textContent;
    expect([name("attention"), name("working"), name("done")]).toEqual([undefined, "In progress 2", "Done 1"]);
    expect(row("working")?.textContent).toBe("DocsConductor · Worker ended with work leftScreenmoving tabs");
    expect(row("done")?.textContent).toBe("API");
    expect(row("pr")?.querySelector("[data-pr-mark]")?.getAttribute("data-pr-mark")).toBe("merged");
    expect(row("workers")?.textContent).toBe("2 working / up to 3");
    await act(async () => map.q('[data-mission-summary-node="docs"]')!.click());
    expect(map.q('[data-mission-panel="docs"]')).not.toBeNull();
  } finally { await map.unmount(); }
});

it("runs every mission's map down until the operator turns it across with →, which Settings › Mission keeps, and the turned map fits again (UI_BEHAVIOR, A mission's map)", async () => {
  const map = await mount({ items: [mission({ nodes: [node("a", { status: "working" }), node("b"), node("c", { depends_on: ["a"] })] })] });
  try {
    const flow = () => map.q("[data-dependency-graph]")!.dataset.dependencyFlow;
    const option = (which: "down" | "across") => map.q(`[data-mission-direction-option="${which}"]`) as HTMLButtonElement;
    expect([flow(), option("down").dataset.state, option("across").dataset.state]).toEqual(["down", "on", "off"]);
    await act(async () => (map.q('[data-mission-zoom-step="out"]') as HTMLButtonElement).click());
    expect(map.q("[data-mission-canvas]")!.dataset.missionCanvas).toBe("zoomed");
    // Pressing the way it already runs sends nothing.
    await act(async () => option("down").click());
    expect(map.events).toEqual([]);
    await act(async () => option("across").click());
    expect(map.events).toEqual([{ schema_version: 2, kind: "conductor_settings_set", payload: { map_flow: "across" } }]);
    // The map turns on the core's echo, not before it, and fits again.
    expect(flow()).toBe("down");
    await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, ui_state: { ...state.rest!.ui_state!, conductor_settings: { concurrency: 4, default_scope: "pr", candidates: [], default_strength: "normal", clean_merged: true, map_flow: "across" } } } })));
    expect([flow(), option("across").dataset.state, map.q("[data-mission-canvas]")!.dataset.missionCanvas]).toEqual(["across", "on", "fit"]);
  } finally { await map.unmount(); }
});

it("opens the map at 맞춤 and steps it with − and + between 40% and 200%, 맞춤 bringing it back (UI_BEHAVIOR, A mission's map)", async () => {
  const map = await mount({ items: [mission({ nodes: [node("a", { status: "working" }), node("b", { depends_on: ["a"] })] })] });
  try {
    const level = () => map.q("[data-mission-zoom-level]")?.textContent;
    const step = (which: "in" | "out" | "fit") => map.q(`[data-mission-zoom-step="${which}"]`) as HTMLButtonElement;
    const canvas = () => map.q("[data-mission-canvas]")!.dataset.missionCanvas;
    // jsdom lays nothing out, so the whole map fits at its own size.
    expect([level(), canvas(), step("fit").getAttribute("aria-pressed")]).toEqual(["100%", "fit", "true"]);
    await act(async () => step("out").click());
    expect([level(), canvas(), step("fit").getAttribute("aria-pressed")]).toEqual(["90%", "zoomed", "false"]);
    for (let press = 0; press < 8; press += 1) await act(async () => step("out").click());
    expect([level(), step("out").disabled]).toEqual(["40%", true]);
    for (let press = 0; press < 20; press += 1) await act(async () => step("in").click());
    expect([level(), step("in").disabled]).toEqual(["200%", true]);
    await act(async () => step("fit").click());
    expect([level(), canvas(), step("fit").getAttribute("aria-pressed")]).toEqual(["100%", "fit", "true"]);
  } finally { await map.unmount(); }
});

it("lights a device worker's Terminal and opens its pane there by the address its row has (hide-private issue 679)", async () => {
  const pane = "remote:mini:pane:h15:p3";
  const remote = [{ target_id: "mini", state: "connected", message: null, session: { workspaces: [], agents: [{ pane_id: pane, mark: "going", activity: "working" }], active_tab_ids: {}, focused_workspace_id: null, focused_checkout_id: null, focused_tab_id: null, focused_pane_id: null, pane_layouts: [] } }];
  const nodes = [node("cpu", { status: "working", agent_id: "agent-27621", pane_id: pane, machine: "mini", work: { state: "working" } })];
  const map = await mount({ items: [mission({ nodes })] }, remote);
  try {
    await act(async () => card(map, "cpu").click());
    const terminal = map.q('[data-mission-detail="cpu"] [data-node-terminal]') as HTMLButtonElement;
    expect(terminal.disabled).toBe(false);
    await act(async () => terminal.click());
    expect(map.events.find((event) => event.kind === "focus_pane")?.payload).toMatchObject({ pane_id: pane });
  } finally { await map.unmount(); }
});

it("tells a node being worked from one whose worker rests and what it waits on, or has no worker (hide-private issue 680)", async () => {
  const nodes = [
    node("busy", { status: "working", work: { state: "working" } }),
    node("rests", { status: "working", work: { state: "waiting", on: { kind: "review" } } }),
    node("gone", { status: "working", work: { state: "no_worker" } }),
    node("stopped", { status: "working", work: { state: "stopped", cause: { kind: "limit", line: "You've hit your limit · resets 4pm" }, since_unix_ms: Date.now() - 3 * 60_000 } }),
    node("unseen", { status: "working" }),
  ];
  const map = await mount({ items: [mission({ nodes })] });
  try {
    const look = (key: string) => [card(map, key).querySelector("[data-node-work]")?.textContent ?? null, card(map, key).querySelector("[data-node-mark]")?.hasAttribute("data-node-mark-still"), card(map, key).querySelector("[data-node-mark]")?.getAttribute("aria-label")];
    expect(look("busy")).toEqual([null, false, "Working"]);
    expect(look("rests")).toEqual(["Waiting · review", true, "Waiting · review"]);
    expect(look("gone")).toEqual(["No worker", true, "No worker"]);
    // A worker stopped with nothing to wait on says why first and how long ago at the end, its ring still (mission-56).
    expect(look("stopped")).toEqual(["Stopped · usage limit · You've hit your limit · resets 4pm· 3m", true, "Stopped · usage limit · You've hit your limit · resets 4pm"]);
    expect(card(map, "stopped").querySelector("[data-node-work]")?.getAttribute("title")).toBe("Stopped 3m ago · usage limit · You've hit your limit · resets 4pm");
    // Hide cannot see the worker's row: the card stays as it was, in progress and turning.
    expect(look("unseen")).toEqual([null, false, "In progress"]);
    expect(card(map, "rests").getAttribute("aria-label")).toContain("Waiting · review");
  } finally { await map.unmount(); }
});

it("says in 내 차례 how many letters the Conductor has not got and what to do, with its terminal, until they are delivered (hide-private issue 678)", async () => {
  const held = mission({ nodes: [node("a", { status: "working" })], conductor_letters: { count: 3, reason: "draft", since_unix_ms: 1 } });
  const map = await mount({ items: [held] });
  try {
    const item = map.q('[data-mission-turn-item="letters"]')!;
    expect(item.getAttribute("data-mission-turn-reason")).toBe("draft");
    expect(item.textContent).toContain("Letters waiting for the Conductor: 3 · unsent text in the Conductor's input: send or clear it");
    await act(async () => item.querySelector<HTMLElement>("[data-mission-conversation]")!.click());
    expect(map.events.at(-1)).toMatchObject({ kind: "focus_pane", payload: { pane_id: "p1" } });
    await map.rerender({ ...held, conductor_letters: null });
    expect(map.q("[data-mission-turn]")).toBeNull();
  } finally { await map.unmount(); }
});

it("keeps a looked-at or closed View on the decision, marked so, and opens a closed one again from its link in one event (PRD mission-human-gate B20)", async () => {
  const views = [
    { device_id: "mac", checkout_path: "/repo", display_id: "d7", view: "v7", kind: "browser" as const, label: "screens.html", by: "conductor" as const },
    { device_id: "mac", checkout_path: "/repo", display_id: "d8", view: "v8", kind: "file" as const, label: "notes.md", closed: true, by: "worker" as const },
  ];
  const map = await mount({ items: [asked({ decision_views: views })] });
  await act(async () => useShellStore.setState((state) => ({ rest: { ...state.rest!, navigator: { ...state.rest!.navigator!, workspaces: [{ id: "w", device_id: "mac", checkouts: [{ id: "c", path: "/repo" }] }] } } as unknown as SnapshotRest })));
  try {
    const rows = [...map.container.querySelectorAll<HTMLElement>("[data-mission-banner-view]")];
    expect(rows.map((row) => row.dataset.missionBannerViewState)).toEqual(["seen", "closed"]);
    expect(rows[0]?.querySelector('[aria-label="Seen"]')).not.toBeNull();
    expect(rows[1]?.textContent).toContain("Closed");
    expect(rows[1]?.textContent).toContain("Reopen ›");
    await act(async () => rows[1]!.click());
    expect(map.events.filter((event) => event.kind === "mission_view_reopen")).toEqual([{ schema_version: 2, kind: "mission_view_reopen", payload: { mission: "mission-1", key: 3, view: "v8" } }]);
    // An open one comes forward as any View does, with no reopen.
    await act(async () => rows[0]!.click());
    expect(map.events.filter((event) => event.kind === "mission_view_reopen")).toHaveLength(1);
    expect(map.events.at(-1)).toMatchObject({ kind: "focus_checkout", payload: { workspace_id: "w", checkout_id: "c", display_id: "d7" } });
  } finally { await map.unmount(); }
});

it("lists the Views linked to the decision, seen or not, each bringing its View forward, and offers only a quiet 자료 요청 with none", async () => {
  const views = [
    { device_id: "mac", checkout_path: "/repo", display_id: "d7", view: "v7", kind: "browser" as const, label: "screens.html", by: "conductor" as const },
    { device_id: "mac", checkout_path: "/repo", display_id: "d8", view: "v8", kind: "file" as const, label: "notes.md", unseen: true, by: "worker" as const },
    { device_id: "mac", checkout_path: "/repo", display_id: "d9", view: "v9", kind: "diff" as const, label: "screen.diff", by: "worker" as const },
    { device_id: "mac", checkout_path: "/repo", display_id: "d10", view: "v10", kind: "file" as const, label: "measure.csv", by: "worker" as const },
  ];
  const linked = await mount({ items: [asked({ decision_views: views })] });
  try {
    const rows = () => [...linked.container.querySelectorAll<HTMLElement>("[data-mission-banner-view]")];
    expect(linked.q("[data-mission-banner-views]")?.firstElementChild?.textContent).toBe("To look at 4");
    expect(rows().map((row) => row.dataset.missionBannerView)).toEqual(["d7", "d8", "d9"]);
    // The one not looked at wears the eye; who linked it is said on each row.
    expect(rows().map((row) => row.querySelector('[data-mark="look"]') !== null)).toEqual([false, true, false]);
    expect(rows()[0]?.dataset.missionBannerViewBy).toBe("conductor");
    await act(async () => linked.q("[data-mission-banner-views-more]")!.click());
    expect(rows()).toHaveLength(4);
    // A row brings its View forward, never a letter to the Conductor.
    await act(async () => rows()[0]!.click());
    expect(linked.events.some((event) => event.kind === "mission_material")).toBe(false);
    // With Views linked, no request is offered.
    expect(linked.q("[data-mission-material]")).toBeNull();
  } finally { await linked.unmount(); }

  const none = await mount({ items: [asked()] });
  try {
    expect(none.q("[data-mission-banner-views]")).toBeNull();
    const ask = none.q('[data-mission-material="ask"]')!;
    expect(ask.textContent).toBe("Ask for material");
    // It is a row of its own above the choices, which stay pressable (PRD mission-human-gate B19).
    expect(ask.closest("div")?.querySelector("[data-mission-option]")).not.toBeNull();
    expect(ask.compareDocumentPosition(none.q("[data-mission-option]")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await act(async () => ask.click());
    expect(none.events.filter((event) => event.kind === "mission_material")).toEqual([{ schema_version: 2, kind: "mission_material", payload: { mission: "mission-1", key: 3 } }]);
    await none.rerender(asked({ question: { ...asked().question!, material_asked: true } }));
    expect(none.q('[data-mission-material="asked"]')?.textContent).toBe("Material asked");
    expect(none.q('[data-mission-material="ask"]')).toBeNull();
  } finally { await none.unmount(); }
});

// F1: the last held PR may be settled while an earlier one still needs a merge.
it("names the latest open PR and the additional open count in the merge turn", async () => {
  for (const open of [[31, 33], [31]]) {
    const map = await mount({ items: [mission({ status: "running", landing_order: ["db"], nodes: [node("db", {
      title: "DB", status: "done", pr: 37, pr_state: "merged", prs: [{ number: 31, state: "mergeable", url: null }, { number: 33, state: "mergeable", url: null }, { number: 37, state: "merged", url: null }],
      landing: { state: "left", action: "merge" }, merge: { by: "operator", state: "due", why: "DB review", prs: open },
    })] })] });
    try {
      const chip = map.q('[data-mission-turn-item="merge"] [data-mission-turn-node="db"]')!;
      expect(chip.textContent).toBe(open.length === 2 ? "DB#33+1" : "DB#31");
      expect(chip.querySelector('[data-pr-mark]')?.getAttribute("data-pr-mark")).toBe("mergeable");
    } finally { await map.unmount(); }
  }
});
