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

/** A finger on the target: iOS sends pointer events and a click, never `contextmenu`. */
function touch(type: "pointerdown" | "pointermove" | "pointerup", target: Element, x = 20, y = 20) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
  Object.defineProperty(event, "pointerType", { value: "touch" });
  target.dispatchEvent(event);
}

async function renderPressable(onOpen: () => void) {
  await act(async () => {
    root.render(
      <ContextMenu>
        <ContextMenuTrigger><button onClick={onOpen}>Mac mini</button></ContextMenuTrigger>
        <ContextMenuContent><ContextMenuItem>Remove device</ContextMenuItem></ContextMenuContent>
      </ContextMenu>,
    );
  });
  return document.querySelector("button")!;
}

it("opens the menu on a held finger, and the click that ends the press does not press the target", async () => {
  const onOpen = vi.fn();
  const row = await renderPressable(onOpen);
  await act(async () => {
    touch("pointerdown", row);
    await vi.advanceTimersByTimeAsync(500);
  });
  expect(document.querySelector('[role="menu"]')?.textContent).toContain("Remove device");
  await act(async () => { touch("pointerup", row); row.click(); });
  expect(onOpen).not.toHaveBeenCalled();
});

it("leaves a tap and a scrolling finger to the target, with no menu", async () => {
  const onOpen = vi.fn();
  const row = await renderPressable(onOpen);
  await act(async () => {
    touch("pointerdown", row);
    await vi.advanceTimersByTimeAsync(200);
    touch("pointerup", row);
    row.click();
    await vi.advanceTimersByTimeAsync(600);
  });
  expect(onOpen).toHaveBeenCalledTimes(1);
  await act(async () => {
    touch("pointerdown", row);
    touch("pointermove", row, 20, 60);
    await vi.advanceTimersByTimeAsync(600);
  });
  expect(document.querySelector('[role="menu"]')).toBeNull();
});

/** A mouse's pointer event on `target`. */
function pointer(type: "pointerdown" | "pointerup", target: Element) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: 20, clientY: 20 });
  Object.defineProperty(event, "pointerType", { value: "mouse" });
  target.dispatchEvent(event);
}

async function renderChoosable(onSelect: () => void) {
  await act(async () => {
    root.render(
      <ContextMenu>
        <ContextMenuTrigger><button>page.html</button></ContextMenuTrigger>
        <ContextMenuContent><ContextMenuItem onSelect={onSelect}>Open to the side</ContextMenuItem></ContextMenuContent>
      </ContextMenu>,
    );
  });
  return document.querySelector("button")!;
}

const item = () => document.querySelector<HTMLElement>('[role="menuitem"]')!;

it("leaves the menu open and chooses nothing when the right button that opened it is released over an item", async () => {
  // macOS sends `contextmenu` on the press, so the menu opens under the pointer and the release lands on its first item.
  const onSelect = vi.fn();
  const row = await renderChoosable(onSelect);
  await act(async () => {
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }));
    await vi.runOnlyPendingTimersAsync();
  });
  await act(async () => { pointer("pointerup", item()); await vi.runOnlyPendingTimersAsync(); });
  expect(onSelect).not.toHaveBeenCalled();
  expect(document.querySelector('[role="menu"]')).not.toBeNull();
  // The next click on the item chooses it.
  await act(async () => { pointer("pointerdown", item()); pointer("pointerup", item()); item().click(); await vi.runOnlyPendingTimersAsync(); });
  expect(onSelect).toHaveBeenCalledTimes(1);
});
