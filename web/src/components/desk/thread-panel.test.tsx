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
import type { AgentRow, DeskMentions, SessionThreadSection, SnapshotRest, ThreadPage } from "../../snapshot";
import { useShellStore } from "../../store";
import { receiveThreadInputResult } from "../../sessionDraftSends";
import { useUiStore } from "../../ui";
import { NO_SESSION_DRAFTS, SESSION_DRAFT_BYTES } from "../../sessionDrafts";
import { TooltipProvider } from "../ui/tooltip";
import { ThreadPanel } from "./thread-panel";
import { forgetScrolls } from "../../conversationScroll";
import { drawnRows, layOutConversation, scrollConversation, watchResizes } from "./thread-test-layout";
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
  return { generation: 4, state: "ready", pane_id: "w1:p2", machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", input: null, queue: null, choice: {},
    tail: { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said("라이트는 끝났어요")], parts: {}, sessions: {} }, ...extra };
}

// Display snapshots and request completions are separate wire messages.
function completeInput(next: SessionThreadSection) {
  useShellStore.setState({ sessionThread: next });
  if (next.input) receiveThreadInputResult(next.input);
}

/** The rows each turn draws, oldest first: a turn is the rows it draws, not one element. */
function turnsIn(container: HTMLElement): HTMLElement[][] {
  const turns = new Map<string, HTMLElement[]>();
  for (const row of container.querySelectorAll<HTMLElement>("[data-desk-thread-turn]")) turns.set(row.dataset.deskThreadTurn!, [...(turns.get(row.dataset.deskThreadTurn!) ?? []), row]);
  return [...turns.values()];
}
const within = (rows: readonly HTMLElement[], selector: string) => rows.flatMap((row) => Array.from(row.querySelectorAll<HTMLElement>(selector)));
const wordsOf = (rows: readonly HTMLElement[]) => rows.map((row) => row.textContent).join("");
const drawnWords = (container: HTMLElement) => drawnRows(container).map((row) => row.textContent ?? "");
const layOut = layOutConversation;
let laidOut: () => Promise<void>;

let saved: ReturnType<typeof useShellStore.getState>;
let events: { kind: string; payload: Record<string, unknown> }[];
let mounted: ReturnType<typeof createRoot>[] = [];

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  // The input box holds the corner's notices above it by its measured box (`noticeFloor.ts`).
  ({ laidOut } = watchResizes());
  layOut();
  saved = useShellStore.getState();
  useUiStore.setState({ threadLease: null, paneConversation: null });
  useShellStore.setState({ connection: "live", threadInputResult: null });
  useUiStore.setState({ sessionDrafts: NO_SESSION_DRAFTS });
  events = [];
  configureAttachments({ dispatch: (event) => events.push({ kind: event.kind, payload: event.payload as Record<string, unknown> }), sendBinary: () => undefined });
});

afterEach(async () => {
  clearPreviews();
  for (const root of mounted) await act(async () => root.unmount());
  mounted = [];
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  // A conversation left above its foot is remembered by its pane, which every test shares.
  forgetScrolls();
  document.body.innerHTML = "";
});

async function mount(card: DeskJob | null = job, foot: ReactNode = null, { asks = [], ask = null, onCopy = () => Promise.resolve(true), onAsk = () => undefined, pane = "w1:p2", answer = 0 }: { asks?: AskLine[]; ask?: ReactNode; onCopy?: CopyText; onAsk?: (id: string, status: "open" | "dismissed") => void; pane?: string; answer?: number } = {}) {
  const actions = { openThread: vi.fn(), closeThread: vi.fn(), readOlderThread: vi.fn(), switchThread: vi.fn(() => "switch-1"), observeCatalog: vi.fn(() => noop), requestFileIndex: vi.fn(), deskCommandsCheck: vi.fn(), inputQueue: vi.fn(), interruptThread: vi.fn(() => "stop-1") };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  const noop = () => undefined;
  const draw = (raised: number) => root.render(
    <TooltipProvider>
      <ThreadPanel job={card} paneId={pane} answer={raised} actions={actions} foot={foot} ask={ask} asks={asks} onAsk={onAsk} onTerminal={noop} onMission={noop} outputOf={(output) => output} onOutput={noop} onSession={noop} onCopy={onCopy} />
    </TooltipProvider>,
  );
  await act(async () => draw(answer));
  const answerAgain = (raised: number) => act(async () => draw(raised));
  const box = () => container.querySelector("textarea")!;
  const type = async (value: string) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(box(), value);
      box().dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const enter = (composing = false) => act(async () => { box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, isComposing: composing })); });
  return { actions, container, root, box, type, enter, answerAgain };
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

it("sends the box's words on Enter, takes them out of the box at once, and never sends mid-composition", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("다크도 부탁해");
  await enter(true);
  expect(events).toEqual([]);
  await enter();
  expect(events).toHaveLength(1);
  expect(events[0]).toMatchObject({ kind: "thread_input", payload: { generation: 4, pane_id: "w1:p2", text: "다크도 부탁해", stages: [] } });
  const request = events[0]!.payload.request_id as string;
  expect(box().value).toBe("");
  await act(async () => useShellStore.setState({ sessionThread: section({ input: { request_id: request, state: "sending" } }) }));
  await act(async () => completeInput(section({ input: { request_id: request, state: "sent" } })));
  expect(box().value).toBe("");
});

// Review round 1 F1 (mission-109): a send the pane took while its panel was closed never comes back as a draft to go twice.
it("brings no sent words back when the panel closed before the pane took them", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await first.type("accepted once");
  await first.enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => first.root.unmount());
  await act(async () => completeInput(section({ input: { request_id: request, state: "sent" } })));
  const back = await mount();
  expect(back.box().value).toBe("");
});

// Review round 1 F2, round 3: an edit that would make the words longer past one send is not taken, and the line
// says so; the words already in the box stay whole.
it("does not take an edit that makes the words longer past one send, keeping the words it holds, and says so", async () => {
  const { container, box, type } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("hello");
  await type("x".repeat(SESSION_DRAFT_BYTES + 10));
  expect(box().value).toBe("hello");
  expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe("Too long to send in one go");
});

it("puts refused words back before any words written since, so neither is lost", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("계속");
  await enter();
  const request = events[0]!.payload.request_id as string;
  await type("다른 말");
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "asleep" } })));
  expect(box().value).toBe("계속\n다른 말");
});

// Review round 2 F1 (mission-109): words the pane refused after their panel closed come back when it opens again.
it("gives words refused while its panel was closed back to the box when the session opens again", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await first.type("not delivered, must not vanish");
  await first.enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => first.root.unmount());
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "closed" } })));
  const back = await mount();
  expect(back.box().value).toBe("not delivered, must not vanish");
});

// Review round 3 F1: the core's own order when another session opened while the words were on their way: the
// closed thread and the new one carry no answer, then the late answer lands on the new one by its request id.
it("frees the box and gives refused words back when the answer lands on a thread opened after the send", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await first.type("늦은 답");
  await first.enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => first.root.unmount());
  await act(async () => useShellStore.setState({ sessionThread: section({ state: "closed", pane_id: null }) }));
  const back = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section({ generation: 5 }) }));
  expect(back.container.querySelector<HTMLButtonElement>('[data-desk-composer-send="true"]')?.disabled).toBe(true);
  await act(async () => completeInput(section({ generation: 5, input: { request_id: request, state: "refused", reason: "closed" } })));
  expect(back.box().value).toBe("늦은 답");
  await back.enter();
  expect(events.filter((event) => event.kind === "thread_input")).toHaveLength(2);
});

// Review round 3 F7: words refused at the cap and words written since both come back, nothing cut.
it("gives back refused words at the cap together with every word written since", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  const full = "x".repeat(SESSION_DRAFT_BYTES);
  await type(full);
  await enter();
  const request = events[0]!.payload.request_id as string;
  await type("new words");
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "asleep" } })));
  expect(box().value).toBe(`${full}\nnew words`);
});

// Conductor's round 3 check: typing into a box that a restore left past one send never cuts its tail.
it("keeps a restored box past one send whole while typing would lengthen it, and takes an edit that shortens it", async () => {
  const { container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  const full = "x".repeat(SESSION_DRAFT_BYTES);
  await type(full);
  await enter();
  const request = events[0]!.payload.request_id as string;
  await type("new words");
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "asleep" } })));
  const restored = `${full}\nnew words`;
  expect(box().value).toBe(restored);
  await type(`${restored}!`);
  expect(box().value).toBe(restored);
  expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe("Too long to send in one go");
  await type(restored.slice(1));
  expect(box().value).toBe(restored.slice(1));
});

it("holds the next send while one the closed panel sent is still unanswered", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await first.type("first");
  await first.enter();
  await act(async () => first.root.unmount());
  const back = await mount();
  await back.type("second");
  await back.enter();
  expect(events.filter((event) => event.kind === "thread_input")).toHaveLength(1);
  expect(back.box().value).toBe("second");
});

