// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { RELATION_ANSWER_TIMEOUT_MS } from "./lineage";
import { PaneAskMark, PaneHeaderBand, PanePrChip, PaneRecordButton } from "./PaneHeaderBand";
import type { AgentPullRequest, AgentRow, PaneBand, PaneHeader, PaneFocusRequest, PrState, SnapshotRest, Workspace } from "./snapshot";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import type { DispatchFn } from "./ws";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
const savedScrollIntoView = Element.prototype.scrollIntoView;
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Element.prototype.scrollIntoView = savedScrollIntoView;
  document.body.innerHTML = "";
});

const header: PaneHeader = { working: false, notice: null, band: { kind: "raised", reason: null, since_unix_ms: null, read: false, action: {pane_id: "child", label: "자식 검증"}, more: 0,
  raised: { verb: "approval", what: "하위 검증 권한이 필요합니다", raised_pane_id: "child", origin_title: "자식 검증", pane_id: "child", title: "자식 검증", agent_kind: "codex", open_pane_id: "child", since_unix_ms: null, path: ["부모", "자식 검증"], checkout: "feature", human_notice: true } } };

async function mount(value = header, agents: AgentRow[] = [], options: { rest?: SnapshotRest; paneId?: string } = {}) {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  Element.prototype.scrollIntoView = () => {};
  initializeInterfaceI18n("en");
  useUiStore.setState({relation: null});
  useShellStore.setState({agents, rest: options.rest ?? {status: {pane_focus_request: null}} as SnapshotRest});
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions(event => {events.push(event); return true;});
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneHeaderBand paneId={options.paneId ?? "parent"} header={value} actions={actions}/></TooltipProvider>));
  const answer = async (outcome: PaneFocusRequest) => {
    await act(async () => useShellStore.setState({rest: {status: {pane_focus_request: outcome}} as SnapshotRest}));
  };
  return {container, actions, events, answer, unmount: async () => { await act(async () => root.unmount()); }};
}

it("keeps pending, correlated refusal, retry and retired-target feedback in the source band", async () => {
  const {container, actions, events, answer, unmount} = await mount();
  try {
    const button = () => container.querySelector<HTMLButtonElement>("[data-pane-band-open]")!;
    await act(async () => button().click());
    expect(events).toHaveLength(1);
    const first = useUiStore.getState().relation!;
    expect(button().disabled).toBe(true);
    expect(container.querySelector('[role="status"]')?.textContent).toContain("자식 검증");
    await act(async () => actions.followRelation("parent", "child", "자식 검증"));
    expect(events).toHaveLength(1);
    await answer({request_id: "older", target_pane_id: "child", phase: "failed", message: "Old refusal", retryable: true});
    expect(button().disabled).toBe(true);
    await answer({request_id: first.requestId, target_pane_id: "child", phase: "failed", message: "Herdr refused this move", retryable: true});
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Herdr refused this move");
    expect(button().textContent).toBe("Retry");
    await act(async () => button().click());
    expect(events).toHaveLength(2);
    const second = useUiStore.getState().relation!;
    expect(second.requestId).not.toBe(first.requestId);
    await answer({request_id: first.requestId, target_pane_id: "child", phase: "succeeded", message: null, retryable: false});
    expect(button().disabled).toBe(true);
    await answer({request_id: second.requestId, target_pane_id: "child", phase: "failed", message: "Child no longer exists", retryable: false});
    expect(button().disabled).toBe(true);
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Child no longer exists");
  } finally { await unmount(); }
});

// Approved B2 fixture: five direct children, one of each state; the question
// belongs to the parent until the core explicitly raises it to the operator.
function childrenFixture(): AgentRow[] {
  const seed = legacyAgentRow({ id: "parent", pane_id: "parent", identity_label: "Parent", agent_kind: "claude", mark: "going", group: "working", status_code: "working", detail: null, changed_at_unix_ms: null, emphasized: false, unread: false, demand: "none", activity: "working" });
  const children = ["working", "result", "idle", "unknown", "question"].map((id, index) => ({
    ...seed, id, pane_id: id, identity_label: `Child ${id}`,
    mark: id === "unknown" ? "unknown" as const : id === "idle" || id === "question" ? "quiet" as const : seed.mark,
    status_code: id === "unknown" ? "unknown" as const : id === "question" ? "question" as const : id === "result" ? "done" as const : id === "idle" ? "idle" as const : seed.status_code,
    activity: id === "unknown" ? "unknown" : id === "result" || id === "idle" ? "stopped" : seed.activity, demand: id === "question" ? "question" : "none",
    lineage_parent_pane_id: "parent", lineage_child_pane_ids: [],
    state: { ...seed.state, tree_rank: id === "question" ? 0 : index + 1, line: id === "question" ? { text: "Should these changes share a PR?", mode: "quiet" as const, tone: { kind: "subtle" as const, read: false } } : null },
  }));
  return [{ ...seed, lineage_child_pane_ids: [...children.map((child) => child.pane_id), "closed"], direct_child_counts: { error: 0, approval: 0, question: 1, working: 1, done: 1 } }, ...children];
}

// PRD B1-B2: the resting row names no child; the tree button shows the first
// children's marks in tree order, `+N` for the rest, and keeps its full counts.
it("puts the tree and the first children's marks first while keeping state counts accessible", async () => {
  const agents = childrenFixture().map((row) => row.pane_id === "result" ? { ...row, mark: "done" as const } : row.pane_id === "idle" ? { ...row, mark: "quiet" as const } : row);
  const { container, unmount } = await mount({ working: true, notice: null, band: null }, agents);
  try {
    const row = container.querySelector('[data-pane-header-band="children"]')!;
    const trigger = row.querySelector<HTMLButtonElement>("[data-pane-tree]")!;
    expect(trigger.getAttribute("aria-label")).toBe("Child agents 5 · Child requests: 1 · Working: 1 · Reported results: 1 · Unknown state: 1 · Resting: 1");
    expect([...trigger.querySelectorAll("[data-pane-tree-mark]")].map((mark) => [mark.getAttribute("data-pane-tree-mark"), mark.getAttribute("data-mark")])).toEqual([["question", "quiet"], ["working", "going"], ["result", "done"]]);
    // The unknown child draws no mark, so it is counted with the rest.
    expect(trigger.textContent).toBe("+2");
    expect(row.firstElementChild?.firstElementChild).toBe(trigger);
    // An ordinary child's question is its parent's to answer: no raised count.
    expect(trigger.querySelector('[data-mark="turn"]')).toBeNull();
    expect(row.querySelectorAll("[data-pane-child-name]")).toHaveLength(0);
    expect(container.querySelector("[data-pane-working-line]")).not.toBeNull();
  } finally { await unmount(); }
});

