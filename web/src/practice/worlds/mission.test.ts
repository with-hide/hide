import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/mission";
import type { CoreEvent } from "../world";
import { BEATS, CONDUCTOR, MISSION_WORLD, type MissionModel } from "./mission";

const doneAt = (id: string, model: MissionModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const typedInto = (pane_id: string, text: string): CoreEvent => ({ kind: "key", payload: { pane_id, bytes_base64: btoa(text) } });
/** The banner's answer, as `actions.answerMission` dispatches it. */
const answer = (payload: Record<string, unknown>): CoreEvent => ({ kind: "mission_answer", payload: { mission: "practice-mission", key: 1, ...payload } });
const apply = (model: MissionModel, event: CoreEvent) => MISSION_WORLD.apply(model, event)!;
/** The Conductor's own turn ending: the beat the world asks for, played. */
const beat = (model: MissionModel) => apply(model, MISSION_WORLD.later!(model)!.event);
const missionIn = (model: MissionModel) => MISSION_WORLD.snapshot(model, Date.now()).rest.missions!.items[0] ?? null;

const planning = () => apply(MISSION_WORLD.initial(), typedInto(CONDUCTOR, "a mission\r"));
const planned = () => beat(planning());
const running = () => apply(planned(), typedInto(CONDUCTOR, "go\r"));
const asking = () => beat(beat(running()));
/** The work after the answer runs to the end: the mail, the check, the report. */
const finish = (model: MissionModel) => beat(beat(beat(model)));
const answered = () => apply(asking(), answer({ option: 1 }));

describe("chapter 0's plan, approved before anything runs (UI_BEHAVIOR, Practice window)", () => {
  it("plans for a while before the plan stands", () => {
    const model = planning();
    expect(missionIn(model)).toBeNull();
    expect(doneAt("ask", model)).toBe(true);
    expect(doneAt("thinking", model)).toBe(false);
    expect(MISSION_WORLD.later!(model)!.at - model.times[1]!).toBe(BEATS[1]);
  });

  it("plans every part and asks for the go, with no worker started", () => {
    const model = planned();
    expect(doneAt("thinking", model)).toBe(true);
    expect(missionIn(model)!.status).toBe("planning");
    expect(missionIn(model)!.nodes.map((node) => node.status)).toEqual(Array(6).fill("planned"));
    expect(MISSION_WORLD.panes(model)).toEqual([CONDUCTOR]);
    expect(MISSION_WORLD.later!(model)).toBeNull();
  });

  it("starts the survey on the go, in its own tab, and asks nothing yet", () => {
    const model = running();
    expect(doneAt("approve", model)).toBe(true);
    expect(missionIn(model)!.status).toBe("running");
    expect(missionIn(model)!.question).toBeNull();
    expect(MISSION_WORLD.panes(model)).toHaveLength(2);
  });

  it("starts the next parts as the ones they wait on finish, two at a time", () => {
    const working = (model: MissionModel) => missionIn(model)!.nodes.filter((node) => node.status === "working").map((node) => node.key);
    expect(working(running())).toEqual(["survey"]);
    expect(working(beat(running()))).toEqual(["db", "screen"]);
    expect(working(asking())).toEqual(["screen", "api"]);
  });
});

describe("chapter 0's question, from 내 차례 to the map's banner (UI_BEHAVIOR, Practice window)", () => {
  it("comes a while into the run, with the Conductor's choices and the one it recommends", () => {
    const question = missionIn(asking())!.question!;
    expect(doneAt("running", asking())).toBe(true);
    expect(question.options![question.recommend! - 1]).toBeDefined();
  });

  it("is looked at when the Conductor is opened while it asks", () => {
    const opened = apply(asking(), { kind: "focus_pane", payload: { pane_id: CONDUCTOR, origin: "operator" } });
    expect(doneAt("needsyou", opened)).toBe(true);
  });

  it("changes the plan on a pressed choice or a written answer, and the Conductor reads which", () => {
    const model = asking();
    const chosen = missionIn(model)!.question!.options![0]!;
    for (const [given, words] of [[{ option: 1 }, chosen], [{ text: "  keep them apart  " }, "keep them apart"]] as const) {
      const after = apply(model, answer(given));
      expect(doneAt("answer", after)).toBe(true);
      expect(missionIn(after)!.decision).toBeNull();
      expect(MISSION_WORLD.screen(after, CONDUCTOR, 200)).toContain(`→ ${words}`);
    }
  });

  it("refuses an answer to another question, a choice it never gave, or one with nothing asked", () => {
    const model = asking();
    expect(MISSION_WORLD.apply(model, answer({ key: 2, option: 1 }))).toBeNull();
    expect(MISSION_WORLD.apply(model, answer({ option: 9 }))).toBeNull();
    expect(MISSION_WORLD.apply(model, answer({ text: "   " }))).toBeNull();
    expect(MISSION_WORLD.apply(running(), answer({ option: 1 }))).toBeNull();
  });

  it("still takes the answer said in the conversation", () => {
    expect(doneAt("answer", apply(asking(), typedInto(CONDUCTOR, "yes\r")))).toBe(true);
  });
});

describe("chapter 0's end: the report and Memory (UI_BEHAVIOR, Practice window)", () => {
  it("finishes a while after the answer, with the Conductor's report", () => {
    expect(doneAt("graph", answered())).toBe(false);
    const done = finish(answered());
    expect(doneAt("graph", done)).toBe(true);
    expect(missionIn(done)!.report).not.toBeNull();
    expect(MISSION_WORLD.later!(done)).toBeNull();
  });

  it("keeps the decision in Memory only once the work is done", () => {
    const cards = (model: MissionModel) => MISSION_WORLD.snapshot(model, Date.now()).frames!.memory!.project!.rows.length;
    expect(cards(finish(answered()))).toBe(cards(answered()) + 1);
  });

  it("adds the part that takes password sign-in out only when the answer drops it", () => {
    const keys = (model: MissionModel) => missionIn(model)!.nodes.map((node) => node.key);
    expect(keys(answered())).not.toContain("drop");
    const dropped = apply(asking(), answer({ option: 2 }));
    expect(keys(dropped)).toContain("drop");
    expect(missionIn(finish(dropped))!.nodes.every((node) => node.status === "done")).toBe(true);
  });
});