it("keeps the words and says why when the pane refused them, and holds send while the pane runs another session", async () => {
  const { container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("계속");
  await enter();
  const request = events[0]!.payload.request_id as string;
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "asleep" } })));
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
  await act(async () => completeInput(section({ generation: 5, pane_id: "w1:p9", input: { request_id: request, state: "refused", reason: "closed" } })));
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
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: typed } }], parts: {}, sessions: {} } }) }));
  const sent = container.querySelector("[data-desk-sent-attachments]");
  expect([...sent!.querySelectorAll("[data-desk-attachment]")].map((tile) => tile.querySelector('[role="img"]')?.getAttribute("aria-label"))).toEqual(["화면 1.png", "install.log"]);
  expect(container.querySelector('[data-desk-thread-line="user"] p')?.textContent).toBe("이것 좀 봐 줘");
});

it("draws an image the session showed as its tile's picture, made from the file's bytes as every tile makes one (feedback 2 frame M 나)", async () => {
  const reads: Record<string, unknown>[] = [];
  configureFileBytes((event) => { reads.push(event.payload as Record<string, unknown>); });
  const { container } = await mount();
  const shown = { sessionUpdate: "tool_call", toolCallId: "f", kind: "execute", title: "hide file open /repo/shots/dark.png", status: "completed" };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: { record: "r1", start: 0, updates: [shown], parts: { f: { kind: "view", view: "file", target: "/repo/shots/dark.png" } }, sessions: {} } }) }));
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
  expect(drawPreview).toHaveBeenCalledWith("image", new Uint8Array([1, 2]), "/repo/shots/dark.png", expect.any(AbortSignal));
});

it("draws a turn an agent recorded as interrupted as one quiet line, never as the operator's bubble", async () => {
  const { container } = await mount();
  const interrupted = { sessionUpdate: "user_message_chunk", content: { type: "text", text: "[Request interrupted by user]" }, _meta: { hide: { origin: "interrupted" } } };
  const tail = { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said("라이트는"), interrupted], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const line = container.querySelector('[data-desk-thread-line="interrupted"]');
  expect(line?.textContent).toBe("Interrupted");
  expect(container.textContent).not.toContain("Request interrupted");
  // The only bubble is the operator's own words.
  expect(Array.from(container.querySelectorAll('[data-desk-thread-line="user"] [data-desk-thread-bubble]')).map((one) => one.textContent)).toEqual(["다크도 찍어줘"]);
});

const operator = (text: string) => ({ sessionUpdate: "user_message_chunk", content: { type: "text", text } });
const shell = (toolCallId: string, command: string, status = "completed") => ({ sessionUpdate: "tool_call", toolCallId, kind: "execute", title: command, status, rawInput: { command } });
const edit = (toolCallId: string, path: string, oldText: string | null, newText: string, status = "completed") => ({ sessionUpdate: "tool_call", toolCallId, kind: "edit", title: `Edit ${path}`, status, content: [{ type: "diff", path, oldText, newText }] });
const heldMission = { id: "mission-3", title: "Desk 1차", status: "running", decision: null, conductor: { present: true }, nodes: [{ key: "a", depends_on: [], status: "done", attention: null }, { key: "b", depends_on: [], status: "working", attention: null }] };
const shown = async (tail: ThreadPage) => act(async () => useShellStore.setState({ rest: { missions: { items: [heldMission] } } as unknown as SnapshotRest, sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));

it("draws what one turn leads to once per target, with its newest state, and a mission chip without its title beside it", async () => {
  const { container } = await mount({ ...job, state: "done" });
  await shown({
    record: "r1", start: 0, updates: [
      operator("미션 돌려줘"),
      shell("m1", "hide mission show --mission mission-3"),
      shell("l1", "hide request send agent-7 --body 첫 편지"),
      shell("m2", "hide mission node add --mission mission-3"),
      shell("l2", "hide request send agent-7 --body 둘째 편지"),
      shell("m3", "hide mission show --mission mission-3"),
      said("다 했어요"),
      operator("한 번 더"),
      shell("m4", "hide mission show --mission mission-3"),
      said("그대로예요"),
    ],
    parts: {
      m1: { kind: "mission", mission: "mission-3" },
      l1: { kind: "letter", to: "agent-7", body: "첫 편지" },
      m2: { kind: "mission", mission: "mission-3" },
      l2: { kind: "letter", to: "agent-7", body: "둘째 편지" },
      m3: { kind: "mission", mission: "mission-3" },
      m4: { kind: "mission", mission: "mission-3" },
    },
    sessions: {},
  });
  const parts = (turn: number) => within(turnsIn(container)[turn]!, "[data-desk-hide-part]").map((part) => part.getAttribute("data-desk-hide-part"));
  const first = () => turnsIn(container)[0]!;
  // Three mission calls and two letters to one session are one chip and one letter line, where the turn first reached them.
  expect(parts(0)).toEqual(["mission", "letter"]);
  expect(within(first(), '[data-desk-hide-part="letter"]')[0]?.textContent).toContain("둘째 편지");
  expect(wordsOf(first())).not.toContain("첫 편지");
  // The chip names the mission once, inside it.
  expect(within(first(), '[data-desk-hide-part="mission"]')[0]?.textContent?.split("Desk 1차").length).toBe(2);
  // Another turn that reaches the same mission draws it again: the rule holds within a turn.
  expect(parts(1)).toEqual(["mission"]);
  // Unfolded, the work draws each target once too.
  await act(async () => within(first(), "[data-desk-turn-toggle]")[0]!.click());
  expect(parts(0)).toEqual(["mission", "letter"]);
});

it("folds a running turn's calls into one line with its newest call under it, never what came out, and unfolds every call", async () => {
  const { container } = await mount();
  const tail = (newest: object[]) => ({
    record: "r1", start: 0, updates: [
      operator("칩을 고쳐줘"),
      said("먼저 칩을 고칠게요."),
      shell("r0", "rg -n mission web/src"),
      { sessionUpdate: "tool_call_update", toolCallId: "r0", content: [{ type: "content", content: { type: "text", text: "web/src/threadView.ts:105: mission" } }] },
      shell("m1", "hide mission show --mission mission-3"),
      edit("e1", "web/src/desk.ts", "one\ntwo", "one\n2\nthree"),
      ...newest,
    ],
    parts: { m1: { kind: "mission", mission: "mission-3" } },
    sessions: {},
  }) as ThreadPage;
  await shown(tail([edit("e2", "web/src/thread-view.tsx", "a\nb", "a2\nb2", "in_progress")]));
  const turn = turnsIn(container)[0]!;
  const toggle = within(turn, "[data-desk-turn-toggle]")[0]!;
  expect(toggle.getAttribute("data-desk-turn-toggle")).toBe("folded");
  // One command is a command, in the singular; two files are files.
  expect(toggle.textContent).toBe("Working · 1 command · 2 files");
  // What it said and the mission it reached stay in view; its calls and their output do not.
  expect(wordsOf(turn)).toContain("먼저 칩을 고칠게요.");
  expect(within(turn, '[data-desk-hide-part="mission"]')).toHaveLength(1);
  expect(within(turn, '[data-desk-thread-line="tool"]')).toHaveLength(0);
  expect(wordsOf(turn)).not.toContain("threadView.ts:105");
  // The newest call is its file and what it changed.
  expect(within(turn, "[data-desk-thread-newest]")[0]?.textContent).toMatch(/^Edit web\/src\/thread-view\.tsx\+2 −2/);
  // A command that comes next takes its place, as its first line alone.
  await shown(tail([edit("e2", "web/src/thread-view.tsx", "a\nb", "a2\nb2"), shell("r1", "pnpm --dir web test\n  --run desk", "in_progress")]));
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toMatch(/^pnpm --dir web testRunning/);
  // An agent whose record carries no change shows the file alone.
  await shown(tail([{ sessionUpdate: "tool_call", toolCallId: "e3", kind: "edit", title: "Edit web/src/app.ts", status: "completed" }]));
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe("Edit web/src/app.ts");
  // Unfolded, every call stands as before, with what came out.
  await act(async () => (container.querySelector("[data-desk-turn-toggle]") as HTMLElement).click());
  expect(container.querySelectorAll('[data-desk-thread-line="tool"]')).toHaveLength(3);
  expect(container.textContent).toContain("threadView.ts:105");
  expect(container.querySelector("[data-desk-thread-newest]")).toBeNull();
});

it("tells parts apart by what they lead to, not by how they are drawn or typed", async () => {
  const { container } = await mount({ ...job, state: "done" });
  await shown({
    record: "r1", start: 0, updates: [
      operator("둘 다 리뷰 맡기고 노트 열어줘"),
      shell("s1", "hide agent spawn --name review"),
      shell("s2", "hide agent spawn --name review"),
      shell("v1", "hide file open notes.md"),
      shell("v2", "hide file open ./notes.md"),
      said("맡겼어요"),
    ],
    parts: {
      s1: { kind: "spawn", name: "review", agent_id: "agent-11", intent: null },
      s2: { kind: "spawn", name: "review", agent_id: "agent-12", intent: null },
      v1: { kind: "view", view: "file", target: "notes.md", view_id: "v1" },
      v2: { kind: "view", view: "file", target: "./notes.md", view_id: "v1" },
    },
    sessions: {},
  });
  // Two sessions that ended under one name are two sessions; one View opened under two spellings is one output.
  expect(container.querySelectorAll('[data-desk-hide-part="spawn"]')).toHaveLength(2);
  expect(container.querySelectorAll("[data-desk-output]")).toHaveLength(1);
});

it("names a running turn's newest call whatever it was: a Hide call by its command, a search by its term", async () => {
  const { container } = await mount();
  const running = (calls: object[], parts: ThreadPage["parts"] = {}) => shown({ record: "r1", start: 0, updates: [operator("찾아줘"), ...calls], parts, sessions: {} } as ThreadPage);
  // A call drawn as a Hide part is still the newest step, as the command it was.
  await running([shell("r", "rg old-step"), shell("m", "hide mission show --mission mission-3")], { m: { kind: "mission", mission: "mission-3" } });
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe("hide mission show --mission mission-3");
  // The same where a reader keeps the command under another input word or none, titling the call by it (Codex, Antigravity).
  const hide = "hide mission show --mission mission-3";
  for (const shape of [{ rawInput: { cmd: hide } }, { rawInput: { CommandLine: hide } }, {}]) {
    await running([shell("r", "rg old-step"), { sessionUpdate: "tool_call", toolCallId: "m", kind: "execute", title: hide, status: "completed", ...shape }], { m: { kind: "mission", mission: "mission-3" } });
    expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe(hide);
  }
  // A search whose reader titles it by the tool alone keeps its term from the input (Pi, OpenCode).
  await running([{ sessionUpdate: "tool_call", toolCallId: "g", kind: "search", title: "grep", status: "completed", rawInput: { pattern: "needle", path: "src" } }]);
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe("grep needle");
  // A read that names its file only in its place shows the file.
  await running([{ sessionUpdate: "tool_call", toolCallId: "rd", kind: "read", title: "read", status: "completed", locations: [{ path: "src/app.ts" }] }]);
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe("read src/app.ts");
  // A title that already says it is not repeated.
  await running([{ sessionUpdate: "tool_call", toolCallId: "cg", kind: "search", title: "Grep needle", status: "completed", rawInput: { pattern: "needle" } }]);
  expect(container.querySelector("[data-desk-thread-newest]")?.textContent).toBe("Grep needle");
});

it("folds a finished turn under 작업 · 파일 n · 명령 m with no newest call", async () => {
  const { container } = await mount({ ...job, state: "done" });
  await shown({ record: "r1", start: 0, updates: [operator("고쳐줘"), shell("r0", "rg -n mission"), edit("e1", "web/src/desk.ts", "a", "b"), said("고쳤어요")], parts: {}, sessions: {} });
  expect(container.querySelector("[data-desk-turn-toggle]")?.textContent).toBe("Work · 1 file · 1 command");
  expect(container.querySelector("[data-desk-thread-newest]")).toBeNull();
  expect(container.querySelectorAll('[data-desk-thread-line="tool"]')).toHaveLength(0);
});

/** Waits until the lazily loaded Markdown language parsed the messages, so a code block carries its copy button. */
const codeCopyDrawn = (container: HTMLElement) => act(() => vi.waitFor(() => expect(container.querySelector("[data-code-block-action]")).not.toBeNull()));

it("draws the agent's words as Markdown, copies a code block's code, and offers no copy under a message for now (operator decision 2026-11-27)", async () => {
  const copied: [string, string][] = [];
  const { container } = await mount(job, null, { onCopy: async (text, what) => { copied.push([text, what]); return true; } });
  const words = "**라이트**는 끝났어요.\n\n```\nopen https://play.google.com/console\n```";
  const tail = { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said(words)], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await codeCopyDrawn(container);
  const message = container.querySelector('[data-desk-thread-line="agent"]')!;
  expect(message.querySelector('[data-markdown-text="true"]')).not.toBeNull();
  // Neither the answer nor the operator's bubble carries a copy row.
  expect(container.querySelectorAll('[data-copy-words="message"]')).toHaveLength(0);
  await act(async () => message.querySelector<HTMLElement>('[data-code-block-action] [data-copy-words="code block"]')!.click());
  expect(copied).toEqual([["open https://play.google.com/console", "code block"]]);
  // A copy that took says so on the button that made it.
  expect(message.querySelector('[data-copy-words="code block"]')?.getAttribute("data-copied")).toBe("true");
});

it("keeps every message's view while the operator types and while the last message streams, so a selection in one stays", async () => {
  // Mounted as Desk mounts it: no mentions passed.
  const { container, type } = await mount();
  const answer = (id: string, text: string) => ({ sessionUpdate: "agent_message_chunk", messageId: id, content: { type: "text", text } });
  const turns = (last: string) => ({ record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "첫 질문" } }, answer("a", "첫 **답**"), { sessionUpdate: "user_message_chunk", content: { type: "text", text: "둘째 질문" } }, answer("b", last)], parts: {}, sessions: {} });
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

