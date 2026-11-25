// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useShellStore } from "../store";
import { AppearanceGroup } from "./AppearanceGroup";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
});

it("sends the tool rail counts switch to the core and waits for a live connection", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  const sent: unknown[] = [];
  const actions = createActions((event) => { sent.push(event); return true; });
  const toggle = () => container.querySelector<HTMLElement>("[data-rail-counts]")!;
  try {
    await act(async () => {
      useShellStore.setState({ connection: "closed", rest: { ui_state: {} } } as never);
      root.render(<TooltipProvider><AppearanceGroup actions={actions} /></TooltipProvider>);
    });
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(toggle().hasAttribute("disabled")).toBe(true);
    await act(async () => useShellStore.setState({ connection: "live" } as never));
    await act(async () => toggle().click());
    expect(sent).toEqual([{ schema_version: 2, kind: "rail_counts_set", payload: { shown: false } }]);
    await act(async () => useShellStore.setState({ rest: { ui_state: { rail_counts_shown: false } } } as never));
    expect(toggle().getAttribute("aria-checked")).toBe("false");
  } finally {
    await act(async () => root.unmount());
  }
});
