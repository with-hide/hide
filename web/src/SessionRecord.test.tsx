// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { SessionRecord, SlotSessionRecord } from "./SessionRecord";
import type { AgentRow, MissedWork, SessionRecord as RecordValue, SnapshotRest, WorkOutput, WorkRequest, WorkSession, WorkTask } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { legacyAgentRow } from "../test/legacyAgentRow";
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
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const task = (id: string, name: string, at: number, done: boolean, end: WorkTask["end"] = "done"): WorkTask => ({ id, chapter_id: "c", name, at_unix_ms: at, last_at_unix_ms: at, end, done, unjudged: false });
const request = (offset: number, text: string, task_ids: string[], author: WorkRequest["author"] = "operator", judged = true): WorkRequest => ({ offset, at_unix_ms: AT + offset, text, author, task_ids, chapter_id: "c", judged });
const missed = (id: string, text: string, turn_offset: number, status: MissedWork["status"] = "open"): MissedWork => ({ id, kind: "problem", text, status, at_unix_ms: AT, turn_offset, issue_repository: null, issue_number: null, creating: false, backlog_key: null });
const output = (number: number, state: string, task_id: string | null, source: WorkSession): WorkOutput => ({ kind: "pr", repository: "acme/app", number, title: `PR title ${number}`, url: `https://example.invalid/pull/${number}`, state, stale: false, last_read_at_unix_ms: AT, created_at_unix_ms: AT, task_id, source, source_name: source.session_id });
const child: RecordValue = { session: CHILD, title: "Child work", fallback_title: "Codex", native_title: null, manual_title: null, state: "idle", last_end: "done", cwd: null, path: null, started_at_unix_ms: AT + 2, ended_at_unix_ms: null, chapters: [], tasks: [], requests: [], missed: [], outputs: [], children: [] };
const BASE: RecordValue = {
  session: SESSION, title: "Session goal", fallback_title: "Claude", native_title: null, manual_title: null, state: "idle", last_end: "unfinished", cwd: null, path: null, started_at_unix_ms: AT, ended_at_unix_ms: null,
  chapters: [{ id: "c", goal: "Only goal", at_unix_ms: AT }],
  tasks: [task("t1", "Write the record", AT + 1, true), task("t2", "Check the search", AT + 2, false, "unfinished")],
  requests: [request(1, "Please write the record", ["t1"]), request(2, "Check the search too", ["t2"]), request(3, "How is it going?", [])],
  missed: [missed("m-open", "Retry the failed draft", 2), missed("m-closed", "Spacing of the heading", 2, "resolved")],
  // The child's output carries the parent's task id: ids name turn offsets, which repeat across sessions.
  outputs: [output(1100, "review", "t2", SESSION), output(1093, "merged", "t2", CHILD)],
  children: [child],
};

async function mount(record: RecordValue | null, options: { summaries?: boolean; rest?: SnapshotRest; overview?: boolean } = {}) {
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
      ? <SessionRecord actions={actions} target={SESSION} frame={{ kind: "overview", trail, onTrail: () => undefined, onRead: () => undefined, onLeave: () => undefined }} />
      : <SlotSessionRecord actions={actions} target={SESSION} />}</TooltipProvider>));
  };
  await showTrail();
  return { container, sent, showTrail, unmount: async () => { await act(async () => root.unmount()); } };
}

const text = (element: Element | null | undefined) => element?.textContent ?? "";

it("uses the core's child completion facts and never reuses an old END for an unjudged child", async () => {
  const unfinished = { ...child, last_end: "done" as const, tasks: [task("child-task", "Deliver report", AT, false, "waiting")] };
  for (const [progress, expected] of [["done", "Done"], ["unjudged", "Unavailable"], ["other", "Not done"]] as const) {
    const record: RecordValue = { ...BASE, children: [unfinished], child_progress: { children: [progress], done: progress === "unjudged" ? null : Number(progress === "done") } };
    const view = await mount(record);
    try { expect(text(view.container.querySelector("[data-record-child-state]"))).toBe(expected); } finally { await view.unmount(); }
    const off = await mount(record, { summaries: false });
    try { expect(text(off.container.querySelector("[data-record-child-state]"))).toBe("Unavailable"); } finally { await off.unmount(); }
  }
});

