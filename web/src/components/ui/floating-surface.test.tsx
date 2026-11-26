// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Dialog, DialogContent, DialogTitle } from "./dialog";
import { Popover, PopoverAnchor, PopoverContent } from "./popover";

let root: Root | null = null;

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));

afterEach(() => {
  const mounted = root;
  root = null;
  if (mounted) act(() => mounted.unmount());
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

function mount(node: React.ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const mounted = createRoot(container);
  root = mounted;
  act(() => mounted.render(node));
}

const menu = (
  <Popover open>
    <PopoverAnchor />
    <PopoverContent data-testid="menu">Pick</PopoverContent>
  </Popover>
);

it("draws a floating part opened inside a modal one step above the modal, and one on the page on the popover step", () => {
  mount(menu);
  expect(document.querySelector("[data-testid=menu]")?.className).not.toContain("--popover-nested");
  act(() => root?.unmount());
  root = null;
  document.body.replaceChildren();

  mount(
    <Dialog open>
      <DialogContent aria-describedby={undefined}>
        <DialogTitle>Settings</DialogTitle>
        {menu}
      </DialogContent>
    </Dialog>,
  );
  // The menu renders through its own portal, outside the modal's element, and still takes the nested step.
  const content = document.querySelector("[data-testid=menu]");
  expect(content?.closest("[data-slot=dialog-content]")).toBeNull();
  expect(content?.className).toContain("[--popover:var(--popover-nested)]");
});
