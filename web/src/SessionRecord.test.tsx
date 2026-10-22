// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { SessionRecord, SlotSessionRecord } from "./SessionRecord";
import type { MissedWork, RecordStatus, SessionRecord as RecordValue, SnapshotRest, WorkFlow, WorkOutput, WorkRequest, WorkSession } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { REFERENCE_FOLDS, sidebarScene } from "./gallery/sceneData";

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  vi.useRealTimers();
  document.body.innerHTML = "";
});

const AT = 1_790_000_000_000;
const MIN = 60_000;
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const request = (offset: number, text: string, patch: Partial<WorkRequest> = {}): WorkRequest => ({ offset, at_unix_ms: AT + offset * MIN, text, author: "operator", images: 0, summary: null, answers: null, letter_from: null, count: 1, goal_command: false, ...patch });
const ask = (id: string, text: string): MissedWork => ({ id, kind: "ask", text, status: "open", at_unix_ms: AT, turn_offset: 1, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const output = (number: number, state: string, source: WorkSession): WorkOutput => ({ kind: "pr", repository: "acme/app", number, title: `PR title ${number}`, url: `https://example.invalid/pull/${number}`, state, stale: false, last_read_at_unix_ms: AT, created_at_unix_ms: AT + number, source, source_name: source.session_id });
const flow = (id: string, name: string, at: number, requests: WorkRequest[], result: WorkFlow["result"] = null): WorkFlow => ({ id, name, at_unix_ms: at, requests, request_count: requests.filter((one) => !one.letter_from).length, result });
const status = (end: RecordStatus["end"], line: string, limited: string | null = null): RecordStatus => ({ end, line, at_unix_ms: AT + 20 * MIN, limited });
const EMPTY = { status: null, asks: [], flows: [], omitted_requests: 0, outputs: [], pr_groups: [], artifacts: [], children: [] };
const child: RecordValue = { ...EMPTY, session: CHILD, title: "Child work", fallback_title: "Codex", native_title: null, manual_title: null, state: "idle", cwd: null, path: null, started_at_unix_ms: AT + 2, ended_at_unix_ms: null, status: status("done", "Report sent") };
const BASE: RecordValue = {
  ...EMPTY,
  session: SESSION, title: "Session goal", fallback_title: "Claude", native_title: null, manual_title: null, state: "idle", cwd: null, path: null, started_at_unix_ms: AT, ended_at_unix_ms: null,
  status: status("done", "Record written"),
  flows: [
    flow("flow-1", "Board drawing", AT, [request(1, "Draw the record board", { summary: "Draw the record board" })], { end: "done", line: "Board drawn", at_unix_ms: AT + 5 * MIN }),
    flow("flow-2", "Record PRD", AT + 10 * MIN, [request(10, "Write the PRD now", { summary: "Write the PRD" }), request(11, "How is it going?", { summary: "Question: how far along" })]),
  ],
  outputs: [output(1100, "review", SESSION), output(1093, "merged", CHILD)],
  pr_groups: [{ child: null, name: "Session goal", outputs: [output(1100, "review", SESSION)] }, { child: CHILD, name: "Child work", outputs: [output(1093, "merged", CHILD)] }],
  children: [child],
};

async function mount(record: RecordValue | null, options: { summaries?: boolean; rest?: SnapshotRest; overview?: boolean; locator?: { request_offset: number; query?: string } } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }] }, status: { background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: options.summaries !== false, chosen: true, agent_summary: options.summaries !== false } } } as unknown as SnapshotRest;
  useShellStore.setState({ rest: options.rest ?? rest, sessionRecord: record ? { session: SESSION, loading: false, failure: null, record } : null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const sent: { kind: string; payload: unknown }[] = [];
  const actions = createActions((event) => { sent.push(event as { kind: string; payload: unknown }); return true; });
  const showTrail = async (trail: readonly WorkSession[] = []) => {
    await act(async () => root.render(<TooltipProvider>{options.overview
      ? <SessionRecord actions={actions} target={SESSION} frame={{ kind: "overview", trail, locator: options.locator, onTrail: () => undefined, onRead: () => undefined, onLeave: () => undefined }} />
      : <SlotSessionRecord actions={actions} target={SESSION} />}</TooltipProvider>));
  };
  await showTrail();
  return { container, sent, showTrail, unmount: async () => { await act(async () => root.unmount()); } };
}

const text = (element: Element | null | undefined) => element?.textContent ?? "";

const flowOf = (container: Element, id: string) => container.querySelector(`[data-record-flow="${id}"]`)!;
const toggleFlow = async (container: Element, id: string) => act(async () => container.querySelector<HTMLButtonElement>(`[data-record-flow-toggle="${id}"]`)!.click());

async function expand(container: Element, id = "child") {
  await act(async () => container.querySelector<HTMLButtonElement>(`[data-record-child="${id}"] > button`)!.click());
}

it("draws no task checklist, no findings tab and no tabs at all (B2)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(container.querySelector('[role="tab"], [data-record-task], [data-record-work]')).toBeNull();
    expect(text(container)).not.toMatch(/Requested work|Findings/);
    const order = ["[data-record-now]", "[data-record-flows]", "[data-record-prs]", "[data-record-delegated]"].map((selector) => container.querySelector(selector)!);
    for (let index = 1; index < order.length; index += 1) expect(order[index - 1]!.compareDocumentPosition(order[index]!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  } finally { await unmount(); }
});

it("opens the current flow and folds a past one to its result, which a click opens (B3, B13)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    const past = flowOf(container, "flow-1");
    expect(text(past.querySelector("[data-record-flow-name]"))).toBe("Board drawing");
    expect(text(past.querySelector("[data-record-flow-result]"))).toContain("Board drawn");
    expect(past.querySelector("[data-record-request]")).toBeNull();
    expect(past.querySelector("[data-record-flow-toggle]")!.getAttribute("aria-expanded")).toBe("false");
    const current = flowOf(container, "flow-2");
    expect(current.getAttribute("data-current")).toBe("true");
    expect(current.querySelectorAll("[data-record-request]")).toHaveLength(2);
    expect(current.querySelector("[data-record-flow-result]")).toBeNull();
    await toggleFlow(container, "flow-1");
    expect(past.querySelectorAll("[data-record-request]")).toHaveLength(1);
  } finally { await unmount(); }
});