it("leaves an unjudged task without an old verdict and keeps a completed task's live question", async () => {
  const view = await mount({ ...BASE, outputs: [], tasks: [{ ...task("unknown", "Not judged", AT, false, "unfinished"), unjudged: true }, task("finished", "Delivered", AT, true, "question")] });
  try {
    expect(text(view.container.querySelector('[data-record-task="unknown"]'))).toBe("Not judged");
    expect(view.container.querySelector('[data-record-task="unknown"] [data-task-box="empty"]')).not.toBeNull();
    await act(async () => useShellStore.setState({ agents: [legacyAgentRow({ pane_id: "w1:p1", session_id: SESSION.session_id, agent_kind: SESSION.agent, status_code: "question", mark: "turn" }) as AgentRow] }));
    expect(view.container.querySelector('[data-record-task="finished"] [data-task-box="checked"]')).not.toBeNull();
    expect(view.container.querySelector('[data-record-task="finished"] [data-task-asks]')).not.toBeNull();
  } finally { await view.unmount(); }
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

async function tab(container: Element, name: string) {
  const trigger = Array.from(container.querySelectorAll<HTMLButtonElement>('[role="tab"]')).find((node) => text(node).startsWith(name))!;
  await act(async () => trigger.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
}

async function expand(container: Element, id = "child") {
  await act(async () => container.querySelector<HTMLButtonElement>(`[data-record-child="${id}"] > button`)!.click());
}

it("keeps the current session's PRs in its tab and each child's PRs in that child's detail", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    const tasks = Array.from(container.querySelectorAll("[data-record-task]"));
    expect(tasks.map((row) => row.getAttribute("data-record-task"))).toEqual(["t1", "t2"]);
    expect(tasks.map((row) => row.querySelector("[data-task-box]")?.getAttribute("data-task-box"))).toEqual(["checked", "empty"]);
    expect(tasks[1]!.querySelector("[data-record-output]")).toBeNull();
    await tab(container, "PR");
    expect(Array.from(container.querySelectorAll('[data-record-output="pr"]')).map((node) => node.getAttribute("data-record-output-number"))).toEqual(["1100"]);
    expect(text(container.querySelector('[data-record-child="child"]'))).toContain("Codex · Done· PR 1");
    await expand(container);
    expect(container.querySelector('[data-record-child-detail="child"] [data-record-output-number="1093"]')).not.toBeNull();
    expect(container.querySelector('[data-record-child-detail="child"] [data-record-output-number="1100"]')).toBeNull();
  } finally { await unmount(); }
});

it("shows task progress, the full original request, and a flat child list after the tabs", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(text(container.querySelector("[data-record-work] h3"))).toBe("Requested work· 1 / 2 done");
    expect(text(container.querySelector('[data-record-task="t2"] [data-record-task-request]'))).toContain("Check the search too");
    expect(container.querySelector('[data-record-task] [data-record-child]')).toBeNull();
    expect(text(container.querySelector('[data-record-delegated] h3'))).toBe("Sessions working together1");
    expect(text(container.querySelector('[data-record-child-state]'))).toBe("Done");
    expect(text(container.querySelector("[data-record-place]"))).toContain("Claude Code");
  } finally { await unmount(); }
});

it("shows only open findings and keeps handled findings out of the card", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(Array.from(container.querySelectorAll("[data-missed-work]")).map((node) => node.getAttribute("data-missed-work"))).toEqual(["m-open"]);
    expect(text(container.querySelector("[data-record-missed]"))).toContain("ProblemRetry the failed draft");
    expect(container.textContent).not.toMatch(/Undo|Reopen|Handled/);
  } finally { await unmount(); }
  const handled = await mount({ ...BASE, missed: [missed("m-done", "Done already", 2, "backlog")] });
  try { expect(handled.container.querySelector("[data-missed-work]")).toBeNull(); } finally { await handled.unmount(); }
});

it("keeps local YYYY-MM-DD day boundaries when the four-turn fold splits a day", async () => {
  // Approved date grouping: a year boundary and a folded partial day retain their dates.
  const at = (day: number, hour: number, minute: number) => new Date(2026, 11, day, hour, minute).getTime();
  const requests = [at(30, 12, 0), at(31, 12, 0), at(31, 23, 59), at(32, 0, 0), at(32, 0, 1), at(32, 0, 2)].map((at_unix_ms, i) => ({ ...request(i, `original request ${i}`, []), at_unix_ms }));
  const view = await mount({ ...BASE, requests: [...requests].reverse() }, { summaries: false });
  try {
    expect(text(view.container.querySelector("[data-record-raw-request] time"))).toBe("2027-01-01 00:02");
    await tab(view.container, "Request history");
    const days = () => Array.from(view.container.querySelectorAll("[data-record-date]")).map(text);
    expect(days()).toEqual(["2026-12-31", "2027-01-01"]);
    expect(Array.from(view.container.querySelectorAll("[data-record-request] time")).map(text)).toEqual(["23:59", "00:00", "00:01", "00:02"]);
    await act(async () => view.container.querySelector<HTMLButtonElement>("[data-record-words-earlier]")!.click());
    expect(days()).toEqual(["2026-12-30", "2026-12-31", "2027-01-01"]);
  } finally { await view.unmount(); }
});

