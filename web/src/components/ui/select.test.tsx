// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Dialog, DialogContent, DialogTitle } from "./dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./select";

let root: Root | null = null;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  // Radix hands the keyboard back from a timer; a fake one lets the test run it at the moment it names.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  const mounted = root;
  root = null;
  if (mounted) act(() => mounted.unmount());
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

/**
 * A language select beside the control the operator moves to next, inside a
 * dialog as Settings lays them out: the dialog's focus trap takes the keyboard
 * when the picked option leaves, before the select's own hand-back runs.
 */
function mountSelect() {
  const closeAutoFocus = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const mounted = createRoot(container);
  root = mounted;
  act(() => mounted.render(
    <Dialog open>
      <DialogContent aria-describedby={undefined}>
        <DialogTitle>Settings</DialogTitle>
        <Select defaultValue="en">
          <SelectTrigger aria-label="language"><SelectValue /></SelectTrigger>
          <SelectContent onCloseAutoFocus={closeAutoFocus}>
            <SelectItem value="en">English</SelectItem>
            <SelectItem value="ko">Korean</SelectItem>
          </SelectContent>
        </Select>
        <input type="range" aria-label="size" />
      </DialogContent>
    </Dialog>,
  ));
  const trigger = document.querySelector<HTMLButtonElement>("[aria-label=language]")!;
  const next = document.querySelector<HTMLInputElement>("[aria-label=size]")!;
  return { trigger, next, closeAutoFocus };
}

/**
 * Opens the select from the keyboard and picks Korean; Radix queues its
 * hand-back to the trigger on a timer. The microtasks a browser runs before
 * that timer (the dialog's focus trap observes the removal) settle here.
 */
async function pickKorean(trigger: HTMLButtonElement) {
  trigger.focus();
  act(() => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  const option = [...document.querySelectorAll<HTMLElement>("[role=option]")].find((row) => row.textContent === "Korean")!;
  act(() => option.focus());
  act(() => { option.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(document.querySelector("[role=option]")).toBeNull();
  await act(async () => {});
}

/** Runs the hand-back Radix queued when the select closed. */
const runHandback = () => act(() => { vi.runOnlyPendingTimers(); });

it("hands the keyboard back to its trigger when nothing took it after the pick", async () => {
  const { trigger, closeAutoFocus } = mountSelect();
  await pickKorean(trigger);
  expect(closeAutoFocus).not.toHaveBeenCalled();
  expect(document.activeElement?.getAttribute("role")).toBe("dialog");
  runHandback();
  expect(closeAutoFocus).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(trigger);
});

it("leaves the keyboard on a control focused after the pick when its late hand-back runs", async () => {
  const { trigger, next, closeAutoFocus } = mountSelect();
  await pickKorean(trigger);
  act(() => next.focus());
  expect(closeAutoFocus).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(next);
  runHandback();
  expect(closeAutoFocus).toHaveBeenCalledTimes(1);
  expect(document.activeElement).toBe(next);
});
