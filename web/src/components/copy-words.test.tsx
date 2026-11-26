// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CopyableMessage } from "./copy-words";

// Where a message's `복사` stands (copy-pos 가, operator 2026-11-26): an
// answer's always, under its end at the left; the operator's own under the
// bubble at the right, shown on hover, keyboard focus or after it copied, in
// a row held the whole time so the message never changes height. jsdom draws
// no hover and lays nothing out, so the test reads what decides both: the
// classes on the row that holds `복사`.

let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

async function render(own: boolean): Promise<{ message: HTMLElement; row: HTMLElement; button: HTMLElement }> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  mounted.push(root);
  await act(async () => root.render(<CopyableMessage text="**원래** 말" copy={async () => true} own={own}><p data-words="true">원래 말</p></CopyableMessage>));
  const message = host.firstElementChild as HTMLElement;
  const row = message.querySelector<HTMLElement>("[data-message-copy]")!;
  return { message, row, button: row.querySelector<HTMLElement>('[data-copy-words="message"]')! };
}

/** The row's classes that apply before any hover, focus or copy: the ones with no variant. */
const base = (row: HTMLElement) => Array.from(row.classList).filter((name) => !name.includes(":"));
/** What a hover, focus, copy or a surface with no pointer changes on the row. */
const revealed = (row: HTMLElement) => Array.from(row.classList).filter((name) => name.includes(":")).map((name) => name.slice(name.lastIndexOf(":") + 1));
/** A class that takes the row out of the message's flow or its space away, so showing `복사` would move the message or cover it. */
const OUT_OF_FLOW = /^(absolute|fixed|hidden|invisible|-?translate-|top-|right-|bottom-|left-|h-0|max-h-0)/;

it("shows an answer's 복사 always, after its words at the left, as borderless words", async () => {
  const { message, row, button } = await render(false);
  // After the words, never over them.
  expect(message.lastElementChild).toBe(row);
  expect(row.previousElementSibling?.getAttribute("data-words")).toBe("true");
  expect(base(row)).toContain("self-start");
  // Nothing hides it until a hover.
  expect(base(row).filter((name) => /^(opacity-0|invisible|hidden|pointer-events-none)$/.test(name))).toEqual([]);
  expect(revealed(row)).toEqual([]);
  // The code block head's quiet words, not the boxed outline button.
  expect(button.textContent).toContain("Copy");
  expect(button.className).not.toMatch(/\bborder\b|bg-background/);
});

it("holds the operator's 복사 under the bubble at the right, hidden until hover, focus or a copy, without changing the message's height", async () => {
  const { message, row, button } = await render(true);
  expect(message.lastElementChild).toBe(row);
  expect(base(row)).toContain("self-end");
  // Hidden before the pointer comes, and not pressable while hidden.
  expect(base(row)).toEqual(expect.arrayContaining(["opacity-0", "pointer-events-none"]));
  // Shown under the pointer, with the keyboard in the message, while it says it copied, and on a surface with no pointer.
  const variants = Array.from(row.classList).filter((name) => name.endsWith(":opacity-100")).map((name) => name.split(":")[0]);
  expect(variants).toEqual(expect.arrayContaining(["group-hover/message", "group-focus-within/message", "has-data-copied", "hoverless"]));
  // The row is in the message's flow from the start and showing it changes only its opacity and whether it takes a press,
  // so the message is as tall before the hover as after it.
  expect(base(row).filter((name) => OUT_OF_FLOW.test(name))).toEqual([]);
  expect(new Set(revealed(row))).toEqual(new Set(["opacity-100", "pointer-events-auto"]));
  expect(button.isConnected).toBe(true);
});

