// @vitest-environment jsdom
// Every chapter's world against what the shell can send it. A learner may
// press any chord or click any pane on a practice screen, so every command the
// shell's keyboard runs and every pane focus is sent, and the world must
// answer it (or answer nothing) and still draw its snapshot and every pane's
// screen (docs/UI_BEHAVIOR.md, Practice window). A new chapter is checked here
// by being added.

import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import type { HostBridge } from "../host";
import { installKeyboard } from "../keyboard";
import { REGISTRY } from "../shortcuts";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import type { PracticeChapter } from "./chapter";
import { CHAPTERS } from "./chapters";
import { standInHost } from "./mount";
import type { CoreEvent } from "./world";

// Importing the actions loads xterm; answer its canvas probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});

const NOW = Date.UTC(2026, 9, 23, 12);
/** The app menu's command listener the shell installs: a command run by id, as a chord or the menu would. */
let menu: ((id: string) => void) | null = null;

beforeAll(() => {
  vi.useFakeTimers({ now: NOW });
  const host = standInHost("darwin");
  window.hideHost = new Proxy(host, {
    get: (target, name) => (name === "onCommand" ? (listener: (id: string) => void) => ((menu = listener), () => {}) : target[name as keyof HostBridge]),
  });
});

afterAll(() => {
  browserCanvas.restore();
  vi.useRealTimers();
  delete window.hideHost;
});

/** One chapter's world behind the real shell actions and keyboard, as `PracticePlayer` puts it. */
function stage(chapter: PracticeChapter) {
  const world = chapter.world;
  let model = world.initial();
  const send = (event: CoreEvent) => {
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
  const uninstall = installKeyboard(actions);
  // Every command the keyboard runs, each pane clicked, and a close confirmed on its sheet.
  const inputs = (from: unknown): { name: string; run: () => void }[] => [
    ...REGISTRY.map((command) => ({ name: command.id, run: () => menu!(command.id) })),
    ...world.panes(from).map((id) => ({ name: `focus ${id}`, run: () => actions.focusPane(id) })),
  ];
  /** The state after `input` from `from`, drawn as the screen would draw it. */
  const after = (from: unknown, input: { run: () => void }) => {
    show(from);
    useUiStore.setState(calm, true);
    input.run();
    if (useUiStore.getState().pendingClose) actions.confirmClose();
    const reached = model;
    show(reached);
    return reached;
  };
  return { world, inputs, after, uninstall, initial: model };
}

describe.each(CHAPTERS.map((chapter) => [chapter.id, chapter] as const))("practice chapter %s", (_id, chapter) => {
  it("answers every command and pane focus, and still draws", () => {
    expect(menu).toBeNull();
    const { inputs, after, uninstall, initial } = stage(chapter);
    try {
      expect(menu).not.toBeNull();
      for (const input of inputs(initial)) {
        try {
          after(initial, input);
        } catch (error) {
          throw new Error(`${input.name}: ${String(error)}`, { cause: error });
        }
      }
    } finally {
      uninstall();
      menu = null;
    }
  }, 30_000);
});
