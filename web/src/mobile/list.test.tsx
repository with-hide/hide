// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "./App";
import type { PhoneAgent } from "./protocol";
import { patch, usePhone } from "./store";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const initial = usePhone.getState();
afterEach(() => {
  usePhone.setState(initial, true);
  vi.unstubAllGlobals();
});

function agent(pane: string, over: Partial<PhoneAgent>): PhoneAgent {
  return { device_id: "local", pane_id: pane, root_pane_id: pane, group: "seen", mark: "quiet", wait: null, descendants_working: 0, agent_kind: "claude", title: `작업 ${pane}`, place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: false, holds_notification: false, ...over };
}

// The phone dims an agent whose state is unknown, and only that one, as the desktop lists do (status-marks B16).
it("dims an unknown row on the phone's list and leaves a sleeping one bright", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  patch({ screen: "app", groups: [{ group: "seen", count: 3, rows: [
    agent("unknown", { mark: "unknown", status_code: "unknown", line: { text: "", mode: "kept", tone: "subtle" } }),
    agent("asleep", { mark: "asleep", status_code: "sleeping" }),
    agent("idle", {}),
  ].map((row) => ({ kind: "agent" as const, ...row })) }] });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  try {
    const item = (pane: string) => container.querySelector(`[data-phone-agent="local|${pane}"]`)!.closest("[data-phone-row]")!;
    expect(item("unknown").className).toContain("opacity-(--opacity-dimmed)");
    expect(item("asleep").className).not.toContain("opacity-(--opacity-dimmed)");
    expect(item("idle").className).not.toContain("opacity-(--opacity-dimmed)");
  } finally {
    await act(async () => root.unmount());
  }
});
