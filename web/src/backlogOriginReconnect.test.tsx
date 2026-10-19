// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, expect, it, vi } from "vitest";
import { createActions, type Actions } from "./actions";
import { useBacklogOrigin } from "./backlogOrigin";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

const canvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => canvas.restore());

function Panel({ actions }: { actions: Actions }) {
  useBacklogOrigin(actions, { id: "w1", device_id: "device-a" }, "github:acme/repo#7");
  return null;
}

it("reads an open item's origin again after a reconnect, whose new connection holds nothing yet", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const shell = useShellStore.getState();
  const calls: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { calls.push(event); return true; });
  useShellStore.setState({ connection: "live" });
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Panel actions={actions} />));
    const read = { schema_version: 2, kind: "backlog_origin", payload: { workspace_id: "w1", device_id: "device-a", task_key: "github:acme/repo#7" } };
    expect(calls).toEqual([read]);
    // The socket drops and comes back: the daemon let the old connection's hold go.
    await act(async () => useShellStore.setState({ connection: "reconnecting" }));
    await act(async () => useShellStore.setState({ connection: "live" }));
    expect(calls).toEqual([read, read]);
  } finally {
    await act(async () => root.unmount());
    useShellStore.setState(shell, true);
    vi.unstubAllGlobals();
  }
  expect(calls.at(-1)).toEqual({ schema_version: 2, kind: "backlog_origin_close", payload: { workspace_id: "w1", device_id: "device-a", task_key: "github:acme/repo#7" } });
  expect(calls.filter((call) => call.kind === "backlog_origin_close")).toHaveLength(1);
});
