// @vitest-environment jsdom
// Answering from the phone (docs/UI_BEHAVIOR.md, The phone app; Doop 03-06): a computer whose hello says `answer` takes a reply of
// several lines as long as one phone message holds, and a computer before it keeps the one-line reply of 2,000 characters, so a
// new phone never sends an old computer what it would refuse.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { PhoneAgent, PhoneAsk, ServerFrame } from "./protocol";

vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

// Each test loads the whole phone app afresh, which a loaded machine takes longer than the default 5 s to do.
vi.setConfig({ testTimeout: 30_000 });

class FakeSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static instances: FakeSocket[] = [];
  readyState = 1;
  sent: Record<string, unknown>[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor() {
    FakeSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close() {}
}

function agent(over: Partial<PhoneAgent> = {}): PhoneAgent {
  return { device_id: "local", pane_id: "w1:p1", root_pane_id: "w1:p1", group: "seen", mark: "going", wait: null, descendants_working: 0, agent_kind: "claude", title: "리뷰 반영", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: false, holds_notification: false, ...over };
}

/** A phone paired and connected to a fake hided whose hello carries `hello`, with one agent listed and its detail open. */
async function openDetail(hello: Partial<Extract<ServerFrame, { type: "hello" }>>, listed = agent()) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("WebSocket", FakeSocket);
  FakeSocket.instances = [];
  vi.resetModules();
  const { App } = await import("./App");
  const connection = await import("./connection");
  const { patch } = await import("./store");
  const container = document.createElement("div");
  const root = createRoot(container);
  patch({ screen: "pair", pair: { v: 1, endpoint: "https://mac.example.ts.net", code: "code" } });
  connection.pairNow();
  const socket = FakeSocket.instances[0]!;
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  await act(async () => socket.onopen?.());
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off", ...hello });
  await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...listed }] }], devices: [], interface_language: "en", color_theme: null });
  await act(async () => root.render(<App />));
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="local|w1:p1"]')!.click());
  const field = () => container.querySelector<HTMLInputElement | HTMLTextAreaElement>("[data-phone-reply]")!;
  const type = (text: string) =>
    act(async () => {
      const element = field();
      const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, text);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    });
  // The page is not attached to a document here, so a click on Send does not submit its form by itself.
  const submit = () => act(async () => field().form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const inputs = () => socket.sent.filter((message) => message.type === "input");
  const send = () => container.querySelector<HTMLButtonElement>("[data-phone-send]")!;
  return { container, socket, serve, field, type, submit, inputs, send, unmount: () => act(async () => root.unmount()) };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

it("sends a reply of several lines as one input to a computer that answers, and refuses one past a phone message before it leaves", async () => {
  const { container, field, type, submit, inputs, send, unmount } = await openDetail({ answer: true });
  try {
    expect(field()).toBeInstanceOf(HTMLTextAreaElement);
    // The return key writes a new line; only the send sends.
    expect(field().getAttribute("enterkeyhint")).toBe("enter");
    await type("문구 2건은 반영하고,\n테스트는\t경로 구분자만");
    expect(send().disabled).toBe(false);
    await submit();
    expect(inputs()).toEqual([expect.objectContaining({ type: "input", device_id: "local", pane_id: "w1:p1", text: "문구 2건은 반영하고,\n테스트는\t경로 구분자만" })]);

    // 16 KiB of quotes is twice that as JSON: the phone names it and sends nothing.
    await type('"'.repeat(9000));
    expect(send().disabled).toBe(true);
    expect(container.querySelector("[data-phone-input-error]")!.textContent).toBe("This reply is too long to send at once. Split it into shorter replies.");
    await submit();
    expect(inputs()).toHaveLength(1);
  } finally {
    await unmount();
  }
});

it("keeps the one-line reply of 2,000 characters on a computer before answering", async () => {
  const { container, field, type, submit, inputs, send, unmount } = await openDetail({});
  try {
    expect(field()).toBeInstanceOf(HTMLInputElement);
    await type("x".repeat(2001));
    expect(send().disabled).toBe(true);
    expect(container.querySelector("[data-phone-input-error]")!.textContent).toBe("Replies can contain up to 2,000 characters at a time.");

    // A one-line field drops a pasted new line itself; a tab pasted into it is refused on the phone, as that computer would refuse it.
    await type("yes\tplease");
    await submit();
    expect(inputs()).toEqual([]);
    expect(container.querySelector("[data-phone-input-error]")!.textContent).toBe("Replies can contain only one line of text.");
  } finally {
    await unmount();
  }
});

it("draws the agent's question with each choice's note and sends the picked choice as a choice for that ask, never as its words", async () => {
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent({ demand: "question", status_code: "question", changed_at_unix_ms: 100 }));
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "코멘트 3건을 모두 읽었어요.", truncated: false, at_ms: 1 }], before: null, running: null });
    const asked: PhoneAsk = { kind: "question", turn: "100", since_ms: 100, text: "리뷰 코멘트 3건을 모두 반영할까요?", truncated: false, answerable: true, choices: [
      { id: "1", label: "모두 반영 (Recommended)", note: "문구 2건과 테스트 1건" },
      { id: "2", label: "문구 수정 2건만", note: "테스트는 다음에" },
      { id: "3", label: "반영하지 않기" },
    ] };
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: asked });
    const card = container.querySelector<HTMLElement>('[data-phone-ask="question"]')!;
    expect(card.querySelector("[data-phone-ask-text]")!.textContent).toBe("리뷰 코멘트 3건을 모두 반영할까요?");
    // The conversation answers with the card; the quick keys stay with the terminal (Doop 05).
    expect(container.querySelector("[data-phone-key]")).toBeNull();
    expect(container.querySelector("[data-phone-reply]")!.getAttribute("placeholder")).toBe("Or write a reply");
    expect(card.textContent).toContain("테스트는 다음에");
    // The agent's own recommendation mark becomes the tag; the label keeps its words alone.
    expect(card.querySelector("[data-phone-ask-recommended]")!.closest("label")!.textContent).toContain("모두 반영");
    expect(card.textContent).not.toContain("(Recommended)");
    // Nothing is sent until a choice is marked and Answer pressed.
    const answerButton = () => card.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!;
    expect(answerButton().disabled).toBe(true);
    await act(async () => card.querySelector<HTMLElement>('[data-phone-ask-choice="2"]')!.click());
    expect(card.querySelector("[data-phone-ask-hint]")).not.toBeNull();
    expect(inputs()).toEqual([]);
    await act(async () => answerButton().click());
    expect(inputs()).toEqual([expect.objectContaining({ type: "input", device_id: "local", pane_id: "w1:p1", turn: "100", kind: "question", choice: "2" })]);
    expect(inputs()[0]).not.toHaveProperty("text");
    await serve({ type: "input_result", request_id: String(inputs()[0]!.request_id), ok: true, reason: null });
    expect(container.querySelector('[data-phone-ask-sent="2"]')).not.toBeNull();
    // Other choices under the same turn are another question, answered afresh; so is a question the computer ties to no call
    // under a new turn, with the same words.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { ...asked, choices: [{ id: "1", label: "Keep files" }, { id: "2", label: "Delete files" }] } });
    expect(container.querySelector("[data-phone-ask-sent]")).toBeNull();
    expect(container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.disabled).toBe(true);
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { ...asked, turn: "140", since_ms: 140 } });
    expect(container.querySelector("[data-phone-ask-sent]")).toBeNull();
    // Once the agent asks nothing, the card goes.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: null });
    expect(container.querySelector("[data-phone-ask]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("shows a permission its computer has no keys for with the command it waits on, and answers it in the terminal", async () => {
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent({ demand: "approval", status_code: "approval", changed_at_unix_ms: 100, agent_kind: "pi" }));
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드해 볼게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { kind: "permission", turn: "100", since_ms: 100, command: "pnpm --dir site build", command_truncated: false, answerable: false, choices: [] } });
    const card = container.querySelector<HTMLElement>('[data-phone-ask="permission"]')!;
    expect(card.querySelector("[data-phone-ask-command]")!.textContent).toBe("pnpm --dir site build");
    expect(card.textContent).toContain("Run this command?");
    expect(card.querySelector("[data-phone-ask-send]")).toBeNull();
    await act(async () => card.querySelector<HTMLButtonElement>("[data-phone-ask-terminal]")!.click());
    expect(container.querySelector('[data-phone-view="terminal"]')!.getAttribute("data-state")).toBe("active");
    expect(container.querySelectorAll("[data-phone-key]")).toHaveLength(5);
    expect(container.querySelector("[data-phone-reply]")!.getAttribute("placeholder")).toBe("Send to the terminal");
    expect(inputs()).toEqual([]);
  } finally {
    await unmount();
  }
});

