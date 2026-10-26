import { describe, expect, it } from "vitest";
import chapter from "../chapters/mission";
import type { CoreEvent } from "../world";
import { CONDUCTOR, MISSION_WORLD, type MissionModel } from "./mission";

const doneAt = (id: string, model: MissionModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const typedInto = (pane_id: string, text: string): CoreEvent => ({ kind: "key", payload: { pane_id, bytes_base64: btoa(text) } });
/** The banner's answer, as `actions.answerMission` dispatches it. */
const answer = (payload: Record<string, unknown>): CoreEvent => ({ kind: "mission_answer", payload: { mission: "practice-mission", key: 1, ...payload } });

const running = () => MISSION_WORLD.apply(MISSION_WORLD.initial(), typedInto(CONDUCTOR, "a mission\r"))!;
const missionIn = (model: MissionModel) => MISSION_WORLD.snapshot(model, Date.now()).rest.missions!.items[0]!;

describe("chapter 8's decision, answered on the map's banner (UI_BEHAVIOR, Practice window)", () => {
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