it("opens a child from the tree popover", async () => {
  const { container, events, unmount } = await mount({ working: false, notice: null, band: null }, childrenFixture());
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-tree]")!.click());
    await act(async () => document.querySelector<HTMLButtonElement>('[data-agent-child="working"]')!.click());
    expect(useUiStore.getState().relation).toMatchObject({ sourcePaneId: "parent", targetPaneId: "working" });
    expect(events).toHaveLength(1);
  } finally { await unmount(); }
});

it("puts the pane's own approval after the tree and keeps raised counts in its name", async () => {
  const agents = childrenFixture();
  agents[0]!.descendant_mark = { kind: "raised", count: 2 };
  const { container, unmount } = await mount({ working: false, notice: null, band: { ...header.band!, kind: "approval", reason: "Run the verification command", raised: undefined, action: null, more: 1 } }, agents);
  try {
    const row = container.querySelector('[data-pane-header-band="approval"]')!;
    expect(row.firstElementChild?.firstElementChild?.getAttribute("data-pane-tree")).toBe("parent");
    expect(row.querySelector("[data-verb]")?.getAttribute("data-verb")).toBe("approval");
    expect(row.querySelector("[data-pane-band-who]")).toBeNull();
    expect(row.querySelector("[data-pane-child-names]")).toBeNull();
    const trigger = row.querySelector<HTMLButtonElement>("[data-pane-tree]")!;
    expect(trigger.textContent).toBe("+2");
    expect(trigger.getAttribute("aria-label")).toContain("Descendant requests needing you: 2");
    expect(row.textContent).toContain("Run the verification command");
    expect(container.querySelector("[data-pane-band-more]")).toBeNull();
  } finally { await unmount(); }
});

it("shows only the core-owned progress when no request waits, with or without children", async () => {
  const progress = "검증 로그를 확인하고 있어요";
  const withChildren = await mount({ working: true, notice: null, band: null, progress }, childrenFixture());
  try {
    const row = withChildren.container.querySelector('[data-pane-header-band="progress"]')!;
    expect(row.firstElementChild?.firstElementChild?.getAttribute("data-pane-tree")).toBe("parent");
    expect(row.querySelector('[data-pane-own-progress]')?.textContent).toBe(progress);
    expect(row.textContent).not.toContain("Child question");
  } finally { await withChildren.unmount(); }
  const solo = await mount({ working: true, notice: null, band: null, progress }, [], { paneId: "solo" });
  try {
    expect(solo.container.querySelector('[data-pane-tree]')).toBeNull();
    expect(solo.container.querySelector('[data-pane-own-progress]')?.textContent).toBe(progress);
  } finally { await solo.unmount(); }
});

it("never counts a core unknown child as resting or a result when the wire omits the unknown count", async () => {
  const [parent, ...all] = childrenFixture();
  const child = all.find((row) => row.pane_id === "unknown")!;
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, [{ ...parent!, lineage_child_pane_ids: [child.pane_id], direct_child_counts: { error: 0, approval: 0, question: 0, working: 0, done: 0 } }, child]);
  try {
    expect(container.querySelector("[data-pane-tree]")?.getAttribute("aria-label")).toBe("Child agents 1 · Unknown state: 1");
    // With no child to draw a mark for, the face keeps the number.
    expect(container.querySelector("[data-pane-tree]")?.textContent).toBe("1");
  } finally { await unmount(); }
});

it("skips a child of unknown state and marks the next known one", async () => {
  const agents = childrenFixture().map((row) => row.pane_id === "unknown" ? { ...row, state: { ...row.state, tree_rank: 1 } } : row.pane_id === "working" ? { ...row, state: { ...row.state, tree_rank: 2 } } : row);
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, agents);
  try {
    const trigger = container.querySelector<HTMLButtonElement>("[data-pane-tree]")!;
    expect([...trigger.querySelectorAll("[data-pane-tree-mark]")].map((mark) => mark.getAttribute("data-pane-tree-mark"))).toEqual(["question", "working", "result"]);
    expect(trigger.textContent).toBe("+2");
  } finally { await unmount(); }
});

it("says the parent waits on its working children after the request, in the sidebar's words", async () => {
  const { container, unmount } = await mount({ working: false, notice: null, band: null, request: { text: "Find the missing requests", images: 0 }, waiting_on_children: 2 }, childrenFixture());
  try {
    const row = container.querySelector('[data-pane-header-band="waiting"]')!;
    expect(row.firstElementChild?.firstElementChild?.getAttribute("data-pane-tree")).toBe("parent");
    expect(row.querySelector("[data-pane-turn-request-text]")?.textContent).toBe("Find the missing requests");
    expect(row.querySelector("[data-pane-children-wait]")?.textContent).toBe("Waiting · 2 children working");
    expect(row.querySelector("[data-pane-own-progress]")).toBeNull();
  } finally { await unmount(); }
});

it("draws each child's logo after its status mark in the tree popover", async () => {
  const agents = childrenFixture().map((row) => row.pane_id === "working" ? { ...row, agent_kind: "codex" } : row);
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, agents);
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-tree]")!.click());
    const child = document.querySelector('[data-agent-child="working"]')!;
    const mark = child.querySelector("[data-mark]")!;
    const logo = child.querySelector("[data-agent-mark]")!;
    expect(logo.getAttribute("data-agent-mark")).toBe("codex");
    expect(mark.compareDocumentPosition(logo) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  } finally { await unmount(); }
});

it("removes the child band when its last live child leaves and keeps the no-child working line", async () => {
  const agents = childrenFixture();
  const { container, unmount } = await mount({ working: true, notice: null, band: null }, agents);
  try {
    await act(async () => useShellStore.setState({ agents: [{ ...agents[0]!, lineage_child_pane_ids: [] }] }));
    expect(container.querySelector("[data-pane-tree]")).toBeNull();
    expect(container.querySelector("[data-pane-header-band]")).toBeNull();
    expect(container.querySelector("[data-pane-working-line]")).not.toBeNull();
  } finally { await unmount(); }
});

