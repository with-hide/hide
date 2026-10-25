// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { PaneMovePlus, PaneMovePreview, PaneMoveProvider, usePaneMove } from "./PaneMoveControls";
import { paneMoveStartTarget } from "./paneMove";
import type { AgentLayout } from "./agentLayout";
import { emptyScope } from "../test/legacyAgentScope";
import type { AsyncOperation, Checkout, PaneRow, Tab } from "./snapshot";
import { useShellStore } from "./store";
import type { DispatchFn } from "./ws";

vi.mock("./terminals", () => ({ mountTerminalPreview: () => false }));
const pane = (id: string): PaneRow => ({ id, herdr_label: id, terminal_title: null, cwd: "/fixture", status_code: "idle", requires_close_confirmation: false, requires_close_status_check: false, identity_label: null });
const tab = (id: string, panes: string[]): Tab => ({ id, workspace_id: "w1", checkout_id: "checkout", label: id, empty: false, delegated: false, panes: panes.map(pane), sidebar: { number: 1, manual_name: null, named_by_operator: false, focused_pane_id: panes[0] } });
const tabs = [tab("t1", ["p1", "p2"]), tab("t2", ["p3"]), tab("t3", ["p4"])];
const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, active_tab_id: "t1", strip: [], next_tab_label: "4", tabs, agent_scope: emptyScope() };
const layout: AgentLayout = { root: { area: { id: "a1", active: "t1", displays: tabs.map(tab => ({ id: tab.id! })) } }, active_area: "a1", canvases: {}, group_id: "group", limits: { areas: 6, depth: 3, displays: 64 }, display_count: 3 };
function Controls() {
  const move = usePaneMove()!;
  return <><div data-agent-tab-bar="a1"><span data-area-item="t2">Target</span><button data-agent-new-tab="a1"><PaneMovePlus /></button></div><div data-pane-view="p1" onPointerDown={event => move.press("p1", "Source", event)}>Source</div><div data-pane-view="p2">Sibling</div><button data-menu-move="" onClick={() => { const root = move.menu("p1")[0]!; const area = root.children![0]!; move.select(area.children![0]!.id); }}>Move</button><span data-busy="">{String(move.mutationBlocked("p1"))}:{String(move.mutationBlocked("p4"))}</span><PaneMovePreview areaId="a1" /></>;
}