it("sends the operator to the terminal when the computer refuses a choice because the terminal no longer shows the card's ask", async () => {
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent({ demand: "approval", status_code: "approval", changed_at_unix_ms: 100 }));
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "지울게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { kind: "permission", turn: "100", since_ms: 100, command: "rm -rf build", command_truncated: false, answerable: true, choices: [{ id: "allow" }, { id: "allow_always" }, { id: "deny" }] } });
    const card = container.querySelector<HTMLElement>('[data-phone-ask="permission"]')!;
    await act(async () => card.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
    await act(async () => card.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
    await serve({ type: "input_result", request_id: String(inputs()[0]!.request_id), ok: false, reason: "screen_differs" });
    expect(card.querySelector("[data-phone-ask-error]")!.textContent).toBe("The terminal no longer shows what this card asks. Answer in the terminal tab.");
    // The choice cannot be sent again from here; the terminal is where it is answered.
    expect(card.querySelector("[data-phone-ask-send]")).toBeNull();
    expect(card.querySelector<HTMLButtonElement>('[data-phone-ask-choice="deny"]')!.disabled).toBe(true);
    await act(async () => card.querySelector<HTMLButtonElement>('[data-phone-ask-terminal="refused"]')!.click());
    expect(container.querySelector('[data-phone-view="terminal"]')!.getAttribute("data-state")).toBe("active");
    expect(inputs()).toHaveLength(1);
  } finally {
    await unmount();
  }
});