it("opens a nested parent's raised branch using only root-owned asks within that subtree", async () => {
  const [seed] = childrenFixture();
  const leafAsk = { ...header.band!.raised!, raised_pane_id: "leaf", pane_id: "leaf", open_pane_id: "leaf", what: "Allow this nested verification" };
  const agents = [
    { ...seed!, id: "root", pane_id: "root", lineage_child_pane_ids: ["parent", "outside"], raised: [leafAsk, { ...leafAsk, raised_pane_id: "outside", pane_id: "outside", open_pane_id: "outside", what: "Unrelated branch request" }] },
    { ...seed!, lineage_parent_pane_id: "root", lineage_path_pane_ids: ["root", "parent"], lineage_child_pane_ids: ["grand"], raised: [], descendant_mark: { kind: "raised" as const, count: 1 } },
    { ...seed!, id: "grand", pane_id: "grand", lineage_parent_pane_id: "parent", lineage_path_pane_ids: ["root", "parent", "grand"], lineage_child_pane_ids: ["leaf"] },
    { ...seed!, id: "leaf", pane_id: "leaf", lineage_parent_pane_id: "grand", lineage_path_pane_ids: ["root", "parent", "grand", "leaf"], lineage_child_pane_ids: [] },
    { ...seed!, id: "outside", pane_id: "outside", lineage_parent_pane_id: "root", lineage_path_pane_ids: ["root", "outside"], lineage_child_pane_ids: [] },
  ];
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, agents);
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-tree]")!.click());
    const popup = document.querySelector('[data-agent-tree="parent"]')!;
    expect(popup.querySelector('[data-agent-child="grand"]')).not.toBeNull();
    expect(popup.querySelector('[data-agent-child="leaf"]')?.textContent).toContain("Allow this nested verification");
    expect(popup.textContent).not.toContain("Unrelated branch request");
    await act(async () => popup.querySelector<HTMLButtonElement>('[data-tree-ask-open="leaf"]')!.click());
    expect(useUiStore.getState().relation).toMatchObject({ sourcePaneId: "parent", targetPaneId: "leaf" });
  } finally { await unmount(); }
});

it.each(["parent", "child", "descendant"] as const)("preserves topology but replaces stale child states when the %s device disconnects", async (scope) => {
  const agents = childrenFixture();
  agents[0]!.descendant_mark = { kind: "raised", count: 2 };
  if (scope === "descendant") agents.push({ ...agents[1]!, id: "deep", pane_id: "deep", device_id: "mini", lineage_parent_pane_id: "working", lineage_path_pane_ids: ["parent", "working", "deep"], lineage_child_pane_ids: [] });
  else agents[scope === "parent" ? 0 : 1]!.device_id = "mini";
  const rest = { status: { pane_focus_request: null, remote: [{ target_id: "mini", state: "unreachable", message: "ssh: connect to host mini port 22: Connection refused" }] } } as unknown as SnapshotRest;
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, agents, { rest });
  try {
    const trigger = container.querySelector<HTMLButtonElement>("[data-pane-tree]")!;
    expect(trigger.getAttribute("aria-label")).toBe("Child agents 5 · Hide cannot reach mini");
    // A disconnected device's children draw no mark (status-marks B16).
    expect(trigger.querySelector("[data-mark]")).toBeNull();
    expect(container.querySelector("[data-pane-child-count]")).toBeNull();
    expect(container.querySelector('[data-pane-child-connection]')?.textContent).toBe("Hide cannot reach mini");
    await act(async () => trigger.click());
    expect(document.querySelector('[data-agent-tree="parent"]')).not.toBeNull();
    if (scope === "child") {
      const child = document.querySelector('[data-agent-child="working"]')!;
      // Its line names the device and says it is disconnected (status-marks B16).
      expect(child.getAttribute("aria-label")).toContain("mini disconnected");
      expect(child.getAttribute("aria-label")).not.toContain("Working");
    }
  } finally { await unmount(); }
});

it("keeps the child connection reason visible beside the parent's own progress", async () => {
  const agents = childrenFixture();
  agents[1]!.device_id = "mini";
  const rest = { status: { pane_focus_request: null, remote: [{ target_id: "mini", state: "unreachable", message: "ssh: connect to host mini port 22: Connection refused" }] } } as unknown as SnapshotRest;
  const { container, unmount } = await mount({ working: true, notice: null, band: null, progress: "Checking my changes" }, agents, { rest });
  try {
    expect(container.querySelector('[data-pane-own-progress]')?.textContent).toBe("Checking my changes");
    expect(container.querySelector('[data-pane-child-connection]')?.textContent).toBe("Hide cannot reach mini");
  } finally { await unmount(); }
});

it("keeps online raised requests accessible when another descendant's device is unavailable", async () => {
  const agents = childrenFixture();
  const ask = { ...header.band!.raised!, raised_pane_id: "online-ask", pane_id: "online-ask", open_pane_id: "online-ask", what: "Allow the online nested check" };
  agents[0]!.descendant_mark = { kind: "raised", count: 1 };
  agents[0]!.raised = [ask];
  agents[1]!.lineage_child_pane_ids = ["online-ask"];
  agents.find((row) => row.pane_id === "question")!.device_id = "mini";
  agents.push({ ...agents[1]!, id: "online-ask", pane_id: "online-ask", lineage_parent_pane_id: "working", lineage_path_pane_ids: ["parent", "working", "online-ask"], lineage_child_pane_ids: [] });
  const rest = { status: { pane_focus_request: null, remote: [{ target_id: "mini", state: "unreachable", message: "ssh: connect to host mini port 22: Connection refused" }] } } as unknown as SnapshotRest;
  const { container, unmount } = await mount({ working: false, notice: null, band: { ...header.band!, kind: "approval", raised: undefined, action: null, more: 1 } }, agents, { rest });
  try {
    const trigger = container.querySelector<HTMLButtonElement>("[data-pane-tree]")!;
    expect(trigger.getAttribute("aria-label")).not.toContain("Descendant requests needing you");
    expect(container.querySelector("[data-pane-band-more]")).toBeNull();
    await act(async () => trigger.click());
    const popup = document.querySelector('[data-agent-tree="parent"]')!;
    expect(popup.getAttribute("data-agent-tree-mode")).toBe("raised");
    expect(popup.querySelector('[data-agent-child="online-ask"]')?.textContent).toContain("Allow the online nested check");
    expect(popup.querySelector<HTMLButtonElement>('[data-tree-ask-open="online-ask"]')!.disabled).toBe(false);
  } finally { await unmount(); }
});

// The tree draws a waiting child's 대기 line and dims an unknown child (status-marks B16, B29).
it("draws a waiting child's line in the child tree and its name, and dims an unknown child", async () => {
  const agents = childrenFixture();
  const working = agents.find((row) => row.pane_id === "working")!;
  Object.assign(working, { status_code: "waiting", wait: "checks", activity: "stopped", state: { ...working.state, line: { text: "", mode: "waiting", tone: { kind: "subtle", read: false }, pull_request: 58 } } });
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, agents);
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-tree]")!.click());
    const popup = document.querySelector('[data-agent-tree="parent"]')!;
    const waiting = popup.querySelector('[data-agent-child="working"]')!;
    expect(waiting.querySelector('[data-tree-line="waiting"]')?.textContent).toBe("Waiting · CI · PR #58");
    // The line stands in for the status word, so the name says it once.
    expect(waiting.getAttribute("aria-label")).toBe("Child working, Claude Code, Waiting · CI · PR #58");
    const unknown = popup.querySelector('[data-agent-child="unknown"]')!;
    expect(unknown.querySelector('[data-tree-line="kept"]')?.textContent).toBe("Status unknown");
    expect(unknown.className).toContain("opacity-(--opacity-dimmed)");
    expect(waiting.className).not.toContain("opacity-(--opacity-dimmed)");
  } finally { await unmount(); }
});

