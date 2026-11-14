// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { App } from "./App";
import type { PhoneAgent } from "./protocol";
import { applyFrame, patch, usePhone } from "./store";

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

// PRD hide-external-control B46: the phone has no tool's cell, so a tool's person's turn or View reaches the
// list named, its second line led by the tool's name as the desktop's sidebar row is; the operator's own row is not.
it("leads a tool's row's second line with the tool's name, and the operator's row with none", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  patch({ screen: "app", groups: [{ group: "needs_you", count: 3, rows: [
    agent("asking", { group: "needs_you", mark: "turn", tool: "nightly", place: "fixture · main" }),
    agent("look", { group: "needs_you", mark: "look", tool: "nightly" }),
    agent("own", { group: "needs_you", mark: "turn", place: "fixture · main" }),
  ].map((row) => ({ kind: "agent" as const, ...row })) }] });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  try {
    const row = (pane: string) => container.querySelector<HTMLElement>(`[data-phone-agent="local|${pane}"]`)!;
    const second = (pane: string) => row(pane).querySelector("[data-phone-tool]")?.parentElement?.textContent ?? null;
    expect(second("asking")).toBe("nightly·fixture · main");
    expect(second("look")).toBe("nightly");
    expect(row("own").querySelector("[data-phone-tool]")).toBeNull();
    expect(row("asking").getAttribute("aria-label")).toContain("nightly, fixture · main");
  } finally {
    await act(async () => root.unmount());
  }
});

// A list hided cut to fit one relay message says how many agents it left out, in the PR list's line.
it("says under the list how many agents the Mac left out of the frame", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const rows = [{ kind: "agent" as const, ...agent("idle", {}) }];
  patch({ screen: "app", interfaceLanguage: "en" });
  applyFrame({ type: "agents", groups: [{ group: "seen", count: 40, rows }], omitted: 39, devices: [], interface_language: "en", color_theme: null });
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<App />));
  try {
    expect(container.querySelector("[data-phone-agents-omitted]")!.textContent).toBe("39 more agents are not shown here · see them on the desktop");
    await act(async () => applyFrame({ type: "agents", groups: [{ group: "seen", count: 1, rows }], devices: [], interface_language: "en", color_theme: null }));
    expect(container.querySelector("[data-phone-agents-omitted]")).toBeNull();
  } finally {
    await act(async () => root.unmount());
  }
});
