// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { TooltipProvider } from "./components/ui/tooltip";
import { BUILDER, FOCUS_WORLD, type FocusModel } from "./practice/worlds/focus";
import type { ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { ViewAreas } from "./ViewAreas";
import type { DispatchFn } from "./ws";

vi.mock("./terminals", () => ({ mountTerminalPreview: () => false }));

type Event = Parameters<DispatchFn>[0];
const pair = (model: FocusModel) => FOCUS_WORLD.apply(model, { kind: "pair_view", payload: { pane_id: BUILDER } })!;

describe("the desk's ✕ in focus mode (UI_BEHAVIOR, Focus mode)", () => {
  let root: Root, container: HTMLDivElement;
  let saved: ReturnType<typeof useShellStore.getState>;
  let events: Event[];
  beforeEach(() => {
    saved = useShellStore.getState(); events = [];
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    for (const [name, px] of Object.entries({ "tab-preferred": 160, "tab-title-min": 48, "tab-selected-max": 240, "tab-icon-identity": 16, "control-sm": 28, "workspace-area-min": 200, "view-area-min-height": 100, "resize-handle": 4, "tab-strip": 32 })) document.documentElement.style.setProperty(`--size-${name}`, String(px));
    // The tree lays out from its measured body, which jsdom does not have.
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(800);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(600);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove(); useShellStore.setState(saved, true); useUiStore.setState({ laneExpanded: false }); vi.unstubAllGlobals(); vi.restoreAllMocks(); document.documentElement.removeAttribute("style");
  });

  async function draw(model: FocusModel, display: Partial<ViewDisplaySnapshot> = {}) {
    // No document is sent, so the page is not drawn: the tree is what is under test.
    const { rest } = FOCUS_WORLD.snapshot(model, Date.now());
    const tree = rest.workspace_view?.layout?.root;
    if (tree && "area" in tree) tree.area.displays = tree.area.displays.map((shown) => ({ ...shown, ...display }));
    useShellStore.setState({ connection: "live", rest, editor: null, documents: {} });
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><ViewAreas actions={actions} /></TooltipProvider>));
    return actions;
  }
  const close = () => container.querySelector<HTMLButtonElement>("[data-lane-documents-close]");

  it("is drawn beside the paired pane, with no ◧, and sends what ⌘E sends: the desk folds, no lane close", async () => {
    const actions = await draw(pair(FOCUS_WORLD.initial()));
    expect(close()).not.toBeNull();
    expect(container.querySelector("[data-desk-pair]")).toBeNull();
    await act(async () => close()!.click());
    const pressed = events.splice(0);
    actions.toggleLane();
    expect(pressed).toEqual(events);
    expect(pressed.map((event) => [event.kind, event.payload])).toEqual([["pair_view_desk", { shown: false }]]);
  });

  it("has Expand beside it in and outside the mode, which fills the body with the desk and returns it, sending nothing", async () => {
    const expand = () => container.querySelector<HTMLButtonElement>("[data-desk-expand]")!;
    for (const model of [FOCUS_WORLD.initial(), pair(FOCUS_WORLD.initial())]) {
      await draw(model);
      expect(expand().compareDocumentPosition(close()!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(expand().getAttribute("aria-pressed")).toBe("false");
      await act(async () => expand().click());
      expect([useUiStore.getState().laneExpanded, expand().getAttribute("aria-pressed")]).toEqual([true, "true"]);
      await act(async () => expand().click());
      expect(useUiStore.getState().laneExpanded).toBe(false);
    }
    expect(events).toEqual([]);
  });

  it("names a pasted image as one on its tab and path line, and a file under home by its ~/ path", async () => {
    const title = () => container.querySelector("[role=tab] [data-tab-title]")?.textContent;
    const line = () => container.querySelector("[data-editor-path]")?.textContent;
    await draw(FOCUS_WORLD.initial(), { pasted: true, path: "/Users/someone/.hide/state/TerminalClipboard/hide-a37308bf-2590-4cbb-9079-3d38116992f9.png", label: "hide-a37308bf-2590-4cbb-9079-3d38116992f9.png", home_relative: "~/.hide/state/TerminalClipboard/hide-a37308bf-2590-4cbb-9079-3d38116992f9.png" });
    expect([title(), line()]).toEqual(["Pasted image", "Pasted image"]);
    await draw(FOCUS_WORLD.initial(), { path: "/Users/someone/Downloads/report.pdf", label: "report.pdf", home_relative: "~/Downloads/report.pdf" });
    expect([title(), line()]).toEqual(["report.pdf", "~/Downloads/report.pdf"]);
    await draw(FOCUS_WORLD.initial(), { path: "/Volumes/disk/report.pdf", label: "report.pdf" });
    expect(line()).toBe("/Volumes/disk/report.pdf");
  });

  it("keeps its ◧ and closes the documents outside the mode", async () => {
    await draw(FOCUS_WORLD.initial());
    expect(container.querySelector("[data-desk-pair]")).not.toBeNull();
    await act(async () => close()!.click());
    expect(events.map((event) => [event.kind, event.payload])).toEqual([["workspace_view", { documents: false }]]);
  });
});
