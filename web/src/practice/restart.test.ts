// @vitest-environment jsdom
// Every chapter started over from each of its steps, at the model level. The
// practice window starts a chapter over from the list, from its end card, from
// Back over a step that changed the screen, or on resume, and each of those
// makes a fresh world (`PracticePlayer`) that the coach then drives through
// the steps before the one asked for, on its own (`PracticeCoach`, `resume`),
// the way the learner drove them the first time. So a restart from step k is
// right when a fresh world, sent what reached each step before k (and each
// step's own `enter` event), is the world the first run had when step k began,
// drawn the same, and plays the rest to the same end. The browser plays each
// chapter once (`web/e2e/practice.spec.ts`); a restart from every step costs
// nothing here, where a world answers an event in microseconds, and a world
// that carries state from one run into the next (an initial model shared with
// its events, a model changed in place, a clock read twice) fails the step
// that shows it, by name.
//
// The first run reaches each step the way `worlds.test.ts` finds a step's state:
// its own command, then a pane click, then any other command, and a step that
// waits is run on by the world's own beats (`later`) as the browser's clock is
// run ahead. A step done by typing, a drag or a menu keeps the state before it,
// which is the state its events are checked from.

import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import type { AccessCall } from "../accessCall";
import type { AutomationCall } from "../automationCall";
import { clientI18n } from "../i18n/translator";
import type { MessageKey } from "../i18n/catalogs";
import type { PracticeChapter } from "./chapter";
import { CHAPTERS } from "./chapters";
import { installStandInHost, NOW, removeStandInHost, stage, type StageInput } from "./stage";
import type { AgentsModel } from "./worlds/agents";
import { CONVERSATION_PANE } from "./worlds/conversation";
import { LINK } from "./worlds/connect";
import { LOGIN_SESSION, PR_NUMBER } from "./worlds/find";
import { MR_NUMBER } from "./worlds/gitlab";
import { PIN_TARGET } from "./worlds/project";
import { PRACTICE_COPY, type CoreEvent, type PracticeWorld } from "./world";

// Importing the actions loads xterm; answer its canvas probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});

/** How many times a step's own command is pressed in a row when its state is looked for. */
const PRESSES = 3;
/** More beats than any chapter's world has; a world that needs more never lets its waiting step end. */
const BEATS = 30;

beforeAll(installStandInHost);

afterAll(() => {
  browserCanvas.restore();
  removeStandInHost();
});

/** What one step of the first run sent its world: its own `enter` event, then what reached its state. */
type StepRun = { events: CoreEvent[] };

/**
 * The world after `event`, as the player takes it: an event the world has no answer for leaves the model as it was, and
 * the two calls the window waits on (the automation form's and Settings' bots, `PracticePlayer`) are answered by the
 * world's own `call` and `access`.
 */
function applied(world: PracticeWorld<unknown>, model: unknown, event: CoreEvent): unknown {
  if (event.kind === "automation_call") return world.call?.(model, event.payload.call as AutomationCall)?.model ?? model;
  if (event.kind === "access_call") return world.access?.(model, event.payload.call as AccessCall)?.model ?? model;
  return world.apply(model, event) ?? model;
}

/** The world after `events`, each taken in turn. */
function replay(world: PracticeWorld<unknown>, model: unknown, events: CoreEvent[]): unknown {
  return events.reduce((held, event) => applied(world, held, event), model);
}

/** Everything a step's state shows: the model, the snapshot the shell draws from it and each terminal's screen. */
function drawn(world: PracticeWorld<unknown>, model: unknown): string {
  return JSON.stringify({ model, snapshot: world.snapshot(model, NOW), screens: world.panes(model).map((id) => world.screen(model, id, 80)) });
}

const event = (kind: string, payload: Record<string, unknown> = {}): CoreEvent => ({ kind, payload });
const search = (query: string) => event("session_work_search", { query });

/**
 * What the shell sends a world when the learner presses what a step points at, for the steps a command or typed words
 * do not do, keyed `<chapter>/<step>`. The shell's own components send these (`actions.ts`, the pairing and phone
 * screens); a step with no entry here, or one whose events its world leaves unfinished, keeps the state before it.
 */
const PRESSED: Record<string, CoreEvent[] | ((model: never) => CoreEvent[])> = {
  "overview/search": [search("")],
  // The reviewer, once closed and reopened, is the pane the desk numbered after it (`ag<n>`).
  "agents/look": (model: AgentsModel) => [event("pair_view", { pane_id: model.panes.find((pane) => pane.id.startsWith("ag"))?.id ?? "rev", display_id: "view1" })],
  "mission/chip": [event("detail_open", { item: { kind: "mission", mission: "practice-mission" } })],
  "mission/leave": [event("workspace_view", { open: false })],
  "mission/answer": [event("mission_answer", { mission: "practice-mission", key: 1, option: 1 })],
  "find/open": [search("")],
  "find/type": [search("login")],
  "find/pick": [event("detail_open", { item: { kind: "record", session: LOGIN_SESSION } })],
  "find/pr": [search("#3")],
  "find/prPick": [event("pr_open", { pr_number: PR_NUMBER })],
  "find/star": [event("revisit_star", { pane_id: "copy" })],
  "project/close": [event("workspace_view", { open: false })],
  "project/pin": [event("workspace_pin_set", { workspace_id: PIN_TARGET, pinned: true })],
  "gitlab/open": [event("pr_open", { workspace_id: "shop-api", pr_number: MR_NUMBER })],
  "conversation/open": [event("session_thread_open", { pane_id: CONVERSATION_PANE })],
  "conversation/back": [event("session_thread_close")],
  "connect/connect": [event("pairing_join", { link: LINK, request_id: "practice-join" })],
  "connect/same": [event("pairing_compare", { same: true })],
  "automation/make": [event("automation_call", { call: { call: "add", name: "Morning briefing", task: "Summarize last night's merges", schedule: { kind: "daily", time: "08:00" }, agent: { kind: "claude" } } })],
  "bots/make": [event("access_call", { call: { call: "invite", name: "nightly", mode: { kind: "own" } } })],
  "bots/copy": [event(PRACTICE_COPY, { what: "bot sentence" })],
  "answer/pick": [event("practice_phone_pick", { choice: "2" })],
  "answer/answer": [event("practice_phone_answer", { choice: "2" })],
  "attachments/attach": [event("practice_phone_attach")],
  "attachments/send": [event("practice_phone_send")],
};

