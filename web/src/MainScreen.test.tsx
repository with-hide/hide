// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MainScreen } from "./MainScreen";
import { useHistoryStore } from "./overviewHistoryState";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// This view never draws a terminal. Answer xterm's canvas capability probe
// at the browser boundary while retaining the real screen and actions.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
const savedHistory = useHistoryStore.getState();

afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  useHistoryStore.setState(savedHistory, true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("searches a disconnected device's retained records while live views keep their reconnect state", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  initializeInterfaceI18n("en");
  const rest = {
    navigator: { devices: [{ id: "local", kind: "local" }, { id: "mini", kind: "remote", label: "Mini" }], workspaces: [] },
    status: { remote: [{ target_id: "mini", state: "unreachable", message: "Mini is disconnected" }] },
  } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [], connection: "live", sessionWorkSearch: null });
  useUiStore.getState().setMainView("history");
  useUiStore.getState().setOverview({ projectId: null, panel: null });
  const events: { kind: string; payload: unknown }[] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<TooltipProvider><MainScreen actions={actions} deviceId="mini" /></TooltipProvider>));
    expect(container.querySelector("[data-overview-history]")).not.toBeNull();
    expect(container.querySelector("[data-device-unavailable]")).toBeNull();
    await act(async () => vi.advanceTimersByTime(180));
    const request = events.find((event) => event.kind === "session_work_search")?.payload as { query: string; options: { device_id: string; request_id: string } };
    expect(request).toMatchObject({ query: "", options: { device_id: "mini" } });
    await act(async () => useShellStore.setState({ sessionWorkSearch: {
      ...request, loading: false, failure: null, results: [{
        session: { device_id: "mini", agent: "codex", session_id: "retained" }, title: "Retained work", matched: "Please preserve this request", at_unix_ms: 1_790_000_000_000, cwd: null, status: "archived",
      }],
    } }));
    expect(container.textContent).toContain("Retained work");
    await act(async () => useUiStore.getState().setMainView("work"));
    expect(container.querySelector('[data-device-unavailable="mini"]')).not.toBeNull();
  } finally { await act(async () => root.unmount()); }
});