// A sleeping child remains accessible in the tree after inline names leave the header.
it("names a sleeping child and its state in the tree", async () => {
  const [parent, ...all] = childrenFixture();
  const child = all.find((row) => row.pane_id === "idle")!;
  const sleeping = { ...child, mark: "asleep" as const, status_code: "sleeping" as const, state: { ...child.state, line: { text: "Split the parser", mode: "kept" as const, tone: { kind: "subtle" as const, read: true } } } };
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, [{ ...parent!, lineage_child_pane_ids: [sleeping.pane_id], direct_child_counts: { error: 0, approval: 0, question: 0, working: 0, done: 0 } }, sleeping]);
  try {
    expect(container.querySelector('[data-pane-child-name="idle"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-tree]")!.click());
    const name = document.querySelector('[data-agent-child="idle"]')!;
    expect(name.getAttribute("aria-label")).toContain("Sleeping");
  } finally { await unmount(); }
});

it("turns a missing core answer into a retryable timeout beside the move", async () => {
  const {container, unmount} = await mount();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-band-open]")!.click());
    await act(async () => vi.advanceTimersByTime(RELATION_ANSWER_TIMEOUT_MS));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("did not answer in time");
    expect(container.querySelector<HTMLButtonElement>("[data-pane-band-open]")!.disabled).toBe(false);
  } finally { await unmount(); }
});

it("leads a raised ask with its verb, then the child name and request", async () => {
  const {container, unmount} = await mount();
  try {
    const band = container.querySelector("[data-pane-header-band=raised]")!;
    const who = band.querySelector("[data-pane-band-who]")!;
    expect(who.getAttribute("data-pane-band-who")).toBe("child");
    expect(who.textContent).toBe("자식 검증(child)");
    expect(who.querySelector('[data-mark="turn"]')).toBeNull();
    const verb = band.querySelector("[data-verb=approval]")!;
    expect(verb.textContent).toBe("Approval");
    expect(verb.compareDocumentPosition(who) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(band.textContent).toContain("하위 검증 권한이 필요합니다");
    expect(band.querySelector("[data-pane-band-more]")).toBeNull();
  } finally { await unmount(); }
});

it("names the raised descendant while a draft opens its holder", async () => {
  const raised = { ...header.band!.raised!, verb: "draft" as const, what: null, raised_pane_id: "leaf", origin_title: "Leaf", pane_id: "middle", title: "Middle", open_pane_id: "middle", path: ["Root", "Middle"] };
  const {container, unmount} = await mount({ ...header, band: { ...header.band!, action: { pane_id: "middle", label: "Middle" }, raised } }, [], {paneId: "root"});
  try {
    const band = container.querySelector("[data-pane-header-band=raised]")!;
    expect(band.querySelector("[data-verb=draft]")?.textContent).toBe("Draft");
    expect(band.querySelector("[data-pane-band-who=leaf]")?.textContent).toBe("Leaf(descendant)");
    expect(band.textContent).toContain("My draft keeps the question from going through");
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-band-open]")!.click());
    expect(useUiStore.getState().relation).toMatchObject({ sourcePaneId: "root", targetPaneId: "middle" });
  } finally { await unmount(); }
});

it("marks a nested raised agent as a descendant", async () => {
  const raised = { ...header.band!.raised!, raised_pane_id: "leaf", origin_title: "Leaf", pane_id: "leaf", title: "Leaf", open_pane_id: "leaf", path: ["Root", "Middle", "Leaf"] };
  const {container, unmount} = await mount({ ...header, band: { ...header.band!, action: { pane_id: "leaf", label: "Leaf" }, raised } }, [], {paneId: "root"});
  try {
    expect(container.querySelector("[data-pane-band-who=leaf]")?.textContent).toBe("Leaf(descendant)");
  } finally { await unmount(); }
});

// B8: a raised descendant on a device that is not connected cannot be opened, and the band says why.
it("blocks Open on a raised descendant whose device is not connected and says why", async () => {
  const pane = "remote:mini:pane:w1:p2";
  const band = header.band!;
  const {container, unmount} = await mount({ ...header, band: { ...band, action: {pane_id: pane, label: "자식 검증"}, raised: { ...band.raised!, raised_pane_id: pane, pane_id: pane, open_pane_id: pane } } });
  try {
    await act(async () => useShellStore.setState({rest: {status: {pane_focus_request: null, remote: [{target_id: "mini", state: "unreachable", message: "ssh: connect to host mini port 22: Connection refused"}]}} as unknown as SnapshotRest}));
    expect(container.querySelector<HTMLButtonElement>("[data-pane-band-open]")!.disabled).toBe(true);
    expect(container.querySelector("[data-pane-band-unreachable]")?.textContent).toBe("Hide cannot reach mini");
  } finally { await unmount(); }
});

it("opens the card of the displayed repository even when another project has the same PR number", async () => {
  const {actions, events, unmount} = await mount();
  const url = "https://github.com/acme/other/pull/7";
  try {
    await act(async () => useShellStore.setState({rest: {navigator: {focused_device_id: "local", devices: [{id: "local", kind: "local"}], workspaces: [
      {id: "here", device_id: "local", pull_requests: [{number: 7, url: "https://github.com/acme/here/pull/7"}], checkouts: []},
      {id: "other", device_id: "local", pull_requests: [{number: 7, url}], checkouts: []},
    ] as unknown as Workspace[]}} as SnapshotRest}));
    await act(async () => actions.openSessionPullRequest({workspace_id: "here", number: 7, url}));
    // One event opens the card; the fixture has no Workspace in front, so the core opens the PR's (PRD sidebar-task-lens B17).
    expect(events.filter((event) => event.kind === "pr_open")).toEqual([{schema_version: 2, kind: "pr_open", payload: {workspace_id: "other", pr_number: 7, open_workspace: true}}]);
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    actions.openSessionPullRequest({workspace_id: "here", number: 7, url: "https://github.com/acme/absent/pull/7"});
    expect(open).toHaveBeenCalledWith("https://github.com/acme/absent/pull/7", "_blank", "noopener,noreferrer");
  } finally { await unmount(); }
});

// The pane header draws the Sessions row's PR mark: a parent counts its
// descendants' pull requests, a pane without children shows only its own.
function pullRow(pane: string, numbers: number[], children: string[] = [], states: PrState[] = numbers.map(() => "pending")): AgentRow {
  const seed = legacyAgentRow({ pane_id: pane, identity_label: `${pane} 세션`, lineage_child_pane_ids: children });
  const pulls: AgentPullRequest[] = numbers.map((number) => ({ number, title: `PR ${number}`, url: `https://github.com/acme/app/pull/${number}`, badge: "open", checks: "pending", head_branch: "feat/work", closing_issues: [], live: true, duty: true, created: true, settled_at_unix_ms: null }));
  return { ...seed, state: { ...seed.state, pr: numbers.length ? { count: numbers.length, worst: states[0]!, pulls: numbers.map((_, index) => ({ state: states[index]!, index })) } : null }, request: { verb: "working", verb_since_unix_ms: 0, request: null, later_by: null, pull_requests: pulls } };
}

