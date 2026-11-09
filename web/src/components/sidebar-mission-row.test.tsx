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
// Needs You item, with 이어받기 and 확인하기 under the line that says why;
// the node's choices wait in the mission's 내 차례 card.
afterEach(async () => {
  await clientI18n.changeLanguage("en");
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

it("offers take over and 확인하기 on the mission's own Needs You item, and 확인하기 opens the mission without settling anything", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const { mission, rest } = missionFixture("no-conductor", Date.UTC(2026, 10, 1));
  useShellStore.setState({ rest, connection: "live" });
  const [ask] = missionAsks([mission], [mission.id]);
  expect(ask).toBeDefined();
  const sent: unknown[] = [];
  const opened: string[] = [];
  const actions = { ...createActions((event) => sent.push(event)), openMission: (id: string) => { opened.push(id); } };
  const container = document.createElement("ul");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><SidebarMissionRow ask={ask!} inset="0" actions={actions} /></TooltipProvider>));
  try {
    const row = container.querySelector<HTMLElement>(`[data-mission-ask="${mission.id}"]`)!;
    const buttons = [...row.querySelectorAll<HTMLButtonElement>("[data-mission-ask-choices] button")].map((button) => button.textContent);
    expect(buttons).toEqual(["지휘 다시 열기", "확인하기"]);
    expect(row.querySelector("[data-attention-choice]")).toBeNull();
    await act(async () => row.querySelector<HTMLButtonElement>("[data-mission-ask-look]")!.click());
    expect(opened).toEqual([mission.id]);
    expect(sent.filter((event) => (event as { kind: string }).kind === "mission_attention")).toEqual([]);
  } finally {
    await act(async () => root.unmount());
  }
});

// A mission whose Conductor is at work offers only the node's choices: 지휘 다시 열기 there read as a second, unrelated thing to do (operator report on M41).
it("offers no take over while a Conductor holds the mission", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  await clientI18n.changeLanguage("ko");
  const { mission, rest } = missionFixture("no-conductor", Date.UTC(2026, 10, 1));
  const held = { ...mission, conductor: { ...mission.conductor, present: true } };
  useShellStore.setState({ rest, connection: "live" });
  const [ask] = missionAsks([held], [held.id]);
  const container = document.createElement("ul");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><SidebarMissionRow ask={ask!} inset="0" actions={createActions(() => true)} /></TooltipProvider>));
  try {
    const buttons = [...container.querySelectorAll<HTMLButtonElement>("[data-mission-ask-choices] button")].map((button) => button.textContent);
    expect(buttons).toEqual(["확인하기"]);
  } finally {
    await act(async () => root.unmount());
  }
});

