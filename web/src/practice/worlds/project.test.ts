import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/project";
import type { CoreEvent } from "../world";
import { isPinned, PIN_TARGET, PROJECT_WORLD } from "./project";

const pin = (workspace_id: string, pinned: boolean): CoreEvent => ({ kind: "workspace_pin_set", payload: { workspace_id, pinned } });
const listed = (model = PROJECT_WORLD.initial()) => PROJECT_WORLD.snapshot(model, Date.now()).rest.navigator!.workspaces!.map((row) => `${row.id}:${row.pinned}`);

describe("the project flow's pin (UI_BEHAVIOR, Practice window)", () => {
  it("lists two projects with none pinned, the pin step not yet done", () => {
    const model = PROJECT_WORLD.initial();
    expect(listed(model)).toEqual(["hide-practice:false", "docs-site:false"]);
    expect(chapter.steps.find((step) => step.id === "pin")!.done!(model)).toBe(false);
  });

  it("lists a pinned project first and sets it back with the pin pressed again", () => {
    const pinned = PROJECT_WORLD.apply(PROJECT_WORLD.initial(), pin(PIN_TARGET, true))!;
    expect(isPinned(pinned, PIN_TARGET)).toBe(true);
    expect(listed(pinned)).toEqual(["docs-site:true", "hide-practice:false"]);
    expect(chapter.steps.find((step) => step.id === "pin")!.done!(pinned)).toBe(true);
    expect(listed(PROJECT_WORLD.apply(pinned, pin(PIN_TARGET, false))!)).toEqual(["hide-practice:false", "docs-site:false"]);
  });

  it("answers nothing to a pin it has no project for, or one already as asked", () => {
    expect(PROJECT_WORLD.apply(PROJECT_WORLD.initial(), pin("elsewhere", true))).toBeNull();
    expect(PROJECT_WORLD.apply(PROJECT_WORLD.initial(), pin(PIN_TARGET, false))).toBeNull();
  });
});