it("keeps an answered permission while its call stays open under new turns, and answers the next one afresh", async () => {
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent({ demand: "approval", status_code: "approval", changed_at_unix_ms: 100 }));
  try {
    const waiting = (turn: string, call?: string): PhoneAsk => ({ kind: "permission", turn, since_ms: Number(turn), command: "pnpm build", command_truncated: false, ...(call ? { call } : {}), answerable: true, choices: [{ id: "allow" }, { id: "deny" }] });
    const allow = async () => {
      await act(async () => container.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
      await act(async () => container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
      await serve({ type: "input_result", request_id: String(inputs().at(-1)!.request_id), ok: true, reason: null });
    };
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: waiting("100", "call-a") });
    await allow();
    // The allowed command runs and the agent's state moves while the same call is open: still sent, nothing offered.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: waiting("130", "call-a") });
    expect(container.querySelector('[data-phone-ask-sent="allow"]')).not.toBeNull();
    expect(container.querySelector("[data-phone-ask-send]")).toBeNull();
    // The next call asks for the same command: a new ask.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: waiting("160", "call-b") });
    expect(container.querySelector("[data-phone-ask-sent]")).toBeNull();
    // Asks tied to no call (Codex records a call only once it is allowed) are told apart by their turn.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: waiting("200") });
    await allow();
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: waiting("230") });
    expect(container.querySelector("[data-phone-ask-sent]")).toBeNull();
    expect(container.querySelector("[data-phone-ask-send]")).not.toBeNull();
    expect(inputs()).toHaveLength(2);
  } finally {
    await unmount();
  }
});

it("never sends a choice again once the computer could not say whether it landed, whatever the phone does next", async () => {
  const { container, socket, serve, inputs, unmount } = await openDetail({ answer: true }, agent({ demand: "approval", status_code: "approval", changed_at_unix_ms: 100 }));
  try {
    const connection = await import("./connection");
    const { usePhone } = await import("./store");
    const { askIdentity } = await import("./protocol");
    const asked: PhoneAsk = { kind: "permission", turn: "100", since_ms: 100, command: "rm -rf build", command_truncated: false, call: "call-a", answerable: true, choices: [{ id: "allow" }, { id: "deny" }] };
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: asked });
    await act(async () => container.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
    await act(async () => container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
    await serve({ type: "input_result", request_id: String(inputs()[0]!.request_id), ok: false, reason: "uncertain" });
    expect(container.querySelector('[data-phone-ask-terminal="refused"]')).not.toBeNull();
    // Leaving the agent and coming back, and the same ask told again under a new turn, keep it in the terminal.
    await act(async () => connection.closeDetail());
    await act(async () => connection.openDetail({ device_id: "local", pane_id: "w1:p1" }));
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { ...asked, turn: "130", since_ms: 130 } });
    expect(container.querySelector("[data-phone-ask-send]")).toBeNull();
    expect(container.querySelector('[data-phone-ask-terminal="refused"]')).not.toBeNull();
    connection.sendChoice("deny");
    expect(inputs()).toHaveLength(1);
    // A choice still on its way when the link drops may have landed too.
    const next: PhoneAsk = { ...asked, call: "call-b", turn: "160" };
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: next });
    await act(async () => container.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
    await act(async () => container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
    await act(async () => socket.onclose?.());
    expect(usePhone.getState().answers[askIdentity({ device_id: "local", pane_id: "w1:p1" }, next)]).toMatchObject({ state: "failed", inTerminal: true });
  } finally {
    await unmount();
  }
});

