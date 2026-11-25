// @vitest-environment jsdom
// Another computer's screen in this window draws with this window's View menu
// choices, not its own core's (UI_BEHAVIOR.md, Device rail, Another
// computer's screen; operator decision 2026-11-25): the showing window sends
// them with its language and name, and a change reaches the open screen at
// once. The page hears the showing window once per load, so the stand-in
// bridge is on the page before any module loads, and the tests tell it in turn.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { AgentRowMark } from "./AgentRowMark";
import { SidebarMenus } from "./components/sidebar-menus";
import { TooltipProvider } from "./components/ui/tooltip";
import { usePaneTurnRowMode } from "./drawnDisplay";
import { initializeInterfaceI18n } from "./i18n/instance";
import { useShellStore } from "./store";

type Told = { language: string; name: string | null; display?: unknown };

const bridge = vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
  const heard: { tell: ((viewer: { language: string; name: string | null; display?: unknown }) => void) | null } = { tell: null };
  (window as unknown as { hideRemote: unknown }).hideRemote = { bridge: 1, onViewer: (listener: typeof heard.tell) => { heard.tell = listener; } };
  return heard;
});

beforeAll(() => initializeInterfaceI18n("en"));
afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

function TurnRow() {
  return <span data-turn-row={usePaneTurnRowMode()} />;
}

/** A computer's screen whose own core chose hidden logos and two lines, as the showing window tells it. */
async function remoteScreen() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  useShellStore.setState({ connection: "live", rest: { ui_state: { agent_logo_mode: "hidden", pane_turn_row_mode: "two_lines" } } } as never);
  const actions = createActions(() => true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><AgentRowMark kind="claude" /><TurnRow /><SidebarMenus actions={actions} place="footer" /></TooltipProvider>));
  const drawn = () => ({
    logo: container.querySelector("[data-agent-mark]")?.getAttribute("data-agent-mark") ?? null,
    turnRow: container.querySelector("[data-turn-row]")?.getAttribute("data-turn-row"),
    viewMenu: container.querySelector("[data-view-menu-trigger]") !== null,
  });
  return { drawn, tell: async (viewer: Told) => act(async () => bridge.tell!(viewer)), unmount: () => act(async () => root.unmount()) };
}

it("draws rows and turn rows with the showing window's choices over its own core's, and follows each change at once", async () => {
  const screen = await remoteScreen();
  // Before the window has said anything, the screen draws its own core's choices.
  expect(screen.drawn()).toEqual({ logo: null, turnRow: "two_lines", viewMenu: true });

  await screen.tell({ language: "ko", name: "Mac mini", display: { agent_logo_mode: "shown", pane_turn_row_mode: "one_line" } });
  // Its own View menu would change only its core's choice, which nothing here draws, so it has none.
  expect(screen.drawn()).toEqual({ logo: "claude", turnRow: "one_line", viewMenu: false });

  await screen.tell({ language: "ko", name: "Mac mini", display: { agent_logo_mode: "hidden", pane_turn_row_mode: "off" } });
  expect(screen.drawn()).toEqual({ logo: null, turnRow: "off", viewMenu: false });
  await screen.unmount();
});

it("keeps its own core's choices and its View menu when the showing window's desktop sends none, or a mode this build does not know", async () => {
  const screen = await remoteScreen();
  await screen.tell({ language: "ko", name: "Mac mini" });
  expect(screen.drawn()).toEqual({ logo: null, turnRow: "two_lines", viewMenu: true });
  await screen.tell({ language: "ko", name: "Mac mini", display: { agent_logo_mode: "shown", pane_turn_row_mode: "three_lines" } });
  expect(screen.drawn()).toEqual({ logo: null, turnRow: "two_lines", viewMenu: true });
  await screen.unmount();
});
