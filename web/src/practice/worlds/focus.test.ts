import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/focus";
import type { CoreEvent } from "../world";
import { BUILDER, CHANGE_AFTER, FOCUS_WORLD, OTHER, type FocusModel } from "./focus";

const doneAt = (id: string, model: FocusModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: FocusModel, event: CoreEvent) => FOCUS_WORLD.apply(model, event)!;
const pairView = (pane_id?: string): CoreEvent => ({ kind: "pair_view", payload: pane_id ? { pane_id } : {} });
const focus = (pane_id: string): CoreEvent => ({ kind: "focus_pane", payload: { pane_id, origin: "operator" } });
const view = (model: FocusModel) => FOCUS_WORLD.snapshot(model, Date.now()).rest.workspace_view!;
const beat = (model: FocusModel) => apply(model, FOCUS_WORLD.later!(model)!.event);
const page = (model: FocusModel) => Object.values(FOCUS_WORLD.snapshot(model, Date.now()).frames!.documents!)[0]!.contents_utf8;

describe("the focus flow's world (UI_BEHAVIOR, Practice window)", () => {
  it("starts with two agents in one tab, the page on the builder's desk and focus mode off", () => {
    const model = FOCUS_WORLD.initial();
    expect(FOCUS_WORLD.panes(model)).toEqual([BUILDER, OTHER]);
    const start = view(model);
    expect(start.pair_mode).toEqual({ on: false, ratio: null });
    expect(start.tray.pair).toBeNull();
    expect(start.panes_with_views).toEqual([BUILDER]);
    expect(start.layout!.display_count).toBe(1);
    expect(start.open).toBe(false);
    expect(doneAt("enter", model)).toBe(false);
  });

  it("draws the builder beside its page once focus mode is asked for on it, as the core publishes it", () => {
    const model = apply(FOCUS_WORLD.initial(), pairView(BUILDER));
    const drawn = view(model);
    expect(drawn.pair_mode).toEqual({ on: true, ratio: null });
    expect(drawn.tray.pair).toEqual({ tab_id: "t1", pane_id: BUILDER, alone: false, shown: 1, folded: [] });
    expect(drawn.layout!.root).toMatchObject({ area: { active: "view1" } });
    expect(doneAt("enter", model)).toBe(true);
    expect(FOCUS_WORLD.later!(model)).toMatchObject({ event: { kind: "practice_beat" } });
  });

  it("draws the other agent alone, with no page to show, and takes the mode with a pane chosen", () => {
    const alone = apply(FOCUS_WORLD.initial(), pairView(OTHER));
    expect(view(alone).tray.pair).toMatchObject({ pane_id: OTHER, alone: true, shown: 0 });
    expect(doneAt("enter", alone)).toBe(false);
    const back = apply(alone, focus(BUILDER));
    expect(view(back).tray.pair).toMatchObject({ pane_id: BUILDER, alone: false });
    expect(doneAt("enter", back)).toBe(true);
    expect(view(apply(back, focus(OTHER))).tray.pair).toMatchObject({ pane_id: OTHER, alone: true });
  });

  it("changes the page a few seconds after the mode is entered, whether or not it is still on", () => {
    const entered = apply(FOCUS_WORLD.initial(), pairView(BUILDER));
    const wait = FOCUS_WORLD.later!(entered)!;
    expect(wait.at - entered.enteredAt!).toBe(CHANGE_AFTER);
    expect(page(entered)).toContain("# Spring sale");
    expect(doneAt("live", entered)).toBe(false);
    const changed = beat(apply(entered, pairView()));
    expect(page(changed)).toContain("# Summer sale");
    expect(doneAt("live", changed)).toBe(true);
    expect(FOCUS_WORLD.later!(changed)).toBeNull();
    expect(FOCUS_WORLD.screen(changed, BUILDER, 80)).toContain("Summer sale");
  });

  it("plays no change before the mode was entered", () => {
    expect(FOCUS_WORLD.later!(FOCUS_WORLD.initial())).toBeNull();
  });

  it("leaves the mode on its way back and answers nothing when it is already off", () => {
    const on = apply(FOCUS_WORLD.initial(), pairView(BUILDER));
    const off = apply(on, pairView());
    expect(view(off).pair_mode!.on).toBe(false);
    expect(view(off).tray.pair).toBeNull();
    expect(doneAt("leave", off)).toBe(true);
    expect(FOCUS_WORLD.apply(off, pairView())).toBeNull();
    expect(FOCUS_WORLD.apply(on, pairView(BUILDER))).toBeNull();
  });

  it("folds the desk to the pane alone and stands it again, and asks nothing for a pane with no page", () => {
    const on = apply(FOCUS_WORLD.initial(), pairView(BUILDER));
    const folded = apply(on, { kind: "pair_view_desk", payload: { shown: false } });
    expect(view(folded).tray.pair).toMatchObject({ pane_id: BUILDER, alone: true, shown: 1 });
    expect(view(apply(folded, { kind: "pair_view_desk", payload: { shown: true } })).tray.pair).toMatchObject({ alone: false });
    const alone = apply(FOCUS_WORLD.initial(), pairView(OTHER));
    expect(FOCUS_WORLD.apply(alone, { kind: "pair_view_desk", payload: { shown: true } })).toBeNull();
  });

  it("draws the pane alone when the page's tab is closed, and keeps the mode on", () => {
    const on = apply(FOCUS_WORLD.initial(), pairView(BUILDER));
    const closed = apply(on, { kind: "view_layout", payload: { action: "close", display_id: "view1" } });
    expect(view(closed).tray.pair).toMatchObject({ pane_id: BUILDER, alone: true, shown: 0 });
    expect(view(closed).panes_with_views).toEqual([]);
    expect(view(closed).pair_mode!.on).toBe(true);
  });
});
