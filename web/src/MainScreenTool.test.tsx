// @vitest-environment jsdom
// PRD hide-external-control D-54, B47: the tool's Overview page, the center a tool's tile opens, says a tool with
// no agents and no missions holds nothing yet.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { ToolCell } from "./agentScope";
import { TooltipProvider } from "./components/ui/tooltip";
import { initializeInterfaceI18n } from "./i18n/instance";
import { MainScreen } from "./MainScreen";
import type { SnapshotRest } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const savedShell = useShellStore.getState();
const savedUi = useUiStore.getState();
afterEach(() => {
  useShellStore.setState(savedShell, true);
  useUiStore.setState(savedUi, true);
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

async function drawn() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const tool: ToolCell = { connection: "c0ffee00c0ffee00", name: "nightly", turn: 0, devices: [] };
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: [{ id: "alpha", label: "alpha", device_id: "local", path: "/fixture", checkouts: [{ id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", tabs: [] }] }], focused_device_id: "local", tools: [tool] }, status: { remote: [] }, missions: { items: [], failed: false } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.setState({ mainView: "work", screen: { kind: "main", tool: tool.connection } });
  const sent: { kind: string; payload: Record<string, unknown> }[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MainScreen actions={createActions((event) => { sent.push(event as never); return true; })} deviceId="local" tool={tool} /></TooltipProvider>));
  return { container, sent, unmount: () => act(async () => root.unmount()) };
}

it("says a tool with no agents or missions holds nothing yet", async () => {
  const { container, unmount } = await drawn();
  expect(container.querySelector("[data-tool-cell-empty]")?.textContent).toBe("Nothing handed to it yet");
  await unmount();
});
