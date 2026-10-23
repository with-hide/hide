import { describe, expect, it } from "vitest";
import chapter from "../chapters/switch";
import type { CoreEvent } from "../world";
import { SWITCH_WORLD, type SwitchModel } from "./switch";

const doneAt = (id: string, model: SwitchModel) => chapter.steps.find((step) => step.id === id)!.done!(model);

/** The model after each event in turn, every one of them answered. */
function play(...events: CoreEvent[]): SwitchModel {
  return events.reduce((model, event) => {
    const after = SWITCH_WORLD.apply(model, event);
    expect(after, `${event.kind} moved nothing`).not.toBeNull();
    return after!;
  }, SWITCH_WORLD.initial());
}

// A held ⌃⇥ commits the agent it stopped on as one `focus_pane`; a project comes forward by its last tab or its checkout.
const toPane = (pane_id: string): CoreEvent => ({ kind: "focus_pane", payload: { pane_id, origin: "operator" } });
const toTab = (workspace_id: string, tab_id: string): CoreEvent => ({ kind: "focus_tab", payload: { workspace_id, tab_id } });
const toCheckout = (workspace_id: string): CoreEvent => ({ kind: "focus_checkout", payload: { workspace_id, checkout_id: `${workspace_id}:main` } });

describe("chapter 2's moves between agents and projects", () => {
  it("finishes the recent step on whichever agent ⌃⇥ stopped on, and the project step on the other project", () => {
    const once = play(toPane("login"));
    expect(doneAt("recent", once)).toBe(true);
    expect(doneAt("project", once)).toBe(false);
    expect(doneAt("project", play(toPane("login"), toCheckout("practice-docs")))).toBe(true);

    // ⌃ held over ⇥ twice lands on the docs agent, in the other project; the project before is then practice-app.
    const twice = play(toPane("docs"));
    expect(doneAt("recent", twice)).toBe(true);
    expect(doneAt("project", twice)).toBe(false);
    expect(doneAt("project", play(toPane("docs"), toTab("practice-app", "app2")))).toBe(true);
  });

  it("keeps the recent step open while the keyboard stays on the agent it started in", () => {
    expect(doneAt("recent", play(toTab("practice-app", "app1")))).toBe(false);
  });
});
