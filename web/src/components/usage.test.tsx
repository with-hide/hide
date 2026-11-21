// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { ledgerScene } from "../gallery/usageHistorySceneData";
import { clientI18n } from "../i18n/translator";
import type { SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { TooltipProvider } from "./ui/tooltip";
import { Usage } from "./usage";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
afterEach(() => { document.body.innerHTML = ""; });

const node = ledgerScene({ node: "local", agents: [], history: null }, "reference");
// A checkout in front with its workspace drawn: Overview opens over it.
const inWorkspace = {
  navigator: {
    devices: [{ id: "local", kind: "local", label: "This Mac" }],
    focused_checkout_id: "app-main",
    workspaces: [{ id: "app", label: "app", path: "/app", device_id: "local", checkouts: [{ id: "app-main", tabs: [] }] }],
  },
  workspace_view: { checkout_id: "app-main" },
} as unknown as SnapshotRest;

// UI_BEHAVIOR.md, Usage: full history opens the device Overview's Usage tab with no project chosen.
async function clickHistory(rest: SnapshotRest | null) {
  useShellStore.setState({ connection: "live", usage: { nodes: [node] }, rest });
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, mainView: "projects" });
  useUiStore.getState().setOverview({ projectId: "app" });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => { await clientI18n.changeLanguage("en"); root.render(<TooltipProvider><Usage actions={createActions(() => {})} /></TooltipProvider>); });
  await act(async () => document.querySelector<HTMLButtonElement>("[data-usage-trigger]")!.click());
  await act(async () => document.querySelector<HTMLButtonElement>("[data-usage-history-link]")!.click());
  return root;
}

it("opens Overview's Usage tab over the workspace from the popover's History", async () => {
  const root = await clickHistory(inWorkspace);
  try {
    await vi.waitFor(() => expect(useUiStore.getState().overviewOpen).toBe(true));
    const ui = useUiStore.getState();
    expect([ui.screen?.kind, ui.mainView, ui.overview.projectId]).toEqual(["workspace", "usage", null]);
    expect(document.querySelector("[data-usage-popover]")).toBeNull();
  } finally { await act(async () => root.unmount()); }
});

it("opens the Overview page on its Usage tab when no checkout is in front", async () => {
  const root = await clickHistory(null);
  try {
    await vi.waitFor(() => expect(useUiStore.getState().screen?.kind).toBe("main"));
    expect(useUiStore.getState().mainView).toBe("usage");
  } finally { await act(async () => root.unmount()); }
});