it("draws a request as its summary and opens the words as sent in place, or cuts unsummarized words at 120 characters (B5)", async () => {
  const long = `${"긴 요청 ".repeat(40)}\n끝`;
  const record = { ...BASE, flows: [flow("flow-1", "Only", AT, [request(1, "Please write the record with every section", { summary: "Write the record" }), request(2, long)])] };
  const { container, unmount } = await mount(record);
  try {
    const summarized = container.querySelector('[data-record-request="1"]')!;
    expect(text(summarized)).toContain("Write the record");
    expect(summarized.querySelector("[data-record-request-raw]")).toBeNull();
    await act(async () => summarized.querySelector<HTMLButtonElement>("button")!.click());
    expect(text(summarized.querySelector("[data-record-request-raw]"))).toBe("Please write the record with every section");
    const raw = container.querySelector('[data-record-request="2"] button span[title]')!;
    expect(raw.getAttribute("title")!.endsWith("…")).toBe(true);
    expect([...raw.getAttribute("title")!].length).toBe(121);
    expect(raw.getAttribute("title")).not.toContain("\n");
  } finally { await unmount(); }
});

it("marks a reply with the question it answers, a letter dim under its sender, a repeat, an image count and a /goal (B7, B15, B23-B25)", async () => {
  const record = { ...BASE, flows: [flow("flow-1", "Only", AT, [
    request(1, "1", { summary: "Plan A", answers: "Which plan" }),
    request(2, "🦉 letter from impl-a", { letter_from: "impl-a" }),
    request(3, "Rerun it", { summary: "Rerun", count: 2 }),
    request(4, "Look at this", { summary: "Check this screen", images: 2 }),
    request(5, "Ship before release", { summary: "Ship before release", goal_command: true }),
  ])] };
  const { container, unmount } = await mount(record);
  try {
    expect(text(container.querySelector('[data-record-request="1"] [data-record-answers]'))).toBe("↩ Which plan:");
    const letter = container.querySelector('[data-record-request="2"]')!;
    expect(letter.getAttribute("data-record-letter")).toBe("impl-a");
    expect(text(letter)).toContain("impl-a");
    expect(text(flowOf(container, "flow-1").querySelector("[data-record-flow-name] + span"))).not.toContain("5");
    expect(text(container.querySelector('[data-record-request="3"] [data-record-repeat]'))).toBe("×2");
    expect(text(container.querySelector('[data-record-request="4"] [data-record-images]'))).toBe("2 images");
    expect(container.querySelector('[data-record-request="5"] [data-record-goal]')).not.toBeNull();
  } finally { await unmount(); }
});

