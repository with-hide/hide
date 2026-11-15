// @vitest-environment jsdom
// PRD hide-external-control D-54, D-55, B47, B48: the tool's Overview page lists the Views the tool or its agents
// opened beside its agents and missions, a press opens one, and a tool with nothing at all says so.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import type { ToolCell, ToolView } from "./agentScope";
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

const view = (id: string, unseen: boolean): ToolView => ({ view_id: id, kind: "browser", label: `${id}.html`, device_id: "local", checkout_path: "/fixture/alpha", unseen, opened_unix_ms: Date.now() - 1000 });

async function drawn(views: ToolView[]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  initializeInterfaceI18n("en");
  const tool: ToolCell = { connection: "c0ffee00c0ffee00", name: "nightly", turn: 0, devices: [], views };
  const rest = { navigator: { devices: [{ id: "local", kind: "local" }], workspaces: [{ id: "alpha", label: "alpha", device_id: "local", path: "/fixture", checkouts: [{ id: "alpha-main", workspace_id: "alpha", label: "main", path: "/fixture/alpha", tabs: [] }] }], focused_device_id: "local", tools: [tool] }, status: { remote: [] }, missions: { items: [], failed: false } } as unknown as SnapshotRest;
  useShellStore.setState({ rest, agents: [], focusedPaneId: null, connection: "live", linkSummaries: null });
  useUiStore.setState({ mainView: "work", toolCell: tool.connection });
  const sent: { kind: string; payload: Record<string, unknown> }[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MainScreen actions={createActions((event) => { sent.push(event as never); return true; })} deviceId="local" tool={tool} /></TooltipProvider>));
  return { container, sent, unmount: () => act(async () => root.unmount()) };
}

it("lists the tool's Views in its Overview, marks the unseen one, and opens one on a press", async () => {
  const { container, sent, unmount } = await drawn([view("d2", true), view("d1", false)]);
  expect(container.querySelector("[data-tool-cell-empty]"), "a tool with only Views is not empty").toBeNull();
  const section = container.querySelector("[data-tool-overview-views]")!;
  expect(section.querySelector("h2")?.textContent).toBe("Opened screens2");
  expect([...section.querySelectorAll("[data-tool-view]")].map((entry) => [entry.getAttribute("data-tool-view"), entry.hasAttribute("data-tool-view-unseen")])).toEqual([["d2", true], ["d1", false]]);
  expect(section.innerHTML).not.toContain("warning");
  await act(async () => section.querySelector<HTMLElement>('[data-tool-view="d2"]')!.click());
  expect(sent.find((event) => event.kind === "focus_checkout")?.payload).toMatchObject({ workspace_id: "alpha", checkout_id: "alpha-main", display_id: "d2" });
  await unmount();
});

it("says a tool with no agents, missions or Views holds nothing yet", async () => {
  const { container, unmount } = await drawn([]);
  expect(container.querySelector("[data-tool-overview-views]")).toBeNull();
  expect(container.querySelector("[data-tool-cell-empty]")?.textContent).toBe("Nothing handed to it yet");
  await unmount();
});
