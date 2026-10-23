// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { ShortcutsTab } from "./ShortcutsTab";

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

afterEach(() => {
  document.body.innerHTML = "";
});

const publish = (patch: Record<string, unknown>) =>
  act(async () => {
    const rest = useShellStore.getState().rest;
    useShellStore.setState({ rest: { ...rest, ...patch, status: { ...rest?.status, ...(patch.status as object) } } } as never);
  });

// The core clears `last_error` at its next event of any kind, so a refusal that only lived there would
// make the tab read "no error yet" as "still saving" (code-web 3, `useRefusalSince`).
it("keeps a refused save visible, and not as saving, after the core clears last_error", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const saved = useShellStore.getState();
  await act(async () => {
    useShellStore.setState({ connection: "live", rest: { status: {}, ui_state: { browser_shortcut_bindings: { toggle_sidebar: "Ctrl+B" } } } } as never);
    root.render(<TooltipProvider><ShortcutsTab actions={actions} /></TooltipProvider>);
  });
  const q = (selector: string) => container.querySelector(selector) as HTMLElement | null;
  const saving = () => container.textContent?.includes("Saving") ?? false;

  await act(async () => { q("[data-shortcut-reset-all]")?.click(); });
  expect(events.filter((event) => event.kind === "ui_state_update")).toHaveLength(1);
  expect(saving()).toBe(true);

  const refused = { kind: "ui_state.shortcuts_invalid", message: "Shortcuts were not saved: too many or too long", retryable: false, occurred_at: Date.now() + 1000 };
  await publish({ status: { last_error: refused } });
  expect(q("[data-shortcut-save-error]")?.textContent).toBe("The shortcuts were not saved: there are too many, or one is too long.");
  expect(saving()).toBe(false);

  await publish({ status: { last_error: null } });
  expect(q("[data-shortcut-save-error]")?.textContent).toBe("The shortcuts were not saved: there are too many, or one is too long.");
  expect(saving()).toBe(false);
  expect(q("[data-shortcut-reset-all]")?.hasAttribute("disabled")).toBe(false);

  await act(async () => root.unmount());
  useShellStore.setState(saved, true);
});