const answer = (text: string, at: number) => ({ sessionUpdate: "agent_message_chunk", messageId: text, content: { type: "text", text }, _meta: { hide: { at_ms: at } } });
const asked = (...updates: ThreadPage["updates"]) => ({ record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "스크린샷 찍어 줘" } }, ...updates], parts: {}, sessions: {} });

it("draws only the rows near the viewport, one long turn's messages each a row of its own, and draws a row again on return", async () => {
  layOut({ viewport: 400, row: 100 });
  const { container, type } = await mount();
  // One running turn whose work holds forty messages: it must not stay drawn whole.
  const updates = Array.from({ length: 40 }, (_, index) => [
    answer(`Past **answer ${index}**`, index + 1),
    { sessionUpdate: "tool_call", toolCallId: `call-${index}`, kind: "execute", title: "echo work", status: "completed" },
  ]).flat();
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: asked(...updates, answer("Newest **answer**", 100)) }), sessionThreadHistory: { generation: 4, pages: [] } }));
  // The rows it drew are measured taller than their estimates, and the conversation follows its foot as they grow.
  await laidOut();
  await laidOut();
  expect(container.querySelector("[data-desk-thread-lines]")?.getAttribute("data-desk-thread-lines")).toBe("82");
  // It opens at its foot: the newest answer is drawn, the first is not, and no more rows than the viewport and a few past it.
  const bounded = () => expect(drawnRows(container).length).toBeLessThanOrEqual(4 + 2 * 6 + 2);
  bounded();
  expect(drawnWords(container).some((words) => words.includes("Newest answer"))).toBe(true);
  expect(drawnWords(container).some((words) => words.includes("answer 0"))).toBe(false);
  await type("keep my draft");
  await scrollConversation(container, 0);
  bounded();
  expect(drawnWords(container).some((words) => words.includes("answer 0"))).toBe(true);
  expect(drawnWords(container).some((words) => words.includes("Newest answer"))).toBe(false);
  expect(container.querySelector("textarea")!.value).toBe("keep my draft");
  // Back at the foot, the newest answer is drawn again as Markdown.
  await scrollConversation(container, "end");
  const newest = drawnRows(container).find((row) => row.textContent?.includes("Newest answer"));
  expect(newest?.querySelector(".cm-editor")).not.toBeNull();
  expect(container.querySelector("textarea")!.value).toBe("keep my draft");
});

it("keeps a turn's unfolded work while its rows leave the viewport and come back", async () => {
  layOut({ viewport: 400, row: 100 });
  const { container } = await mount({ ...job, state: "done" });
  const later = Array.from({ length: 20 }, (_, index) => [operator(`later ${index}`), said(`later answer ${index}`)]).flat();
  await shown({ record: "r1", start: 0, updates: [operator("고쳐줘"), shell("r0", "rg -n first-turn"), said("고쳤어요"), ...later], parts: {}, sessions: {} });
  await scrollConversation(container, 0);
  await act(async () => container.querySelector<HTMLElement>("[data-desk-turn-toggle]")!.click());
  expect(container.querySelector("[data-desk-turn-toggle]")?.getAttribute("data-desk-turn-toggle")).toBe("open");
  expect(drawnWords(container).some((words) => words.includes("rg -n first-turn"))).toBe(true);
  await scrollConversation(container, "end");
  expect(drawnWords(container).some((words) => words.includes("first-turn") || words.includes("고쳐줘"))).toBe(false);
  await scrollConversation(container, 0);
  expect(container.querySelector("[data-desk-turn-toggle]")?.getAttribute("data-desk-turn-toggle")).toBe("open");
  expect(drawnWords(container).some((words) => words.includes("rg -n first-turn"))).toBe(true);
});

