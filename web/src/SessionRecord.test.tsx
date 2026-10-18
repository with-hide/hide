// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { SlotSessionRecord } from "./SessionRecord";
import type { AgentRow, MissedWork, SessionRecord as RecordValue, SnapshotRest, WorkOutput, WorkRequest, WorkSession, WorkTask } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { legacyAgentRow } from "../test/legacyAgentRow";

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

const AT = 1_790_000_000_000;
const SESSION: WorkSession = { device_id: "local", agent: "claude", session_id: "root" };
const CHILD: WorkSession = { device_id: "local", agent: "codex", session_id: "child" };
const task = (id: string, name: string, at: number, done: boolean, end: WorkTask["end"] = "done"): WorkTask => ({ id, chapter_id: "c", name, at_unix_ms: at, last_at_unix_ms: at, end, done });
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

async function mount(record: RecordValue | null, options: { summaries?: boolean } = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }] }, status: { background_ai: { provider: "claude", providers: [], unavailable_reason: null, enabled: options.summaries !== false, chosen: true, agent_summary: options.summaries !== false } } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, sessionRecord: record ? { session: SESSION, loading: false, failure: null, record } : null });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const sent: { kind: string; payload: unknown }[] = [];
  const actions = createActions((event) => { sent.push(event as { kind: string; payload: unknown }); return true; });
  await act(async () => root.render(<TooltipProvider><SlotSessionRecord actions={actions} target={SESSION} /></TooltipProvider>));
  return { container, sent, unmount: async () => { await act(async () => root.unmount()); } };
}

const text = (element: Element | null | undefined) => element?.textContent ?? "";

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

it("draws no PR list: a task's PR stands on its line, a child's on the child's row, and no status words (B15)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(container.querySelector("[data-record-outputs]")).toBeNull();
    expect(Array.from(container.querySelectorAll('[data-record-output="pr"]')).map((node) => node.getAttribute("data-record-output-number"))).toEqual(["1100", "1093"]);
    expect(container.querySelector('[data-record-child="child"] [data-record-output-number="1093"]')).not.toBeNull();
    const tasks = Array.from(container.querySelectorAll("[data-record-task]"));
    expect(tasks.map((row) => row.getAttribute("data-record-task"))).toEqual(["t1", "t2"]);
    expect(tasks.map((row) => row.querySelector("[data-task-box]")?.getAttribute("data-task-box"))).toEqual(["checked", "empty"]);
    // A PR-bound task's PR mark says the state, never a word on the line.
    expect(tasks[1]!.querySelector("[data-record-task-pr] [aria-label]")?.getAttribute("aria-label")).toMatch(new RegExp("^#" + "1100 "));
    expect(text(tasks[1]!.querySelector("[data-task-word]"))).toBe("");
  } finally { await unmount(); }
});

it("heads the checklist with its count and the delegated children in words, hangs each child under the task it started in, and drops the place line's child count (B1, B3, B5)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    expect(text(container.querySelector("[data-record-work] h3"))).toBe("Assigned work1/2· Delegated 1");
    const row = container.querySelector('[data-record-task="t2"] [data-record-child="child"]')!;
    expect(row.querySelector("[data-record-child-arrow]")).not.toBeNull();
    // A finished child is a check and a quiet title, with no state word.
    expect(row.querySelector("[data-agent-status-mark]")?.getAttribute("data-agent-status-mark")).toBe("done-check");
    expect(row.querySelector("[data-record-child-state]")).toBeNull();
    expect(container.querySelector("[data-record-delegated]")).toBeNull();
    expect(text(container.querySelector("[data-record-place]"))).not.toMatch(/child/i);
    expect(container.querySelector("[data-child-progress], [data-child-segment]")).toBeNull();
  } finally { await unmount(); }
  // A child that started before every task stands at the end, under a small `Delegated`.
  const early = await mount({ ...BASE, children: [{ ...child, started_at_unix_ms: AT, state: "idle", last_end: "unfinished" }] });
  try {
    const rest = early.container.querySelector("[data-record-delegated]")!;
    expect(text(rest.querySelector("h4"))).toBe("Delegated");
    expect(rest.querySelector("[data-record-child-arrow]")).toBeNull();
    expect(text(rest.querySelector("[data-record-child-state]"))).toBe("Idle");
  } finally { await early.unmount(); }
});

