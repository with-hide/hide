// @vitest-environment jsdom
import { Blob as NodeBlob } from "node:buffer";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureAttachments } from "../../attachments";
import { configureFileBytes, receiveBytes } from "../../fileBytes";
import { keepSent } from "../../deskComposer";
import type { CopyText } from "../human-turn-copy";
import type { AskLine } from "../open-asks";
import type { DeskJob } from "../../desk";
import type { SessionThreadSection } from "../../snapshot";
import { useShellStore } from "../../store";
import { TooltipProvider } from "../ui/tooltip";
import { ThreadPanel } from "./thread-panel";
import { clearPreviews } from "../../filePreview";
import { drawPreview } from "../../previewDraw";

// A test window has no image decoder: a tile's picture is stood in once the file's bytes arrived.
vi.mock("../../previewDraw", () => ({ drawPreview: vi.fn(async () => ({ state: "picture", picture: "data:image/jpeg;drawn", pages: null, seconds: null })) }));

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
  // The input box holds the corner's notices above it by its measured box (`noticeFloor.ts`).
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  saved = useShellStore.getState();
  useShellStore.setState({ connection: "live" });
  events = [];
  configureAttachments({ dispatch: (event) => events.push({ kind: event.kind, payload: event.payload as Record<string, unknown> }), sendBinary: () => undefined });
});

