import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/mission";
import type { CoreEvent } from "../world";
import { CONDUCTOR, MISSION_WORLD, WORKER, type MissionModel } from "./mission";

const doneAt = (id: string, model: MissionModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const typedInto = (pane_id: string, text: string): CoreEvent => ({ kind: "key", payload: { pane_id, bytes_base64: btoa(text) } });
/** The banner's answer, as `actions.answerMission` dispatches it. */
const answer = (payload: Record<string, unknown>): CoreEvent => ({ kind: "mission_answer", payload: { mission: "practice-mission", key: 1, ...payload } });

const planned = () => MISSION_WORLD.apply(MISSION_WORLD.initial(), typedInto(CONDUCTOR, "a mission\r"))!;
const running = () => MISSION_WORLD.apply(planned(), typedInto(CONDUCTOR, "go\r"))!;
const missionIn = (model: MissionModel) => MISSION_WORLD.snapshot(model, Date.now()).rest.missions!.items[0]!;
const focus = (pane_id: string): CoreEvent => ({ kind: "focus_pane", payload: { pane_id, request_id: `focus-${pane_id}` } });

describe("chapter 0's plan, approved before anything runs (UI_BEHAVIOR, Practice window)", () => {
  it("plans every part and asks for the go, with no worker started", () => {
    const model = planned();
    const mission = missionIn(model);
    expect(mission.status).toBe("planning");
    expect(mission.nodes.map((node) => node.status)).toEqual(["planned", "planned", "planned"]);
    expect(MISSION_WORLD.panes(model)).toEqual([CONDUCTOR]);
    expect(doneAt("approve", model)).toBe(false);
  });

  it("starts the workers on the go, each in its own tab", () => {
    const model = running();
    expect(doneAt("approve", model)).toBe(true);
    expect(missionIn(model).status).toBe("running");
    expect(MISSION_WORLD.panes(model)).toHaveLength(3);
  });
});

describe("chapter 0's decision, answered on the map's banner (UI_BEHAVIOR, Practice window)", () => {
  it("asks with the Conductor's choices and the one it recommends", () => {
    const question = missionIn(running()).question!;
    expect(question.options!.length).toBeGreaterThan(1);
    expect(question.options![question.recommend! - 1]).toBeDefined();
  });

  it("changes the plan on a pressed choice or a written answer, and the Conductor reads which", () => {
    const model = running();
    const chosen = missionIn(model).question!.options![0]!;
    for (const [given, words] of [[{ option: 1 }, chosen], [{ text: "  keep them apart  " }, "keep them apart"]] as const) {
      const after = MISSION_WORLD.apply(model, answer(given))!;
      expect(doneAt("answer", after)).toBe(true);
      expect(missionIn(after).decision).toBeNull();
      expect(MISSION_WORLD.screen(after, CONDUCTOR, 200)).toContain(`→ ${words}`);
    }
  });

  it("refuses an answer to another question, a choice it never gave, or one with nothing asked", () => {
    const model = running();
    expect(MISSION_WORLD.apply(model, answer({ key: 2, option: 1 }))).toBeNull();
    expect(MISSION_WORLD.apply(model, answer({ option: 9 }))).toBeNull();
    expect(MISSION_WORLD.apply(model, answer({ text: "   " }))).toBeNull();
    expect(MISSION_WORLD.apply(MISSION_WORLD.initial(), answer({ option: 1 }))).toBeNull();
  });

  it("still takes the answer said in the conversation", () => {
    expect(doneAt("answer", MISSION_WORLD.apply(running(), typedInto(CONDUCTOR, "yes\r"))!)).toBe(true);
  });
});

describe("chapter 0's end: the worker looked in on, the report, and Memory (UI_BEHAVIOR, Practice window)", () => {
  const answered = () => MISSION_WORLD.apply(running(), answer({ option: 1 }))!;

  it("finishes once the learner has looked in on the worker and comes back", () => {
    const back = MISSION_WORLD.apply(answered(), focus(CONDUCTOR));
    expect(back === null || !doneAt("back", back)).toBe(true);
    const visited = MISSION_WORLD.apply(answered(), focus(WORKER))!;
    expect(doneAt("back", visited)).toBe(false);
    const done = MISSION_WORLD.apply(visited, focus(CONDUCTOR))!;
    expect(doneAt("back", done)).toBe(true);
    expect(missionIn(done).report).not.toBeNull();
  });

  it("keeps the decision in Memory only once the work is done", () => {
    const cards = (model: MissionModel) => MISSION_WORLD.snapshot(model, Date.now()).frames!.memory!.project!.rows.length;
    const done = MISSION_WORLD.apply(MISSION_WORLD.apply(answered(), focus(WORKER))!, focus(CONDUCTOR))!;
    expect(cards(done)).toBe(cards(answered()) + 1);
  });
});