it("lists only the open missed work in its own section, named by its task, with no undo for what was handled (B12, B13)", async () => {
  const { container, unmount } = await mount(BASE);
  try {
    const section = container.querySelector("[data-record-missed]")!;
    expect(text(section.querySelector("h3"))).toBe("Missed1");
    expect(Array.from(section.querySelectorAll("[data-missed-work]")).map((node) => node.getAttribute("data-missed-work"))).toEqual(["m-open"]);
    expect(text(section.querySelector("[data-missed-note]"))).toBe(" · Check the search");
    expect(container.querySelector('[data-missed-work="m-closed"]')).toBeNull();
    expect(container.textContent).not.toMatch(/Undo|Reopen|Handled/);
  } finally { await unmount(); }
  const handled = await mount({ ...BASE, missed: [missed("m-done", "Done already", 2, "backlog")] });
  try { expect(handled.container.querySelector("[data-record-missed]")).toBeNull(); } finally { await handled.unmount(); }
});

it("ends with the requests, the newest four open behind `n earlier`, and no words button or bubbles (B24, B25)", async () => {
  const requests = Array.from({ length: 6 }, (_, index) => request(index + 1, `request ${index + 1}\nmore`, []));
  const { container, unmount } = await mount({ ...BASE, requests });
  try {
    expect(container.querySelector("[data-record-words-bubble], [data-record-words] [aria-pressed]")).toBeNull();
    const section = container.querySelector("[data-record-words]")!;
    expect(text(section.querySelector("h3"))).toBe("My words6");
    const rows = () => Array.from(section.querySelectorAll("[data-record-request]")).map((row) => text(row).replace(/\d{2}:\d{2}$/, ""));
    expect(rows()).toEqual(["Merequest 3", "Merequest 4", "Merequest 5", "Merequest 6"]);
    await act(async () => section.querySelector<HTMLButtonElement>("[data-record-words-earlier]")!.click());
    expect(rows()).toHaveLength(6);
  } finally { await unmount(); }
  // A parent's requests are received, and say who asked.
  const delegated = await mount({ ...BASE, requests: BASE.requests.map((one) => ({ ...one, author: "parent" as const })) });
  try {
    expect(text(delegated.container.querySelector("[data-record-words] h3"))).toBe("Requests received3");
    expect(text(delegated.container.querySelector("[data-record-request]"))).toMatch(/^Parent/);
  } finally { await delegated.unmount(); }
  // Opened as its own item, a child's record names its asker from the live parent row (B24).
  useShellStore.setState({ agents: [
    legacyAgentRow({ pane_id: "p0", agent_kind: "claude", session_id: "lead", identity_label: "Release lead" }),
    legacyAgentRow({ pane_id: "p1", agent_kind: "claude", session_id: "root", lineage_parent_pane_id: "p0" }),
  ] });
  const named = await mount({ ...BASE, requests: BASE.requests.map((one) => ({ ...one, author: "parent" as const })) });
  try {
    expect(text(named.container.querySelector("[data-record-request]"))).toMatch(/^Release lead/);
  } finally { await named.unmount(); }
});

it("shows the newest unjudged request as Just asked while the session works, and not once it stopped", async () => {
  const unjudged = { ...BASE, requests: [...BASE.requests, request(4, "Install and measure again", [], "operator", false)] };
  const working = await mount({ ...unjudged, state: "working" });
  try {
    expect(text(working.container.querySelector("[data-record-latest-request]"))).toBe("Just askedInstall and measure again");
  } finally { await working.unmount(); }
  const idle = await mount({ ...unjudged, state: "idle" });
  try {
    expect(idle.container.querySelector("[data-record-latest-request]")).toBeNull();
  } finally { await idle.unmount(); }
});

