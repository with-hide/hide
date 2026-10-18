// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useUiStore } from "../../ui";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "./context-menu";

let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
    await vi.runOnlyPendingTimersAsync();
  });
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

async function openMenu(returnToInput = false) {
  await act(async () => {
    root.render(
      <ContextMenu returnToInput={returnToInput}>
        <ContextMenuTrigger asChild>
          <div><button>Project</button><button>More actions</button></div>
        </ContextMenuTrigger>
        <ContextMenuContent><ContextMenuItem>Rename</ContextMenuItem></ContextMenuContent>
      </ContextMenu>,
    );
  });
  const buttons = document.querySelectorAll("button");
  expect(buttons).toHaveLength(2);
  const project = buttons.item(0);
  const more = buttons.item(1);
  if (!returnToInput) project.focus();
  await act(async () => {
    project.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await vi.runOnlyPendingTimersAsync();
  });
  expect(document.querySelector('[role="menu"]')).not.toBeNull();
  return { project, more };
}

function closeMenu() {
  act(() => { useUiStore.getState().escapeLayers.at(-1)!(); });
  expect(document.querySelector('[role="menu"]')).toBeNull();
}

it("returns focus to the context-menu target after Escape", async () => {
  const { project } = await openMenu();
  closeMenu();
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.activeElement).toBe(project);
});

it("keeps focus on the next control when the context-menu hand-back arrives late", async () => {
  const { more } = await openMenu();
  closeMenu();
  // Hold Radix's unmount timer until the operator has chosen the next keyboard stop.
  more.focus();
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.activeElement).toBe(more);
});

it("returns a View menu to the input it borrowed from another area", async () => {
  const input = document.createElement("input");
  document.body.append(input);
  input.focus();
  await openMenu(true);
  closeMenu();
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.activeElement).toBe(input);
});