async function mountChip(paneId: string, agents: AgentRow[]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  useShellStore.setState({ agents, rest: { status: { pane_focus_request: null } } as SnapshotRest });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PanePrChip paneId={paneId} actions={actions} /></TooltipProvider>));
  return { container, unmount: async () => { await act(async () => root.unmount()); } };
}

it("shows a parent's PR mark over its descendants as the Sessions row does, and a childless pane's own", async () => {
  const parent = pullRow("parent", [], ["child"]);
  const child = pullRow("child", [1073]);
  const { container, unmount } = await mountChip("parent", [parent, child]);
  try {
    const mark = container.querySelector<HTMLButtonElement>("[data-agent-pr=parent]")!;
    // One PR in the family is that PR's number; the digits-only form is a count of several.
    expect(mark.textContent).toBe("#" + "1073");
    expect(mark.getAttribute("aria-label")).toBe("#" + "1073 CI running or review pending");
  } finally { await unmount(); }
  const own = await mountChip("child", [parent, child]);
  try {
    expect(own.container.querySelector("[data-agent-pr=child]")?.textContent).toBe("#" + "1073");
  } finally { await own.unmount(); }
  // Several PRs are counted per state in each state's colour, worst first:
  // a mergeable PR beside a merged one is green 1 purple 1, not green 2.
  const several = await mountChip("parent", [parent, pullRow("child", [1073, 1074], [], ["mergeable", "merged"])]);
  try {
    const mark = several.container.querySelector<HTMLButtonElement>("[data-agent-pr=parent]")!;
    expect(Array.from(mark.querySelectorAll("[data-pr-count]"), (part) => [part.getAttribute("data-pr-count"), part.textContent])).toEqual([["mergeable", "1"], ["merged", "1"]]);
    expect(mark.getAttribute("aria-label")).toBe("PR 2 · Mergeable 1 · Merged 1");
  } finally { await several.unmount(); }
  const none = await mountChip("parent", [{ ...parent, lineage_child_pane_ids: [] }]);
  try {
    expect(none.container.querySelector("[data-agent-pr]")).toBeNull();
  } finally { await none.unmount(); }
});

it("leaves a header alone when a snapshot section it does not draw moves, and redraws it when its own agents or device do", async () => {
  const agents = childrenFixture();
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: [] }, ui_state: { theme: "dark" }, status: { pane_focus_request: null, remote: [] } } as unknown as SnapshotRest;
  useShellStore.setState({ agents, rest });
  const draws: Record<string, number> = { shell: 0, parent: 0 };
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  const actions = createActions(() => true);
  const plain: PaneHeader = { working: false, notice: null, band: null };
  await act(async () => root.render(<TooltipProvider>
    {["shell", "parent"].map((id) => <Profiler key={id} id={id} onRender={() => { draws[id] = (draws[id] ?? 0) + 1; }}><PaneHeaderBand paneId={id} header={plain} actions={actions} /></Profiler>)}
  </TooltipProvider>));
  try {
    expect(container.querySelector('[data-pane-tree="parent"]')).not.toBeNull();
    const settled = { ...draws };
    await act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, ui_state: { theme: "light" } } as SnapshotRest }));
    await act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, navigator: { ...useShellStore.getState().rest!.navigator, workspaces: [] } } as unknown as SnapshotRest }));
    expect(draws).toEqual(settled);
    // The device a parent's totals depend on moves: the parent redraws, a plain pane does not.
    await act(async () => useShellStore.setState({ rest: { ...useShellStore.getState().rest!, status: { pane_focus_request: null, remote: [{ target_id: "mini", state: "unreachable", message: "ssh: connect to host mini port 22: Connection refused" }] } } as unknown as SnapshotRest }));
    expect(draws.parent).toBeGreaterThan(settled.parent!);
    expect(draws.shell).toBe(settled.shell);
    // An agent row moves: every header that selects the agents is asked again.
    const before = draws.parent!;
    await act(async () => useShellStore.setState({ agents: agents.map((row) => row.pane_id === "parent" ? { ...row, lineage_child_pane_ids: row.lineage_child_pane_ids!.slice(0, 2) } : row) }));
    expect(draws.parent).toBeGreaterThan(before);
  } finally { await act(async () => root.unmount()); }
});

// PRD overview-device-tasks B22: the record button still names the agent the
// way the plain head does; only what a press does is added.
it("names the pane's title and agent product on its record button", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const agent = legacyAgentRow({ id: "a", pane_id: "pane-1", identity_label: "Fix the login", agent_kind: "claude", session_id: "s-1", device_id: "local" });
  useShellStore.setState({ agents: [agent], rest: { status: { pane_focus_request: null } } as SnapshotRest });
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneRecordButton paneId="pane-1" title="Fix the login" product="Claude Code" actions={actions} plain={<span data-plain="true" />}>Fix the login</PaneRecordButton></TooltipProvider>));
  try {
    const button = container.querySelector<HTMLButtonElement>("[data-pane-record]")!;
    const name = button.getAttribute("aria-label") ?? "";
    expect(name).toContain("Fix the login");
    expect(name).toContain("Claude Code");
    expect(container.querySelector("[data-plain]")).toBeNull();
  } finally { await act(async () => root.unmount()); }
});

// A device's rows reach the page without a device on them; the record button
// must still name the device the pane runs on, since the core keeps that
// session's ledger row under the device id and not under its own node id. A
// dialed device and a node (a machine whose own panes appear as
// `remote:<node id>:pane:...` in a window whose core runs elsewhere) are both
// such devices.
it.each([["hq3mini", "a dialed device"], ["b06a260a-6c14-5287-9caf-672044350d94", "a node"]])("opens the session record of a pane on %s (%s) under that device, not under the core's node", async (target) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const core = "14ad0319-60d7-5a7c-ae5a-f71550dc8a60";
  const pane = `remote:${target}:pane:w1:p4`;
  const agent = legacyAgentRow({ id: "a", pane_id: pane, identity_label: "Fix the login", agent_kind: "claude", session_id: "s-1" });
  expect(agent.device_id).toBeUndefined();
  useShellStore.getState().applyFrame({ type: "snapshot", payload: { revision: 1, rest: {
    navigator: { devices: [{ id: core, kind: "local" }, { id: target, kind: "remote" }] },
    status: { pane_focus_request: null, remote: [{ target_id: target, state: "connected", message: null, herdr_version: null, session: { agents: [agent] } }] },
  } as unknown as SnapshotRest } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneRecordButton paneId={pane} title="Fix the login" product="Claude Code" actions={createActions(() => true)} plain={<span />}>Fix the login</PaneRecordButton></TooltipProvider>));
  try {
    // No workspace is in front here, so the press shows the record in the Overview of the session's device.
    await act(async () => container.querySelector<HTMLButtonElement>("[data-pane-record]")!.click());
    const { deviceId, panel } = useUiStore.getState().overview;
    expect(deviceId).toBe(target);
    expect(panel).toEqual({ kind: "record", session: { device_id: target, agent: "claude", session_id: "s-1" } });
  } finally { await act(async () => root.unmount()); }
});

