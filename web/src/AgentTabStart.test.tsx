// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "./components/ui/tooltip";
import { createActions } from "./actions";
import { AgentTabStart, InlineStartHost } from "./AgentTabStart";
import type { AgentStartReceipt, AgentStartTarget } from "./agentLayout";
import { useInlineStarts, inlineStartKey } from "./startDraft";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import type { Checkout } from "./snapshot";
import type { DispatchFn } from "./ws";

const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());
const target: AgentStartTarget = { group_id: "g1", area_id: "a1", tab_id: "draft-1", add_pane: false };
const key = inlineStartKey("local", "/fixture", target);

describe("inline tab start (group-tabs B7–B9, B22, B27)", () => {
  let root: Root;
  let container: HTMLDivElement;
  let shell: ReturnType<typeof useShellStore.getState>;
  let ui: ReturnType<typeof useUiStore.getState>;
  let events: Parameters<DispatchFn>[0][];
  beforeEach(() => {
    shell = useShellStore.getState(); ui = useUiStore.getState(); events = [];
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    useInlineStarts.setState({ drafts: {}, retired: new Set() });
    useShellStore.setState({ connection: "live", rest: legacyRest({ navigator: { devices: [{ id: "local", kind: "local", label: "Fixture", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() }], workspaces: [{ id: "project", path: "/fixture", device_id: "local", label: "Fixture", registered: true, temporary: false, pinned: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [] }] } }, []) });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove();
    useInlineStarts.setState({ drafts: {}, retired: new Set() }); useShellStore.setState(shell, true); useUiStore.setState(ui, true);
    vi.useRealTimers(); vi.unstubAllGlobals();
  });
  const starts = () => events.filter((event) => event.kind === "agent_start_in_checkout");
  const button = (kind: string) => container.querySelector<HTMLButtonElement>(`[data-group-start-${kind}]`)!;
  const mount = async (shownTarget = target) => {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><InlineStartHost /><AgentTabStart workspace={{ device_id: "local", path: "/fixture" }} target={shownTarget} groupName="Fixture" actions={actions} /></TooltipProvider>));
  };
  const publishReceipts = async (shown: AgentStartTarget, receipts: AgentStartReceipt[]) => {
    const rest = useShellStore.getState().rest!;
    const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false,  exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), agent_groups: { selected: shown.group_id, groups: [{ id: shown.group_id, name: "Fixture", tabs: ["draft-1"] }], drafts: ["draft-1"], start_receipts: receipts } };
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0]!, checkouts: [checkout] }] }, workspace_view: { device_id: "local", path: "/fixture", open: false, documents: false, tool: null, prs: { only_checkout: false }, pinned: false, document_width: null, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view", agent_layout: { root: { area: { id: shown.area_id, active: "draft-1", displays: [{ id: "draft-1" }] } }, active_area: shown.area_id, limits: { areas: 6, depth: 3, displays: 64 }, display_count: 1, canvases: {}, group_id: shown.group_id } } } }));
  };
  it("starts one shell in the selected draft without a prompt or model, even with an unavailable agent selection", async () => {
    useInlineStarts.getState().put(key, { text: "keep this instruction", selection: { kind: "claude", model: "unavailable-model" }, request: null, phase: "failed", taskId: null, failure: { message: "Model unavailable" } });
    await mount();
    await act(async () => { button("terminal").click(); button("terminal").click(); });
    expect(starts()).toHaveLength(1);
    expect(starts()[0]!.payload).toMatchObject({ provider: "terminal", checkout_path: "/fixture", target });
    expect(starts()[0]!.payload).not.toHaveProperty("prompt");
    expect(starts()[0]!.payload).not.toHaveProperty("model");
    expect(container.querySelector("textarea")!.value).toBe("keep this instruction");
  });
  it("keeps the original request after timeout and only asks to confirm it", async () => {
    vi.useFakeTimers(); await mount();
    await act(async () => button("agent").click());
    const original = starts()[0]!.payload as { request_id: string };
    await act(async () => vi.advanceTimersByTime(90_000));
    const confirm = [...container.querySelectorAll("button")].find((row) => row.textContent === "Check start status")!;
    await act(async () => confirm.click());
    expect(starts()).toHaveLength(2);
    expect(starts()[1]!.payload).toMatchObject({ request_id: original.request_id, target, confirm_only: true });
    expect(button("agent").disabled).toBe(true);
    expect(button("terminal").disabled).toBe(true);
    expect(container.querySelector("textarea")!.readOnly).toBe(true);
  });
  it.each(["terminal", "claude"] as const)("immediately permits another status-only check after an uncertain %s confirmation fails", async (provider) => {
    vi.useFakeTimers();
    const text = "확인 중에도 보존할 한글 지시문";
    useInlineStarts.getState().put(key, { text, selection: { kind: "claude", model: null }, request: null, phase: "editing", taskId: null, failure: null });
    await mount();
    await act(async () => button(provider === "terminal" ? "terminal" : "agent").click());
    const original = starts()[0]!.payload as { request_id: string };
    await publishReceipts(target, [{ request_id: original.request_id, ...target, draft_tab_id: target.tab_id, live_tab_id: null, pane_id: null, state: "pending", agent_kind: provider === "terminal" ? null : provider, agent_phase: null }]);
    const message = "The original start is not yet confirmed. Retry confirmation; no second process was started.";
    for (const taskId of [42, 43]) {
      const rest = useShellStore.getState().rest!;
      await act(async () => useShellStore.setState({ rest: { ...rest, task_operation: { id: taskId, kind: "agent_start", phase: "failed", repository_root: "/fixture", branch: null, base_branch: null, path: "/fixture", pane_id: null, agent_kind: provider === "terminal" ? null : provider, message, agent_phase: null, agent_message: null, request_id: original.request_id } } }));
      expect(container.querySelector('[role="alert"]')?.textContent).toBe(message);
      const confirm = [...container.querySelectorAll("button")].find((row) => row.textContent === "Check start status")!;
      expect(confirm).toBeDefined();
      expect(confirm.disabled).toBe(false);
      expect(button("agent").disabled).toBe(true);
      expect(button("terminal").disabled).toBe(true);
      expect(container.querySelector("textarea")!.value).toBe(text);
      expect(container.querySelector("textarea")!.readOnly).toBe(true);
      await act(async () => confirm.click());
      expect(starts().at(-1)!.payload).toMatchObject({ request_id: original.request_id, target, provider, confirm_only: true });
    }
    expect(starts()).toHaveLength(3);
  });
  it.each(["failed", "agent_failed"] as const)("accepts multiple Korean input events after a restored %s and retries the same confirmed intent", async (phase) => {
    const receipt: AgentStartReceipt = { request_id: "start-restored", group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: phase === "agent_failed" ? "live-1" : null, pane_id: phase === "agent_failed" ? "pane-1" : null, add_pane: false, state: phase === "failed" ? "failed" : "ready", agent_kind: "claude", agent_phase: phase === "agent_failed" ? "failed" : null };
    await publishReceipts(target, [receipt]);
    if (phase === "agent_failed") {
      const rest = useShellStore.getState().rest!;
      useShellStore.setState({ rest: { ...rest, task_operation: { id: 42, kind: "agent_start", phase: "ready", repository_root: "/fixture", branch: null, base_branch: null, path: "/fixture", pane_id: "pane-1", agent_kind: "claude", message: null, agent_phase: "failed", agent_message: "Enter the replacement instruction", request_id: receipt.request_id } } });
    }
    await mount();
    const field = container.querySelector("textarea")!;
    for (const text of ["다", "다시", "다시 시작할", "다시 시작할 한글 지시문"]) {
      expect(field.readOnly).toBe(false);
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(field, text);
        field.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(field.value).toBe(text);
      expect(field.readOnly).toBe(false);
      expect(button("agent").disabled).toBe(false);
      expect(useInlineStarts.getState().drafts[key]!.phase).toBe(phase);
    }
    expect(starts()).toHaveLength(0);
    await act(async () => button("agent").click());
    expect(starts()).toHaveLength(1);
    expect(starts()[0]!.payload).toMatchObject({ request_id: receipt.request_id, target, provider: "claude", prompt: "다시 시작할 한글 지시문" });
    expect(field.readOnly).toBe(true);
    expect(button("agent").disabled).toBe(true);
  });
  it("retains a full store's empty-group input through draft reservation and failure, retrying the same intent", async () => {
    const emptyTarget = { ...target, tab_id: null };
    const emptyKey = inlineStartKey("local", "/fixture", emptyTarget);
    const input = { text: "Keep my 작업", selection: { kind: "claude" as const, model: "chosen-model" }, request: null, phase: "editing" as const, taskId: null, failure: null };
    for (let index = 0; index < 255; index++) useInlineStarts.getState().put(inlineStartKey("local", `/other/${index}`, emptyTarget), input);
    useInlineStarts.getState().put(emptyKey, input);
    await mount(emptyTarget);
    await act(async () => button("agent").click());
    const original = starts()[0]!.payload as { request_id: string };
    const receipt: AgentStartReceipt = { request_id: original.request_id, group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: null, pane_id: null, add_pane: false, state: "pending", agent_kind: "claude", agent_phase: null };
    const publish = async (state: AgentStartReceipt["state"]) => {
      const rest = useShellStore.getState().rest!;
      const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false,  exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), agent_groups: { selected: "g1", groups: [{ id: "g1", name: "Fixture", tabs: ["draft-1"] }], drafts: ["draft-1"], start_receipts: [{ ...receipt, state }] } };
      await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0]!, checkouts: [checkout] }] } } }));
    };
    const observedSizes: number[] = [];
    const unsubscribe = useInlineStarts.subscribe((state) => observedSizes.push(Object.keys(state.drafts).length));
    await publish("pending");
    unsubscribe();
    await mount(target);
    expect(observedSizes).toEqual([256]);
    expect(useInlineStarts.getState().drafts[emptyKey]).toBeUndefined();
    expect(useInlineStarts.getState().drafts[key]).toMatchObject({ text: input.text, selection: input.selection, phase: "pending", request: { id: original.request_id, target, prompt: input.text, model: "chosen-model" } });
    expect(container.querySelector("textarea")!.value).toBe(input.text);
    expect(button("agent").disabled).toBe(true);
    await publish("failed");
    expect(container.querySelector("textarea")!.value).toBe(input.text);
    expect(useInlineStarts.getState().drafts[key]!.phase).toBe("failed");
    await act(async () => button("agent").click());
    expect(starts()).toHaveLength(2);
    expect(starts()[1]!.payload).toMatchObject({ request_id: original.request_id, target, prompt: input.text, model: "chosen-model" });
  });
  it.each(["terminal", "claude"] as const)("retries a failed terminal intent through its terminal button with %s selected", async (kind) => {
    const input = { text: "터미널 실패 뒤에도 보관", selection: { kind, model: null }, request: null, phase: "editing" as const, taskId: null, failure: null };
    useInlineStarts.getState().put(key, input);
    await mount();
    await act(async () => button("terminal").click());
    const original = starts()[0]!.payload as { request_id: string };
    const receipt: AgentStartReceipt = { request_id: original.request_id, group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: null, pane_id: null, add_pane: false, state: "failed", agent_kind: null, agent_phase: null };
    const rest = useShellStore.getState().rest!;
    const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false,  exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), agent_groups: { selected: "g1", groups: [{ id: "g1", name: "Fixture", tabs: ["draft-1"] }], drafts: ["draft-1"], start_receipts: [receipt] } };
    await act(async () => useShellStore.setState({ rest: { ...rest, navigator: { ...rest.navigator, workspaces: [{ ...rest.navigator!.workspaces![0]!, checkouts: [checkout] }] } } }));
    expect(button("terminal").disabled).toBe(false);
    expect(button("terminal").textContent).toBe("Retry terminal start");
    expect(button("agent").disabled).toBe(true);
    expect(container.querySelector("textarea")!.value).toBe(input.text);
    await act(async () => button("terminal").click());
    expect(starts()).toHaveLength(2);
    expect(starts()[1]!.payload).toMatchObject({ request_id: original.request_id, target, provider: "terminal" });
    expect(starts()[1]!.payload).not.toHaveProperty("prompt");
    expect(starts()[1]!.payload).not.toHaveProperty("model");
  });
  it.each([
    ["terminal", "g2"], ["terminal", null], ["agent", "g2"], ["agent", null], ["agent", "g1"],
  ] as const)("starts a fresh %s intent after definite failure and relocation to %s", async (kind, group) => {
    const input = { text: "이동한 작업의 한글 입력 유지", selection: { kind: "claude" as const, model: "chosen-model" }, request: null, phase: "editing" as const, taskId: null, failure: null };
    useInlineStarts.getState().put(key, input);
    await mount();
    await act(async () => button(kind).click());
    const original = starts()[0]!.payload as { request_id: string };
    const receipt: AgentStartReceipt = { request_id: original.request_id, group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: null, pane_id: null, add_pane: false, state: "failed", agent_kind: kind === "terminal" ? null : "claude", agent_phase: null };
    await publishReceipts(target, [receipt]);
    expect(useInlineStarts.getState().drafts[key]!.phase).toBe("failed");
    const moved = { ...target, group_id: group, area_id: "a2" };
    await publishReceipts(moved, [receipt]); await mount(moved);
    expect(container.querySelector("textarea")!.value).toBe(input.text);
    expect(container.querySelector("textarea")!.readOnly).toBe(false);
    expect(useInlineStarts.getState().drafts[key]).toMatchObject({ selection: input.selection, request: null, phase: "editing" });
    expect(starts()).toHaveLength(1);
    await act(async () => button(kind).click());
    const next = starts()[1]!.payload as { request_id: string };
    expect(next.request_id).not.toBe(original.request_id);
    expect(next).toMatchObject({ target: moved, provider: kind === "terminal" ? "terminal" : "claude" });
    if (kind === "agent") expect(next).toMatchObject({ prompt: input.text, model: "chosen-model" });
    await publishReceipts(moved, [receipt]);
    expect(useInlineStarts.getState().drafts[key]!.request!.id).toBe(next.request_id);
    const ready: AgentStartReceipt = { ...receipt, request_id: next.request_id, group_id: group, area_id: "a2", state: "ready", live_tab_id: "live-1", pane_id: "pane-1", agent_phase: null };
    await publishReceipts(moved, [receipt, ready]);
    expect(useInlineStarts.getState().drafts[key]).toBeUndefined();
    // A later move back must not resurrect the superseded failure's blank form.
    await publishReceipts(target, [receipt, ready]);
    expect(useInlineStarts.getState().drafts[key]).toBeUndefined();
    expect(starts()).toHaveLength(2);
  });
  it.each(["pending", "failed"] as const)("keeps a %s receipt's unresolved intent after relocation and only confirms the original target", async (state) => {
    vi.useFakeTimers(); await mount();
    await act(async () => button("agent").click());
    const original = starts()[0]!.payload as { request_id: string };
    const receipt: AgentStartReceipt = { request_id: original.request_id, group_id: "g1", area_id: "a1", draft_tab_id: "draft-1", live_tab_id: null, pane_id: null, add_pane: false, state, agent_kind: "claude", agent_phase: null };
    await publishReceipts(target, [receipt]);
    if (state === "failed") await act(async () => button("agent").click());
    const submitted = starts().length;
    const moved = { ...target, group_id: "g2", area_id: "a2" };
    await publishReceipts(moved, [receipt]); await mount(moved);
    expect(useInlineStarts.getState().drafts[key]).toMatchObject({ phase: "pending", request: { id: original.request_id, target } });
    await act(async () => vi.advanceTimersByTime(90_000));
    await publishReceipts(moved, [receipt]);
    expect(useInlineStarts.getState().drafts[key]).toMatchObject({ phase: "unknown", request: { id: original.request_id, target } });
    expect(button("agent").disabled).toBe(true);
    expect(button("terminal").disabled).toBe(true);
    expect(container.querySelector("textarea")!.readOnly).toBe(true);
    const confirm = [...container.querySelectorAll("button")].find((row) => row.textContent === "Check start status")!;
    await act(async () => confirm.click());
    expect(starts()).toHaveLength(submitted + 1);
    expect(starts().at(-1)!.payload).toMatchObject({ request_id: original.request_id, target, confirm_only: true });
  });
  it("keeps draft input through group move, removal and dissolve without starting, then qualifies the current target", async () => {
    const input = { text: "이 한글 작업과 선택을 유지해 주세요", selection: { kind: "claude" as const, model: "chosen-model" }, request: null, phase: "editing" as const, taskId: null, failure: null };
    useInlineStarts.getState().put(key, input);
    for (const group of ["g1", "g2", null, "g3", null]) {
      await mount({ ...target, group_id: group, area_id: "a2" });
      expect(container.querySelector("textarea")!.value).toBe(input.text);
      expect(useInlineStarts.getState().drafts[inlineStartKey("local", "/fixture", { ...target, group_id: group })]?.selection).toEqual(input.selection);
      expect(Object.keys(useInlineStarts.getState().drafts)).toHaveLength(1);
      expect(starts()).toHaveLength(0);
    }
    await act(async () => button("agent").click());
    expect(starts()[0]!.payload).toMatchObject({ provider: "claude", model: "chosen-model", prompt: input.text, target: { ...target, group_id: null, area_id: "a2" } });
  });
  it("does not start during Korean composition or while the connection is down", async () => {
    await mount();
    await act(async () => container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: true })));
    expect(starts()).toHaveLength(0);
    await act(async () => useShellStore.getState().setConnection("connecting"));
    expect(button("agent").disabled).toBe(true); expect(button("terminal").disabled).toBe(true);
    await act(async () => button("terminal").click()); expect(starts()).toHaveLength(0);
  });
});