it("lists what waits on the operator, puts one away with Skip and brings it back with Undo, and draws nothing when none waits (B11, B12)", async () => {
  const { container, sent, unmount } = await mount({ ...BASE, asks: [ask("ask-1", "Approve the PRD?")] });
  try {
    expect(text(container.querySelector("[data-record-asks]"))).toContain("Approve the PRD?");
    sent.length = 0;
    await act(async () => container.querySelector<HTMLButtonElement>('[data-record-ask-dismiss="ask-1"]')!.click());
    expect(sent.map((event) => event.payload)).toEqual([{ session: SESSION, id: "ask-1", status: "dismissed" }]);
    expect(container.querySelector('[data-record-ask="ask-1"]')!.getAttribute("data-dismissed")).toBe("true");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-record-ask-undo="ask-1"]')!.click());
    expect((sent.at(-1)!.payload as { status: string }).status).toBe("open");
  } finally { await unmount(); }
  const none = await mount(BASE);
  try { expect(none.container.querySelector("[data-record-asks]")).toBeNull(); } finally { await none.unmount(); }
});

it("shows the last real turn on top with a usage limit as a badge, and blocked when every turn hit it (B10)", async () => {
  const limited = await mount({ ...BASE, status: status("question", "Only the rehearsal is left", "Limit reached · resets 13:00") });
  try {
    const now = limited.container.querySelector("[data-record-now]")!;
    expect(now.getAttribute("data-record-now")).toBe("question");
    expect(text(now)).toContain("Only the rehearsal is left");
    expect(text(now.querySelector("[data-record-limited]"))).toBe("Limit reached · resets 13:00");
  } finally { await limited.unmount(); }
  const all = await mount({ ...BASE, status: status("blocked", "Usage limit", "Usage limit") });
  try { expect(all.container.querySelector("[data-record-now]")!.getAttribute("data-record-now")).toBe("blocked"); } finally { await all.unmount(); }
});

it("groups descendants' PRs under the direct child and folds merged and closed ones into one line (B18, B19)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(container.querySelector('[data-record-pr-group="own"] [data-record-pr-row="1100"]')).not.toBeNull();
    const childGroup = container.querySelector('[data-record-pr-group="child"]')!;
    expect(text(childGroup)).toContain("Child work");
    expect(childGroup.querySelector('[data-record-pr-row="1093"]')).toBeNull();
    const fold = container.querySelector<HTMLButtonElement>("[data-record-settled]")!;
    expect(text(fold)).toContain("1 merged or closed");
    await act(async () => fold.click());
    expect(childGroup.querySelector('[data-record-pr-row="1093"]')).not.toBeNull();
  } finally { await unmount(); }
});

it("lists outputs other than PRs once, and no line without them (B22)", async () => {
  const shown = await mount({ ...BASE, artifacts: ["Record PRD", "record-cases.html"] });
  try { expect([...shown.container.querySelectorAll("[data-record-artifact]")].map(text)).toEqual(["Record PRD", "record-cases.html"]); } finally { await shown.unmount(); }
  const none = await mount(BASE);
  try { expect(none.container.querySelector("[data-record-artifacts]")).toBeNull(); } finally { await none.unmount(); }
});

it("draws the raw request list and the PRs only with summaries off, saying so once (B28)", async () => {
  const { container, unmount } = await mount({ ...BASE, asks: [ask("ask-1", "Approve?")], artifacts: ["Record PRD"] }, { summaries: false });
  try {
    expect(container.querySelector("[data-record-flows], [data-record-asks], [data-record-artifacts], [data-record-limited]")).toBeNull();
    expect(container.querySelectorAll("[data-record-raw-requests] [data-record-request]")).toHaveLength(3);
    expect(text(container.querySelector('[data-record-request="10"]'))).toContain("Write the PRD now");
    expect(text(container).split("AI summaries off")).toHaveLength(2);
    expect(container.querySelector('[data-record-pr-row="1100"]')).not.toBeNull();
  } finally { await unmount(); }
});

