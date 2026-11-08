// @vitest-environment jsdom
// The Mission tool's `내 차례` rows (PRD mission-tool B5, B6): a turn pill says what the
// operator does with no `!`, and the row of a mission no Conductor holds carries its
// 이어받기 button and no pill, while a press on any row still opens its map.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { initializeInterfaceI18n } from "./i18n/instance";
import { TooltipProvider } from "./components/ui/tooltip";
import { MissionTool } from "./MissionTool";
import type { Mission, SnapshotRest, Workspace } from "./snapshot";
import { useShellStore } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
// One project object: the store selector would loop on a new one each read.
const project = vi.hoisted(() => ({ id: "w", path: "/repo", device_id: "mac", label: "repo" }));
vi.mock("./projectStatus", async (importOriginal) => ({ ...(await importOriginal<typeof import("./projectStatus")>()), frontProject: () => project as unknown as Workspace }));

afterEach(() => { vi.unstubAllGlobals(); document.body.innerHTML = ""; });

const base = (id: string, extra: Partial<Mission>): Mission => ({ id, title: `Mission ${id}`, workspace_id: "w", project: "/repo", checkout_path: "/repo", device_id: "mac", status: "running", scope: "pr", cap: 4, exempt: false, strength: null, conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: true, takes_answers: true }, nodes: [], merge_order: [], landing_order: [], stack_waiting: 0, check: null, github: null, decision: null, question: null, answer: null, answer_failed: null, delay: null, report: null, created_at_unix_ms: 1, started_at_unix_ms: 1, closed_at_unix_ms: null, running: 0, stale: false, github_stale: false, github_read_at_unix_ms: null, ...extra });

it("draws the action as the pill with no `!`, draws no pill on the row that has the 이어받기 button, and opens a row on a press", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  initializeInterfaceI18n("en");
  const gone = base("gone", { conductor: { pane_id: "p1", name: "conductor", agent_kind: "claude", session: null, present: false, takes_answers: true } });
  const asking = base("asking", { decision: "Which order?" });
  useShellStore.setState({ rest: { navigator: { devices: [{ id: "mac", kind: "local", label: "This Mac" }], workspaces: [] }, missions: { items: [gone, asking] }, status: {} } as unknown as SnapshotRest });
  const opened: string[] = [];
  const actions = { ...createActions(() => true), openMission: (id: string) => { opened.push(id); } };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><MissionTool actions={actions} /></TooltipProvider>));
  try {
    const row = (id: string) => container.querySelector<HTMLElement>(`[data-mission-row="${id}"]`)!;
    expect(row("gone").dataset.missionRowGroup).toBe("turn");
    expect(row("gone").querySelector("[data-mission-row-pill]")).toBeNull();
    expect(row("gone").querySelector("[data-mission-takeover]")!.textContent).toBe("Take over");
    expect(row("asking").querySelector("[data-mission-row-pill]")!.textContent).toBe("Answer");
    for (const id of ["gone", "asking"]) expect(row(id).textContent).not.toContain("!");
    await act(async () => row("gone").querySelector<HTMLButtonElement>("button[aria-label]")!.click());
    expect(opened).toEqual(["gone"]);
  } finally { await act(async () => root.unmount()); }
});
