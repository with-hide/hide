// @vitest-environment jsdom
// Every chapter's world against what the shell can send it. A learner may
// press any chord or click any pane on a practice screen, so at each step a
// chapter reaches, every command the shell's keyboard runs and every pane
// focus is sent, and the world must answer it (or answer nothing) and still
// draw its snapshot and every pane's screen (docs/UI_BEHAVIOR.md, Adding a
// chapter), and a step done inside a palette names the command that opens it.
// A new chapter is checked here by being added.

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
/** How many events deep a step's state is looked for from the step before it. */
const DEPTH = 3;
/** The states one step's search may visit, so a world with many states stays bounded. */
const STATES = 400;

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
  const show = (next: unknown) => {
    model = next;
    const { rest, agents, focused, usage } = world.snapshot(next, NOW);
    for (const id of world.panes(next)) expect(typeof world.screen(next, id, 80)).toBe("string");
    useShellStore.setState({ rest, agents, connection: "live", focusedPaneId: focused, usage: usage ? { nodes: [usage] } : null });
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
  it("answers every command and pane focus at each step, and still draws", () => {
    expect(menu).toBeNull();
    const { inputs, after, uninstall, initial } = stage(chapter);
    try {
      expect(menu).not.toBeNull();
      const key = (model: unknown) => JSON.stringify(model);
      const answered = new Map<string, { model: unknown; name: string }[]>();
      /** Sends every input once from `model`, naming the step and the path when a world fails. */
      const everyInput = (model: unknown, where: string) => {
        const known = answered.get(key(model));
        if (known) return known;
        const results = inputs(model).map((input) => {
          try {
            return { model: after(model, input), name: input.name };
          } catch (error) {
            throw new Error(`${where}, then ${input.name}: ${String(error)}`, { cause: error });
          }
        });
        answered.set(key(model), results);
        return results;
      };
      let current = initial;
      for (const step of chapter.steps) {
        // A step's state is looked for among what the shell can send; a step
        // only a palette, a drag or typing reaches keeps the state before it.
        if (step.done && !step.done(current)) {
          const seen = new Set([key(current)]);
          let found = false;
          let frontier: { model: unknown; path: string }[] = [{ model: current, path: step.id }];
          search: for (let depth = 0; depth < DEPTH && frontier.length; depth += 1) {
            const next: typeof frontier = [];
            for (const { model, path } of frontier) {
              for (const reached of everyInput(model, path)) {
                if (seen.has(key(reached.model)) || seen.size >= STATES) continue;
                seen.add(key(reached.model));
                const there = { model: reached.model, path: `${path} > ${reached.name}` };
                if (step.done(reached.model)) {
                  current = reached.model;
                  found = true;
                  break search;
                }
                next.push(there);
              }
            }
            frontier = next;
          }
          // Not found within the bound is not the same as unreachable by the shell: say so, as the step is checked from the state before it.
          if (!found && seen.size >= STATES) console.warn(`practice chapter ${chapter.id}: ${step.id} was not reached within ${STATES} states; checked from the state before it`);
        }
        everyInput(current, `at ${step.id}`);
      }
    } finally {
      uninstall();
      menu = null;
    }
  });

  // Esc closes a palette mid-step; ⏎ alone has to bring it back (docs/UI_BEHAVIOR.md, Adding a chapter).
  it("gives every step done inside a palette the command that opens it", () => {
    for (const step of chapter.steps) {
      const inPalette = step.target?.includes("[data-palette]") || (step.gesture !== undefined && "type" in step.gesture && "input" in step.gesture.type);
      if (inPalette) expect(step.command, step.id).toBeDefined();
    }
  });
});