it("forgets the fold of a turn whose page was let go, so the same turn read again starts folded", async () => {
  const { container } = await mount({ ...job, state: "done" });
  const older = { record: "r1", start: 0, updates: [operator("고쳐줘"), shell("r0", "rg -n first-turn"), said("고쳤어요")], parts: {}, sessions: {} };
  const tail = { record: "r1", start: 500, updates: [operator("다음"), said("했어요")], parts: {}, sessions: {} };
  const read = (pages: ThreadPage[]) => act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages } }));
  await read([older]);
  await act(async () => container.querySelector<HTMLElement>("[data-desk-turn-toggle]")!.click());
  expect(container.querySelector("[data-desk-turn-toggle]")?.getAttribute("data-desk-turn-toggle")).toBe("open");
  // The core let the older page go, then read it again.
  await read([]);
  await read([older]);
  expect(container.querySelector("[data-desk-turn-toggle]")?.getAttribute("data-desk-turn-toggle")).toBe("folded");
});

it("starts a record replaced at the same path with fresh rows, none of the old record's folds on them", async () => {
  const { container } = await mount({ ...job, state: "done" });
  const page = (record: string, task: string, command: string) => ({ record, start: 0, updates: [operator(task), shell("c1", command), said(`${task} 했어요`)], parts: {}, sessions: {} });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: page("r1", "고쳐줘", "rg -n old-record") }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await act(async () => container.querySelector<HTMLElement>("[data-desk-turn-toggle]")!.click());
  expect(drawnWords(container).some((words) => words.includes("rg -n old-record"))).toBe(true);
  // The agent's record was replaced: the core reads the new file from its start, in the same thread, with no empty frame between.
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: page("r2", "다른 일", "rg -n new-record") }) }));
  expect(container.querySelector("[data-desk-turn-toggle]")?.getAttribute("data-desk-turn-toggle")).toBe("folded");
  expect(drawnWords(container).some((words) => words.includes("rg -n new-record"))).toBe(false);
});

it("reads a page in above without moving the row the operator reads", async () => {
  layOut({ viewport: 400, row: 40 });
  const { container } = await mount({ ...job, state: "done" });
  const pageOf = (from: number) => ({ record: "r1", start: from * 100, updates: Array.from({ length: 20 }, (_, index) => [operator(`question ${from + index}`), said(`answer ${from + index}`)]).flat(), parts: {}, sessions: {} });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: pageOf(20), has_older: true }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await scrollConversation(container, 600);
  const box = container.querySelector<HTMLElement>("[data-desk-thread-conversation]")!;
  // The row at the viewport's top edge, and how far below the edge it stands.
  const reading = () => {
    const row = drawnRows(container).find((one) => Number.parseFloat(one.style.top) + 40 > box.scrollTop)!;
    return { key: row.dataset.threadRow, words: row.textContent, offset: Number.parseFloat(row.style.top) - box.scrollTop };
  };
  const before = reading();
  await act(async () => useShellStore.setState({ sessionThreadHistory: { generation: 4, pages: [pageOf(0)] } }));
  expect(container.querySelector("[data-desk-thread-lines]")?.getAttribute("data-desk-thread-lines")).toBe("80");
  expect(reading()).toEqual(before);
});

it("keeps the row being read in place when a row above it grows, while the reader scrolls up", async () => {
  const heights = new Map<string, number>();
  layOut({ viewport: 400, row: (row) => heights.get(row.dataset.threadRow!) ?? 40 });
  const { container } = await mount({ ...job, state: "done" });
  await shown({ record: "r1", start: 0, updates: Array.from({ length: 30 }, (_, index) => [operator(`question ${index}`), said(`answer ${index}`)]).flat(), parts: {}, sessions: {} });
  await laidOut();
  const box = container.querySelector<HTMLElement>("[data-desk-thread-conversation]")!;
  const heightOf = (row: HTMLElement) => heights.get(row.dataset.threadRow!) ?? 40;
  // The row at the viewport's top edge, and how far below the edge it stands.
  const reading = () => {
    const row = drawnRows(container).find((one) => Number.parseFloat(one.style.top) + heightOf(one) > box.scrollTop)!;
    return { key: row.dataset.threadRow, offset: Number.parseFloat(row.style.top) - box.scrollTop };
  };
  await scrollConversation(container, 1400);
  await laidOut();
  await scrollConversation(container, 1200);
  await laidOut();
  // Every row drawn is measured. The reader scrolls up a little, and a row above what they read grows, as a picture
  // that arrives late makes it; nothing else changes height.
  await scrollConversation(container, box.scrollTop - 20);
  const before = reading();
  const above = drawnRows(container).find((one) => Number.parseFloat(one.style.top) + heightOf(one) < box.scrollTop)!;
  heights.set(above.dataset.threadRow!, 140);
  await laidOut();
  expect(reading()).toEqual(before);
});

it("draws the message a question goes to wherever it stands, keeps it drawn while lit, and lets it go after", async () => {
  layOut({ viewport: 400, row: 100 });
  const scrolled: string[] = [];
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.textContent ?? ""); };
  const { container } = await mount({ ...job, state: "done" }, null, { asks: [{ id: "far-0", text: "어느 화면을 찍을지", at_unix_ms: 1_500, quote: "어느 화면을 찍을까요?" }] });
  const later = Array.from({ length: 30 }, (_, index) => [operator(`later ${index}`), answer(`later answer ${index}`, 2_000 + index)]).flat();
  await shown({ record: "r1", start: 0, updates: [operator("찍어줘"), answer("어느 화면을 찍을까요? 라이트와 다크가 있어요.", 1_000), ...later], parts: {}, sessions: {} });
  const asking = () => drawnWords(container).some((words) => words.includes("어느 화면을 찍을까요?"));
  expect(asking()).toBe(false);
  vi.useFakeTimers();
  try {
    await act(async () => container.querySelector<HTMLElement>('[data-desk-thread-ask="far-0"]')!.click());
    expect(asking()).toBe(true);
    expect(scrolled.some((text) => text.includes("어느 화면을 찍을까요?"))).toBe(true);
    await act(async () => vi.advanceTimersByTime(2_000));
    // Lit no more and still away from the viewport, the message is let go.
    expect(asking()).toBe(false);
  } finally {
    vi.useRealTimers();
  }
});

it("keeps every row between a selection's ends drawn while it stands, so a copy holds all of it", async () => {
  layOut({ viewport: 400, row: 100 });
  const { container } = await mount({ ...job, state: "done" });
  await shown({ record: "r1", start: 0, updates: Array.from({ length: 30 }, (_, index) => [operator(`question ${index}`), said(`answer ${index}`)]).flat(), parts: {}, sessions: {} });
  await scrollConversation(container, 0);
  const start = drawnRows(container).find((row) => row.textContent?.includes("answer 1"))!;
  const select = (to: Node | null) => act(async () => {
    const selection = document.getSelection()!;
    if (to) selection.setBaseAndExtent(start, 0, to, 0);
    else selection.removeAllRanges();
    document.dispatchEvent(new Event("selectionchange"));
  });
  await select(start.nextElementSibling);
  // The drag goes on to the foot: the browser scrolls, and the rows between both ends stay drawn.
  await scrollConversation(container, "end");
  const last = drawnRows(container).at(-1)!;
  await select(last);
  const words = drawnWords(container);
  for (let index = 1; index < 30; index += 1) expect(words.some((one) => one.includes(`answer ${index}`)), `answer ${index}`).toBe(true);
  expect(document.getSelection()!.toString()).toContain("answer 1");
  await select(null);
  expect(drawnRows(container).length).toBeLessThanOrEqual(4 + 2 * 6);
});

it("leaves a press on a message's words to the browser, so a drag can select across messages, and a code block's 복사 still takes its press", async () => {
  const { container } = await mount({ ...job, state: "done" });
  await shown({ record: "r1", start: 0, updates: [operator("보여줘"), said("첫 답이에요"), operator("코드도"), said("이 코드예요\n\n```\nconst a = 1;\n```")], parts: {}, sessions: {} });
  await codeCopyDrawn(container);
  const reached: EventTarget[] = [];
  const seen = (event: Event) => reached.push(event.target!);
  document.addEventListener("mousedown", seen);
  // A message that takes a press finds its place by the words' boxes, which a test window does not lay out.
  const laidOut = { getClientRects: () => [] as unknown as DOMRectList, getBoundingClientRect: () => new DOMRect() };
  for (const [name, value] of Object.entries(laidOut)) Object.defineProperty(Range.prototype, name, { configurable: true, writable: true, value });
  try {
    const press = (target: Element, modifier = false) => {
      const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, metaKey: modifier, ctrlKey: modifier });
      target.dispatchEvent(event);
      return event;
    };
    // The words: no message takes the press, so the browser's own selection starts and can run on into the next message.
    const words = container.querySelector('[data-desk-thread-line="agent"] .cm-line')!;
    const event = press(words);
    expect(event.defaultPrevented).toBe(false);
    expect(reached).not.toContain(words);
    // A ⌘- or Ctrl-press is still the message's, which opens a link under it.
    press(words, true);
    expect(reached).toContain(words);
    // A code block's 복사 is pressed as before, and its head is no part of a copied selection.
    const copy = container.querySelector('[data-code-block-action] [data-copy-words="code block"]')!;
    press(copy);
    expect(reached).toContain(copy);
    expect(container.querySelector("[data-code-block-head]")?.className).toContain("select-none");
  } finally {
    document.removeEventListener("mousedown", seen);
    for (const name of Object.keys(laidOut)) delete (Range.prototype as unknown as Record<string, unknown>)[name];
  }
});

