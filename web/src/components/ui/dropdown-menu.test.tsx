// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "./dropdown-menu";

let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

it("marks the chosen row of a short choice with a green ✓ on a lit row, and a long list's with the quiet dot", async () => {
  await act(async () => {
    root.render(
      <DropdownMenu open modal={false}>
        <DropdownMenuTrigger>Open</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuRadioGroup value="a">
            <DropdownMenuRadioItem value="a" mark="check">First</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="b" mark="check">Second</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="c">Third</DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
  });
  const rows = [...document.body.querySelectorAll<HTMLElement>('[data-slot="dropdown-menu-radio-item"]')];
  const mark = (row: HTMLElement) => row.querySelector("svg")?.getAttribute("class") ?? null;
  expect(rows.map((row) => [row.textContent, row.getAttribute("data-state"), row.className.includes("data-[state=checked]:bg-accent")])).toEqual([["First", "checked", true], ["Second", "unchecked", true], ["Third", "unchecked", false]]);
  expect([mark(rows[0]!)?.includes("text-success"), mark(rows[1]!), mark(rows[2]!)]).toEqual([true, null, null]);
});
