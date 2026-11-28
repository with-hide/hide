// One practice chapter's world behind the real shell actions and keyboard, as
// `PracticePlayer` puts it, for the tests that send a world what the shell can
// send it (`worlds.test.ts`: every command and pane focus at each step;
// `restart.test.ts`: every step played again from a fresh world). A test file
// imports this after it has answered xterm's canvas probe at the browser
// boundary, because the actions load xterm.

import { expect, vi } from "vitest";
import { createActions } from "../actions";
import type { HostBridge } from "../host";
import { installKeyboard } from "../keyboard";
import { REGISTRY } from "../shortcuts";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { PracticeChapter } from "./chapter";
import { standInHost } from "./mount";
import type { CoreEvent } from "./world";

export const NOW = Date.UTC(2026, 9, 23, 12);

/** The app menu's command listener the shell installs: a command run by id, as a chord or the menu would. */
let menu: ((id: string) => void) | null = null;

/** The command listener the keyboard holds now, or null once the stage is torn down. */
export const menuListener = () => menu;

/** The page's clock stopped at `NOW`, and a stand-in host whose command listener `stage` takes; `removeStandInHost` undoes it. */
export function installStandInHost() {
  vi.useFakeTimers({ now: NOW });
  const host = standInHost("darwin");
  window.hideHost = new Proxy(host, {
    get: (target, name) => (name === "onCommand" ? (listener: (id: string) => void) => ((menu = listener), () => {}) : target[name as keyof HostBridge]),
  });
}

export function removeStandInHost() {
  vi.useRealTimers();
  delete window.hideHost;
}

/** One input the shell can send: a command by id, or a pane clicked. */
export type StageInput = { name: string; run: () => void };

/**
 * `record` keeps the events each input sends the world (`play`'s `events`), for the test that replays them;
 * without it nothing is kept or copied, so a test that only looks at the states pays nothing for them.
 */
export function stage(chapter: PracticeChapter, { record = false }: { record?: boolean } = {}) {
  const world = chapter.world;
  let model = world.initial();
  /** The events the world was sent since the last input started, when recording. */
  const sent: CoreEvent[] = [];
  const send = (event: CoreEvent) => {
    if (record) sent.push(event);
    const after = world.apply(model, event);
    if (after) model = after;
    return true;
  };
  const actions = createActions(send);
  // A world answers most inputs with the state it had, and every input starts
  // from its state drawn again, so each state is drawn and its screens read
  // once and handed to the store as often as an input starts from it: drawing
  // it again for every input was most of this test's time, and on a Windows
  // runner past vitest's 5 s (hide-private issue 114).
  const drawn = new Map<unknown, Partial<ReturnType<typeof useShellStore.getState>>>();
  const show = (next: unknown) => {
    model = next;
    let state = drawn.get(next);
    if (!state) {
      const { rest, agents, focused, usage } = world.snapshot(next, NOW);
      for (const id of world.panes(next)) expect(typeof world.screen(next, id, 80)).toBe("string");
      state = { rest, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null };
      drawn.set(next, state);
    }
    useShellStore.setState(state);
  };
  show(model);
  useUiStore.setState({ screen: { kind: "workspace" }, overviewOpen: false, laneExpanded: false, stackFront: "view" });
  const calm = useUiStore.getState();
  const uninstallKeyboard = installKeyboard(actions);
  // Every command the keyboard runs, each pane clicked, and a close confirmed on its sheet.
  const inputs = (from: unknown): StageInput[] => [
    ...REGISTRY.map((command) => ({ name: command.id, run: () => menu!(command.id) })),
    ...world.panes(from).map((id) => ({ name: `focus ${id}`, run: () => actions.focusPane(id) })),
  ];
  /** The state after `input` from `from`, drawn as the screen would draw it, and the events the world was sent on the way. */
  const play = (from: unknown, input: StageInput) => {
    show(from);
    useUiStore.setState(calm, true);
    if (record) sent.length = 0;
    input.run();
    if (useUiStore.getState().pendingClose) actions.confirmClose();
    const reached = model;
    show(reached);
    return { model: reached, events: record ? [...sent] : [] };
  };
  const uninstall = () => {
    uninstallKeyboard();
    menu = null;
  };
  return { world, inputs, play, uninstall, initial: model };
}