it("shows a question in the agent's own sentence above the box and lights only that sentence in its message for 2 s (Doop N3 가)", async () => {
  const scrolled: string[] = [];
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.textContent ?? ""); };
  const { container } = await mount(job, null, { asks: [{ id: "missed-1-0", text: "다크 화면을 넣을지", at_unix_ms: 1_500, quote: "다크 화면도 넣을까요?" }] });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: asked(answer("라이트를 찍었어요. 다크 화면도 넣을까요? 넣으면 두 배예요.", 1_000)) }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const line = container.querySelector("[data-desk-thread-asks]")!;
  const row = line.querySelector('[data-desk-thread-ask-row="missed-1-0"]')!;
  // One row above the box: 확인할 것, the agent's sentence, when it was asked and ×; no message unfolds under it.
  expect(row.querySelector("[data-desk-thread-ask-lead]")?.textContent).toBe("To check");
  expect(row.querySelector('[data-desk-thread-ask-words="missed-1-0"]')?.textContent).toBe("다크 화면도 넣을까요?");
  expect(row.querySelector("[data-desk-thread-ask-ago]")?.textContent).toMatch(/\S ago$/);
  expect(row.querySelector('[data-desk-thread-ask-dismiss="missed-1-0"]')).not.toBeNull();
  expect(line.querySelector("[data-desk-thread-ask-body]")).toBeNull();
  expect(line.compareDocumentPosition(container.querySelector("textarea")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  // The question colour is only on 확인할 것: the box is the neutral ground inside a hairline.
  expect(row.querySelector("[data-desk-thread-ask-lead]")?.className).toContain("text-open-ask");
  expect([/warning|open-ask/.test(line.className), line.className.includes("border-hairline"), line.className.includes("bg-code-block")]).toEqual([false, true, true]);

  vi.useFakeTimers();
  try {
    await act(async () => row.querySelector<HTMLElement>('[data-desk-thread-ask="missed-1-0"]')!.click());
    const lit = () => Array.from(container.querySelectorAll("[data-desk-thread-message] [data-markdown-lit]")).map((one) => one.textContent);
    expect(lit()).toEqual(["다크 화면도 넣을까요?"]);
    // The lit sentence is what is brought to the middle of the conversation.
    expect(scrolled.at(-1)).toBe("다크 화면도 넣을까요?");
    expect(row.getAttribute("data-lit")).toBe("true");
    await act(async () => vi.advanceTimersByTime(1_999));
    expect(lit()).toHaveLength(1);
    await act(async () => vi.advanceTimersByTime(1));
    expect(lit()).toEqual([]);
    expect(row.getAttribute("data-lit")).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

it("goes to the answer of a question kept without its sentence, in Hide AI's words, lighting nothing", async () => {
  const scrolled: string[] = [];
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.textContent ?? ""); };
  const { container } = await mount(job, null, { asks: [{ id: "missed-1-0", text: "다크 화면을 넣을지", at_unix_ms: 1_500 }] });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: asked(answer("라이트를 찍었어요. 다크도 넣을까요?", 1_000)) }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const row = container.querySelector('[data-desk-thread-ask-row="missed-1-0"]')!;
  expect(row.querySelector("[data-desk-thread-ask-words]")?.textContent).toBe("다크 화면을 넣을지");
  await act(async () => row.querySelector<HTMLElement>('[data-desk-thread-ask="missed-1-0"]')!.click());
  expect(scrolled.at(-1)).toContain("라이트를 찍었어요.");
  expect(container.querySelector("[data-markdown-lit]")).toBeNull();
});

