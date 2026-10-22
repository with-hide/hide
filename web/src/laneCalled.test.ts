// @vitest-environment jsdom
import { afterAll, describe, expect, it, vi } from "vitest";
import { laneCalled } from "./actions";

// Importing the actions loads xterm; answer its canvas probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

const reveal = (is_directory: boolean) => ({
  schema_version: 2 as const,
  kind: "reveal_path" as const,
  payload: { path: "/projects/studio/src", workspace_id: "project:studio", checkout_id: "checkout:studio", is_directory },
});

describe("laneCalled", () => {
  // PRD right-lane B15, B16: a linked file reaches the lane through the
  // core's call number; a linked folder opens the files tool's tree, so the
  // lane comes up at once; an agent or tab chosen leaves it as it is.
  it("puts the lane up for a linked folder or a tool, and nothing for a linked file", () => {
    expect(laneCalled(reveal(true))).toBe(true);
    expect(laneCalled(reveal(false))).toBe(false);
    expect(laneCalled({ schema_version: 2, kind: "workspace_view", payload: { tool: "project" } })).toBe(true);
    expect(laneCalled({ schema_version: 2, kind: "workspace_view", payload: { tool: null } })).toBe(false);
    expect(laneCalled({ schema_version: 2, kind: "workspace_view", payload: { pinned: true } })).toBe(false);
  });

  it("leaves the lane as it is for an agent or tab chosen", () => {
    expect(laneCalled({ schema_version: 2, kind: "focus_pane", payload: { pane_id: "w1:p1" } } as never)).toBe(false);
    expect(laneCalled({ schema_version: 2, kind: "focus_tab", payload: { tab_id: "w1:t2" } } as never)).toBe(false);
    expect(laneCalled({ schema_version: 2, kind: "remote_control", payload: { action: "focus_tab", tab_id: "w1:t2" } } as never)).toBe(false);
  });
});
