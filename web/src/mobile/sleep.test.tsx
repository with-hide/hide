// @vitest-environment jsdom
// The phone's detail of an agent whose process is ended (docs/UI_BEHAVIOR.md,
// The phone app): the pane under it is a bare shell, so the detail says the
// agent's state in the desktop's words, offers Wake in the same pane, and
// leaves the reply bar and the quick keys off.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentSleep } from "../snapshot";
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

const sleeping: AgentSleep = { state: "sleeping", since_unix_ms: 1, progress: "파서를 나눴어요" };

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
  // This page's own socket, the only one: another would be a connection an earlier test left running.
  expect(FakeSocket.instances).toHaveLength(1);
  const socket = FakeSocket.instances[0]!;
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  await act(async () => socket.onopen?.());
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off" });
  await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...listed }] }], devices: [], interface_language: "en", color_theme: null });
  await act(async () => root.render(<App />));
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="local|w1:p1"]')!.click());
  // Unmounting ends the connection's timers and socket too: a reconnect left running would open its socket in the next test's
  // page, which would then serve frames to it.
  const unmount = () =>
    act(async () => {
      root.unmount();
      connection.stop();
    });
  return { container, socket, serve, root, unmount };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

const replyBarOff = (container: HTMLElement) => {
  const keys = [...container.querySelectorAll<HTMLButtonElement>("[data-phone-key]")];
  expect(keys).toHaveLength(5);
  expect(keys.every((key) => key.disabled)).toBe(true);
  expect(container.querySelector<HTMLInputElement>("[data-phone-reply]")!.disabled).toBe(true);
};

it("shows a sleeping agent as Sleeping with Wake instead of its shell, and Wake sends the pane's own wake", async () => {
  const { container, socket, serve, unmount } = await openDetail(agent({ sleep: sleeping }));
  try {
    const state = container.querySelector('[data-phone-sleep="sleeping"]')!;
    expect(state.textContent).toContain("Sleeping");
    expect(state.textContent).toContain("파서를 나눴어요");
    // The shell's rows are not drawn, and the conversation switch is not offered over them.
    expect(container.querySelector("[data-phone-scrollback]")).toBeNull();
    expect(container.querySelector("[data-phone-conversation]")).toBeNull();
    expect(container.querySelector("[data-phone-view]")).toBeNull();
    replyBarOff(container);

    await act(async () => container.querySelector<HTMLElement>('[data-phone-wake="wake"]')!.click());
    const wake = socket.sent.find((message) => message.type === "wake_agent");
    expect(wake).toMatchObject({ type: "wake_agent", device_id: "local", pane_id: "w1:p1" });
    expect(wake).not.toHaveProperty("fresh");
    // One wake waits for its answer at a time; the answer frees the button.
    expect(container.querySelector<HTMLButtonElement>('[data-phone-wake="wake"]')!.disabled).toBe(true);
    await serve({ type: "sleep_result", request_id: wake!.request_id as string, sleep_id: "local|w1:p1", ok: true, reason: null });
    expect(container.querySelector<HTMLButtonElement>('[data-phone-wake="wake"]')!.disabled).toBe(false);

    // The core's next list says it is waking: the state follows, with no Wake to press twice.
    await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...agent({ status_code: "waking", sleep: { state: "waking", since_unix_ms: Date.now() - 3 * 3_600_000 } }) }] }], devices: [], interface_language: "en", color_theme: null });
    const waking = container.querySelector('[data-phone-sleep="waking"]')!;
    expect(waking.textContent).toContain("Waking");
    expect(waking.textContent).toContain("Resuming the conversation from");
    expect(container.querySelector("[data-phone-wake]")).toBeNull();
    replyBarOff(container);
  } finally {
    await unmount();
  }
});

it("says why a resume failed and offers Retry and Start new session in the same pane", async () => {
  const failed: AgentSleep = { state: "failed", reason: "The conversation file is gone", since_unix_ms: 1 };
  const { container, socket, unmount } = await openDetail(agent({ status_code: "sleep_failed", sleep: failed }));
  try {
    const state = container.querySelector('[data-phone-sleep="failed"]')!;
    expect(state.textContent).toContain("Couldn’t resume this conversation");
    expect(state.textContent).toContain("The conversation file is gone");
    replyBarOff(container);

    await act(async () => container.querySelector<HTMLElement>('[data-phone-wake="fresh"]')!.click());
    expect(socket.sent.find((message) => message.type === "wake_agent")).toMatchObject({ pane_id: "w1:p1", fresh: true });
  } finally {
    await unmount();
  }
});