it("reads the pages before for a question asked further up, and says so when the conversation no longer holds it", async () => {
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = () => undefined;
  const { actions, container } = await mount(job, null, { asks: [{ id: "missed-1-0", text: "배포할지", at_unix_ms: 500, quote: "지금 배포할까요?" }] });
  await act(async () => useShellStore.setState({ sessionThread: section({ has_older: true, tail: asked(answer("새로 시작했어요.", 1_000)) }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await act(async () => container.querySelector<HTMLElement>('[data-desk-thread-ask="missed-1-0"]')!.click());
  expect(actions.readOlderThread).toHaveBeenCalledWith(4);
  expect(container.querySelector("[data-desk-thread-ask-gone]")).toBeNull();
  // Every page read and still not there: one quiet line says so, and the question stays.
  await act(async () => useShellStore.setState({ sessionThread: section({ has_older: false, tail: asked(answer("새로 시작했어요.", 1_000)) }) }));
  expect(container.querySelector('[data-desk-thread-ask-gone="missed-1-0"]')?.textContent).toBe("Not in the conversation");
  expect(container.querySelector('[data-desk-thread-ask-row="missed-1-0"]')).not.toBeNull();
  // A gone row has nothing to press, and reads no more pages.
  expect(container.querySelector('[data-desk-thread-ask="missed-1-0"]')).toBeNull();
  expect(actions.readOlderThread).toHaveBeenCalledTimes(1);
});

it("draws a question whose message is gone dim, whole and inert, with a hollow dot, and back as N3 가 once its message is found (Doop N3b 가)", async () => {
  vi.stubGlobal("CSS", { escape: (value: string) => value });
  Element.prototype.scrollIntoView = () => undefined;
  const long = `${"이 화면을 어떻게 나눌지 정해야 해요. ".repeat(12)}어느 쪽으로 할까요?`;
  const asks: AskLine[] = [
    { id: "missed-1-0", text: "화면을 나눌지", at_unix_ms: 500, quote: long },
    { id: "missed-2-0", text: "다크도 찍을지", at_unix_ms: 1_500, quote: "다크도 찍을까요?" },
  ];
  const { container } = await mount(job, null, { asks });
  const thread = (...lines: ThreadPage["updates"]) => useShellStore.setState({ sessionThread: section({ has_older: false, tail: asked(...lines, answer("라이트를 찍었어요. 다크도 찍을까요?", 1_000)) }), sessionThreadHistory: { generation: 4, pages: [] } });
  // No answer written by the time the first was found: the record holds no message of it.
  await act(async () => thread());
  // Folded, the line takes the first question's look: dim.
  const folded = container.querySelector<HTMLElement>("[data-desk-thread-asks-more]")!;
  expect(folded.querySelector("[data-desk-thread-ask-lead]")?.className).toContain("text-open-ask");
  expect(folded.textContent).toContain("이 화면을");
  expect(folded.querySelector("span.truncate")?.className).toContain("text-muted-foreground");
  await act(async () => folded.click());
  const look = (id: string) => {
    const row = container.querySelector(`[data-desk-thread-ask-row="${id}"]`)!;
    const words = row.querySelector<HTMLElement>("[data-desk-thread-ask-words]")!;
    return {
      dot: row.querySelector("[data-desk-thread-ask-dot]")?.getAttribute("data-desk-thread-ask-dot"),
      dim: words.className.includes("text-muted-foreground"),
      clamped: words.className.includes("line-clamp-2"),
      said: words.textContent,
      tail: row.querySelector("[data-desk-thread-ask-gone]")?.textContent ?? null,
      pressable: row.querySelector("[data-desk-thread-ask]") !== null,
      dismissable: row.querySelector("[data-desk-thread-ask-dismiss]") !== null,
    };
  };
  expect(look("missed-1-0")).toEqual({ dot: "ring", dim: true, clamped: false, said: `${Array.from(long).slice(0, 200).join("")}…`, tail: "Not in the conversation", pressable: false, dismissable: true });
  expect(look("missed-2-0")).toEqual({ dot: "true", dim: false, clamped: true, said: "다크도 찍을까요?", tail: null, pressable: true, dismissable: true });
  // Read again with the message in it: the row is N3 가 at once.
  await act(async () => thread(answer(long, 400)));
  expect(look("missed-1-0")).toEqual({ dot: "true", dim: false, clamped: true, said: long, tail: null, pressable: true, dismissable: true });
});

it("folds several questions to 확인할 것 N and the first, unfolds them under 확인할 것 N alone, oldest first, and keeps 무시하기 struck through with 되돌리기 (Doop N3 가)", async () => {
  const set: [string, string][] = [];
  const asks: AskLine[] = [
    { id: "missed-2-0", text: "본인 확인을 직접 해 줄 수 있는지", at_unix_ms: 2_500, quote: "본인 확인을 직접 해 줄 수 있어?" },
    { id: "missed-1-0", text: "다크도 찍을지", at_unix_ms: 1_500, quote: "다크도 찍을까요?" },
  ];
  const { container } = await mount(job, null, { asks, onAsk: (id, status) => set.push([id, status]) });
  await act(async () => useShellStore.setState({ sessionThread: section({ tail: asked(answer("라이트를 찍었어요. 다크도 찍을까요?", 1_000), answer("본인 확인을 직접 해 줄 수 있어?", 2_000)) }), sessionThreadHistory: { generation: 4, pages: [] } }));
  const line = () => container.querySelector("[data-desk-thread-asks]")!;
  const rows = () => Array.from(line().querySelectorAll("[data-desk-thread-ask-row]")).map((row) => row.getAttribute("data-desk-thread-ask-row"));
  const more = line().querySelector<HTMLElement>("[data-desk-thread-asks-more]")!;
  expect(more.textContent).toBe("To check 2 ·다크도 찍을까요?");
  expect(rows()).toEqual([]);

  await act(async () => more.click());
  expect(line().getAttribute("data-desk-thread-asks-open")).toBe("true");
  // The head says how many wait and folds; the rows follow, oldest first, each with its dot, its time and ×.
  const head = () => line().querySelector("[data-desk-thread-ask-lead]")?.textContent;
  expect(head()).toBe("To check 2");
  expect(line().querySelector("[data-desk-thread-asks-fold]")?.textContent).toBe("Collapse");
  expect(rows()).toEqual(["missed-1-0", "missed-2-0"]);
  expect(line().querySelectorAll("[data-desk-thread-ask-dot]")).toHaveLength(2);
  expect(line().querySelectorAll("[data-desk-thread-ask-ago]")).toHaveLength(2);

  // × is 무시하기: the core is told, and the row stays struck through with 되돌리기 until it is brought back.
  await act(async () => line().querySelector<HTMLElement>('[data-desk-thread-ask-dismiss="missed-2-0"]')!.click());
  expect(set).toEqual([["missed-2-0", "dismissed"]]);
  const away = line().querySelector('[data-desk-thread-ask-row="missed-2-0"]')!;
  expect(away.getAttribute("data-dismissed")).toBe("true");
  expect(head()).toBe("To check 1");
  await act(async () => away.querySelector<HTMLElement>('[data-desk-thread-ask-undo="missed-2-0"]')!.click());
  expect(set).toEqual([["missed-2-0", "dismissed"], ["missed-2-0", "open"]]);
  expect(head()).toBe("To check 2");
  await act(async () => line().querySelector<HTMLElement>("[data-desk-thread-asks-fold]")!.click());
  expect(line().getAttribute("data-desk-thread-asks-open")).toBe("false");
});

it("folds the unfolded question list on Esc before Esc reaches the panel it stands in", async () => {
  const asks: AskLine[] = [
    { id: "missed-1-0", text: "다크도 찍을지", at_unix_ms: 1_500 },
    { id: "missed-2-0", text: "본인 확인을 직접 해 줄 수 있는지", at_unix_ms: 2_500 },
  ];
  const panel = vi.fn();
  const pop = useUiStore.getState().pushEscape(panel);
  try {
    const { container } = await mount(job, null, { asks });
    await act(async () => useShellStore.setState({ sessionThread: section(), sessionThreadHistory: { generation: 4, pages: [] } }));
    const line = () => container.querySelector("[data-desk-thread-asks]")!;
    await act(async () => line().querySelector<HTMLElement>("[data-desk-thread-asks-more]")!.click());
    expect(line().getAttribute("data-desk-thread-asks-open")).toBe("true");
    // Esc runs the innermost layer, as keyboard.ts does.
    await act(async () => useUiStore.getState().escapeLayers.at(-1)!());
    expect(line().getAttribute("data-desk-thread-asks-open")).toBe("false");
    expect(panel).not.toHaveBeenCalled();
    // Folded, the next Esc is the panel's again.
    expect(useUiStore.getState().escapeLayers.at(-1)).toBe(panel);
  } finally {
    pop();
  }
});

it("draws a code block under a head row with its language and 복사, the code below it, a block in a quote too (Doop N 1 나)", async () => {
  const copied: string[] = [];
  const { container } = await mount(job, null, { onCopy: async (text) => { copied.push(text); return true; } });
  const words = "한 줄:\n\n```\nopen https://play.google.com/console\n```\n\n여러 줄:\n\n```bash\npnpm --dir web install\npnpm --dir web build\n```\n\n> 인용 안:\n> ```sh\n> echo hi\n> ```";
  const tail = { record: "r1", start: 0, updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "알려 줘" } }, said(words)], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await act(() => vi.waitFor(() => expect(container.querySelectorAll("[data-code-block-head]")).toHaveLength(3)));
  const [one, several] = Array.from(container.querySelectorAll<HTMLElement>("[data-code-block]"));
  // The head names the fence's language, or says it is code when it names none.
  expect([one, several].map((block) => block!.querySelector("[data-code-block-language]")?.textContent)).toEqual(["Code", "bash"]);
  // The code is the block's own text, without its fences, and 복사 stands in the head, never in the code.
  expect(one!.querySelector("pre")?.textContent).toBe("open https://play.google.com/console");
  expect(several!.querySelector("pre")?.textContent).toBe("pnpm --dir web install\npnpm --dir web build");
  for (const block of [one!, several!]) {
    const button = block.querySelector('[data-code-block-head] [data-copy-words="code block"]');
    expect(button?.textContent).toBe("Copy");
    expect(block.querySelector("pre")?.contains(button!)).toBe(false);
  }
  await act(async () => several!.querySelector<HTMLElement>('[data-copy-words="code block"]')!.click());
  // A block opened in a quote keeps its lines in the text and still has its head and 복사, which copies the code without the quote's marks.
  const quoted = container.querySelector<HTMLElement>(".cm-md-code-head-row [data-code-block-head]")!;
  expect(quoted.querySelector("[data-code-block-language]")?.textContent).toBe("sh");
  await act(async () => quoted.querySelector<HTMLElement>('[data-copy-words="code block"]')!.click());
  expect(copied).toEqual(["pnpm --dir web install\npnpm --dir web build", "echo hi"]);
  // 복사 says it copied, in its own words.
  expect(several!.querySelector('[data-copy-words="code block"]')?.textContent).toBe("Copied");
});

// mission-106: the session's box has Desk's `/` and `@` menus and its model and effort under it, its agent fixed.
const claudeRow = { pane_id: "w1:p2", agent_kind: "claude", state: { working: false, asking: false } } as AgentRow;
const projectRest = { navigator: { workspaces: [{ id: "w", device_id: "mac", label: "herdr-ide", path: "/repo", checkouts: [{ id: "c", path: "/repo", tabs: [{ panes: [{ id: "w1:p2" }] }] }] }] } } as unknown as SnapshotRest;
const deskSessions: DeskMentions = {
  sessions: [
    { id: "w1:p9", name: "grab-market 기준선", agent_kind: "claude", mark: "going", detail: null, parent: null, project: "shop", branch: null, last_prompt_at_unix_ms: null },
    { id: "w1:p2", name: "갤러리 캡처 비교", agent_kind: "claude", mark: "going", detail: null, parent: null, project: "herdr-ide", branch: null, last_prompt_at_unix_ms: null },
  ],
  projects: [{ id: "/shop", name: "shop", sessions: ["w1:p9"] }],
};
const key = (box: HTMLTextAreaElement, name: string) => act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true })); });

it("opens the session's agent's own commands on `/`, in Desk's panel, and writes the one picked into the box", async () => {
  const { actions, box, type } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [claudeRow], deskCommands: { lists: { claude: [{ name: "/compact", description: "Free up context" }, { name: "/review", description: "Review the diff" }], codex: [{ name: "/codex-only", description: "" }] }, unread: {} } }));
  await type("/r");
  await type("/re");
  // One check that the list is current per opening of the menu, none per key.
  expect(actions.deskCommandsCheck.mock.calls).toEqual([["claude"]]);
  const rows = Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-command]")).map((row) => row.dataset.deskCommand);
  expect(rows).toEqual(["/review"]);
  await key(box(), "Enter");
  expect(box().value).toBe("/review ");
  // A pick is no send: the words go when the operator sends them, as the agent's own command.
  expect(events).toEqual([]);
});

it("says why the session's agent has no list yet, the same as Desk's box, and sends `/` words as typed", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [claudeRow], deskCommands: { lists: {}, unread: { claude: "reading" } } }));
  await type("/compact");
  expect(document.body.querySelector('[data-desk-command-menu="waiting"]')).not.toBeNull();
  await act(async () => useShellStore.setState({ deskCommands: { lists: {}, unread: { claude: "sign_in" } } }));
  expect(document.body.querySelector('[data-desk-command-unread="sign_in"]')).not.toBeNull();
  await enter();
  expect(events[0]).toMatchObject({ kind: "thread_input", payload: { text: "/compact" } });
  // Sent, the words leave the box at once.
  expect(box().value).toBe("");
});

