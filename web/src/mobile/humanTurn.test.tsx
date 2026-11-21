// @vitest-environment jsdom
// A person's turn on the phone (PRD ask-human B9, B10, B19): the list row
// and the detail say what and where with Done, which sends hided one
// message naming the declaration and says when it did not land.
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

function agent(over: Partial<PhoneAgent>): PhoneAgent {
  return { device_id: "local", pane_id: "w1:p1", root_pane_id: "w1:p1", group: "seen", mark: "asleep", wait: null, descendants_working: 0, agent_kind: "claude", title: "파서 나누기", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "sleeping", demand: "none", emphasized: false, holds_notification: false, ...over };
}

/** A phone paired and connected to a fake hided, with one agent listed and its detail open. */
async function openDetail(listed: PhoneAgent) {
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
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off" });
  await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...listed }] }], devices: [], interface_language: "en", color_theme: null });
  await act(async () => root.render(<App />));
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="local|w1:p1"]')!.click());
  return { container, socket, serve, root, unmount: () => act(async () => root.unmount()) };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

const declared = agent({ group: "needs_you", mark: "turn", status_code: "question", demand: "question", emphasized: true, device_label: "mini", human_turn: { id: "agent-7.1700", kind: "auth", place: "github.com/login", pane_id: "w1:p1", title: "파서 나누기", since_unix_ms: 1, own: true } });

it("says a person's turn with Done, sends one message naming the declaration, and says when it did not land", async () => {
  const { container, socket, serve, unmount } = await openDetail(declared);
  try {
    const turn = container.querySelector<HTMLElement>('[data-phone-human-turn="agent-7.1700"]')!;
    expect(turn.querySelector("[data-phone-human-place]")!.textContent).toBe("Sign-in · github.com/login");
    expect(turn.querySelector("a")).toBeNull();
    const done = turn.querySelector<HTMLButtonElement>("[data-phone-human-done]")!;
    await act(async () => done.click());
    const sent = socket.sent.filter((message) => message.type === "human_turn_done");
    expect(sent).toEqual([{ type: "human_turn_done", request_id: expect.any(String), device_id: "local", pane_id: "w1:p1", declaration: "agent-7.1700" }]);
    expect(done.disabled).toBe(true);
    await act(async () => done.click());
    expect(socket.sent.filter((message) => message.type === "human_turn_done")).toHaveLength(1);
    await serve({ type: "human_turn_result", request_id: sent[0]!.request_id as string, ok: false, reason: "unavailable" });
    expect(turn.querySelector('[role="status"]')!.textContent).toBe("Done did not reach your Mac. Press it again.");
    expect(done.disabled).toBe(false);
  } finally {
    await unmount();
  }
});
