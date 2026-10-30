// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureAttachments } from "../../attachments";
import type { DeskJob } from "../../desk";
import type { SessionThreadSection } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { ThreadPanel } from "./thread-panel";

vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

const job: DeskJob = {
  id: "gallery", agent_id: "agent-7", pane_id: "w1:p2", title: "갤러리 캡처 비교", agent_kind: "claude", project: { workspace_id: "w", label: "herdr-ide" }, location: { kind: "worktree", label: "desk-threads" },
  state: "working", queue_position: null, now: "캡처 중", plan: null, question: null, summary: null, pull_requests: [], outputs: [], joined: false,
  reason: null, failure: null, ask: null, mark: "going", mark_read: false, machine: null, disconnected: false, changed_at_unix_ms: 0, read: false, mission: null, workers: [],
};

const said = (text: string) => ({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });

function section(extra: Partial<SessionThreadSection> = {}): SessionThreadSection {
  return { generation: 4, state: "ready", pane_id: "w1:p2", machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", input: null,
    tail: { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said("라이트는 끝났어요")], parts: {}, sessions: {} }, ...extra };
}

let saved: ReturnType<typeof useShellStore.getState>;
let events: { kind: string; payload: Record<string, unknown> }[];
let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  saved = useShellStore.getState();
  events = [];
  configureAttachments({ dispatch: (event) => events.push({ kind: event.kind, payload: event.payload as Record<string, unknown> }), sendBinary: () => undefined });
});

afterEach(async () => {
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function mount() {
  const actions = { openThread: vi.fn(), closeThread: vi.fn(), readOlderThread: vi.fn() };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  const noop = () => undefined;
  await act(async () => root.render(
    <TooltipProvider>
      <ThreadPanel job={job} actions={actions} pinned={false} onBack={noop} onPin={noop} onClose={noop} onTerminal={noop} onMission={noop} onOutput={noop} onSession={noop} />
    </TooltipProvider>,
  ));
  const box = () => container.querySelector("textarea")!;
  const type = async (value: string) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(box(), value);
      box().dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const enter = (composing = false) => act(async () => { box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: composing })); });
  return { actions, container, root, box, type, enter };
}

it("opens its pane's thread while shown, draws what the core read, and closes it after", async () => {
  const { actions, container, root } = await mount();
  expect(actions.openThread).toHaveBeenCalledWith("w1:p2");
  // Before the core holds this pane's thread, the record is still being read.
  expect(container.querySelector('[data-desk-thread-reading="true"]')).not.toBeNull();
  await act(async () => useShellStore.setState({ sessionThread: section(), sessionThreadHistory: { generation: 4, pages: [] } }));
  expect(container.querySelector('[data-desk-thread-reading="true"]')).toBeNull();
  expect(container.textContent).toContain("다크도 찍어줘");
  expect(container.textContent).toContain("라이트는 끝났어요");
  await act(async () => root.unmount());
  mounted = [];
  expect(actions.closeThread).toHaveBeenCalledTimes(1);
});

it("sends the box's words on Enter, keeps them until the pane took them, and never sends mid-composition", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("다크도 부탁해");
  await enter(true);
  expect(events).toEqual([]);
  await enter();
  expect(events).toHaveLength(1);
  expect(events[0]).toMatchObject({ kind: "thread_input", payload: { generation: 4, pane_id: "w1:p2", text: "다크도 부탁해", stages: [] } });
  const request = events[0]!.payload.request_id as string;
  await act(async () => useShellStore.setState({ sessionThread: section({ input: { request_id: request, state: "sending" } }) }));
  expect(box().value).toBe("다크도 부탁해");
  await act(async () => useShellStore.setState({ sessionThread: section({ input: { request_id: request, state: "sent" } }) }));
  expect(box().value).toBe("");
});

it("keeps the words and says why when the pane refused them, and holds send while the pane runs another session", async () => {
  const { container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("계속");
  await enter();
  const request = events[0]!.payload.request_id as string;
  await act(async () => useShellStore.setState({ sessionThread: section({ input: { request_id: request, state: "refused", reason: "asleep" } }) }));
  expect(box().value).toBe("계속");
  expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe("The session is asleep: wake it to send");

  await act(async () => useShellStore.setState({ sessionThread: section({ pane: "changed" }) }));
  expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe("This pane runs another session now: this thread sends nothing there");
  expect(container.querySelector<HTMLButtonElement>('[data-desk-composer-send="true"]')?.disabled).toBe(true);
  await enter();
  expect(events).toHaveLength(1);
});