afterEach(async () => {
  clearPreviews();
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

async function mount(card: DeskJob | null = job, foot: ReactNode = null, { asks = [], onCopy = () => Promise.resolve(true) }: { asks?: AskLine[]; onCopy?: CopyText } = {}) {
  const actions = { openThread: vi.fn(), closeThread: vi.fn(), readOlderThread: vi.fn() };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  const noop = () => undefined;
  await act(async () => root.render(
    <TooltipProvider>
      <ThreadPanel job={card} paneId={job.pane_id} actions={actions} foot={foot} asks={asks} onTerminal={noop} onMission={noop} outputOf={(output) => output} onOutput={noop} onSession={noop} onCopy={onCopy} />
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

it("keeps a sleeping session's record above the foot that wakes it once its card is gone with its pane (PRD desk-2-ui B43)", async () => {
  const { container } = await mount(null, <p data-foot="true">에이전트 깨우기</p>);
  await act(async () => useShellStore.setState({ sessionThread: section({ pane: "gone" }), sessionThreadHistory: { generation: 4, pages: [] } }));
  expect(container.textContent).toContain("다크도 찍어줘");
  expect(container.textContent).toContain("라이트는 끝났어요");
  // The foot stands in the input box's place, after the conversation.
  const conversation = container.querySelector('[data-desk-thread-conversation="true"]')!;
  expect(conversation.compareDocumentPosition(container.querySelector('[data-foot="true"]')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(container.querySelector("textarea")).toBeNull();
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

it("takes a file dragged anywhere over the panel into its box, lighting only the box's border, and none while the wake line stands in its place (Doop L 나)", async () => {
  const { container } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  const frame = () => container.querySelector("[data-desk-composer-frame]")!.getAttribute("data-desk-composer-frame");
  const conversation = container.querySelector("[data-desk-thread-conversation]")!;
  await drag(conversation, "dragenter");
  expect(frame()).toBe("drop");
  // Nothing else is drawn over the panel or the box while the files are over it.
  expect(container.querySelector("[data-desk-composer-drop]")).toBeNull();
  await drag(conversation, "dragleave");
  expect(frame()).toBe("rest");
  await drop(conversation, [readable("mini-login.log", new Uint8Array([1, 2]))]);
  await vi.waitFor(() => expect(container.querySelector('[data-desk-attachment="ready"]')?.textContent).toContain("mini-login.log"));

  const asleep = await mount(job, <p data-wake="true">wake</p>);
  const panel = asleep.container.querySelector("[data-desk-thread-panel]")!;
  const refused = new Event("dragover", { bubbles: true, cancelable: true });
  Object.defineProperty(refused, "dataTransfer", { value: { types: ["Files"], files: [], dropEffect: "none" } });
  await act(async () => { panel.dispatchEvent(refused); });
  expect(refused.defaultPrevented).toBe(false);
});

/** A drag of files from the system over `target`, leaving to nothing when it is `dragleave`. */
async function drag(target: Element, kind: "dragenter" | "dragleave") {
  const event = new Event(kind, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", { value: { types: ["Files"], files: [], dropEffect: "none" } });
  Object.defineProperty(event, "relatedTarget", { value: null });
  await act(async () => { target.dispatchEvent(event); });
}

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

it("draws an image the session showed as its tile's picture, made from the file's bytes as every tile makes one (feedback 2 frame M 나)", async () => {
  const reads: Record<string, unknown>[] = [];
  configureFileBytes((event) => { reads.push(event.payload as Record<string, unknown>); });
  const { container } = await mount();
  const shown = { sessionUpdate: "tool_call", toolCallId: "f", kind: "execute", title: "hide file open /repo/shots/dark.png", status: "completed" };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: { updates: [shown], parts: { f: { kind: "view", view: "file", target: "/repo/shots/dark.png" } }, sessions: {} } }) }));
  const answer = async (read: Record<string, unknown>, bytes: number[], total: number) => {
    const header = new TextEncoder().encode(JSON.stringify({ type: "file_bytes", request_id: read.request_id, path: "/repo/shots/dark.png", offset: 0, total, eof: true }));
    const frame = new Uint8Array(4 + header.length + bytes.length);
    new DataView(frame.buffer).setUint32(0, header.length, false);
    frame.set(header, 4);
    frame.set(bytes, 4 + header.length);
    await act(async () => receiveBytes(frame.buffer));
  };
  // Its size first, then the whole file, since it is under the image cap.
  expect(reads).toMatchObject([{ path: "/repo/shots/dark.png", length: 1 }]);
  await answer(reads[0]!, [1], 2);
  await vi.waitFor(() => expect(reads).toHaveLength(2));
  expect(reads[1]).toMatchObject({ path: "/repo/shots/dark.png", length: null });
  await answer(reads[1]!, [1, 2], 2);
  const tile = container.querySelector('[data-desk-hide-part="view"] [data-desk-output-thumb="image"]');
  await vi.waitFor(() => expect(tile?.querySelector("[data-desk-tile-picture]")?.getAttribute("src")).toBe("data:image/jpeg;drawn"));
  expect(drawPreview).toHaveBeenCalledWith("image", new Uint8Array([1, 2]), "/repo/shots/dark.png");
});

it("draws a turn an agent recorded as interrupted as one quiet line, never as the operator's bubble", async () => {
  const { container } = await mount();
  const interrupted = { sessionUpdate: "user_message_chunk", content: { type: "text", text: "[Request interrupted by user]" }, _meta: { hide: { origin: "interrupted" } } };
  const tail = { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said("라이트는"), interrupted], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const line = container.querySelector('[data-desk-thread-line="interrupted"]');
  expect(line?.textContent).toBe("Interrupted");
  expect(container.textContent).not.toContain("Request interrupted");
  // The only bubble is the operator's own words.
  expect(Array.from(container.querySelectorAll('[data-desk-thread-line="user"] [data-desk-thread-bubble]')).map((one) => one.textContent)).toEqual(["다크도 찍어줘"]);
});

/** Waits until the lazily loaded Markdown language parsed the messages, so a code block carries its copy button. */
const codeCopyDrawn = (container: HTMLElement) => act(() => vi.waitFor(() => expect(container.querySelector("[data-markdown-code-copy]")).not.toBeNull()));

it("draws the agent's words as Markdown and copies a message's original words and a code block's code (Doop E2)", async () => {
  const copied: [string, string][] = [];
  const { container } = await mount(job, null, { onCopy: async (text, what) => { copied.push([text, what]); return true; } });
  const words = "**라이트**는 끝났어요.\n\n```\nopen https://play.google.com/console\n```";
  const tail = { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said(words)], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await codeCopyDrawn(container);
  const message = container.querySelector('[data-desk-thread-line="agent"]')!;
  expect(message.querySelector('[data-markdown-text="true"]')).not.toBeNull();
  await act(async () => message.querySelector<HTMLElement>('[data-message-copy] [data-copy-words="message"]')!.click());
  await act(async () => message.querySelector<HTMLElement>('[data-markdown-code-copy] [data-copy-words="code block"]')!.click());
  expect(copied).toEqual([[words, "message"], ["open https://play.google.com/console", "code block"]]);
  // A copy that took says so on the button that made it.
  expect(message.querySelector('[data-copy-words="message"]')?.getAttribute("data-copied")).toBe("true");
});

it("keeps every message's view while the operator types and while the last message streams, so a selection in one stays", async () => {
  // Mounted as Desk mounts it: no mentions passed.
  const { container, type } = await mount();
  const answer = (id: string, text: string) => ({ sessionUpdate: "agent_message_chunk", messageId: id, content: { type: "text", text } });
  const turns = (last: string) => ({ updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "첫 질문" } }, answer("a", "첫 **답**"), { sessionUpdate: "user_message_chunk", content: { type: "text", text: "둘째 질문" } }, answer("b", last)], parts: {}, sessions: {} });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: turns("쓰는 중") }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const views = () => Array.from(container.querySelectorAll('[data-desk-thread-line="agent"] .cm-editor'));
  const [first, second] = views();
  expect([first, second].every(Boolean)).toBe(true);
  await type("다");
  expect(views()).toEqual([first, second]);
  // A streamed chunk redraws only the message it grows.
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: turns("쓰는 중, 더 썼어요") }) }));
  expect(views()[0]).toBe(first);
  expect(container.textContent).toContain("쓰는 중, 더 썼어요");
});