it("draws an output no task claims beside the title, and none for a session with no outputs", async () => {
  const loose = await mount({ ...BASE, outputs: [output(1200, "open", null, SESSION)] });
  try {
    expect(loose.container.querySelector('h2 [data-record-output-number="1200"]')).not.toBeNull();
  } finally { await loose.unmount(); }
  const { container, unmount } = await mount({ ...BASE, outputs: [] });
  try {
    expect(container.querySelector("[data-record-output]")).toBeNull();
  } finally { await unmount(); }
  // With Agent summaries off no task line is drawn, so a task's PR stands beside the title instead.
  const off = await mount(BASE, { summaries: false });
  try {
    expect(off.container.querySelector('h2 [data-record-output-number="1100"]')).not.toBeNull();
    expect(off.container.querySelector('[data-record-child="child"] [data-record-output-number="1093"]')).not.toBeNull();
  } finally { await off.unmount(); }
});

it("with Hide AI off lists the children as their own section, the requests at the end, and no checklist or missed work", async () => {
  const { container, unmount } = await mount({ ...BASE, missed: [missed("m-off", "Found while summaries were on", 2)] }, { summaries: false });
  try {
    expect(container.querySelector("[data-record-task], [data-record-work], [data-record-missed]")).toBeNull();
    expect(text(container.querySelector("[data-record-delegated] h3"))).toBe("Delegated1");
    expect(container.querySelector("[data-record-delegated] [data-record-child-arrow]")).toBeNull();
    expect(container.querySelectorAll("[data-record-request]")).toHaveLength(3);
  } finally { await unmount(); }
});

it("draws a descendant's missed item once, named by its session, at the end of the missed section", async () => {
  const withMissed = { ...BASE, children: [{ ...child, tasks: [task("ct", "Child task", AT, true)], missed: [missed("m-child", "Child found this", 99)] }] };
  const { container, unmount } = await mount(withMissed);
  try {
    expect(container.querySelector('[data-record-child="child"]')?.getAttribute("role")).toBe("button");
    const items = container.querySelectorAll('[data-missed-work="m-child"]');
    expect(items).toHaveLength(1);
    expect(text(items[0]!)).toContain("Child found this · Child work");
    expect(Array.from(container.querySelectorAll("[data-record-missed] [data-missed-work]")).at(-1)).toBe(items[0]);
  } finally { await unmount(); }
});

it("opens a child's record as the 보기 slot's next item, so the slot's ← comes back to the parent (PRD sidebar-task-lens B16)", async () => {
  const { container, sent, unmount } = await mount({ ...BASE, children: [child] });
  try {
    sent.length = 0;
    await act(async () => container.querySelector<HTMLElement>('[data-record-child="child"]')!.click());
    expect(sent.map((event) => event.kind)).toEqual(["detail_open"]);
    expect(sent[0]!.payload).toEqual({ item: { kind: "record", session: CHILD } });
    // The slot draws its own head: the record carries no back button of its own.
    expect(container.querySelector("[data-record-up], [data-record-back]")).toBeNull();
  } finally { await unmount(); }
});

it("gives a task's PR button the last read time while GitHub cannot be read", async () => {
  const stale = { ...BASE, outputs: [{ ...output(1100, "review", "t2", SESSION), stale: true, last_read_at_unix_ms: AT }] };
  const { container, unmount } = await mount(stale);
  try {
    const label = container.querySelector("[data-record-task-pr] [data-record-output]")!.getAttribute("aria-label")!;
    expect(label).toContain(new Date(AT).toLocaleString());
  } finally { await unmount(); }
});

it("says so when no request became a task, and lists a task-less record's child under Delegated (B4)", async () => {
  const noTasks = { ...BASE, tasks: [], missed: [], outputs: [], requests: [request(1, "Just a question", [])], children: [child] };
  const { container, unmount } = await mount(noTasks);
  try {
    expect(text(container.querySelector("[data-record-work] h3"))).toBe("Assigned work· Delegated 1");
    expect(container.querySelector('[data-record-delegated] [data-record-child="child"]')).not.toBeNull();
  } finally { await unmount(); }
});

it("heads the record with the now box: ended with the elapsed time and no button, working with the last words and a pane button (B14)", async () => {
  const { container, unmount } = await mount({ ...BASE, ended_at_unix_ms: AT });
  try {
    const box = container.querySelector("[data-record-now]")!;
    expect(box.getAttribute("data-record-now")).toBe("ended");
    expect(box.getAttribute("data-state-box")).toBe("done");
    expect(text(box)).toContain("Done");
    expect(box.querySelector("[data-record-now-button]")).toBeNull();
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