it("says each child's state from its last real turn, and nothing judged with summaries off (B26)", async () => {
  const record = { ...BASE, children: [{ ...child, status: status("waiting", "Waiting for CI") }] };
  const normal = await mount(record);
  try { expect(text(normal.container.querySelector("[data-record-child-state]"))).toBe("Waiting"); } finally { await normal.unmount(); }
  const off = await mount(record, { summaries: false });
  try { expect(text(off.container.querySelector("[data-record-child-state]"))).toBe("Unavailable"); } finally { await off.unmount(); }
});

it("says how many old requests the record no longer shows (B30)", async () => {
  const { container, unmount } = await mount({ ...BASE, omitted_requests: 37 });
  try { expect(text(container.querySelector("[data-record-omitted]"))).toBe("37 older requests are not shown"); } finally { await unmount(); }
});

it("opens the past flow holding a History hit and shows its words with the query marked (B32)", async () => {
  // jsdom lays nothing out, so it has no scrollIntoView.
  Element.prototype.scrollIntoView = vi.fn();
  const { container, unmount } = await mount(BASE, { overview: true, locator: { request_offset: 1, query: "board" } });
  try {
    expect(flowOf(container, "flow-1").querySelector("[data-record-flow-toggle]")!.getAttribute("aria-expanded")).toBe("true");
    const hit = container.querySelector('[data-record-request-match="true"]')!;
    expect(hit.getAttribute("data-record-request")).toBe("1");
    expect(text(hit.querySelector("[data-record-request-raw]"))).toBe("Draw the record board");
    expect(hit.querySelector("[data-record-request-raw] mark")).not.toBeNull();
  } finally {
    await unmount();
    delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  }
});

it("asks for the record once when the slot draws one nothing has read, as after a relaunch, and not when it is read", async () => {
  const restored = await mount(null);
  try {
    expect(restored.sent).toEqual([{ schema_version: 2, kind: "detail_open", payload: { item: { kind: "record", session: SESSION } } }]);
    expect(restored.container.querySelector("[data-record-loading]")).not.toBeNull();
  } finally { await restored.unmount(); }
  const read = await mount(BASE);
  try { expect(read.sent).toEqual([]); } finally { await read.unmount(); }
});

it("asks again when the window lands on a new core session while the slot still draws the record", async () => {
  const restored = await mount(null);
  try {
    expect(restored.sent).toHaveLength(1);
    // The same slot, a self-contained snapshot of a core that has read nothing.
    await act(async () => useShellStore.setState({ viewGeneration: useShellStore.getState().viewGeneration + 1, sessionRecord: null }));
    expect(restored.sent).toHaveLength(2);
    await act(async () => useShellStore.setState({ sessionRecord: { session: SESSION, loading: true, failure: null, record: null } }));
    expect(restored.sent).toHaveLength(2);
  } finally { await restored.unmount(); }
});

it("expands a child in place and opens its full record only with the explicit action", async () => {
  const { container, sent, unmount } = await mount(BASE);
  try {
    sent.length = 0;
    await expand(container);
    expect(sent).toEqual([]);
    const open = Array.from(container.querySelectorAll<HTMLButtonElement>('[data-record-child-detail="child"] button')).find((node) => text(node) === "Open record")!;
    await act(async () => open.click());
    expect(sent.map((event) => event.kind)).toEqual(["detail_open"]);
    expect(sent[0]!.payload).toEqual({ item: { kind: "record", session: CHILD } });
    expect(container.querySelector("[data-record-up], [data-record-back]")).toBeNull();
  } finally { await unmount(); }
});

it("gives a PR button the last read time while GitHub cannot be read", async () => {
  const stale = { ...output(1100, "review", SESSION), stale: true, last_read_at_unix_ms: AT };
  const { container, unmount } = await mount({ ...BASE, outputs: [stale], pr_groups: [{ child: null, name: "Session goal", outputs: [stale] }] });
  try {
    expect(container.querySelector("[data-record-output]")!.getAttribute("aria-label")).toContain(new Date(AT).toLocaleString());
  } finally { await unmount(); }
});