/** The `key` events the coach's typing sends a pane's terminal: the words after ⌃U, then ⏎ (`typeInto`, `PracticeCoach.tsx`). */
function typedWords(step: PracticeChapter["steps"][number]): CoreEvent[] {
  const typing = step.gesture && "type" in step.gesture ? step.gesture.type : null;
  if (!typing || !("pane" in typing)) return [];
  const words = "words" in typing ? (clientI18n.t(typing.words as MessageKey) as string) : typing.text;
  const key = (text: string): CoreEvent => ({ kind: "key", payload: { pane_id: typing.pane, bytes_base64: btoa(String.fromCharCode(...new TextEncoder().encode(text))) } });
  return [key(`\x15${words}`), key("\r")];
}

/** The inputs to try for `step` from `model`: its own command, then each pane clicked, then every other command. */
function candidates(step: PracticeChapter["steps"][number], inputs: StageInput[]): StageInput[] {
  const own = inputs.filter((input) => input.name === step.command);
  const clicks = inputs.filter((input) => input.name.startsWith("focus "));
  return [...own, ...clicks, ...inputs.filter((input) => !own.includes(input) && !clicks.includes(input))];
}

describe.each(CHAPTERS.map((chapter) => [chapter.id, chapter] as const))("practice chapter %s started over", (_id, chapter) => {
  it("plays from each of its steps to its end in a fresh world as the first run did", () => {
    const { world, inputs, play, uninstall, initial } = stage(chapter, { record: true });
    const runs: StepRun[] = [];
    /** The steps with a `done` the first run did not bring about. */
    const unreached: string[] = [];
    /** The model each step began with, as the first run held it, and what it looked like then. */
    const began: { model: unknown; drawn: string; done: boolean }[] = [];
    let model = initial;
    try {
      for (const step of chapter.steps) {
        began.push({ model, drawn: drawn(world, model), done: step.done?.(model) ?? false });
        const events: CoreEvent[] = [];
        if (step.enter) {
          events.push(step.enter);
          model = applied(world, model, step.enter);
        }
        if (step.done && !step.done(model)) {
          if (step.wait) {
            // The world's own beats, run ahead as the browser's clock is.
            for (let beats = 0; beats < BEATS && !step.done(model); beats += 1) {
              const beat = world.later?.(model);
              if (!beat) break;
              events.push(beat.event);
              model = applied(world, model, beat.event);
            }
          } else {
            let reached = false;
            for (let presses = 0, from = model; !reached && step.command && presses < PRESSES; presses += 1) {
              const pressed = inputs(from).find((input) => input.name === step.command);
              if (!pressed) break;
              const result = play(from, pressed);
              events.push(...result.events);
              from = model = result.model;
              reached = step.done(model);
            }
            const pressed = PRESSED[`${chapter.id}/${step.id}`];
            const typing = [...typedWords(step), ...(typeof pressed === "function" ? pressed(model as never) : (pressed ?? []))];
            if (!reached && typing.length) {
              const after = replay(world, model, typing);
              if (step.done(after)) {
                events.push(...typing);
                model = after;
                reached = true;
              }
            }
            for (const input of reached ? [] : candidates(step, inputs(model))) {
              const result = play(model, input);
              if (!step.done(result.model)) continue;
              events.push(...result.events);
              model = result.model;
              break;
            }
          }
        }
        if (step.done && !step.done(model)) unreached.push(step.id);
        runs.push({ events });
      }
    } finally {
      uninstall();
    }
    const end = drawn(world, model);

    // The first run did something in this chapter, or restarting it from each step would prove nothing.
    expect(runs.flatMap((run) => run.events).length, "the first run sent the world nothing").toBeGreaterThan(0);

    // A step the first run could not do is a step restarting checks only from the state before it: give it its press in `PRESSED`.
    expect(unreached, "steps the first run did not reach, which start-over is then not checked past").toEqual([]);

    // A restart from step k: a fresh world sent what reached the steps before k is the world step k began with.
    chapter.steps.forEach((step, k) => {
      let fresh = world.initial();
      for (const run of runs.slice(0, k)) fresh = replay(world, fresh, run.events);
      expect(drawn(world, fresh), `started over from ${step.id}, the world is not the one the first run began it with`).toBe(began[k]!.drawn);
      expect(step.done?.(fresh) ?? false, `started over from ${step.id}, the step is already done or not done as it was the first time`).toBe(began[k]!.done);
      // And the learner then plays it, and every step after, as the first time: the same events reach the same end.
      for (const run of runs.slice(k)) fresh = replay(world, fresh, run.events);
      expect(drawn(world, fresh), `started over from ${step.id}, the chapter does not play to the end the first run reached`).toBe(end);
    });

    // A model the player held is never changed by the events after it, so a step Back goes to is as it was.
    began.forEach((row, k) => expect(drawn(world, row.model), `a later event changed the model step ${chapter.steps[k]!.id} began with`).toBe(row.drawn));
  });
});
