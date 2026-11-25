// @vitest-environment jsdom
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { configureAttachments } from "../../attachments";
import type { CopyText } from "../human-turn-copy";
import type { AskLine } from "../open-asks";
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
  // The input box holds the corner's notices above it by its measured box (`noticeFloor.ts`).
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
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

async function mount(card: DeskJob | null = job, foot: ReactNode = null, { asks = [], onCopy = () => Promise.resolve(true) }: { asks?: AskLine[]; onCopy?: CopyText } = {}) {
  const actions = { openThread: vi.fn(), closeThread: vi.fn(), readOlderThread: vi.fn() };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  mounted.push(root);
  const noop = () => undefined;
  await act(async () => root.render(
    <TooltipProvider>
      <ThreadPanel job={card} paneId={job.pane_id} actions={actions} foot={foot} asks={asks} onTerminal={noop} onMission={noop} onOutput={noop} onSession={noop} onCopy={onCopy} />
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

/** Lets the lazily loaded Markdown language parse each message, and the parts drawn in roots of their own mount. */
const laidOut = () => act(async () => new Promise((resolve) => setTimeout(resolve, 50)));

it("draws the agent's words as Markdown and copies a message's original words and a code block's code (Doop E2)", async () => {
  const copied: [string, string][] = [];
  const { container } = await mount(job, null, { onCopy: async (text, what) => { copied.push([text, what]); return true; } });
  const words = "**라이트**는 끝났어요.\n\n```\nopen https://play.google.com/console\n```";
  const tail = { updates: [{ sessionUpdate: "user_message_chunk", content: { type: "text", text: "다크도 찍어줘" } }, said(words)], parts: {}, sessions: {} };
  await act(async () => useShellStore.setState({ sessionThread: section({ tail }), sessionThreadHistory: { generation: 4, pages: [] } }));
  await laidOut();
  const message = container.querySelector('[data-desk-thread-line="agent"]')!;
  expect(message.querySelector('[data-markdown-text="true"]')).not.toBeNull();
  await act(async () => message.querySelector<HTMLElement>('[data-message-copy] [data-copy-words="message"]')!.click());
  await act(async () => message.querySelector<HTMLElement>('[data-markdown-code-copy] [data-copy-words="code block"]')!.click());
  expect(copied).toEqual([[words, "message"], ["open https://play.google.com/console", "code block"]]);
  // A copy that took says so on the button that made it.
  expect(message.querySelector('[data-copy-words="message"]')?.getAttribute("data-copied")).toBe("true");
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
  await laidOut();
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
