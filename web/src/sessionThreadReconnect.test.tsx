// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { createActions, type Actions } from "./actions";
import { useSessionThread } from "./sessionThread";
import type { SessionThreadSection } from "./snapshot";
import { useUiStore } from "./ui";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

function Panel({ actions }: { actions: Actions }) {
  useSessionThread(actions, "w1:p1");
  return null;
}

it("opens a shown thread again after a reconnect, whose new connection holds nothing yet", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shell = useShellStore.getState();
  const calls: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { calls.push(event); return true; });
  useShellStore.setState({ connection: "live" });
  const root = createRoot(document.createElement("div"));
  const open = { schema_version: 2, kind: "session_thread_open", payload: { pane_id: "w1:p1" } };
  try {
    await act(async () => root.render(<Panel actions={actions} />));
    expect(calls).toEqual([open]);
    // The socket drops and comes back: the daemon let the old connection's hold go.
    await act(async () => useShellStore.setState({ connection: "reconnecting" }));
    expect(calls).toEqual([open]);
    await act(async () => useShellStore.setState({ connection: "live" }));
    expect(calls).toEqual([open, open]);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(shell, true);
    vi.unstubAllGlobals();
  }
  expect(calls.at(-1)).toEqual({ schema_version: 2, kind: "session_thread_close", payload: {} });
  expect(calls.filter((call) => call.kind === "session_thread_close")).toHaveLength(1);
});

function ObservedPanel({ actions }: { actions: Actions }) {
  const { thread, taken, reclaim } = useSessionThread(actions, "w1:p1");
  return <div data-taken={String(taken)}><span>{thread?.generation}</span><button onClick={reclaim}>Reclaim</button></div>;
}

it("opens a changed session without the previous session's taken pages", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState(), local = useUiStore.getState();
  const calls: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { calls.push(event); return true; });
  const container = document.createElement("div"), root = createRoot(container);
  const row = (identity: string) => [{ pane_id: "w1:p1", thread_capability: "ready", thread_identity: identity }] as never;
  const section: SessionThreadSection = { generation: 1, pane_id: "w1:p1", machine: "local", state: "ready", failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: null, choice: {}, input: null };
  try {
    useShellStore.setState({ agents: row("first"), connection: "live", sessionThread: { ...section, generation: 2, pane_id: "w1:p2" } });
    useUiStore.setState({ paneConversation: { paneId: "w1:p1", session: "first" }, threadLease: { paneId: "w1:p1", acquired: true, taken: true, thread: section, history: null } });
    await act(async () => useShellStore.setState({ agents: row("second") }));
    expect(useUiStore.getState().paneConversation).toBeNull();
    expect(useUiStore.getState().threadLease).toBeNull();
    actions.togglePaneConversation("w1:p1");
    await act(async () => root.render(<ObservedPanel actions={actions} />));
    expect(container.querySelector("span")?.textContent).toBe("");
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("false");
    expect(calls.filter((call) => call.kind === "session_thread_open")).toHaveLength(1);
    await act(async () => useShellStore.setState({ sessionThread: { ...section, generation: 3 } }));
    expect(container.querySelector("span")?.textContent).toBe("3");
  } finally {
    await act(async () => root.unmount());
    useUiStore.setState(local, true);
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
  }
});

it("keeps the last thread after another pane takes it, including reconnect and remount, until explicit reclaim", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState(), local = useUiStore.getState();
  const calls: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { calls.push(event); return true; });
  const section = (pane: string, generation: number): SessionThreadSection => ({ generation, pane_id: pane, machine: "local", state: "ready", failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: null, choice: {}, input: null });
  const container = document.createElement("div"), root = createRoot(container);
  try {
    useUiStore.setState({ threadLease: null });
    useShellStore.setState({ connection: "live", sessionThread: null });
    await act(async () => root.render(<ObservedPanel actions={actions} />));
    await act(async () => useShellStore.setState({ sessionThread: section("w1:p1", 1) }));
    // Another window showing this same pane remains a shared hold, even with a new generation.
    await act(async () => useShellStore.setState({ sessionThread: section("w1:p1", 2) }));
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("false");
    await act(async () => useShellStore.setState({ sessionThread: section("w1:p2", 3) }));
    expect(container.querySelector("span")?.textContent).toBe("2");
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("true");
    await act(async () => useShellStore.setState({ connection: "reconnecting" }));
    await act(async () => useShellStore.setState({ connection: "live" }));
    await act(async () => root.render(null));
    await act(async () => root.render(<ObservedPanel actions={actions} />));
    expect(calls.filter((call) => call.kind === "session_thread_open")).toHaveLength(1);
    await act(async () => container.querySelector("button")!.click());
    expect(calls.filter((call) => call.kind === "session_thread_open")).toHaveLength(2);
    // Other sections can publish while the explicit open waits for its
    // answer. The unchanged replacement is not a second takeover.
    await act(async () => useShellStore.setState({ sessionThread: section("w1:p2", 3) }));
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("false");
    await act(async () => useShellStore.setState({ sessionThread: section("w1:p1", 4) }));
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("false");
    expect(container.querySelector("span")?.textContent).toBe("4");
  } finally {
    await act(async () => root.unmount());
    useUiStore.setState(local, true);
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
  }
});

// B17/B18: notifier coalescing can hide both the requested open and its replacement.
it.each(["ready", "closed"] as const)("does not reclaim a superseded first open when only the replacement's %s snapshot is published", async (state) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const saved = useShellStore.getState(), local = useUiStore.getState();
  const calls: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { calls.push(event); return true; });
  const container = document.createElement("div"), root = createRoot(container);
  try {
    useUiStore.setState({ threadLease: null });
    useShellStore.setState({ connection: "live", sessionThread: null });
    await act(async () => root.render(<ObservedPanel actions={actions} />));
    const replacement: SessionThreadSection = { generation: 2, pane_id: state === "closed" ? null : "w1:p2", machine: "local", state, failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: null, choice: {}, input: null };
    await act(async () => useShellStore.setState({ sessionThread: replacement }));
    expect(container.querySelector("[data-taken]")?.getAttribute("data-taken")).toBe("true");
    await act(async () => useShellStore.setState({ connection: "reconnecting" }));
    await act(async () => useShellStore.setState({ connection: "live" }));
    await act(async () => root.render(null));
    await act(async () => root.render(<ObservedPanel actions={actions} />));
    expect(calls.filter((call) => call.kind === "session_thread_open")).toHaveLength(1);
    await act(async () => container.querySelector("button")!.click());
    expect(calls.filter((call) => call.kind === "session_thread_open")).toHaveLength(2);
  } finally {
    await act(async () => root.unmount());
    useUiStore.setState(local, true);
    useShellStore.setState(saved, true);
    vi.unstubAllGlobals();
  }
});