it("opens request history near the top, preserves full text, and expands the newest four to the full history", async () => {
  const requests = Array.from({ length: 6 }, (_, index) => request(index + 1, `request ${index + 1}\nmore`, []));
  const { container, unmount } = await mount({ ...BASE, requests });
  try {
    await tab(container, "Request history");
    const section = container.querySelector("[data-record-words]")!;
    const rows = () => Array.from(section.querySelectorAll("[data-record-request]"));
    expect(rows()).toHaveLength(4);
    expect(text(rows()[0])).toContain("request 3\nmore");
    await act(async () => section.querySelector<HTMLButtonElement>("[data-record-words-earlier]")!.click());
    expect(rows()).toHaveLength(6);
  } finally { await unmount(); }
  const delegated = await mount({ ...BASE, requests: BASE.requests.map((one) => ({ ...one, author: "parent" as const })) });
  try {
    await tab(delegated.container, "Request history");
    expect(text(delegated.container.querySelector("[data-record-request]"))).toMatch(/^Parent/);
  } finally { await delegated.unmount(); }
  useShellStore.setState({ agents: [
    legacyAgentRow({ pane_id: "p0", agent_kind: "claude", session_id: "lead", identity_label: "Release lead" }),
    legacyAgentRow({ pane_id: "p1", agent_kind: "claude", session_id: "root", lineage_parent_pane_id: "p0" }),
  ] });
  const named = await mount({ ...BASE, requests: BASE.requests.map((one) => ({ ...one, author: "parent" as const })) });
  try {
    await tab(named.container, "Request history");
    expect(text(named.container.querySelector("[data-record-request]"))).toMatch(/^Release lead/);
  } finally { await named.unmount(); }
});

it("shows an unjudged request while working without inventing a task", async () => {
  const unjudged = { ...BASE, requests: [...BASE.requests, { ...request(4, "Install and measure again", [], "operator", false), at_unix_ms: new Date(2027, 0, 1, 0, 2).getTime() }] };
  const working = await mount({ ...unjudged, state: "working" });
  try {
    expect(text(working.container.querySelector("[data-record-latest-request]"))).toContain("Install and measure again");
    expect(text(working.container.querySelector("[data-record-latest-request] time"))).toBe("2027-01-01 00:02");
    expect(working.container.querySelectorAll("[data-record-task]")).toHaveLength(2);
  } finally { await working.unmount(); }
  const idle = await mount({ ...unjudged, state: "idle" });
  try { expect(idle.container.querySelector("[data-record-latest-request]")).toBeNull(); } finally { await idle.unmount(); }
});

it("keeps actual PRs available with summaries off and displays the latest raw request", async () => {
  const { container, unmount } = await mount(BASE, { summaries: false });
  try {
    expect(container.querySelector("[data-record-task], [data-record-work], [data-record-missed]")).toBeNull();
    expect(text(container.querySelector('[data-record-raw-request]'))).toContain("AI summaries off");
    expect(text(container.querySelector('[data-record-raw-request]'))).toContain("How is it going?");
    expect(container.querySelector('[data-record-output-number="1100"]')).not.toBeNull();
    expect(container.querySelector('[data-record-output-number="1093"]')).toBeNull();
    expect(text(container.querySelector('[data-record-child="child"]'))).toContain("PR 1");
    await tab(container, "Request history");
    expect(container.querySelectorAll("[data-record-request]")).toHaveLength(3);
  } finally { await unmount(); }
});