it("settles each choice by its own request, so an earlier answer's refusal never frees a later one that may have landed", async () => {
  const waiting = { demand: "approval", status_code: "approval", changed_at_unix_ms: 100 } as const;
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent(waiting));
  try {
    const connection = await import("./connection");
    const second = agent({ ...waiting, pane_id: "w1:p2", root_pane_id: "w1:p2" });
    await serve({ type: "agents", groups: [{ group: "seen", count: 2, rows: [{ kind: "agent", ...agent(waiting) }, { kind: "agent", ...second }] }], devices: [], interface_language: "en", color_theme: null });
    const answer = async (pane: string, call: string) => {
      await serve({ type: "conversation", device_id: "local", pane_id: pane, state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
      await serve({ type: "ask", device_id: "local", pane_id: pane, ask: { kind: "permission", turn: "100", since_ms: 100, command: "pnpm build", command_truncated: false, call, answerable: true, choices: [{ id: "allow" }, { id: "deny" }] } });
      await act(async () => container.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
      await act(async () => container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
    };
    await answer("w1:p1", "call-a");
    // The operator leaves for the second agent before the first answer comes back, and answers there too.
    await act(async () => connection.closeDetail());
    await act(async () => connection.openDetail({ device_id: "local", pane_id: "w1:p2" }));
    await answer("w1:p2", "call-b");
    const [first, later] = inputs().map((input) => String(input.request_id));
    await serve({ type: "input_result", request_id: first!, ok: false, reason: "in_flight" });
    await serve({ type: "input_result", request_id: later!, ok: false, reason: "uncertain" });
    // The second answer may have landed: it is answered in the terminal, and nothing is sent again.
    expect(container.querySelector('[data-phone-ask-terminal="refused"]')).not.toBeNull();
    connection.sendChoice("allow");
    expect(inputs()).toHaveLength(2);
    // The first was refused before anything was typed, so it may be sent again.
    await act(async () => connection.closeDetail());
    await act(async () => connection.openDetail({ device_id: "local", pane_id: "w1:p1" }));
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { kind: "permission", turn: "100", since_ms: 100, command: "pnpm build", command_truncated: false, call: "call-a", answerable: true, choices: [{ id: "allow" }, { id: "deny" }] } });
    expect(container.querySelector('[data-phone-ask-terminal="refused"]')).toBeNull();
    expect(container.querySelector("[data-phone-ask-send]")).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("keeps an answer that may have landed while its call's approval goes and comes back, until the agent asks on another call", async () => {
  const waiting = { demand: "approval", status_code: "approval", changed_at_unix_ms: 100 } as const;
  const { container, serve, inputs, unmount } = await openDetail({ answer: true }, agent(waiting));
  try {
    const connection = await import("./connection");
    const { usePhone } = await import("./store");
    const ask = (call: string, turn: string) => serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: { kind: "permission", turn, since_ms: Number(turn), command: "pnpm build", command_truncated: false, call, answerable: true, choices: [{ id: "allow" }, { id: "deny" }] } });
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "빌드할게요.", truncated: false, at_ms: 1 }], before: null, running: null });
    await ask("call-a", "100");
    await act(async () => container.querySelector<HTMLElement>('[data-phone-ask-choice="allow"]')!.click());
    await act(async () => container.querySelector<HTMLButtonElement>("[data-phone-ask-send]")!.click());
    await serve({ type: "input_result", request_id: String(inputs()[0]!.request_id), ok: false, reason: "uncertain" });
    // The agent works a moment, its row asks nothing under a later turn, and the same call's approval comes back.
    await serve({ type: "ask", device_id: "local", pane_id: "w1:p1", ask: null });
    await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...agent({ status_code: "working", changed_at_unix_ms: 200 }) }] }], devices: [], interface_language: "en", color_theme: null });
    await ask("call-a", "300");
    expect(container.querySelector('[data-phone-ask-terminal="refused"]')).not.toBeNull();
    connection.sendChoice("allow");
    expect(inputs()).toHaveLength(1);
    // Asking on another call ends the first, whose answer then goes.
    await ask("call-b", "400");
    expect(Object.keys(usePhone.getState().answers)).toEqual([]);
  } finally {
    await unmount();
  }
});

