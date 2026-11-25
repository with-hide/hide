// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { createActions, type Actions } from "./actions";
import { useSessionThread } from "./sessionThread";
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