it("says a waiting pane's mark words on its record button, the words its tab and row say", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const agent = legacyAgentRow({ id: "a", pane_id: "pane-1", identity_label: "PR 올리고 CI 확인", agent_kind: "claude", session_id: "s-1", device_id: "local", mark: "waiting", wait: "checks" });
  useShellStore.setState({ agents: [agent], rest: { status: { pane_focus_request: null } } as SnapshotRest });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneRecordButton paneId="pane-1" title="PR 올리고 CI 확인" product="Claude Code" actions={createActions(() => true)} plain={<span />}>PR 올리고 CI 확인</PaneRecordButton></TooltipProvider>));
  try {
    expect(container.querySelector("[data-pane-record]")?.getAttribute("aria-label")).toBe("PR 올리고 CI 확인, Claude Code, In progress · Waiting, View session record");
  } finally { await act(async () => root.unmount()); }
});

// docs/UI_BEHAVIOR.md, Agent panes and the Agents explorer: the pane's own ask starts with
// its verb and says what is asked; one the operator has looked at dims and
// stays until it is answered.
it("words the pane's own ask by its verb and dims it once it is read", async () => {
  const own = (kind: PaneBand["kind"], reason: string | null, read = false): PaneHeader => ({ working: false, notice: null, band: { kind, reason, since_unix_ms: null, read, action: null, more: 0 } });
  const cases: [PaneHeader, string][] = [
    [own("approval", null), "ApprovalStopped at a permission request"],
    [own("answer", "Deploy to staging first?"), "QuestionDeploy to staging first?"],
    [own("blocked", "gh login expired"), "Blockedgh login expired"],
    [own("stopped", "Three tests left"), "UnfinishedThree tests left"],
  ];
  for (const [value, text] of cases) {
    const { container, unmount } = await mount(value, [], { paneId: "solo" });
    try {
      const row = container.querySelector(`[data-pane-header-band="${value.band!.kind}"]`)!;
      expect(row.textContent).toBe(text);
      expect(row.querySelector("[data-pane-band-read]")).toBeNull();
      expect(row.querySelector("[data-pane-band-open]")).toBeNull();
      // Only what the operator answers is drawn in the warning colour; work left is not an ask.
      const asks = value.band!.kind !== "stopped";
      expect(row.className.includes("border-warning")).toBe(asks);
      expect(row.querySelector("[data-verb]")!.className.includes("text-warning")).toBe(asks);
    } finally { await unmount(); }
  }
  const { container, unmount } = await mount(own("answer", "Deploy to staging first?", true), [], { paneId: "solo" });
  try {
    expect(container.querySelector('[data-pane-header-band="answer"] [data-pane-band-read="true"]')?.textContent).toBe("QuestionDeploy to staging first?");
  } finally { await unmount(); }
});

it("draws no second row for a pane with nothing waiting and no children", async () => {
  const { container, unmount } = await mount({ working: false, notice: null, band: null }, [], { paneId: "solo" });
  try {
    expect(container.querySelector("[data-pane-header-band]")).toBeNull();
  } finally { await unmount(); }
});

// PRD pane-last-request B1, B10, B11: the second row leads with the turn's
// request, `<request> → <answer>` with no word before it, while the core says it shows the
// turn; a request with no words counts its images.
it("leads the row with the turn's request before its ask, progress or result", async () => {
  const request = { text: "Proceed as proposed + check unclear states", images: 0 };
  const ask: PaneBand = { kind: "answer", reason: "Bundle questions 1-3 into one PR?", since_unix_ms: null, read: false, action: null, more: 0 };
  const cases: [PaneHeader, string, string][] = [
    [{ working: false, notice: null, band: ask, request }, "answer", "Proceed as proposed + check unclear states→QuestionBundle questions 1-3 into one PR?"],
    [{ working: true, notice: null, band: null, progress: "Reading the status model", request }, "progress", "Proceed as proposed + check unclear states→Reading the status model"],
    [{ working: true, notice: null, band: null, request }, "request", "Proceed as proposed + check unclear states"],
    [{ working: false, notice: null, band: null, progress: "Remote reads every 3 s, PR merged", finished: true, request }, "finished", "Proceed as proposed + check unclear states→DoneRemote reads every 3 s, PR merged"],
    [{ working: false, notice: null, band: null, finished: true, request: { text: null, images: 2 } }, "finished", "2 images→Done"],
  ];
  for (const [value, kind, text] of cases) {
    const { container, unmount } = await mount(value, [], { paneId: "solo" });
    try {
      const row = container.querySelector(`[data-pane-header-band="${kind}"]`)!;
      expect(row.textContent).toBe(text);
      // A working turn with nothing beyond its request draws the request
      // alone; every other turn has the request and the answer as its parts.
      expect([...row.querySelectorAll("[data-pane-turn-part]")].map((part) => part.getAttribute("data-pane-turn-part"))).toEqual(kind === "request" ? ["request"] : ["request", "answer"]);
      expect(row.querySelector("[data-pane-turn-request]")?.getAttribute("aria-label")).toBe(`Request: ${text.split("→")[0]}`);
      // One line, the default, cuts an ask as it cuts a quiet turn: nothing wraps over the terminal.
      expect(row.classList.contains("flex-wrap")).toBe(false);
      expect(row.querySelector('[data-pane-turn-part="request"]')!.classList.contains("max-w-2/5")).toBe(kind !== "request");
    } finally { await unmount(); }
  }
});

function turnAgent(): AgentRow {
  return legacyAgentRow({
    id: "a", pane_id: "solo", identity_label: "Unread-state marks", agent_kind: "claude", session_id: "s-1", device_id: "local",
    request: {
      verb: "answer", verb_since_unix_ms: 0, line: "Bundle questions 1-3 into one PR?", end: "question",
      request: { text: "ok do that. it should stay unread i think?\nalso check for other unclear states\nand a third line", cut: false, images: 0, at_unix_ms: 1, sender: { kind: "operator" } },
      request_summary: "Proceed as proposed + check unclear states",
      recent_turns: [
        { request: "Proposal for the white ring after finishing", end: "done", line: "Two options drafted", at_unix_ms: 2 },
        { request: "Why unread states stick", end: null, line: null, at_unix_ms: 1 },
      ],
      later_by: { kind: "named", name: "lead" },
      pull_requests: [],
    },
  } as Partial<AgentRow>);
}