it("shows the questions Hide found above the box and goes to the answer each was asked in, lit for 2 s (Doop B4)", async () => {
  const scrolled: string[] = [];
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.textContent ?? ""); };
  const answer = (text: string, at: number) => ({ sessionUpdate: "agent_message_chunk", messageId: text, content: { type: "text", text }, _meta: { hide: { at_ms: at } } });
  const asks: AskLine[] = [
    { id: "missed-1-0", text: "다크도 찍을지 물었어요", at_unix_ms: 1_500 },
    { id: "missed-2-0", text: "본인 확인을 직접 해 줄 수 있는지 물었어요", at_unix_ms: 2_500 },
  ];
  const { container } = await mount(job, null, { asks });
  const tail = { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "스크린샷 찍어 줘" } }, answer("라이트를 찍었어요. 다크도 찍을까요?", 1_000), { sessionUpdate: "user_message_chunk", content: { type: "text", text: "결제해놨어" } }, answer("본인 확인은 직접 해 줄 수 있어?", 2_000)], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  // One line above the box says the newest question and how many more there are.
  const line = container.querySelector("[data-desk-thread-asks]")!;
  expect(line.querySelector('[data-desk-thread-ask="missed-2-0"]')?.textContent).toContain("본인 확인을 직접 해 줄 수 있는지 물었어요");
  expect(line.querySelector("[data-desk-thread-asks-more]")?.getAttribute("data-desk-thread-asks-more")).toBe("1");
  expect(line.compareDocumentPosition(container.querySelector("textarea")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

  vi.useFakeTimers();
  try {
    await act(async () => line.querySelector<HTMLElement>('[data-desk-thread-ask="missed-2-0"]')!.click());
    const lit = () => Array.from(container.querySelectorAll('[data-desk-thread-lit="true"]')).map((one) => one.textContent);
    expect(scrolled.at(-1)).toContain("본인 확인은 직접 해 줄 수 있어?");
    expect(lit()).toEqual([expect.stringContaining("본인 확인은 직접 해 줄 수 있어?")]);
    await act(async () => vi.advanceTimersByTime(1_999));
    expect(lit()).toHaveLength(1);
    await act(async () => vi.advanceTimersByTime(1));
    expect(lit()).toEqual([]);
  } finally {
    vi.useRealTimers();
  }
});
