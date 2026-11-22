import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/answer";
import type { CoreEvent } from "../world";
import { ANSWER_WORLD, ASKING, type AnswerModel } from "./answer";

const doneAt = (id: string, model: AnswerModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: AnswerModel, event: CoreEvent) => ANSWER_WORLD.apply(model, event);
const press = (kind: string, choice: string): CoreEvent => ({ kind, payload: { choice } });
const row = (model: AnswerModel) => ANSWER_WORLD.snapshot(model, Date.now()).agents.find((agent) => agent.pane_id === ASKING)!;
const phone = (model: AnswerModel) => ANSWER_WORLD.phone!(model);

describe("the answer flow's world (UI_BEHAVIOR, Practice window)", () => {
  it("marks a choice on the phone without answering, and sends it with 답하기 as the computer takes it", () => {
    const start = ANSWER_WORLD.initial();
    expect(row(start)).toMatchObject({ group: "needs_you", demand: "question" });
    expect(phone(start).ask?.choices.map((choice) => choice.id)).toEqual(["1", "2", "3"]);
    expect(ANSWER_WORLD.screen(start, ASKING, 80)).toContain("❯ 1.");

    const marked = apply(start, press("practice_phone_pick", "2"))!;
    expect(doneAt("pick", marked)).toBe(true);
    expect(doneAt("answer", marked)).toBe(false);
    expect(ANSWER_WORLD.later!(marked)).toBeNull();
    expect(apply(marked, press("practice_phone_pick", "9"))).toBeNull();

    // The keys took the highlight to the chosen row; the agent goes on a moment later.
    const sent = apply(marked, press("practice_phone_answer", "2"))!;
    expect(doneAt("answer", sent)).toBe(true);
    expect(phone(sent)).toMatchObject({ picked: "2", answered: true });
    expect(ANSWER_WORLD.screen(sent, ASKING, 80)).toContain("❯ 2.");
    expect(apply(sent, press("practice_phone_answer", "1"))).toBeNull();

    const beat = ANSWER_WORLD.later!(sent)!;
    const went = apply(sent, beat.event)!;
    expect(doneAt("keys", went)).toBe(true);
    expect(row(went)).toMatchObject({ group: "working", demand: "none" });
    expect(phone(went).ask).toBeNull();
    expect(phone(went).running).toBe(went.movedAt);
    expect(phone(went).said.map((said) => said.who)).toEqual(["agent", "you", "agent"]);
    expect(ANSWER_WORLD.later!(went)).toBeNull();
  });
});
