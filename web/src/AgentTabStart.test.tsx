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
    useInlineStarts.setState({ drafts: {} });
    useShellStore.setState({ connection: "live", rest: legacyRest({ navigator: { devices: [{ id: "local", kind: "local", label: "Fixture", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null, agent_scope: emptyScope() }], workspaces: [{ id: "project", path: "/fixture", device_id: "local", label: "Fixture", registered: true, temporary: false, pinned: false, agent_scope: emptyScope(), inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [] }] } }, []) });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove();
    useInlineStarts.setState({ drafts: {} }); useShellStore.setState(shell, true); useUiStore.setState(ui, true);
    vi.useRealTimers(); vi.unstubAllGlobals();
  });
  const starts = () => events.filter((event) => event.kind === "agent_start_in_checkout");
  const button = (kind: string) => container.querySelector<HTMLButtonElement>(`[data-group-start-${kind}]`)!;
  const mount = async (shownTarget = target) => {
    const actions = createActions((event) => { events.push(event); return true; });
    await act(async () => root.render(<TooltipProvider><InlineStartHost /><AgentTabStart workspace={{ device_id: "local", path: "/fixture" }} target={shownTarget} groupName="Fixture" actions={actions} /></TooltipProvider>));
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
      const checkout: Checkout = { id: "checkout", workspace_id: "project", label: "Fixture", path: "/fixture", branch: null, purpose: null, is_worktree: false, exists: true, has_panes: false, pull_request: null, active_tab_id: null, strip: [], next_tab_label: "1", tabs: [], agent_scope: emptyScope(), agent_groups: { selected: "g1", groups: [{ id: "g1", name: "Fixture", tabs: ["draft-1"] }], drafts: ["draft-1"], start_receipts: [{ ...receipt, state }] } };
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
  it("does not start during Korean composition or while the connection is down", async () => {
    await mount();
    await act(async () => container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: true })));
    expect(starts()).toHaveLength(0);
    await act(async () => useShellStore.getState().setConnection("connecting"));
    expect(button("agent").disabled).toBe(true); expect(button("terminal").disabled).toBe(true);
    await act(async () => button("terminal").click()); expect(starts()).toHaveLength(0);
  });
});
