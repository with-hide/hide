// @vitest-environment jsdom
// Every chapter's world against what the shell can send it. A learner may
// press any chord or click any pane on a practice screen, so at each step a
// chapter reaches, every command the shell's keyboard runs and every pane
// focus is sent, and the world must answer it (or answer nothing) and still
// draw its snapshot and every pane's screen (docs/UI_BEHAVIOR.md, Adding a
// chapter), and a step done inside a palette names the command that opens it.
// A new chapter is checked here by being added.

import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { CHAPTERS } from "./chapters";
import { installStandInHost, menuListener, removeStandInHost, stage } from "./stage";

// Importing the actions loads xterm; answer its canvas probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});

/** How many times a step's own command is pressed in a row when its state is looked for. */
const PRESSES = 3;

beforeAll(installStandInHost);

afterAll(() => {
  browserCanvas.restore();
  removeStandInHost();
});

describe.each(CHAPTERS.map((chapter) => [chapter.id, chapter] as const))("practice chapter %s", (_id, chapter) => {
  it("answers every command and pane focus at each step, and still draws", () => {
    expect(menuListener()).toBeNull();
    const { inputs, play, uninstall, initial } = stage(chapter);
    try {
      expect(menuListener()).not.toBeNull();
      const key = (model: unknown) => JSON.stringify(model);
      const answered = new Map<string, { model: unknown; name: string }[]>();
      /** Sends every input once from `model`, naming the step and the path when a world fails. */
      const everyInput = (model: unknown, where: string) => {
        const known = answered.get(key(model));
        if (known) return known;
        const results = inputs(model).map((input) => {
          try {
            return { model: play(model, input).model, name: input.name };
          } catch (error) {
            throw new Error(`${where}, then ${input.name}: ${String(error)}`, { cause: error });
          }
        });
        answered.set(key(model), results);
        return results;
      };
      let current = initial;
      for (const step of chapter.steps) {
        // A step's state is looked for one input away, then along the step's
        // own command pressed again (a lens toggled through each of its views).
        // A step only typing, a wait, a palette, a drag or a click on something
        // other than a pane reaches keeps the state before it: searching every
        // input three deep for one spent its whole bound for nothing, most of
        // the agents chapter's time and past vitest's 5 s on a Windows runner
        // (#2841), and found no step this search does not.
        if (step.done && !step.done(current)) {
          let reached = everyInput(current, step.id).find((result) => step.done!(result.model))?.model;
          for (let model = current, presses = 1; reached === undefined && step.command && presses < PRESSES; presses += 1) {
            const pressed = everyInput(model, `${step.id} > ${step.command} x${presses}`).find((result) => result.name === step.command);
            if (!pressed) break;
            model = pressed.model;
            const again = everyInput(model, `${step.id} > ${step.command} x${presses + 1}`).find((result) => result.name === step.command);
            if (again && step.done(again.model)) reached = again.model;
          }
          if (reached !== undefined) current = reached;
        }
        everyInput(current, `at ${step.id}`);
      }
    } finally {
      uninstall();
    }
  });

  // Esc closes a palette or a dialog mid-step; ⏎ alone has to bring it back (docs/UI_BEHAVIOR.md, Adding a chapter).
  it("gives every step done inside a palette the command that opens it, and every field typed into what brings it back", () => {
    for (const step of chapter.steps) {
      const typing = step.gesture !== undefined && "type" in step.gesture && "input" in step.gesture.type ? step.gesture.type : null;
      if (step.target?.includes("[data-palette]")) expect(step.command, step.id).toBeDefined();
      if (typing) expect(step.command ?? typing.open, step.id).toBeDefined();
    }
  });
});
