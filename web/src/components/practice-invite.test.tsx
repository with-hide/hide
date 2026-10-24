// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createActions } from "../actions";
import { useShellStore } from "../store";
import type { DispatchFn } from "../ws";
import { PracticeInvite } from "./practice-invite";
import { TooltipProvider } from "./ui/tooltip";

// The sidebar's invitation to practice (docs/UI_BEHAVIOR.md, Practice window):
// shown to an operator who never practiced and never closed it, projects or
// not; gone once any chapter's progress exists or after its ×, which the core keeps.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });
const saved = useShellStore.getState();
afterEach(() => {
  useShellStore.setState(saved, true);
  delete window.hideHost;
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function render(ui: Record<string, unknown> | undefined) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const events: Parameters<DispatchFn>[0][] = [];
  const actions = createActions((event) => { events.push(event); return true; });
  useShellStore.setState({ rest: ui === undefined ? null : ({ ui_state: ui } as never) });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<TooltipProvider><PracticeInvite actions={actions} /></TooltipProvider>));
  return { container, events, unmount: () => act(async () => root.unmount()) };
}

const shown = (container: HTMLElement) => container.querySelector("[data-practice-invite]") !== null;

it("invites only an operator with no practice progress who never closed it, projects or not", async () => {
  for (const [ui, expected] of [
    [undefined, false],
    [{}, true],
    [{ workspace_registrations: [{ id: "p", label: "p", path: "~/p", device_id: "local" }] }, true],
    [{ practice: { overview: { version: 1, step: 0, finished: false } } }, false],
    [{ practice_invite_closed: true }, false],
  ] as const) {
    const { container, unmount } = await render(ui as Record<string, unknown> | undefined);
    expect(shown(container), JSON.stringify(ui)).toBe(expected);
    await unmount();
  }
});

it("opens the practice window on chapter 0, and its × tells the core to close it for good", async () => {
  const openPractice = vi.fn();
  window.hideHost = { kind: "electron", platform: "darwin", openPractice } as never;
  const { container, events, unmount } = await render({});
  await act(async () => container.querySelector<HTMLButtonElement>("[data-practice-invite-open]")!.click());
  expect(openPractice).toHaveBeenCalledWith(expect.any(String), "overview");
  expect(events).toEqual([]);
  await act(async () => container.querySelector<HTMLButtonElement>("[data-practice-invite-close]")!.click());
  expect(events).toEqual([{ schema_version: 2, kind: "practice_invite_close", payload: {} }]);
  // The core's answer is what hides it.
  await act(async () => useShellStore.setState({ rest: { ui_state: { practice_invite_closed: true } } as never }));
  expect(shown(container)).toBe(false);
  await unmount();
});