it("lists the project's files above Desk's sessions on `@`, without the session itself, and writes each as its agent reads it", async () => {
  const { actions, box, type } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [claudeRow], rest: projectRest, deskMentions: deskSessions }));
  vi.useFakeTimers();
  await type("고쳐줘 @gr");
  await act(async () => { vi.advanceTimersByTime(200); });
  vi.useRealTimers();
  expect(actions.requestFileIndex).toHaveBeenCalledWith("/repo", "gr", "mac");
  await act(async () => useShellStore.setState({ fileIndex: { device_id: "mac", root_path: "/repo", query: "gr", files: [{ path: "/repo/docs/grab market.md", relative_path: "docs/grab market.md" }, { path: "/repo/scripts/grader.ts", relative_path: "scripts/grader.ts" }], truncated: false, indexing: false, unavailable: null } }));
  const groups = Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-mention-group]")).map((group) => group.dataset.deskMentionGroup);
  expect(groups).toEqual(["files", "sessions"]);
  expect(Array.from(document.body.querySelectorAll<HTMLElement>("[data-desk-mention-session]")).map((row) => row.dataset.deskMentionSession)).toEqual(["w1:p9"]);
  // No project group: a session's `@` names files and sessions.
  expect(document.body.querySelector('[data-desk-mention-group="projects"]')).toBeNull();
  await key(box(), "Enter");
  expect(box().value).toBe('고쳐줘 @"docs/grab market.md" ');
  await type(`${box().value}@grab`);
  await key(box(), "Enter");
  expect(box().value).toBe('고쳐줘 @"docs/grab market.md" @grab-market 기준선 (pane w1:p9) ');
});

it("shows the session's model and effort with its agent fixed, and a change types the agent's own line through the core", async () => {
  const { actions, container } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section({ choice: { model: "opus" } }), agents: [claudeRow] }));
  const bar = container.querySelector<HTMLElement>('[data-session-agent-bar="claude"]')!;
  expect(bar.querySelector('[data-session-bar="agent"]')?.textContent).toContain("Claude");
  expect(bar.querySelector('[data-session-bar="agent"]')?.tagName).toBe("SPAN");
  expect(bar.querySelector('[data-session-bar="model"]')?.textContent).toContain("opus");
  expect(bar.querySelector('[data-session-bar="effort"]')?.textContent).toContain("Default");
  const effort = bar.querySelector<HTMLButtonElement>('[data-session-bar="effort"]')!;
  await act(async () => {
    effort.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
  });
  const high = document.body.querySelector<HTMLElement>('[data-session-bar-choice="high"]')!;
  await act(async () => high.click());
  expect(actions.switchThread).toHaveBeenCalledWith(4, "w1:p2", { effort: "high" });
});

it("says the reason a change typed nothing and keeps the session's choice", async () => {
  const { actions, container } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section({ choice: { model: "opus" } }), agents: [claudeRow] }));
  for (const [reason, words] of [
    ["agent_asking", "The agent is asking something: answer it first"],
    ["agent_busy", "The agent is working: change it once the turn ends"],
    ["draft", "The terminal's input holds words not sent yet: send or clear them first"],
  ]) {
    actions.switchThread.mockReturnValueOnce(`switch-${reason}`);
    const effort = container.querySelector<HTMLButtonElement>('[data-session-bar="effort"]')!;
    await act(async () => {
      effort.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
    });
    await act(async () => document.body.querySelector<HTMLElement>('[data-session-bar-choice="high"]')!.click());
    await act(async () => completeInput(section({ choice: { model: "opus" }, input: { request_id: `switch-${reason}`, state: "refused", reason } })));
    expect(container.querySelector('[data-desk-composer-notice="true"]')?.textContent).toBe(words);
    expect(container.querySelector('[data-session-bar="model"]')?.textContent).toContain("opus");
  }
  // Each refusal leaves the bar free for the next pick.
  expect(actions.switchThread).toHaveBeenCalledTimes(3);
});

it("holds the bar while the agent's question card is on screen: a change's Enter would answer it", async () => {
  const { actions, container } = await mount(job, null, { ask: <div data-ask-card="true" /> });
  await act(async () => useShellStore.setState({ sessionThread: section({ choice: { model: "opus" } }), agents: [claudeRow] }));
  const effort = container.querySelector<HTMLButtonElement>('[data-session-bar="effort"]')!;
  expect(effort.disabled).toBe(true);
  await act(async () => {
    effort.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
  });
  expect(document.body.querySelector('[data-session-bar-choice="high"]')).toBeNull();
  expect(actions.switchThread).not.toHaveBeenCalled();
});

it("keeps a slot Hide cannot change in a running session read-only, and says so", async () => {
  const { container } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [{ pane_id: "w1:p2", agent_kind: "opencode", state: { working: false, asking: false } } as AgentRow] }));
  const model = container.querySelector<HTMLElement>('[data-session-bar="model"]')!;
  expect(model.dataset.sessionBarFixed).toBe("true");
  expect(model.getAttribute("aria-label")).toContain("cannot change the model here yet");
  expect(container.querySelector('[data-session-bar="effort"]')?.getAttribute("data-session-bar-fixed")).toBe("true");
});

// Operator request 2026-11-27 (mission-109): a session's input keeps what was written in it per pane, as Desk's own
// box keeps its draft, while another session is open beside Desk, and forgets it once the pane took it.
it("keeps a session's words while another session opens beside Desk and gives them back on its return, and forgets them once sent", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await first.type("다크도 부탁해\n라이트도");
  await act(async () => first.root.unmount());
  const other = await mount({ ...job, pane_id: "w1:p3" }, null, { pane: "w1:p3" });
  expect(other.box().value).toBe("");
  await other.type("다른 세션");
  await act(async () => other.root.unmount());
  const back = await mount();
  expect(back.box().value).toBe("다크도 부탁해\n라이트도");
  await back.enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => completeInput(section({ input: { request_id: request, state: "sent" } })));
  expect(back.box().value).toBe("");
  await act(async () => back.root.unmount());
  expect((await mount()).box().value).toBe("");
  expect((await mount({ ...job, pane_id: "w1:p3" }, null, { pane: "w1:p3" })).box().value).toBe("다른 세션");
});

// Operator request 2026-11-27 (mission-109): `답하기` gives the box the keyboard once per press; a count an earlier
// session's press raised is no press for a session opened later.
it("takes the keyboard on a 답하기 pressed while it is open, never on one pressed before it opened", async () => {
  const opened = await mount(job, null, { answer: 3 });
  expect(document.activeElement).not.toBe(opened.box());
  await opened.answerAgain(4);
  expect(document.activeElement).toBe(opened.box());
});

// CR-06: the wire answers each input even when another window's snapshot wins the same burst.
it("ignores display outcomes and settles each direct outcome even when React batches another result", async () => {
  const { box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section() }));
  await type("keep these words");
  await enter();
  const request = events[0]!.payload.request_id as string;
  await act(async () => useShellStore.setState({ sessionThread: section({ input: { request_id: request, state: "sent" } }) }));
  await enter();
  expect(events).toHaveLength(1);
  await act(async () => {
    receiveThreadInputResult({ request_id: request, state: "refused", reason: "in_flight" });
    receiveThreadInputResult({ request_id: "another-window", state: "sent" });
  });
  expect(box().value).toBe("keep these words");
  await enter();
  expect(events).toHaveLength(2);
});

it("keeps new words while reconnecting, before a snapshot, or when the live socket declines them", async () => {
  const panel = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [claudeRow] }));
  await panel.type("not sent yet");
  for (const connection of ["reconnecting", "connecting"] as const) {
    await act(async () => useShellStore.setState({ connection }));
    await panel.enter();
    expect(panel.box().value).toBe("not sent yet");
    expect(useUiStore.getState().sessionDrafts.find((draft) => draft.pane === "w1:p2")?.sending).toBeUndefined();
  }
  expect(events).toEqual([]);
  await act(async () => useShellStore.setState({ connection: "live" }));
  configureAttachments({ dispatch: () => false, sendBinary: () => undefined });
  await panel.enter();
  expect(panel.box().value).toBe("not sent yet");
  expect(useUiStore.getState().sessionDrafts.find((draft) => draft.pane === "w1:p2")?.sending).toBeUndefined();
  configureAttachments({ dispatch: (event) => events.push({ kind: event.kind, payload: event.payload }), sendBinary: () => undefined });
  await panel.enter();
  expect(events).toHaveLength(1);
});

it("keeps an unanswered send uncertain across reconnect and remount until its terminal is visited", async () => {
  const first = await mount();
  await act(async () => useShellStore.setState({ sessionThread: section(), agents: [claudeRow] }));
  await first.type("check the terminal first");
  await first.enter();
  await act(async () => useShellStore.setState({ connection: "reconnecting" }));
  await act(async () => first.root.unmount());
  await act(async () => useShellStore.setState({ connection: "live" }));
  const back = await mount();
  expect(back.box().value).toBe("check the terminal first");
  await back.enter();
  expect(events).toHaveLength(1);
  const terminal = Array.from(back.container.querySelectorAll("button")).find((button) => button.textContent === "Go to terminal")!;
  expect(terminal).toBeDefined();
  await act(async () => terminal.click());
  await back.enter();
  expect(events).toHaveLength(2);
});