describe("pane move gesture", () => {
  let root: Root, container: HTMLDivElement;
  let saved: ReturnType<typeof useShellStore.getState>;
  let events: Parameters<DispatchFn>[0][];
  let hit: Element | null;
  beforeEach(async () => {
    saved = useShellStore.getState(); events = []; hit = null;
    vi.useFakeTimers(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    document.documentElement.style.setProperty("--size-tab-drag-activation", "5");
    Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => hit });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(0, 0, 400, 200));
    useShellStore.setState({ connection: "live", rest: {} });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    const actions = createActions(event => { events.push(event); return true; });
    await act(async () => root.render(<PaneMoveProvider checkout={checkout} layout={layout} deviceId="" actions={actions}><Controls /></PaneMoveProvider>));
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove(); useShellStore.setState(saved, true);
    window.dispatchEvent(new Event("pointerdown"));
    Reflect.deleteProperty(document, "elementFromPoint");
    vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers();
  });
  const element = (selector: string) => container.querySelector<HTMLElement>(selector)!;
  const pointer = async (type: string, x: number, y: number, target: EventTarget = window) => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });
    Object.defineProperties(event, { pointerId: { value: 1 }, isPrimary: { value: true } });
    await act(async () => { target.dispatchEvent(event); });
  };
  const start = () => pointer("pointerdown", 200, 100, element('[data-pane-view="p1"]'));
  const moves = () => events.filter(event => event.kind === "pane_move");
  it("preserves a click below threshold and sends exactly one typed edge move after dragging", async () => {
    hit = element('[data-pane-view="p2"]');
    await start(); await pointer("pointermove", 202, 100); await pointer("pointerup", 202, 100);
    expect(moves()).toHaveLength(0);
    await start(); await pointer("pointermove", 390, 100); await pointer("pointerup", 390, 100);
    expect(moves()).toHaveLength(1);
    expect(moves()[0]!.payload).toMatchObject({ workspace: { device_id: "", checkout_path: "/fixture" }, group_id: "group", source: { pane_id: "p1", tab_id: "t1", area_id: "a1" }, destination: { type: "pane", pane_id: "p2", tab_id: "t1", area_id: "a1", edge: "right" } });
    expect(element('[data-busy]').textContent).toBe("true:false");
  });
  it.each(["Escape", "pointercancel", "blur"])("cancels on %s, releases its cursor and suppresses the drag click", async event => {
    hit = element('[data-pane-view="p2"]'); await start(); await pointer("pointermove", 390, 100);
    await act(async () => { window.dispatchEvent(event === "Escape" ? new KeyboardEvent("keydown", { key: "Escape" }) : new Event(event)); });
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    element('[data-pane-view="p1"]').dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(document.documentElement.hasAttribute("data-agent-drag")).toBe(false);
    await pointer("pointerup", 390, 100); expect(moves()).toHaveLength(0);
  });
  it("reads a hover preview without focus, selection or terminal resize events, then cancels it", async () => {
    hit = element('[data-area-item="t2"]'); await start(); await pointer("pointermove", 390, 20);
    await act(async () => { vi.advanceTimersByTime(450); });
    expect(events.map(event => event.kind)).toEqual(["pane_move_preview"]);
    expect(container.querySelector('[data-pane-move-preview-tab="t2"]')).not.toBeNull();
    const request = events[0]!.payload as { request_id: string };
    await act(async () => useShellStore.setState({ rest: { status: { pane_move_preview: { request_id: request.request_id, tab_id: "t2", phase: "ready", panes: [{ pane_id: "p3", text: "existing terminal content" }] } } } }));
    expect(container.textContent).toContain("existing terminal content");
    await act(async () => { window.dispatchEvent(new Event("blur")); });
    expect(events.map(event => event.kind)).toEqual(["pane_move_preview", "pane_move_preview_cancel"]);
  });
  it("rejects an edge when the target topology changed between hover and release", async () => {
    hit = element('[data-pane-view="p2"]'); await start(); await pointer("pointermove", 390, 100);
    await act(async () => useShellStore.setState({ rest: { pane_layouts: [{ workspace_id: "w1", tab_id: "t1", focused_pane_id: "p1", zoomed: true, root: { type: "pane", pane_id: "p1" } }] } }));
    await pointer("pointerup", 390, 100); expect(moves()).toHaveLength(0);
  });
  it("keeps an uncertain result reserved and checks status without dispatching another move", async () => {
    await act(async () => element('[data-menu-move]').click());
    const request = moves()[0]!.payload as { request_id: string };
    await act(async () => { vi.advanceTimersByTime(15_000); });
    expect(container.textContent).toContain("Check position before moving again");
    await act(async () => element('[data-menu-move]').click());
    expect(moves()).toHaveLength(1);
    const check = [...container.querySelectorAll("button")].find(button => button.textContent === "Check status")!;
    await act(async () => check.click());
    expect(events.filter(event => event.kind === "pane_move_check")).toEqual([{ schema_version: 2, kind: "pane_move_check", payload: { request_id: request.request_id } }]);
  });
  it("restores a published unknown reservation and check action after mounting without local request state", async () => {
    const unknown: AsyncOperation = { id: "retained-move", kind: "pane.move", target_id: "p1", scope_id: "t1", related_scope_ids: ["t1", "t2"], phase: "unknown", stage: "move", message: null, retryable: false };
    await act(async () => useShellStore.setState({ rest: { status: { async_operations: [unknown] } } }));
    expect(element('[data-busy]').textContent).toBe("true:false");
    const check = [...container.querySelectorAll("button")].find(button => button.textContent === "Check status")!;
    await act(async () => check.click());
    expect(events.filter(event => event.kind === "pane_move_check")).toEqual([{ schema_version: 2, kind: "pane_move_check", payload: { request_id: unknown.id } }]);
    expect(moves()).toHaveLength(0);
  });
  it("cancels on connection loss and suppresses a later release click", async () => {
    hit = element('[data-pane-view="p2"]'); await start(); await pointer("pointermove", 390, 100);
    await act(async () => useShellStore.setState({ connection: "connecting" }));
    await act(async () => { vi.advanceTimersByTime(1_000); });
    await pointer("pointerup", 390, 100);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    element('[data-pane-view="p1"]').dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(moves()).toHaveLength(0);
    expect(document.documentElement.hasAttribute("data-agent-drag")).toBe(false);
  });
  it("uses the keyboard menu's same new-tab intent and permits a fresh request after definite refusal", async () => {
    await act(async () => element('[data-menu-move]').click());
    const first = moves()[0]!.payload as { request_id: string };
    const refused: AsyncOperation = { id: first.request_id, kind: "pane.move", target_id: "p1", scope_id: "t1", phase: "failed", stage: "move", message: null, retryable: true };
    await act(async () => useShellStore.setState({ rest: { status: { async_operations: [refused] } } }));
    await act(async () => element('[data-menu-move]').click());
    expect(moves()).toHaveLength(2);
    expect(moves()[1]!.payload).toMatchObject({ destination: { type: "new_tab", area_id: "a1" } });
    expect((moves()[1]!.payload as { request_id: string }).request_id).not.toBe(first.request_id);
  });
});

// The record title is a draggable part of an existing button; its logo and status are not.
it("starts from title text or blank space while preserving ordinary record clicks and excluding controls and the second row", () => {
  const host = document.createElement("section");
  host.innerHTML = '<header><button data-pane-record="session"><span data-logo data-pane-move-excluded>logo</span><span data-status data-pane-status-mark>status</span><span data-pane-move-title><span data-title-text>title</span><span data-title-gap></span></span></button><span data-pane-move-blank></span><button data-menu>menu</button><button data-close>close</button></header><div data-pane-header-band>second row</div>';
  const header = host.querySelector("header")!;
  for (const selector of ["[data-title-text]", "[data-title-gap]", "[data-pane-record]", "[data-pane-move-blank]"]) expect(paneMoveStartTarget(header, host.querySelector(selector))).toBe(true);
  for (const selector of ["[data-logo]", "[data-status]", "[data-menu]", "[data-close]", "[data-pane-header-band]"]) expect(paneMoveStartTarget(header, host.querySelector(selector))).toBe(false);
  const record = vi.fn(); host.querySelector("button")!.addEventListener("click", record); host.querySelector<HTMLElement>("[data-title-text]")!.click();
  expect(record).toHaveBeenCalledOnce();
});
