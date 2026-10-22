// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { Actions } from "./actions";
import { LaneDivider } from "./LaneDivider";
import type { LaneFrame, LaneSizes } from "./workspace";

// The lane's divider keys (PRD right-lane B6, B20; lane-two-columns B17)
// against the widths the lane keeps: the documents' and the one Memory, PRs
// and Changes share (docs/UI_BEHAVIOR.md, The right lane).

const sizes: LaneSizes = { agentMin: 480, viewMin: 360, viewIdeal: 520, lanePage: 900, explorerMin: 180, explorerIdeal: 240, panelMin: 360, panelIdeal: 400, divider: 8 };
const documents: LaneFrame = { mode: "overlay", agents: 1600, width: 520, view: { field: "document_width", width: 520, min: 360 }, tool: null, stacked: false, narrowPin: false };

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

it("lands a key pressed after the documents give way to a tool alone in that tool's width, not on the documents' unconfirmed step", () => {
  const divider = mount();
  divider.render(documents);
  divider.press();
  expect(divider.sent).toEqual([{ document_width: 552, width_request_id: expect.any(String) }]);
  // Memory alone at the same width, before the core confirmed the first step.
  divider.render({ ...documents, view: null, tool: { field: "panel_width", width: 520, min: 360 } });
  divider.press();
  expect(divider.sent[1]).toEqual({ panel_width: 552, width_request_id: expect.any(String) });
});
