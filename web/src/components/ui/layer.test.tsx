// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { useOutsidePressSurface, useReturnFocus } from "./layer";
import { Popover, PopoverAnchor, PopoverContent } from "./popover";

afterEach(() => { document.body.replaceChildren(); });

type Handback = (event: Event) => void;

/** A layer's content part: mounted while open, it reports its hand-back the way Radix calls it after the close. */
function mountLayer(onHandback: (handback: Handback) => void) {
  function Content() {
    onHandback(useReturnFocus(true));
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Content />));
  return () => {
    act(() => root.unmount());
    container.remove();
  };
}

function holder(label: string) {
  const button = document.createElement("button");
  button.textContent = label;
  document.body.append(button);
  button.focus();
  return button;
}

function handBack(handback: Handback) {
  const event = new Event("focusScopeAutoFocusOnUnmount", { cancelable: true });
  handback(event);
  return event;
}

it("hands the keyboard back to the element that held it when its layer closed", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const before = holder("before");
  let handback: Handback = () => {};
  const close = mountLayer((next) => { handback = next; });
  (document.activeElement as HTMLElement).blur();
  close();
  const event = handBack(handback);
  expect(event.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(before);
  before.remove();
});

it("does not take the keyboard back once a newer layer opened and closed after its own closed", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const settingsButton = holder("settings");
  let older: Handback = () => {};
  const closeOlder = mountLayer((next) => { older = next; });
  closeOlder();
  // The older layer's timer is still queued behind input while the next layer takes the keyboard and gives it back.
  holder("terminal");
  const closeNewer = mountLayer(() => {});
  (document.activeElement as HTMLElement).blur();
  closeNewer();
  // Its own hand-back has not run yet, so nothing holds the keyboard when the older one does.
  expect(document.activeElement).toBe(document.body);
  const event = handBack(older);
  expect(event.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(document.body);
  settingsButton.remove();
});

it("counts a layer that replaces another in one commit as opened after the other closed", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  holder("settings");
  let older: Handback = () => {};
  function Older() {
    older = useReturnFocus(true);
    return null;
  }
  function Newer() {
    useReturnFocus(true);
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Older />));
  act(() => root.render(<Newer />));
  (document.activeElement as HTMLElement).blur();
  act(() => root.unmount());
  handBack(older);
  expect(document.activeElement).toBe(document.body);
});

/** A layer whose owner names where the keyboard goes, as Overview does with its own return target. */
function mountNamedLayer(target: () => HTMLElement | null, onHandback: (handback: Handback) => void) {
  const surface = document.createElement("div");
  document.body.append(surface);
  function Content() {
    onHandback(useReturnFocus(true, { target, within: () => surface }));
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Content />));
  return {
    surface,
    close: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

it("hands the keyboard to the target the owner names, not to what held it on open", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const opener = holder("opener");
  const named = document.createElement("button");
  document.body.append(named);
  let handback: Handback = () => {};
  const layer = mountNamedLayer(() => named, (next) => { handback = next; });
  (document.activeElement as HTMLElement).blur();
  layer.close();
  const event = handBack(handback);
  expect(event.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(named);
  opener.remove();
  named.remove();
});

it("does not take the keyboard to a named target once a newer layer opened and closed after its own closed", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const named = document.createElement("button");
  document.body.append(named);
  let older: Handback = () => {};
  const first = mountNamedLayer(() => named, (next) => { older = next; });
  first.close();
  holder("terminal");
  const closeNewer = mountLayer(() => {});
  (document.activeElement as HTMLElement).blur();
  closeNewer();
  const event = handBack(older);
  expect(event.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(document.body);
  named.remove();
});

it("keeps the keyboard where the operator put it outside the named layer, and takes it back from the layer's own surface", () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const named = document.createElement("button");
  document.body.append(named);
  let handback: Handback = () => {};
  const layer = mountNamedLayer(() => named, (next) => { handback = next; });
  const inside = document.createElement("button");
  layer.surface.append(inside);
  inside.focus();
  layer.close();
  handBack(handback);
  expect(document.activeElement).toBe(named);

  const elsewhere = holder("elsewhere");
  handBack(handback);
  expect(document.activeElement).toBe(elsewhere);
  named.remove();
  elsewhere.remove();
});

// A popover waits for the click to close and reads a press whose mousedown
// something stopped as taken elsewhere; the terminal stops its mousedown to
// keep xterm from selecting, so only a surface marked outside closes it.
it("closes an open popover on a press of a surface that stops its mousedown, once marked outside", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  const changes: boolean[] = [];
  function Surface({ marked }: { marked: boolean }) {
    const outside = useOutsidePressSurface();
    // The terminal's own gesture handler, as `terminals.ts` adds it on the way down.
    const stop = (node: HTMLDivElement | null) => node?.addEventListener("mousedown", (event) => event.stopPropagation(), { capture: true });
    return <div ref={(node) => { stop(node); if (marked) outside(node); }} data-surface={marked ? "marked" : "plain"} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (open: boolean) => root.render(<>
    <Surface marked={false} />
    <Surface marked />
    <Popover open={open} onOpenChange={(next) => changes.push(next)}><PopoverAnchor><span /></PopoverAnchor><PopoverContent>card</PopoverContent></Popover>
  </>);
  const press = async (target: Element) => act(async () => {
    for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]) target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0 }));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  try {
    await act(async () => { render(true); await new Promise((resolve) => setTimeout(resolve, 0)); });
    await press(document.querySelector("[data-surface=plain]")!);
    expect(changes, "a plain stopping surface reads as taken elsewhere").toEqual([]);
    await press(document.querySelector("[data-surface=marked]")!);
    expect(changes).toEqual([false]);
  } finally {
    act(() => root.unmount());
    vi.unstubAllGlobals();
  }
});
