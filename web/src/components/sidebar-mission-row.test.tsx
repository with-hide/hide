// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterAll, afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { missionFixture } from "../gallery/MissionScene";
import { clientI18n } from "../i18n/translator";
import { missionAsks } from "../missions";
import { useShellStore } from "../store";
import { SidebarMissionRow } from "./sidebar-mission-row";
import { TooltipProvider } from "./ui/tooltip";

// The row's module chain loads xterm; answer its canvas probe at the browser boundary before it imports.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

// PRD mission-node-simple B14 and the approved mockup: a mission with no
// Conductor or worker row to carry what waits on the operator is its own
// Needs You item, with 이어받기 and the node's choice for its cause beside
// the line that says why.
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("offers take over and the cause's choice on the mission's own Needs You item, and a choice sends one settle", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const { mission, rest } = missionFixture("no-conductor", Date.UTC(2026, 10, 1));
  useShellStore.setState({ rest, connection: "live" });
  const [ask] = missionAsks([mission], "local");
  expect(ask).toBeDefined();
  const sent: unknown[] = [];
  const actions = createActions((event) => sent.push(event));
  const container = document.createElement("ul");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><SidebarMissionRow ask={ask!} inset="0" actions={actions} /></TooltipProvider>));
  try {
    const row = container.querySelector<HTMLElement>(`[data-mission-ask="${mission.id}"]`)!;
    const buttons = [...row.querySelectorAll<HTMLButtonElement>("[data-mission-ask-choices] button")].map((button) => button.textContent);
    expect(buttons).toEqual(["이어받기", "끝으로"]);
    await act(async () => row.querySelector<HTMLButtonElement>('[data-attention-choice="done"]')!.click());
    await act(async () => row.querySelector<HTMLButtonElement>('[data-attention-choice="done"]')!.click());
    expect(sent).toEqual([{ schema_version: 2, kind: "mission_attention", payload: { mission: mission.id, node: ask!.node.key, action: "done", since_unix_ms: ask!.attention.since_unix_ms } }]);
  } finally {
    await act(async () => root.unmount());
  }
});