it("keeps the reply bar for an agent that is awake, and names a refusal that still comes back", async () => {
  const { container, socket, serve, unmount } = await openDetail(agent({ mark: "going", status_code: "idle" }));
  try {
    expect(container.querySelector("[data-phone-sleep]")).toBeNull();
    expect(container.querySelector<HTMLInputElement>("[data-phone-reply]")!.disabled).toBe(false);
    const keys = [...container.querySelectorAll<HTMLButtonElement>("[data-phone-key]")];
    expect(keys.some((key) => key.disabled)).toBe(false);

    // The agent fell asleep just as the key was sent: hided refuses it and the phone says why.
    await act(async () => keys[0]!.click());
    const sent = socket.sent.find((message) => message.type === "input")!;
    await serve({ type: "input_result", request_id: sent.request_id as string, ok: false, reason: "asleep" });
    expect(container.querySelector("[data-phone-input-error]")!.textContent).toContain("asleep");
  } finally {
    await unmount();
  }
});

// Doop 02 and 09: an awake agent's detail says where it stands under its head from the row the phone already has, and while it
// works the foot of its conversation says it thinks, counted on the phone from its last word; no new frame field carries either.
it("draws the state strip from the row and a thinking line while the agent works, which goes once it stops", async () => {
  // The phone counts both times from its own clock, held still here, so they are the times this test set however long the app
  // takes to load: on a loaded Windows runner the line read 5 s against the real clock.
  vi.useFakeTimers({ toFake: ["Date"] });
  const now = Date.now();
  const working = agent({ group: "working", mark: "going", status_code: "working", changed_at_unix_ms: now - 60_000, line: { text: "Reading holder.log", mode: "kept", tone: "subtle" } });
  const { container, serve, unmount } = await openDetail(working);
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", before: null, messages: [
      { id: 1, who: "you", text: "Compare the logs", truncated: false, at_ms: now - 30_000 },
      { id: 2, who: "agent", text: "Reading them now.", truncated: false, at_ms: now - 4_000 },
    ] });
    const strip = container.querySelector("[data-phone-status-strip]")!;
    expect(strip.getAttribute("data-phone-status-strip")).toBe("Working");
    // The list's line leads with its word (`Working · Reading holder.log`); the strip says that word once, as its own.
    expect(strip.querySelector("[data-phone-status-line]")!.textContent).toBe("Reading holder.log");
    expect(strip.textContent).toContain("1m");
    // The thinking line counts from the agent's last word, not the row's older change.
    expect(container.querySelector('[data-phone-activity="Thinking"]')!.textContent).toBe("Thinking·4s");
    await serve({ type: "agents", groups: [{ group: "done", count: 1, rows: [{ kind: "agent", ...working, group: "done", mark: "done", status_code: "done", changed_at_unix_ms: now }] }], devices: [], interface_language: "en", color_theme: null });
    expect(container.querySelector("[data-phone-activity]")).toBeNull();
    expect(container.querySelector("[data-phone-status-strip]")!.getAttribute("data-phone-status-strip")).toBe("Done");
  } finally {
    await unmount();
  }
});

// A reader at the bottom of the conversation sees the thinking line when it appears with no new word: the agents frame that
// turns the agent to work usually comes before any message, and the line would otherwise sit below the visible part.
it("keeps a reader at the bottom down at the thinking line when the agent starts working with no new message", async () => {
  const now = Date.now();
  const idle = agent({ group: "seen", mark: "quiet", status_code: "idle", changed_at_unix_ms: now - 60_000, line: null });
  const { container, serve, unmount } = await openDetail(idle);
  const height = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(400);
  try {
    await serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", before: null, messages: [
      { id: 1, who: "agent", text: "Done for now.", truncated: false, at_ms: now - 30_000 },
    ] });
    const log = container.querySelector<HTMLElement>("[data-phone-conversation]")!;
    expect(log.scrollTop).toBe(400);
    expect(container.querySelector("[data-phone-activity]")).toBeNull();
    // The line adds height; the frame carries no message.
    height.mockReturnValue(460);
    await serve({ type: "agents", groups: [{ group: "working", count: 1, rows: [{ kind: "agent", ...idle, group: "working", mark: "going", status_code: "working", changed_at_unix_ms: now }] }], devices: [], interface_language: "en", color_theme: null });
    expect(container.querySelector("[data-phone-activity]")).not.toBeNull();
    expect(log.scrollTop).toBe(460);
  } finally {
    height.mockRestore();
    await unmount();
  }
});