// PRD pane-last-request B6-B9: the pointer or focus on the request opens the
// turn card under it, a press pins it, and Esc closes a pinned card and stops
// there; a card the pointer only shows leaves Esc to the terminal.
it("opens the turn card from the request and pins it on a press until Esc", async () => {
  const value: PaneHeader = { working: false, notice: null, band: { kind: "answer", reason: "Bundle questions 1-3 into one PR?", since_unix_ms: null, read: false, action: null, more: 0 }, request: { text: "Proceed as proposed + check unclear states", images: 0 } };
  const { container, unmount } = await mount(value, [turnAgent()], { paneId: "solo", rest: { status: { pane_focus_request: null }, terminal: { headers: { solo: value } } } as SnapshotRest });
  const card = () => document.querySelector<HTMLElement>("[data-pane-turn-card=solo]");
  const button = () => container.querySelector<HTMLButtonElement>("[data-pane-turn-request=solo]")!;
  try {
    expect(card()).toBeNull();
    await act(async () => button().dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
    expect(card()?.querySelector("[data-turn-card-summary]")?.textContent).toBe("Proceed as proposed + check unclear states");
    expect(card()?.querySelector("[data-turn-card-original]")?.textContent).toContain("ok do that.");
    expect(card()?.querySelector("[data-turn-card-later]")?.textContent).toBe("Next request from lead");
    expect(card()?.querySelector("[data-turn-card-response]")?.textContent).toBe("QuestionBundle questions 1-3 into one PR?");
    expect(card()?.querySelector("[data-turn-card-sender]")?.textContent).toBe("Me");
    // The rail reads in time order, so the older of the two turns before comes first.
    expect([...card()!.querySelectorAll("[data-turn-card-earlier-item]")].map((item) => [...item.querySelectorAll("p")].map((line) => line.textContent))).toEqual([
      ["Why unread states stick"],
      ["Proposal for the white ring after finishing", "Done · Two options drafted"],
    ]);
    expect(card()?.querySelector("[data-turn-card-record]")?.textContent).toBe("Open record");
    // A card the pointer only shows leaves Esc to the terminal.
    const passed = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => document.body.dispatchEvent(passed));
    expect(card()).not.toBeNull();
    await act(async () => { button().dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })); vi.advanceTimersByTime(200); });
    expect(card()).toBeNull();
    await act(async () => button().click());
    expect(button().hasAttribute("aria-pressed")).toBe(false);
    expect(button().getAttribute("aria-expanded")).toBe("true");
    await act(async () => { button().dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })); vi.advanceTimersByTime(200); });
    expect(card(), "a pinned card stays when the pointer leaves").not.toBeNull();
    await act(async () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(card()).toBeNull();
    await act(async () => button().focus());
    expect(card()).not.toBeNull();
    await act(async () => button().blur());
    expect(card()).toBeNull();
  } finally { await unmount(); }
});

// PRD pane-last-request B6, B9: the keyboard reaches the card's 기록 열기
// with Tab from the focused request, the card stays open while focus is in
// it, Shift+Tab goes back, Tab past its end closes it on the request, and
// the shell's Esc (its innermost layer) closes it and returns focus without
// reopening it. A card the pointer only shows is no Esc layer, so Esc stays
// the terminal's.
it("moves the keyboard into the turn card with Tab and back", async () => {
  const value: PaneHeader = { working: false, notice: null, band: { kind: "answer", reason: "Bundle questions 1-3 into one PR?", since_unix_ms: null, read: false, action: null, more: 0 }, request: { text: "Proceed as proposed + check unclear states", images: 0 } };
  const { container, unmount } = await mount(value, [turnAgent()], { paneId: "solo", rest: { status: { pane_focus_request: null }, terminal: { headers: { solo: value } } } as SnapshotRest });
  const card = () => document.querySelector<HTMLElement>("[data-pane-turn-card=solo]");
  const button = () => container.querySelector<HTMLButtonElement>("[data-pane-turn-request=solo]")!;
  const record = () => card()?.querySelector<HTMLButtonElement>("[data-turn-card-record]") ?? null;
  const key = (target: Element, init: KeyboardEventInit) => act(async () => { target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init })); });
  const layers = () => useUiStore.getState().escapeLayers;
  try {
    await act(async () => button().dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
    expect(card()).not.toBeNull();
    expect(layers(), "a hover card leaves Esc to the terminal").toHaveLength(0);
    await act(async () => { button().dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })); vi.advanceTimersByTime(200); });
    await act(async () => button().focus());
    expect(card()).not.toBeNull();
    expect(layers()).toHaveLength(1);
    await key(button(), { key: "Tab" });
    expect(document.activeElement).toBe(record());
    expect(card(), "focus inside the card keeps it open").not.toBeNull();
    await key(record()!, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(button());
    expect(card()).not.toBeNull();
    await key(button(), { key: "Tab" });
    await key(record()!, { key: "Tab" });
    expect(card(), "Tab past the end closes the card").toBeNull();
    expect(document.activeElement).toBe(button());
    await act(async () => { button().blur(); button().focus(); });
    await key(button(), { key: "Tab" });
    expect(document.activeElement).toBe(record());
    await act(async () => layers().at(-1)!());
    expect(card()).toBeNull();
    expect(document.activeElement).toBe(button());
    expect(layers()).toHaveLength(0);
  } finally { await unmount(); }
});

// The pane title opens nothing on the pointer: the turn card opens from the
// request on the second row only, and a press still opens the session record.
it("opens no card on the record button's hover", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  useShellStore.setState({ agents: [turnAgent()], rest: { status: { pane_focus_request: null } } as SnapshotRest });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PaneRecordButton paneId="solo" title="Unread-state marks" product="Claude Code" actions={createActions(() => true)} plain={<span />}>Unread-state marks</PaneRecordButton></TooltipProvider>));
  try {
    const button = container.querySelector<HTMLButtonElement>("[data-pane-record]")!;
    await act(async () => { button.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })); button.focus(); vi.advanceTimersByTime(1_000); });
    expect(document.querySelector("[data-pane-turn-card]")).toBeNull();
    expect(document.querySelector("[role=tooltip]")).toBeNull();
  } finally { await act(async () => root.unmount()); }
});

