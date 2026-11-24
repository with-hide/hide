// @vitest-environment jsdom
import { Blob as NodeBlob } from "node:buffer";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureAttachments } from "../../attachments";
import { keepSent } from "../../deskComposer";
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

it("gets its words back when the core answers them as closed on another pane's thread", async () => {
  const { container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("계속");
  await enter();
  const request = events[0]!.payload.request_id as string;
  // Another window opened another pane's thread before the core took the words.
  await act(async () => useShellStore.setState({ sessionThread: section({ generation: 5, pane_id: "w1:p9", input: { request_id: request, state: "refused", reason: "closed" } }) }));
  expect(box().value).toBe("계속");
  expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe("This thread is not open");
});

it("asks for the page before once per state the core shows, a failed read included", async () => {
  const { actions, container } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section({ has_older: true, older: "failed" }) }));
  const conversation = container.querySelector('[data-desk-thread-conversation="true"]')!;
  const reachTop = () => act(async () => { conversation.dispatchEvent(new Event("scroll")); });
  await reachTop();
  await reachTop();
  await reachTop();
  expect(actions.readOlderThread).toHaveBeenCalledTimes(1);
  await act(async () => useShellStore.setState({ sessionThread: section({ has_older: true, older: "loading" }) }));
  await reachTop();
  await act(async () => useShellStore.setState({ sessionThread: section({ has_older: true, older: "failed" }) }));
  await reachTop();
  await reachTop();
  expect(actions.readOlderThread).toHaveBeenCalledTimes(2);
  expect(actions.readOlderThread).toHaveBeenLastCalledWith(4);
});

/** A file whose bytes the tray reads in, as a browser's File gives them (jsdom's has no `stream`). */
function readable(name: string, bytes: Uint8Array<ArrayBuffer>): File {
  const file = new File([bytes], name);
  file.stream = () => new NodeBlob([bytes]).stream() as ReadableStream<Uint8Array<ArrayBuffer>>;
  return file;
}

/** A file the tray must never read: past the cap, its bytes stay where they are (desk-2-ui B42). */
function unreadable(name: string, size: number): File {
  const file = new File([], name);
  Object.defineProperty(file, "size", { value: size });
  file.stream = () => { throw new Error(`${name} was read`); };
  file.arrayBuffer = () => Promise.reject(new Error(`${name} was read`));
  return file;
}

async function drop(target: Element, files: File[]) {
  const transfer = { types: ["Files"], files, dropEffect: "none" };
  for (const kind of ["dragenter", "dragover", "drop"]) {
    const event = new Event(kind, { bubbles: true, cancelable: true });
    Object.defineProperty(event, "dataTransfer", { value: transfer });
    await act(async () => { target.dispatchEvent(event); });
  }
}

it("takes dropped files above the box, holds send while one is past 20 MB without reading it, and sends the rest once it is out (desk-2-ui B41, B42)", async () => {
  const { container, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("이 로그도 같이 봐 줘");
  const frame = container.querySelector("[data-desk-composer-frame]")!;
  await drop(frame, [readable("install.log", new Uint8Array([7, 8, 9])), unreadable("all-logs.zip", 20 * 1024 * 1024 + 1)]);
  await vi.waitFor(() => expect(container.querySelector('[data-desk-attachment="ready"]')).not.toBeNull());

  const big = container.querySelector('[data-desk-attachment="too_large"]');
  expect(big?.textContent).toContain("Over 20 MB");
  expect(container.querySelector<HTMLButtonElement>('[data-desk-composer-send="true"]')?.disabled).toBe(true);
  await enter();
  expect(events).toEqual([]);

  await act(async () => container.querySelector<HTMLButtonElement>('[data-desk-attachment-remove="all-logs.zip"]')!.click());
  expect(container.querySelectorAll("[data-desk-attachment]")).toHaveLength(1);
  expect(container.querySelector<HTMLButtonElement>('[data-desk-composer-send="true"]')?.disabled).toBe(false);
  await enter();
  expect(events.map((event) => event.kind)).toEqual(["attachment_stage", "thread_input"]);
  expect(events[0]!.payload).toMatchObject({ name: "install.log", size: 3 });
  expect(events[1]!.payload).toMatchObject({ text: "이 로그도 같이 봐 줘", stages: [events[0]!.payload.request_id] });
});

it("draws the files a sent message typed before its words over its bubble, by the names they were sent under (desk-2-ui B41)", async () => {
  const { container } = await mount();
  const stage = "0b7a3c52-9a1e-4f7d-8c2b-1a2b3c4d5e6f";
  const typed = `"/Users/me/.hide/state/attachments/${stage}-0-화면 1.png" "C:/Users/me/.hide/state/terminal-attachments/hide-${stage}-1.log" 이것 좀 봐 줘`;
  keepSent(`${stage}-1`, "install.log", null);
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: typed } }], parts: {}, sessions: {} } }) }));
  const sent = container.querySelector("[data-desk-sent-attachments]");
  expect([...sent!.querySelectorAll("[data-desk-attachment]")].map((tile) => tile.querySelector('[role="img"]')?.getAttribute("aria-label"))).toEqual(["화면 1.png", "install.log"]);
  expect(container.querySelector('[data-desk-thread-line="user"] p')?.textContent).toBe("이것 좀 봐 줘");
});
