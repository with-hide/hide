// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EntryContextMenu, type MenuEntry } from "./entry-menu";
import { useUiStore } from "../ui";

let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  vi.useFakeTimers();
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => { root.unmount(); await vi.runOnlyPendingTimersAsync(); });
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

async function render(items: MenuEntry[], selected: string[]) {
  await act(async () => root.render(
    <EntryContextMenu label="Session actions" title="Review API" items={() => items} onSelect={(id) => selected.push(id)}>
      <button>Review API</button>
    </EntryContextMenu>,
  ));
}
async function open(items: MenuEntry[], selected: string[]) {
  await render(items, selected);
  const target = document.querySelector("button")!;
  target.focus();
  await act(async () => {
    target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await vi.runOnlyPendingTimersAsync();
  });
  return target;
}
async function key(target: Element, key: string) {
  await act(async () => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    await vi.runOnlyPendingTimersAsync();
  });
}
const copies: MenuEntry[] = [
  { id: "copy_title", label: "Session title", icon: "copy", group: "copy", unavailable: null },
  { id: "copy_id", label: "Session ID", icon: "id", group: "copy", unavailable: "Conversation not reported yet" },
];

it("closes a newly unavailable submenu and keeps it closed when availability returns", async () => {
  const selected: string[] = [];
  await open(copies, selected);
  const group = document.querySelector<HTMLElement>("[data-menu-group]")!;
  group.focus();
  await key(group, "ArrowRight");
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(2);
  await render(copies.map((item) => ({ ...item, unavailable: "Device disconnected" })), selected);
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  expect(document.activeElement).toBe(group);
  await render(copies, selected);
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  expect(selected).toEqual([]);
});

it("opens a group by keyboard and executes only the chosen original action", async () => {
  const selected: string[] = [];
  await open(copies, selected);
  const group = document.querySelector<HTMLElement>("[data-menu-group]")!;
  group.focus();
  await key(group, "ArrowRight");
  expect(document.querySelector('[data-menu-item="copy_id"]')?.getAttribute("aria-disabled")).toBe("true");
  expect(selected).toEqual([]);
  expect(document.activeElement?.textContent).toBe("Session title");
  await key(document.activeElement!, "Enter");
  expect(selected).toEqual(["copy_title"]);
  expect(document.querySelector('[role="menu"]')).toBeNull();
});

it("Escape closes the submenu first and then returns the keyboard to the original row", async () => {
  const selected: string[] = [];
  const target = await open(copies, selected);
  const group = document.querySelector<HTMLElement>("[data-menu-group]")!;
  group.focus();
  await key(group, "ArrowRight");
  // The shell owns Escape in capture phase; a submenu must be its innermost layer.
  act(() => { useUiStore.getState().escapeLayers.at(-1)!(); });
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  expect(document.activeElement).toBe(group);
  act(() => { useUiStore.getState().escapeLayers.at(-1)!(); });
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.activeElement).toBe(target);
  expect(selected).toEqual([]);
});

it("shows one shared refusal on a disabled group and offers no executable child", async () => {
  const selected: string[] = [];
  await open(copies.map((item) => ({ ...item, unavailable: "Device disconnected" })), selected);
  const group = document.querySelector<HTMLElement>("[data-menu-group]")!;
  expect(group.getAttribute("aria-disabled")).toBe("true");
  expect(group.textContent).toBe("Copy detailsDevice disconnected");
  await key(group, "ArrowRight");
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  expect(document.querySelectorAll("[data-menu-reason]")).toHaveLength(1);
  expect(selected).toEqual([]);
});


it("nested destinations close one keyboard level at a time and stay closed after a refusal", async () => {
  const selected: string[] = [];
  const items: MenuEntry[] = [{ id: "move", label: "Move", unavailable: null, children: [{ id: "tab", label: "Target tab", unavailable: null, children: [{ id: "left", label: "Left", unavailable: null }] }] }];
  await open(items, selected);
  const move = document.querySelector<HTMLElement>('[data-menu-item="move"]')!;
  await act(async () => move.focus()); await key(move, "ArrowRight");
  const tab = document.querySelector<HTMLElement>('[data-menu-item="tab"]')!;
  await key(tab, "ArrowRight");
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(3);
  act(() => { useUiStore.getState().escapeLayers.at(-1)!(); });
  await act(async () => { await vi.runOnlyPendingTimersAsync(); });
  expect(document.activeElement).toBe(tab);
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(2);
  await render([{ ...items[0]!, unavailable: "Move pending" }], selected);
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  await render(items, selected);
  expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
  expect(selected).toEqual([]);
});
