// @vitest-environment jsdom
// The waiting line on the phone (PRD desk-queue D-11, B32, B33): a computer whose hello says `queue` sends the open detail's
// waiting words, takes ✕, 보내기, 이어 보내기, 모두 지우기 and 되돌리기 on them, and ■ while the agent works; one before it shows neither.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { PhoneAgent, ServerFrame } from "./protocol";

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
  // This page's own socket, the only one: another would be a connection an earlier test left running.
  expect(FakeSocket.instances).toHaveLength(1);
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
  // Unmounting ends the connection's timers and socket too: a reconnect left running would open its socket in the next test's
  // page, which would then serve frames to it.
  const unmount = () =>
    act(async () => {
      root.unmount();
      connection.stop();
    });
  return { container, socket, serve, field, type, submit, inputs, send, unmount };
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const working = agent({ status_code: "working" });
const line = (id: number, text: string) => ({ id, text, files: [], sending: false });

it("waits a reply sent while the agent works over the bar, takes ✕ on a line, and stops the turn with ■", async () => {
  const { container, socket, serve, type, submit, inputs, field, unmount } = await openDetail({ answer: true, queue: true }, working);
  // ■ stands in send's place; the keyboard's send still sends, to the line.
  expect(container.querySelector("[data-phone-send]")).toBeNull();
  await type("테스트도 같이 돌려 줘");
  await submit();
  const [reply] = inputs();
  expect(reply).toMatchObject({ text: "테스트도 같이 돌려 줘" });
  await serve({ type: "input_result", request_id: reply!.request_id as string, ok: true, reason: null, queued: true });
  expect(field().value).toBe("");
  await serve({ type: "queue", device_id: "local", pane_id: "w1:p1", queue: { state: "waiting", lines: [line(3, "테스트도 같이 돌려 줘")], max: 8, full: false } });
  expect(container.querySelector("[data-phone-queue='waiting']")?.textContent).toContain("Waiting · 1");
  await act(async () => container.querySelector<HTMLElement>("[data-phone-queue-remove='3']")!.click());
  expect(socket.sent.filter((message) => message.type === "queue")).toEqual([expect.objectContaining({ device_id: "local", pane_id: "w1:p1", action: "remove", id: 3 })]);

  await act(async () => container.querySelector<HTMLElement>("[data-phone-stop]")!.click());
  const stop = inputs().at(-1)!;
  expect(stop).toMatchObject({ device_id: "local", pane_id: "w1:p1", interrupt: true });
  // Between the press and the keys the agent asked something: nothing was typed, and the bar says why.
  await serve({ type: "input_result", request_id: stop.request_id as string, ok: false, reason: "agent_asking" });
  expect(container.querySelector("[data-phone-input-error]")?.textContent).toBe("The agent is asking something: answer it first");
  await unmount();
});

it("offers 모두 지우기, 이어 보내기 and each line's 보내기 once paused, 보내기 waiting while the agent works, and 되돌리기 after a clear", async () => {
  const { container, socket, serve, unmount } = await openDetail({ answer: true, queue: true }, working);
  const queues = () => socket.sent.filter((message) => message.type === "queue").map(({ action, id }) => ({ action, id }));
  await serve({ type: "queue", device_id: "local", pane_id: "w1:p1", queue: { state: "paused", lines: [line(1, "하나"), line(2, "둘")], max: 8, full: false } });
  expect(container.querySelector("[data-phone-queue='paused']")?.textContent).toContain("Paused · 2");
  expect(container.querySelector("[data-phone-queue-later]")?.textContent).toBe("You can send it once the work ends");
  expect(container.querySelector<HTMLButtonElement>("[data-phone-queue-send='1']")!.disabled).toBe(true);
  // The agent rests: a line's 보내기 goes.
  await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...agent() }] }], devices: [], interface_language: "en", color_theme: null });
  expect(container.querySelector("[data-phone-queue-later]")).toBeNull();
  await act(async () => container.querySelector<HTMLElement>("[data-phone-queue-send='2']")!.click());
  await act(async () => container.querySelector<HTMLElement>("[data-phone-queue-resume]")!.click());
  await act(async () => container.querySelector<HTMLElement>("[data-phone-queue-clear]")!.click());
  expect(queues()).toEqual([{ action: "send", id: 2 }, { action: "resume", id: undefined }, { action: "clear", id: undefined }]);
  await serve({ type: "queue", device_id: "local", pane_id: "w1:p1", queue: { state: "waiting", lines: [], max: 8, full: false, cleared: { count: 2, until_unix_ms: Date.now() + 10_000 } } });
  expect(container.querySelector("[data-phone-queue-cleared]")?.textContent).toContain("Cleared 2");
  await act(async () => container.querySelector<HTMLElement>("[data-phone-queue-restore]")!.click());
  expect(queues().at(-1)).toEqual({ action: "restore", id: undefined });
  // A press hided could not hand to the core says why under the bar.
  await serve({ type: "queue_result", request_id: "x", ok: false, reason: "gone" });
  expect(container.querySelector("[data-phone-input-error]")?.textContent).toBe("This pane is no longer open.");
  await unmount();
});

it("keeps send and draws no line on a computer whose hello does not hold the waiting line", async () => {
  const { container, unmount } = await openDetail({ answer: true }, working);
  expect(container.querySelector("[data-phone-stop]")).toBeNull();
  expect(container.querySelector("[data-phone-send]")).not.toBeNull();
  await unmount();
});