it("shows an ended session with elapsed time and no terminal button or duplicate action box", async () => {
  const { container, unmount } = await mount({ ...BASE, ended_at_unix_ms: AT });
  try {
    const now = container.querySelector("[data-record-now]")!;
    expect(now.getAttribute("data-record-now")).toBe("ended");
    expect(now.getAttribute("data-state-box")).toBeNull();
    expect(text(now)).toContain("Done");
    expect(now.querySelector("[data-record-now-button]")).toBeNull();
  } finally { await unmount(); }
});

it("holds one restore in flight and keeps a late launch failure on the record for retry", async () => {
  const { rest } = sidebarScene("reference", REFERENCE_FOLDS, AT);
  const checkout = rest.navigator!.workspaces!.find(row => row.id === "hide")!.checkouts[0]!;
  useShellStore.setState({ connection: "live", agents: [] });
  useUiStore.setState({ overviewOpen: true, focusWhenListed: null });
  const view = await mount({ ...BASE, cwd: checkout.path, path: "/fixture/conversation.jsonl" }, { rest, overview: true });
  try {
    const restore = () => view.container.querySelector<HTMLButtonElement>('[data-record-restore-button]')!;
    await act(async () => { restore().click(); restore().click(); });
    const starts = () => view.sent.filter(event => event.kind === "agent_start_in_checkout");
    expect(starts()).toHaveLength(1);
    expect(useUiStore.getState().overviewOpen).toBe(true);
    expect(restore().disabled).toBe(true);
    expect(text(restore())).toBe("Restoring…");
    const requestId = (starts()[0]!.payload as { request_id: string }).request_id;
    const operation = { id: 42, kind: "agent_start" as const, phase: "ready" as const, repository_root: checkout.path, branch: null, base_branch: null, path: checkout.path, pane_id: "resumed", agent_kind: "claude", message: null, agent_phase: "starting", agent_message: null, request_id: requestId };
    await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: operation } }));
    expect(restore().disabled).toBe(true);
    const failed = { ...operation, agent_phase: "failed", agent_message: "agent_not_found" };
    await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: failed } }));
    expect(restore().disabled).toBe(false);
    expect(text(view.container.querySelector('[role="alert"]'))).toContain("Retry");
    expect(useUiStore.getState().focusWhenListed).not.toBe("resumed");
    const retry = Array.from(view.container.querySelectorAll<HTMLButtonElement>('button')).find(button => text(button) === "Retry")!;
    await act(async () => retry.click());
    expect(starts()).toHaveLength(2);
    expect((starts()[1]!.payload as { request_id: string }).request_id).toBe(requestId);
    expect(view.container.querySelector('[role="alert"]')).toBeNull();
    expect(restore().disabled).toBe(true);
    await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: failed, status: { ...rest.status, last_error: { kind: "fixture_refusal", retryable: true, occurred_at: AT, request_id: "another-request", message: "Unrelated refusal" } } } }));
    expect(restore().disabled).toBe(true);
    expect(view.container.querySelector('[role="alert"]')).toBeNull();
    await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: { ...operation, agent_phase: "unknown" } } }));
    expect(restore().disabled).toBe(false);
    expect(text(restore())).toBe("Check start status");
    expect(useUiStore.getState().focusWhenListed).toBeNull();
    expect(useUiStore.getState().overviewOpen).toBe(true);
    await act(async () => restore().click());
    expect(starts()[2]!.payload).toEqual({ ...(starts()[0]!.payload as object), confirm_only: true });
    expect(restore().disabled).toBe(true);
    await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: { ...operation, agent_phase: "started" } } }));
    expect(useUiStore.getState().focusWhenListed).toBe("resumed");
  } finally { await view.unmount(); }
});

