import { createActions } from "./actions";
import { useShellStore } from "./store";
import { describe, expect, it } from "vitest";
import { draggedSidebarWidth, sidebarWidthToSend } from "./sidebarWidth";

const bounds = { min: 220, max: 440 };

describe("draggedSidebarWidth", () => {
  it("follows the pointer between the bounds", () => {
    expect(draggedSidebarWidth(292, 40, bounds)).toBe(332);
    expect(draggedSidebarWidth(292, -50.4, bounds)).toBe(242);
  });

  it("stops at the bounds when the pointer passes them (D-18)", () => {
    expect(draggedSidebarWidth(292, -200, bounds)).toBe(220);
    expect(draggedSidebarWidth(292, 500, bounds)).toBe(440);
    expect(draggedSidebarWidth(230, -10.6, bounds)).toBe(220);
  });
});

describe("sidebarWidthToSend", () => {
  it("sends only a width that moved", () => {
    expect(sidebarWidthToSend(292, 292)).toBeNull();
    expect(sidebarWidthToSend(360, 292)).toBe(292);
  });
});

it("an unrelated UI change cannot echo an older width over a completed drag", () => {
  const sent: { kind: string; payload: Record<string, unknown> }[] = [];
  useShellStore.setState({ rest: { ui_state: { sidebar_width: 292, left_sidebar_visible: true } } });
  const actions = createActions((event) => {
    sent.push(event as { kind: string; payload: Record<string, unknown> });
    return true;
  });
  actions.setSidebarWidth(440);
  // The core's echo has not arrived when another UI action is dispatched.
  actions.toggleLeftSidebar();
  expect(sent[0]?.payload.sidebar_width).toBe(440);
  expect(sent[1]?.payload).not.toHaveProperty("sidebar_width");
  expect(sent[1]?.payload.left_sidebar_visible).toBe(false);
  useShellStore.setState({ rest: null });
});

it("moving one text size right after the other cannot send the first one's older value back", () => {
  const sent: { kind: string; payload: Record<string, unknown> }[] = [];
  useShellStore.setState({ rest: { ui_state: { font_size: 13, terminal_font_size: 14, terminal_font_family: "Menlo", left_sidebar_visible: true } } });
  const actions = createActions((event) => {
    sent.push(event as { kind: string; payload: Record<string, unknown> });
    return true;
  });
  actions.setTerminalFontSize(20);
  // The core's echo of 20 has not arrived when the other slider commits.
  actions.setFontSize(17);
  expect(sent[0]?.payload.terminal_font_size).toBe(20);
  expect(sent[1]?.payload).not.toHaveProperty("terminal_font_size");
  expect(sent[1]?.payload).not.toHaveProperty("terminal_font_family");
  expect(sent[1]?.payload.font_size).toBe(17);
  actions.toggleLeftSidebar();
  expect(sent[2]?.payload).not.toHaveProperty("font_size");
  useShellStore.setState({ rest: null });
});