it("never drops an answer that may still hold its ask to make room, and answers a new one in the terminal instead", async () => {
  const { ANSWERS_KEPT, withAnswer, withoutEnded } = await import("./store");
  const landed = { choice: "allow", state: "failed", notice: { key: "mobile.input.uncertain" }, inTerminal: true } as const;
  const sent = { choice: "allow", state: "sent" } as const;
  let answers = withAnswer({}, "local|w1:p1|permission|call a", landed)!;
  for (let at = 1; at < ANSWERS_KEPT; at++) answers = withAnswer(answers, `local|p${at}|question|turn 100 ["?",null,[]]`, sent)!;
  expect(withAnswer(answers, "local|new|permission|call n", sent)).toBeNull();
  // An answer refused before anything was typed holds nothing, and goes first.
  const refused = { choice: "allow", state: "failed", notice: { key: "mobile.input.inFlight" }, inTerminal: false } as const;
  const room = withAnswer(withAnswer(Object.fromEntries(Object.entries(answers).slice(1)), "local|r|permission|call r", refused)!, "local|new|permission|call n", sent)!;
  expect(Object.keys(room)).not.toContain("local|r|permission|call r");
  expect(room["local|new|permission|call n"]).toEqual(sent);
  // A turn the agent moved past ends that agent's asks known by an earlier turn, and no other's; a call's ask outlives turns.
  const p1 = { device_id: "local", pane_id: "p1" };
  expect(Object.keys(withoutEnded(answers, p1, { turn: 100 }))).toHaveLength(ANSWERS_KEPT);
  const later = withoutEnded(answers, p1, { turn: 101 });
  expect(Object.keys(later)).toHaveLength(ANSWERS_KEPT - 1);
  expect(later["local|p2|question|turn 100 [\"?\",null,[]]"]).toEqual(sent);
  const first = { device_id: "local", pane_id: "w1:p1" };
  expect(withoutEnded(answers, first, { turn: 999 })).toBe(answers);
  expect(withoutEnded(answers, first, { call: "a", turn: 999 })).toBe(answers);
  // An answer on its way stays, whatever the agent asks.
  const sending = { "local|w1:p1|permission|call a": { choice: "allow", state: "sending" } } as const;
  expect(withoutEnded(sending, first, { call: "b", turn: 999 })).toBe(sending);
});

it("says a command runs by the clock from its start, never by what it runs", async () => {
  const { container, serve, unmount } = await openDetail({ answer: true }, agent({ status_code: "working", mark: "going", changed_at_unix_ms: Date.now() - 60_000 }));
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", messages: [{ id: 1, who: "agent", text: "테스트를 돌려 볼게요.", truncated: false, at_ms: 1 }], before: null, running: { kind: "command", since_ms: Date.now() - 42_000 } });
    const line = container.querySelector<HTMLElement>('[data-phone-activity="Running a command"]')!;
    expect(line.textContent).toMatch(/Running a command·0:4\d/);
    // A command ended with no new word: the line goes back to thinking.
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "append", messages: [], running: null });
    expect(container.querySelector('[data-phone-activity="Running a command"]')).toBeNull();
    expect(container.querySelector('[data-phone-activity="Thinking"]')).not.toBeNull();
    // Past the calls the computer kept, it cannot say what runs, and the foot claims neither.
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "append", messages: [], running: { kind: "unknown", since_ms: null } });
    expect(container.querySelector("[data-phone-activity]")).toBeNull();
  } finally {
    await unmount();
  }
});