it("checks the same restore after a timeout instead of launching another intent", async () => {
  const { rest } = sidebarScene("reference", REFERENCE_FOLDS, AT);
  const checkout = rest.navigator!.workspaces!.find(row => row.id === "hide")!.checkouts[0]!;
  useShellStore.setState({ connection: "live", agents: [] });
  const view = await mount({ ...BASE, cwd: checkout.path, path: "/fixture/conversation.jsonl" }, { rest });
  try {
    vi.useFakeTimers();
    const restore = () => view.container.querySelector<HTMLButtonElement>('[data-record-restore-button]')!;
    await act(async () => restore().click());
    await act(async () => vi.advanceTimersByTime(90_000));
    expect(restore().disabled).toBe(false);
    expect(text(restore())).toBe("Check start status");
    await act(async () => restore().click());
    const starts = () => view.sent.filter(event => event.kind === "agent_start_in_checkout");
    expect(starts()).toHaveLength(2);
    expect(starts()[1]!.payload).toEqual({ ...(starts()[0]!.payload as object), confirm_only: true });
    const requestId = (starts()[0]!.payload as { request_id: string }).request_id;
    await act(async () => useShellStore.setState({ rest: { ...rest, status: { ...rest.status, last_error: { kind: "agent_start.confirmation_unavailable", retryable: true, occurred_at: AT, request_id: requestId, message: "The saved receipt is unavailable" } } } }));
    expect(text(restore())).toBe("Check start status");
    expect(restore().disabled).toBe(false);
    expect(text(view.container.querySelector('[role="alert"]'))).toBe("The saved receipt is unavailable");
    await act(async () => restore().click());
    expect(starts()).toHaveLength(3);
    expect(starts()[2]!.payload).toEqual(starts()[1]!.payload);
  } finally { await view.unmount(); }
});

it("keeps a parent's failed restore out of a child opened in the Overview record", async () => {
  const { rest } = sidebarScene("reference", REFERENCE_FOLDS, AT);
  const checkout = rest.navigator!.workspaces!.find(row => row.id === "hide")!.checkouts[0]!;
  useShellStore.setState({ connection: "live", agents: [] });
  const view = await mount({ ...BASE, cwd: checkout.path, path: "/fixture/root.jsonl", children: [{ ...child, cwd: checkout.path, path: "/fixture/child.jsonl" }] }, { rest, overview: true });
  try {
    const restore = () => view.container.querySelector<HTMLButtonElement>('[data-record-restore-button]')!;
    await act(async () => restore().click());
    const starts = () => view.sent.filter(event => event.kind === "agent_start_in_checkout");
    const requestId = (starts()[0]!.payload as { request_id: string }).request_id;
    await act(async () => useShellStore.setState({ rest: { ...rest, status: { ...rest.status, last_error: { kind: "fixture_refusal", retryable: true, occurred_at: AT, request_id: requestId, message: "Root restore refused" } } } }));
    expect(text(view.container.querySelector('[role="alert"]'))).toContain("Root restore refused");
    await view.showTrail([CHILD]);
    expect(view.container.querySelector('[role="alert"]')).toBeNull();
    expect(restore().disabled).toBe(false);
    await act(async () => restore().click());
    expect(starts()[1]!.payload).toMatchObject({ provider: "codex", resume_session_id: "child", resume_session_path: "/fixture/child.jsonl" });
    expect((starts()[1]!.payload as { request_id: string }).request_id).not.toBe(requestId);
  } finally { await view.unmount(); }
});

it("does not restore a removed nested checkout in its still registered parent folder", async () => {
  const { rest } = sidebarScene("reference", REFERENCE_FOLDS, AT);
  const project = rest.navigator!.workspaces!.find(row => row.id === "hide")!;
  const parent = project.checkouts[0]!;
  const removed = { ...parent, id: "removed", path: `${parent.path}/removed`, label: "Removed", exists: false };
  project.checkouts.push(removed);
  useShellStore.setState({ connection: "live", agents: [] });
  const view = await mount({ ...BASE, cwd: `${removed.path}/web`, path: "/fixture/conversation.jsonl" }, { rest });
  try {
    expect(view.container.querySelector<HTMLButtonElement>('[data-record-restore-button]')!.disabled).toBe(true);
    expect(text(view.container.querySelector('[data-record-restore-unavailable]'))).toContain("worktree was cleaned up");
    expect(text(view.container.querySelector('[data-record-checkout="removed"]'))).toContain("Removed");
  } finally { await view.unmount(); }
});
