// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./select";

afterEach(() => { document.body.replaceChildren(); });

/** A language select beside the control the operator moves to next, as Settings lays them out. */
function mountSelect() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(
    <>
      <Select defaultValue="en">
        <SelectTrigger aria-label="language"><SelectValue /></SelectTrigger>
        <SelectContent>
          <SelectItem value="en">English</SelectItem>
          <SelectItem value="ko">Korean</SelectItem>
        </SelectContent>
      </Select>
      <input type="range" aria-label="size" />
    </>,
  ));
  const trigger = container.querySelector<HTMLButtonElement>("[aria-label=language]")!;
  const next = container.querySelector<HTMLInputElement>("[aria-label=size]")!;
  return { trigger, next, unmount: () => act(() => root.unmount()) };
}

/** Opens the select from the keyboard and picks Korean; Radix queues its hand-back to the trigger on a timer. */
function pickKorean(trigger: HTMLButtonElement) {
  trigger.focus();
  act(() => { trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  const option = [...document.querySelectorAll<HTMLElement>("[role=option]")].find((row) => row.textContent === "Korean")!;
  act(() => option.focus());
  act(() => { option.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(document.querySelector("[role=option]")).toBeNull();
}

const timers = () => act(() => new Promise<void>((resolve) => setTimeout(resolve, 10)));

it("hands the keyboard back to its trigger when nothing took it after the pick", async () => {
  const { trigger, unmount } = mountSelect();
  pickKorean(trigger);
  await timers();
  expect(document.activeElement).toBe(trigger);
  unmount();
});

it("leaves the keyboard on a control focused after the pick when its late hand-back runs", async () => {
  const { trigger, next, unmount } = mountSelect();
  pickKorean(trigger);
  act(() => next.focus());
  await timers();
  expect(document.activeElement).toBe(next);
  unmount();
});
