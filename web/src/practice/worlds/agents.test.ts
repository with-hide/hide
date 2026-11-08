import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/agents";
import type { CoreEvent } from "../world";
import { AGENTS_WORLD, LEAD, REV, type AgentsModel } from "./agents";

const doneAt = (id: string, model: AgentsModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const typedInto = (pane_id: string, text: string): CoreEvent => ({ kind: "key", payload: { pane_id, bytes_base64: btoa(text) } });
const apply = (model: AgentsModel, event: CoreEvent) => AGENTS_WORLD.apply(model, event)!;
/** An agent's own turn ending: the beat the world asks for, played. */
const beat = (model: AgentsModel) => apply(model, AGENTS_WORLD.later!(model)!.event);
const rows = (model: AgentsModel) => AGENTS_WORLD.snapshot(model, Date.now()).rest.navigator!.agents!;
const row = (model: AgentsModel, pane: string) => rows(model).find((one) => one.pane_id === pane)!;
const cards = (model: AgentsModel) => AGENTS_WORLD.snapshot(model, Date.now()).rest.navigator!.devices![0]!.agent_scope.tasks!.cards;
const focus = (pane_id: string, extra: Record<string, unknown> = {}): CoreEvent => ({ kind: "focus_pane", payload: { pane_id, origin: "operator", ...extra } });

const started = () => apply(AGENTS_WORLD.initial(), typedInto(LEAD, "start a reviewer\r"));
const asking = () => beat(beat(started()));
const answered = () => apply(apply(asking(), focus(REV)), typedInto(REV, "yes\r"));
const looking = () => beat(answered());

describe("the several-agents flow's script (UI_BEHAVIOR, Practice window)", () => {
  it("starts the reviewer as an agent of its own, in a pane beside the lead, on the lead's first words", () => {
    const model = started();
    expect(doneAt("words", model)).toBe(true);
    expect(AGENTS_WORLD.panes(model)).toEqual([LEAD, "docs", REV]);
    expect(model.tabs[0]!.root).toMatchObject({ type: "split", first: { pane_id: LEAD }, second: { pane_id: REV } });
    expect(model.tabs).toHaveLength(2);
    expect(model.tabs[0]!.focused).toBe(LEAD);
    expect(row(model, REV).delegated).toBeFalsy();
  });

  it("puts a finished agent in Unread and only the one that asks in Needs You", () => {
    const model = asking();
    expect(doneAt("arrives", model)).toBe(true);
    const states = Object.fromEntries(cards(model).map((card) => [card.root, card.state]));
    expect(states).toEqual({ [REV]: "needs_you", [LEAD]: "working", docs: "done" });
    expect(cards(model)[0]!.root).toBe(REV);
  });

  it("is looked at when the reviewer is opened while it asks, and answers when the terminal does", () => {
    expect(doneAt("ask", asking())).toBe(false);
    expect(doneAt("ask", apply(asking(), focus(REV)))).toBe(true);
    expect(doneAt("answer", answered())).toBe(true);
    expect(AGENTS_WORLD.apply(asking(), typedInto("docs", "yes\r"))).toBeNull();
  });

  it("brings a closed reviewer back with its conversation and its View still waiting", () => {
    const closed = apply(looking(), { kind: "close_tab", payload: { tab_id: "t1" } });
    expect(doneAt("close", closed)).toBe(true);
    expect(AGENTS_WORLD.panes(closed)).not.toContain(REV);
    const back = apply(closed, { kind: "reopen_closed", payload: {} });
    expect(doneAt("reopen", back)).toBe(true);
    expect(rows(back).find((one) => one.look)?.look?.label).toBe("review-notes.md");
  });

  it("marks the reviewer 볼 것 and shows its file only when that row is pressed", () => {
    const model = looking();
    expect(doneAt("finishes", model)).toBe(true);
    expect(row(model, REV).mark).toBe("look");
    expect(row(model, REV).look?.label).toBe("review-notes.md");
    // Pressing the pane without the row's reveal leaves the View unseen.
    expect(doneAt("look", apply(model, focus(REV)))).toBe(false);
    const seen = apply(model, focus(REV, { reveal_unseen: true }));
    expect(doneAt("look", seen)).toBe(true);
    expect(row(seen, REV).mark).toBe("quiet");
    expect(AGENTS_WORLD.snapshot(seen, Date.now()).frames!.documents!.file1!.contents_utf8).toContain("Review of the checkout fix");
  });

  it("goes back to the lead after the reviewer came back", () => {
    const back = apply(apply(looking(), { kind: "close_tab", payload: { tab_id: "t1" } }), { kind: "reopen_closed", payload: {} });
    expect(doneAt("switch", back)).toBe(false);
    expect(doneAt("switch", apply(back, focus(LEAD)))).toBe(true);
  });

  it("marks the files the lead changed in the Explorer", () => {
    const frames = AGENTS_WORLD.snapshot(AGENTS_WORLD.initial(), Date.now()).frames!;
    expect(frames.changes!.entries.map((entry) => entry.relative_path)).toContain("src/payment/timeout.ts");
    expect(doneAt("changes", apply(AGENTS_WORLD.initial(), { kind: "workspace_view", payload: { tool: "explorer" } }))).toBe(true);
  });
});
