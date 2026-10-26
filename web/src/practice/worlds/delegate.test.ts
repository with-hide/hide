import { describe, expect, it } from "vitest";
import chapter from "../chapters/delegate";
import type { CoreEvent } from "../world";
import { DELEGATE_WORLD, DOCS, LEAD, SIGNUP, type DelegateModel } from "./delegate";

const doneAt = (id: string, model: DelegateModel) => chapter.steps.find((step) => step.id === id)!.done!(model);

/** Bytes typed into `pane`'s terminal, as the shell sends them: what @, a dropped row or a paste types arrives the same way. */
const typedInto = (pane_id: string, text: string): CoreEvent => ({ kind: "key", payload: { pane_id, bytes_base64: btoa(String.fromCharCode(...new TextEncoder().encode(text))) } });

function play(model: DelegateModel, ...events: CoreEvent[]): DelegateModel {
  return events.reduce((current, event) => DELEGATE_WORLD.apply(current, event) ?? current, model);
}

/** The world after the first three requests, where C begins. */
const atC = () => play(DELEGATE_WORLD.initial(), typedInto(LEAD, "a child\r"), typedInto(LEAD, "tell it\r"), typedInto(LEAD, "the docs writer\r"));

describe("chapter 4's C: naming the signup agent by its reference", () => {
  it("takes the reference @ types into the lead's prompt, with no Enter, as the mention step done and nothing sent", () => {
    const mentioned = play(atC(), typedInto(LEAD, `@claude (pane ${SIGNUP} · Signup copy) `));
    expect(doneAt("mention", mentioned)).toBe(true);
    expect(mentioned.stage).toBe(3);
    // A reference alone is not the learner's words yet, so ⏎ types the step's example in its place.
    expect(DELEGATE_WORLD.prompt!(mentioned, LEAD).trim()).toBe("");
    expect(DELEGATE_WORLD.prompt!(play(mentioned, typedInto(LEAD, "fix the copy")), LEAD).trim()).toBe("fix the copy");
  });

  it("leaves the mention step open for another agent's reference", () => {
    expect(doneAt("mention", play(atC(), typedInto(LEAD, `@codex (pane ${DOCS} · Update the API docs) `)))).toBe(false);
  });

  it("sends the work to the signup agent only when the request holds its pane ID, and asks which agent otherwise", () => {
    const asked = play(atC(), typedInto(LEAD, "update the signup copy\r"));
    expect(asked.stage).toBe(3);
    expect(DELEGATE_WORLD.screen(asked, LEAD, 80)).toContain("Choose it with the @ on my header");

    const sent = play(atC(), typedInto(LEAD, `@claude (pane ${SIGNUP} · Signup copy) update the signup copy\r`));
    expect(doneAt("send", sent)).toBe(true);
    expect(DELEGATE_WORLD.screen(sent, SIGNUP, 80)).toContain("Update the signup copy for the new timeout.");
  });
});
