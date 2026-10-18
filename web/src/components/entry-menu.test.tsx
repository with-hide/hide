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