it("keeps a descendant's findings scoped to its expanded detail while the root remains visible", async () => {
  const withMissed = { ...BASE, children: [{ ...child, tasks: [task("ct", "Child task", AT, true)], missed: [missed("m-child", "Child found this", 99)] }] };
  const { container, unmount } = await mount(withMissed);
  try {
    expect(container.querySelector('[data-missed-work="m-child"]')).toBeNull();
    expect(text(container.querySelector('[data-record-child="child"] > button'))).toContain("Findings 1");
    await expand(container);
    expect(container.querySelectorAll('[data-missed-work="m-child"]')).toHaveLength(1);
    expect(container.querySelector('[data-record-task="t1"]')).not.toBeNull();
    expect(container.querySelector('[data-record-child-detail="child"] [data-missed-work="m-open"]')).toBeNull();
  } finally { await unmount(); }
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
  const { container, unmount } = await mount({ ...BASE, outputs: [{ ...output(1100, "review", "t2", SESSION), stale: true, last_read_at_unix_ms: AT }] });
  try {
    await tab(container, "PR");
    expect(container.querySelector("[data-record-output]")!.getAttribute("aria-label")).toContain(new Date(AT).toLocaleString());
  } finally { await unmount(); }
});

it("shows an actual question as the latest request when no task was extracted", async () => {
  const { container, unmount } = await mount({ ...BASE, tasks: [], missed: [], outputs: [], requests: [request(1, "Just a question", [])] });
  try {
    expect(container.querySelector('[data-record-task]')).toBeNull();
    expect(text(container.querySelector('[data-record-raw-request]'))).toContain("Just a question");
    expect(text(container)).not.toContain("0 / 0");
    expect(container.querySelector('[data-record-delegated] [data-record-child="child"]')).not.toBeNull();
  } finally { await unmount(); }
});

it("shows an ended session with elapsed time and no terminal button or duplicate action box", async () => {
  const { container, unmount } = await mount({ ...BASE, ended_at_unix_ms: AT });
  try {
    const status = container.querySelector("[data-record-now]")!;
    expect(status.getAttribute("data-record-now")).toBe("ended");
    expect(status.getAttribute("data-state-box")).toBeNull();
    expect(text(status)).toContain("Done");
    expect(status.querySelector("[data-record-now-button]")).toBeNull();
  } finally { await unmount(); }
});

// A task's box never draws an agent mark; the live row decides whether a task in progress
// is still at work, and whether a question asks with `!` or says it ended (D-05, B8, B9).
it("asks with `!` only while the session has a live row, and says a task in progress is work left once the row stops", async () => {
  const record: RecordValue = { ...BASE, tasks: [task("t3", "Ship the change", AT + 3, false, "waiting"), task("t4", "Answer me", AT + 4, false, "question")], outputs: [] };
  const row = (mark: "going" | "waiting", device_id?: string) => legacyAgentRow({ pane_id: "p1", agent_kind: "claude", session_id: "root", device_id, mark, group: "working", activity: mark === "going" ? "working" : "stopped", status_code: mark === "going" ? "working" : "waiting" });
  const slot = async (agents: AgentRow[]) => {
    useShellStore.setState({ agents });
    const view = await mount(record);
    try {
      const at = (id: string) => view.container.querySelector(`[data-record-task="${id}"]`)!;
      return [at("t3").querySelector("[data-task-word]")?.getAttribute("data-task-word") ?? null, at("t4").querySelector("[data-task-asks]") !== null, text(at("t4").querySelector("[data-task-word]")), view.container.querySelector("[data-record-task] [data-mark]")];
    } finally { await view.unmount(); }
  };
  expect(await slot([row("going")])).toEqual([null, true, "Needs you · Question", null]);
  expect(await slot([row("waiting")])).toEqual([null, true, "Needs you · Question", null]);
  expect(await slot([])).toEqual(["unfinished", false, "Unfinished · asked, then ended", null]);
  // The same session id on another device is another session.
  expect(await slot([row("going", "mini")])).toEqual(["unfinished", false, "Unfinished · asked, then ended", null]);
});

it("does not call a child done while its task is unfinished, and hides saved END judgments with summaries off", async () => {
  const record = { ...BASE, children: [{ ...child, last_end: "done" as const, tasks: [task("child-task", "Wait for review", AT, false, "waiting")] }] };
  const normal = await mount(record);
  try { expect(text(normal.container.querySelector("[data-record-child-state]"))).toBe("Not done"); } finally { await normal.unmount(); }
  const off = await mount({ ...record, children: [{ ...record.children[0]!, last_end: "question" }] }, { summaries: false });
  try { expect(text(off.container.querySelector("[data-record-child-state]"))).not.toMatch(/Done|Question|Blocked/); } finally { await off.unmount(); }
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