// PRD desk-queue: words sent while the agent works wait over the box, one going at each rest; ■ stops the turn and the line.
const workingRow = (asking = false) => ({ pane_id: "w1:p2", agent_kind: "claude", state: { working: !asking, asking } }) as AgentRow;
const line = (id: number, text: string, extra: Partial<NonNullable<SessionThreadSection["queue"]>["lines"][number]> = {}) => ({ id, text, files: [], sending: false, ...extra });

it("clears the box once the core queues the words, and draws the pane's waiting line over it with each line's ✕ (B1, B2, B7)", async () => {
  const { actions, container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ agents: [workingRow()], sessionThread: section() }));
  await type("테스트도 같이 돌려 줘");
  await enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  const queue = { state: "waiting" as const, lines: [line(1, "테스트도 같이 돌려 줘", { files: ["shot.png"] }), line(2, "끝나면 PR 열어 줘", { sending: true })], max: 8, full: false };
  await act(async () => completeInput(section({ input: { request_id: request, state: "queued" }, queue })));
  expect(box().value).toBe("");
  expect(useUiStore.getState().sessionDrafts.find((draft) => draft.pane === "w1:p2")?.sending ?? false).toBe(false);
  expect(container.querySelector("[data-queue-group='waiting']")?.textContent).toContain("Waiting · 2");
  expect(container.querySelector("[data-queue-line='1'] [data-queue-line-files]")?.textContent).toBe("shot.png");
  // A line on its way has no ✕; a waiting one has, and it is the core's one event.
  expect(container.querySelector("[data-queue-remove='2']")).toBeNull();
  await act(async () => container.querySelector<HTMLElement>("[data-queue-remove='1']")!.click());
  expect(actions.inputQueue).toHaveBeenCalledWith("w1:p2", "remove", 1);
  // Folded, the head alone stands.
  await act(async () => container.querySelector<HTMLElement>("[data-queue-fold]")!.click());
  expect(container.querySelector("[data-queue-lines]")).toBeNull();
});

it("puts ■ in send's place while the agent works, keeps ↵ adding to the line, and says why a stop typed nothing (B10, B12)", async () => {
  const { actions, container, type, enter } = await mount();
  await act(async () => useShellStore.setState({ agents: [workingRow()], sessionThread: section() }));
  expect(container.querySelector("[data-desk-composer-send]")).toBeNull();
  await type("하나 더");
  await enter();
  expect(events.filter((event) => event.kind === "thread_input")).toHaveLength(1);
  const sent = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => completeInput(section({ input: { request_id: sent, state: "queued" } })));
  await act(async () => container.querySelector<HTMLElement>("[data-desk-composer-stop]")!.click());
  expect(actions.interruptThread).toHaveBeenCalledWith(4, "w1:p2");
  await act(async () => completeInput(section({ input: { request_id: "stop-1", state: "refused", reason: "agent_asking" } })));
  expect(container.querySelector("[data-desk-composer-notice]")?.textContent).toBe("The agent is asking something: answer it first");
  // An agent asking is answered with the box: send stands again.
  await act(async () => useShellStore.setState({ agents: [workingRow(true)] }));
  expect(container.querySelector("[data-desk-composer-stop]")).toBeNull();
  expect(container.querySelector("[data-desk-composer-send]")).not.toBeNull();
});

// CR-08/F9: a lost stop answer must recover like another accepted command,
// with no automatic replay and without losing the words already in the box.
it.each(["", "keep these unsent words"])("recovers a lost stop completion after reconnect with draft %j", async (draft) => {
  const { actions, container, box, type, enter } = await mount();
  await act(async () => useShellStore.setState({ agents: [workingRow()], sessionThread: section() }));
  await type(draft);
  await act(async () => container.querySelector<HTMLElement>("[data-desk-composer-stop]")!.click());
  await act(async () => useShellStore.setState({ connection: "reconnecting" }));
  await act(async () => useShellStore.setState({ connection: "live", agents: [claudeRow], sessionThread: section({ input: { request_id: "stop-1", state: "sent" } }) }));
  expect(box().value).toBe(draft);
  await enter();
  expect(events).toHaveLength(0);
  expect(actions.interruptThread).toHaveBeenCalledTimes(1);
  const terminal = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "Go to terminal");
  expect(terminal).toBeDefined();
  await act(async () => terminal!.click());
  await type("send after checking the terminal");
  await enter();
  expect(events).toHaveLength(1);
  expect(events[0]).toMatchObject({ kind: "thread_input", payload: { text: "send after checking the terminal" } });
});

it("offers 모두 지우기, 이어 보내기 and each line's 보내기 once paused, 보내기 waiting while the agent works, and 되돌리기 after a clear (B14-B17, B23)", async () => {
  const { actions, container } = await mount();
  const paused = { state: "paused" as const, lines: [line(1, "테스트도 같이 돌려 줘", { failure: "uncertain" as const }), line(2, "끝나면 PR 열어 줘")], max: 8, full: false };
  await act(async () => useShellStore.setState({ agents: [workingRow()], sessionThread: section({ queue: paused }) }));
  expect(container.querySelector("[data-queue-group='paused']")?.textContent).toContain("Paused · 2");
  expect(container.querySelector("[data-queue-line-failure='uncertain']")?.textContent).toBe("Hide could not tell whether it was sent");
  expect(container.querySelector<HTMLButtonElement>("[data-queue-send='1']")!.disabled).toBe(true);
  await act(async () => useShellStore.setState({ agents: [claudeRow] }));
  expect(container.querySelector<HTMLButtonElement>("[data-queue-send='1']")!.disabled).toBe(false);
  await act(async () => container.querySelector<HTMLElement>("[data-queue-send='2']")!.click());
  await act(async () => container.querySelector<HTMLElement>("[data-queue-resume]")!.click());
  await act(async () => container.querySelector<HTMLElement>("[data-queue-clear]")!.click());
  expect(actions.inputQueue.mock.calls).toEqual([["w1:p2", "send", 2], ["w1:p2", "resume", undefined], ["w1:p2", "clear", undefined]]);
  await act(async () => useShellStore.setState({ sessionThread: section({ queue: { state: "waiting", lines: [], max: 8, full: false, cleared: { count: 2, until_unix_ms: Date.now() + 10_000 } } }) }));
  expect(container.querySelector("[data-queue-cleared]")?.textContent).toContain("Cleared 2");
  await act(async () => container.querySelector<HTMLElement>("[data-queue-restore]")!.click());
  expect(actions.inputQueue).toHaveBeenLastCalledWith("w1:p2", "restore", undefined);
  // Its ten seconds over, 되돌리기 goes.
  await act(async () => useShellStore.setState({ sessionThread: section({ queue: { state: "waiting", lines: [], max: 8, full: false, cleared: { count: 2, until_unix_ms: Date.now() - 1 } } }) }));
  expect(container.querySelector("[data-queue-group]")).toBeNull();
});

it("offers 이어 보내기 on a waiting line only once the core says its last words opened no turn, and says why a 되돌리기 brought nothing back", async () => {
  const { actions, container } = await mount();
  const waiting = { state: "waiting" as const, lines: [line(1, "끝나면 PR 열어 줘")], max: 8, full: false };
  await act(async () => useShellStore.setState({ agents: [claudeRow], sessionThread: section({ queue: waiting }) }));
  expect(container.querySelector("[data-queue-group='waiting']")).not.toBeNull();
  expect(container.querySelector("[data-queue-resume]")).toBeNull();
  await act(async () => useShellStore.setState({ sessionThread: section({ queue: { ...waiting, stalled: true } }) }));
  await act(async () => container.querySelector<HTMLElement>("[data-queue-resume]")!.click());
  expect(actions.inputQueue).toHaveBeenLastCalledWith("w1:p2", "resume", undefined);
  const cleared = { count: 2, until_unix_ms: Date.now() + 10_000, refused: "queue_full" as const };
  await act(async () => useShellStore.setState({ sessionThread: section({ queue: { ...waiting, cleared } }) }));
  expect(container.querySelector("[data-queue-restore-refused='queue_full']")?.textContent).toContain("The waiting line is full");
});

it("words a refusal of a full line and of its files under the box (B8, B9)", async () => {
  const { container, type, enter } = await mount();
  await act(async () => useShellStore.setState({ agents: [workingRow()], sessionThread: section() }));
  await type("아홉 번째");
  await enter();
  const request = events.find((event) => event.kind === "thread_input")!.payload.request_id as string;
  await act(async () => completeInput(section({ input: { request_id: request, state: "refused", reason: "queue_full" } })));
  expect(container.querySelector("[data-desk-composer-notice]")?.textContent).toBe("The waiting line is full");
  expect(container.querySelector("textarea")!.value).toBe("아홉 번째");
});
