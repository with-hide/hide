// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { Actions } from "./actions";
import { LaneDivider } from "./LaneDivider";
import type { LaneFrame, LaneSizes } from "./workspace";

// The lane's divider keys (PRD right-lane B6, B20) against the two widths the
// lane keeps: Files' and the one the other tools share (docs/UI_BEHAVIOR.md,
// The right lane).

const sizes: LaneSizes = { agentMin: 480, laneMin: 360, laneIdeal: 520, treeMin: 180, treeIdeal: 240, divider: 8 };
const files: LaneFrame = { mode: "overlay", field: "lane_width", agents: 1600, width: 520, document: 520, tree: null, treeOnly: false, narrowPin: false };

let unmount: (() => void) | null = null;
afterEach(() => {
  unmount?.();
  unmount = null;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

function mount() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const origin = document.createElement("div");
  Object.defineProperty(origin, "clientWidth", { value: 1600 });
  const host = document.createElement("div");
  document.body.append(origin, host);
  const sent: Record<string, unknown>[] = [];
  const actions = { setWorkspaceView: vi.fn((patch: Record<string, unknown>) => { sent.push(patch); return true; }) } as unknown as Actions;
  const root = createRoot(host);
  const render = (frame: LaneFrame) => act(() => root.render(<LaneDivider kind="lane" label="Resize" workspace="local:/w" acknowledged={null} frame={frame} origin={origin} sizes={sizes} setGuide={() => {}} actions={actions} />));
  unmount = () => act(() => root.unmount());
  const press = () => act(() => { host.querySelector('[data-lane-divider="lane"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })); });
  return { render, press, sent };
}

it("lands a key pressed after the lane turns from Files to another tool in that tool's width, not on Files' unconfirmed step", () => {
  const divider = mount();
  divider.render(files);
  divider.press();
  expect(divider.sent).toEqual([{ lane_width: 552, width_request_id: expect.any(String) }]);
  // Memory at the same width, before the core confirmed the first step.
  divider.render({ ...files, field: "panel_width" });
  divider.press();
  expect(divider.sent[1]).toEqual({ panel_width: 552, width_request_id: expect.any(String) });
});