// The View menu's turn row mode (docs/UI_BEHAVIOR.md, Agent logo view mode):
// two lines stack the request over the answer, and off draws no turn while a
// parent keeps its children button and the pane's own ask rides its head.
it("stacks the turn on two lines and draws none when off, keeping the children button and the pane's own ask", async () => {
  const request = { text: "Proceed as proposed", images: 0 };
  const ask: PaneBand = { kind: "answer", reason: "Bundle into one PR?", since_unix_ms: null, read: false, action: null, more: 0 };
  const rest = (mode: "two_lines" | "off") => ({ status: { pane_focus_request: null }, ui_state: { pane_turn_row_mode: mode } }) as SnapshotRest;

  const two = await mount({ working: false, notice: null, band: ask, request }, [], { paneId: "solo", rest: rest("two_lines") });
  try {
    const row = two.container.querySelector('[data-pane-header-band="answer"]')!;
    expect(row.classList.contains("flex-col")).toBe(true);
    expect(row.textContent).toBe("Proceed as proposed→QuestionBundle into one PR?");
  } finally { await two.unmount(); }

  // A parent with no turn to show keeps one line: the children button, no empty answer under it.
  const resting = await mount({ working: false, notice: null, band: null }, childrenFixture(), { rest: rest("two_lines") });
  try {
    const row = resting.container.querySelector('[data-pane-header-band="children"]')!;
    expect([...row.querySelectorAll("[data-pane-turn-part]")].map((part) => part.getAttribute("data-pane-turn-part"))).toEqual(["request"]);
    expect(row.querySelector("[data-pane-tree]")).not.toBeNull();
  } finally { await resting.unmount(); }

  const off = await mount({ working: true, notice: null, band: null, progress: "Reading the status model", request }, [], { paneId: "solo", rest: rest("off") });
  try {
    expect(off.container.querySelector("[data-pane-header-band]")).toBeNull();
    expect(off.container.querySelector("[data-pane-working-line]")).not.toBeNull();
  } finally { await off.unmount(); }

  const parent = await mount(header, childrenFixture(), { rest: rest("off") });
  try {
    const row = parent.container.querySelector('[data-pane-header-band="children"]')!;
    expect(row.querySelector("[data-pane-tree]")).not.toBeNull();
    expect(row.querySelector("[data-pane-band-open], [data-pane-turn-request]")).toBeNull();
  } finally { await parent.unmount(); }
});

it("puts the pane's own ask on its head only while the turn row is off", async () => {
  const own: PaneHeader = { working: false, notice: null, band: { kind: "approval", reason: null, since_unix_ms: null, read: false, action: null, more: 0 }, request: { text: "Clean the build", images: 0 } };
  const draw = async (value: PaneHeader, mode: "one_line" | "off") => {
    initializeInterfaceI18n("en");
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    useShellStore.setState({ agents: [], rest: { status: { pane_focus_request: null }, ui_state: { pane_turn_row_mode: mode } } as SnapshotRest });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(<TooltipProvider><PaneAskMark paneId="solo" header={value} actions={createActions(() => true)} /></TooltipProvider>));
    const mark = container.querySelector("[data-pane-ask-mark]");
    await act(async () => root.unmount());
    return mark;
  };
  expect((await draw(own, "off"))?.getAttribute("aria-label")).toBe("Approval: Stopped at a permission request");
  expect(await draw(own, "one_line")).toBeNull();
  const left = await draw({ ...own, band: { ...own.band!, kind: "stopped", reason: "Three tests left" } }, "off");
  expect([left?.getAttribute("aria-label"), left?.className.includes("border-warning")]).toEqual(["Unfinished: Three tests left", false]);
  // A raised descendant's ask stays on the children button, not the parent's head.
  expect(await draw(header, "off")).toBeNull();
});

it("says a Conductor's waiting decision on its second line, else the nodes at work, and nothing when no node runs (PRD mission-plan-visibility B9)", async () => {
  const conductor = legacyAgentRow({ pane_id: "parent", agent_kind: "claude", session_id: "s-c", device_id: "mac", identity_label: "root", group: "working", activity: "working", status_code: "working" } as Partial<AgentRow>);
  const node = (key: string, status: string) => ({ key, title: `${key} title`, kind: "change", depends_on: [], status, blocked: null, delivery: "pr", worker: null, agent_id: null, pane_id: null, machine: null, pr: null, base_pr: null, branch: null, pr_state: null, pr_url: null, stage: null, attempts: 0, line: null, change: null, review: null, criteria: null, started_at_unix_ms: null, finished_at_unix_ms: null });
  const mission = (decision: string | null, nodes = [node("api", "working"), node("ui", "review"), node("docs", "planned")]) => ({ id: "m", title: "Ship", workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, conductor: { pane_id: "parent", name: "c", agent_kind: "claude", session: null, present: true }, nodes, merge_order: [], check: null, github: null, decision, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 2, stale: false, github_stale: false, github_read_at_unix_ms: null });
  const rest = (decision: string | null, nodes?: ReturnType<typeof node>[]) => ({ status: { pane_focus_request: null }, missions: { items: [mission(decision, nodes)] } }) as unknown as SnapshotRest;
  const running = await mount({ working: true, notice: null, band: null, progress: "Reading the inbox" }, [conductor], { rest: rest(null) });
  try {
    const line = running.container.querySelector<HTMLElement>("[data-pane-own-progress]")!;
    expect([line.dataset.paneMissionLine, line.textContent]).toEqual(["running", "Running 2 · api title and 1 more"]);
  } finally { await running.unmount(); }
  const deciding = await mount({ working: true, notice: null, band: null, progress: "Reading the inbox" }, [conductor], { rest: rest("Which base?") });
  try {
    const line = deciding.container.querySelector<HTMLElement>("[data-pane-own-progress]")!;
    expect([line.dataset.paneMissionLine, line.textContent]).toEqual(["decision", "! Which base?"]);
    expect(line.className).toContain("text-mission-hold");
  } finally { await deciding.unmount(); }
  // The Conductor's own work left gives the place to the decision; its own question does not.
  const ownBand = (kind: PaneBand["kind"]): PaneHeader => ({ working: false, notice: null, band: { kind, reason: "Waiting for the workers", since_unix_ms: null, read: false, action: null, more: 0 } });
  const stopped = await mount(ownBand("stopped"), [conductor], { rest: rest("Which base?") });
  try {
    expect(stopped.container.querySelector("[data-pane-header-band]")?.getAttribute("data-pane-header-band")).toBe("progress");
    expect(stopped.container.querySelector<HTMLElement>("[data-pane-own-progress]")?.textContent).toBe("! Which base?");
  } finally { await stopped.unmount(); }
  const asking = await mount(ownBand("answer"), [conductor], { rest: rest("Which base?") });
  try {
    expect(asking.container.querySelector("[data-pane-header-band]")?.getAttribute("data-pane-header-band")).toBe("answer");
    expect(asking.container.querySelector("[data-pane-own-progress]")).toBeNull();
  } finally { await asking.unmount(); }
  // No node at work: the mission line owns the place, so the agent's own progress does not stand in.
  const resting = await mount({ working: true, notice: null, band: null, progress: "Reading the inbox" }, [conductor], { rest: rest(null, [node("api", "done"), node("docs", "planned")]) });
  try {
    expect(resting.container.querySelector("[data-pane-own-progress]")).toBeNull();
    expect(resting.container.textContent).not.toContain("Reading the inbox");
  } finally { await resting.unmount(); }
});
