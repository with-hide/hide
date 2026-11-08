import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/devices";
import type { CoreEvent } from "../world";
import { DEVICES_WORLD, MINI, type DevicesModel } from "./devices";

const doneAt = (id: string, model: DevicesModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: DevicesModel, event: CoreEvent) => DEVICES_WORLD.apply(model, event)!;
const snapshot = (model: DevicesModel) => DEVICES_WORLD.snapshot(model, Date.now());
const focusDevice = (device_id: string): CoreEvent => ({ kind: "focus_device", payload: { device_id } });
const newTabThere: CoreEvent = { kind: "remote_control", payload: { target_id: MINI, request_id: "r1", action: "create_tab", workspace_id: "billing-api", checkout_id: "billing-api:main", cwd: "/work/billing-api", label: "3" } };

describe("the several-computers flow's script (UI_BEHAVIOR, Practice window)", () => {
  it("marks the other computer's tile with what waits on you there before it is opened", () => {
    const rest = snapshot(DEVICES_WORLD.initial()).rest;
    expect(rest.ui_state!.device_rail_visible).toBe(true);
    expect(rest.navigator!.devices!.map((device) => [device.id, device.agent_scope.groups.needs_you])).toEqual([["local", 0], [MINI, 1]]);
  });

  it("brings the other computer forward, and its projects and agents come with it", () => {
    const model = apply(DEVICES_WORLD.initial(), focusDevice(MINI));
    expect(doneAt("tile", model)).toBe(true);
    const rest = snapshot(model).rest;
    expect(rest.navigator!.focused_device_id).toBe(MINI);
    const session = rest.status!.remote!.find((row) => row.target_id === MINI)!.session!;
    expect(session.workspaces.map((row) => [row.id, row.device_id])).toEqual([["billing-api", MINI]]);
    expect(session.agents.every((row) => row.device_id === MINI)).toBe(true);
  });

  it("opens a new tab on the other computer when the request names it, and no step is done by this Mac's tabs", () => {
    const there = apply(DEVICES_WORLD.initial(), focusDevice(MINI));
    expect(doneAt("start", there)).toBe(false);
    const started = apply(there, newTabThere);
    expect(doneAt("start", started)).toBe(true);
    const session = snapshot(started).rest.status!.remote![0]!.session!;
    expect(session.pane_layouts).toHaveLength(3);
    expect(DEVICES_WORLD.panes(started)).toHaveLength(3);
  });

  it("is done once the learner is back on this Mac after starting there", () => {
    const started = apply(apply(DEVICES_WORLD.initial(), focusDevice(MINI)), newTabThere);
    expect(doneAt("back", started)).toBe(false);
    expect(doneAt("back", apply(started, focusDevice("local")))).toBe(true);
  });

  it("refuses a device it does not know and a request for another target", () => {
    expect(DEVICES_WORLD.apply(DEVICES_WORLD.initial(), focusDevice("nowhere"))).toBeNull();
    expect(DEVICES_WORLD.apply(DEVICES_WORLD.initial(), { ...newTabThere, payload: { ...newTabThere.payload, target_id: "other" } })).toBeNull();
  });
});
