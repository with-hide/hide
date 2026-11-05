// @vitest-environment jsdom
// The phone's 볼 것 (docs/UI_BEHAVIOR.md, The phone app): the tab and its count, the list, a View opened by its id alone, its
// content put back from chunks, the states that are not content, and the reply that names what is on screen.
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { LookRow, PhoneAgent, ServerFrame } from "./protocol";

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

const AGENT: PhoneAgent = { device_id: "local", pane_id: "w1:p1", root_pane_id: "w1:p1", group: "needs_you", mark: "look", wait: null, descendants_working: 0, agent_kind: "claude", title: "리뷰 반영", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: true, holds_notification: false };

const ROWS: LookRow[] = [
  { id: "v1", kind: "doc", name: "NOTES.md", unseen: true, at: Date.now() - 60_000 },
  { id: "v2", kind: "changes", name: "경로 구분자 수정", unseen: true, at: Date.now() - 120_000 },
  { id: "v3", kind: "image", name: "chart.png", unseen: false, at: Date.now() - 3_600_000 },
];

/** A phone connected to a fake hided, with one agent whose row shows the eye, its detail opened from that row, and its Views listed. */
async function openLooks(rows: LookRow[] = ROWS) {
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
  await serve({ type: "agents", groups: [{ group: "needs_you", count: 1, rows: [{ kind: "agent", ...AGENT }] }], devices: [], interface_language: "en", color_theme: null });
  await act(async () => root.render(<App />));
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="local|w1:p1"]')!.click());
  await serve({ type: "looks", device_id: "local", pane_id: "w1:p1", views: rows, omitted: 0 });
  const last = (type: string) => socket.sent.filter((message) => message.type === type).at(-1)!;
  const press = (selector: string) => act(async () => container.querySelector<HTMLElement>(selector)!.click());
  return { container, socket, serve, last, press, unmount: () => act(async () => root.unmount()) };
}

const base64 = (text: string) => Buffer.from(text).toString("base64");

afterEach(() => {
  vi.unstubAllGlobals();
});

it("opens on 볼 것 from the eye row, counts what is not seen, and splits the list into not seen and seen", async () => {
  const { container, socket, unmount } = await openLooks();
  try {
    expect(socket.sent.find((message) => message.type === "open")).toMatchObject({ type: "open", device_id: "local", pane_id: "w1:p1" });
    const tab = container.querySelector('[data-phone-view="looks"]')!;
    expect(tab.getAttribute("data-state")).toBe("active");
    expect(container.querySelector("[data-phone-looks-unseen]")!.textContent).toBe("2");
    expect([...container.querySelectorAll("[data-phone-look-group]")].map((group) => group.getAttribute("data-phone-look-group"))).toEqual(["unseen", "seen"]);
    expect(container.querySelector('[data-phone-look="v1"]')!.getAttribute("data-phone-look-unseen")).toBe("true");
    expect(container.querySelector('[data-phone-look="v3"]')!.getAttribute("data-phone-look-unseen")).toBe("false");
    // The tab that hided reads is not asked for: 볼 것 is the phone's own choice.
    expect(socket.sent.some((message) => message.type === "view")).toBe(false);
  } finally {
    await unmount();
  }
});

it("has no 볼 것 tab for an agent with no open Views, and shows the conversation instead", async () => {
  const { container, serve, unmount } = await openLooks([]);
  try {
    expect(container.querySelector('[data-phone-view="looks"]')).toBeNull();
    expect(container.querySelector("[data-phone-looks]")).toBeNull();
    // A View opened later brings the tab, and the list it is on does not move.
    await serve({ type: "looks", device_id: "local", pane_id: "w1:p1", views: ROWS, omitted: 0 });
    expect(container.querySelector('[data-phone-view="looks"]')).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("asks for a View by its id alone, draws a Markdown document from its chunks, confirms each, and says it was seen once", async () => {
  const { container, serve, last, socket, press, unmount } = await openLooks();
  try {
    await press('[data-phone-look="v1"]');
    const open = last("look_open");
    // D-16: the request names the agent and the View's id, never a path.
    expect(Object.keys(open).sort()).toEqual(["device_id", "pane_id", "request_id", "type", "view_id"]);
    expect(open.view_id).toBe("v1");
    expect(container.querySelector('[data-phone-look-state="loading"]')).not.toBeNull();

    const text = "# 설치하기\n\n본문이에요.";
    const request_id = open.request_id as string;
    await serve({ type: "look_head", request_id, view_id: "v1", mode: "markdown", size: Buffer.byteLength(text), total: Buffer.byteLength(text), truncated: false, chunks: 1 });
    await serve({ type: "look_chunk", request_id, seq: 0, data: base64(text) });
    expect(last("look_ack")).toEqual({ type: "look_ack", request_id, seq: 0 });
    expect(socket.sent.some((message) => message.type === "look_seen")).toBe(false);
    await serve({ type: "look_end", request_id, view_id: "v1" });

    const body = container.querySelector('[data-phone-look-state="markdown"]')!;
    expect(body.textContent).toContain("설치하기");
    expect(body.textContent).toContain("본문이에요.");
    expect(socket.sent.filter((message) => message.type === "look_seen")).toEqual([{ type: "look_seen", device_id: "local", pane_id: "w1:p1", view_id: "v1" }]);
    expect(container.querySelector("[data-phone-viewing]")!.textContent).toContain("NOTES.md");

    // A reply sent while looking carries the name of what is on screen in front (B30), and the quick keys are not offered.
    expect(container.querySelector("[data-phone-key]")).toBeNull();
    const field = container.querySelector<HTMLInputElement>("[data-phone-reply]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, "표를 고쳐 줘");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // The page is not attached to a document here, so a click on Send does not submit its form by itself.
    await act(async () => field.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(last("input")).toMatchObject({ type: "input", text: "[NOTES.md] 표를 고쳐 줘" });
  } finally {
    await unmount();
  }
});

it("drops the frames of a read the phone gave up, and a chunk out of order fails the read", async () => {
  const { container, serve, last, press, unmount } = await openLooks();
  try {
    await press('[data-phone-look="v1"]');
    const stale = last("look_open").request_id as string;
    await press('[data-phone-back="looks"]');
    await press('[data-phone-look="v3"]');
    const current = last("look_open").request_id as string;
    expect(current).not.toBe(stale);

    // A head of the abandoned read does not start an assembly for the new one.
    await serve({ type: "look_head", request_id: stale, view_id: "v1", mode: "text", size: 3, total: 3, truncated: false, chunks: 1 });
    await serve({ type: "look_chunk", request_id: stale, seq: 0, data: base64("abc") });
    expect(container.querySelector('[data-phone-look-state="loading"]')).not.toBeNull();

    await serve({ type: "look_head", request_id: current, view_id: "v3", mode: "image", size: 400000, total: 400000, truncated: false, chunks: 5 });
    await serve({ type: "look_chunk", request_id: current, seq: 1, data: base64("x") });
    expect(container.querySelector('[data-phone-look-state="failed"]')).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("lists the changed files with their counts and no code, and says when nothing changed", async () => {
  const { container, serve, last, press, unmount } = await openLooks();
  try {
    await press('[data-phone-look="v2"]');
    const request_id = last("look_open").request_id as string;
    await serve({
      type: "look_changes",
      request_id,
      view_id: "v2",
      files: [
        { path: "hide-platform/src/path.rs", status: "modified", added: 6, removed: 2, binary: false },
        { path: "docs/new.md", status: "added", added: 7, removed: 0, binary: false },
        { path: "src/fresh.ts", status: "untracked", added: 2, removed: 0, binary: false },
        { path: "data/huge.csv", status: "untracked", added: null, removed: null, binary: false },
        { path: "assets/logo.png", status: "modified", added: null, removed: null, binary: true },
      ],
      count: 5,
      added: 15,
      removed: 2,
    });
    const body = container.querySelector('[data-phone-look-state="changes"]')!;
    expect(container.querySelector("[data-phone-look-files]")!.getAttribute("data-phone-look-files")).toBe("5");
    const row = (path: string) => body.querySelector(`[data-phone-look-file="${path}"]`)!.textContent ?? "";
    expect(row("hide-platform/src/path.rs")).toContain("hide-platform/src");
    expect(row("docs/new.md")).toContain("New file");
    // A new text file shows its lines and is no binary (B14); one hided did not count shows only that it is new.
    expect(row("src/fresh.ts")).toContain("New file");
    expect(row("src/fresh.ts")).toContain("+2");
    expect(row("src/fresh.ts")).not.toContain("Binary");
    expect(row("data/huge.csv")).toContain("New file");
    expect(row("data/huge.csv")).not.toMatch(/Binary|\+0/);
    expect(row("assets/logo.png")).toContain("Binary");
    expect(body.textContent).toContain("See the code on your Mac");
    expect(container.querySelector("pre")).toBeNull();
    expect(last("look_seen")).toMatchObject({ view_id: "v2" });

    await press('[data-phone-back="looks"]');
    await press('[data-phone-look="v2"]');
    await serve({ type: "look_changes", request_id: last("look_open").request_id as string, view_id: "v2", files: [], count: 0, added: 0, removed: 0 });
    expect(container.querySelector("[data-phone-look-empty]")!.textContent).toContain("Nothing has changed");
  } finally {
    await unmount();
  }
});

it("says a View is not open any more, and a failed read starts over under a new request with 다시", async () => {
  const { container, serve, last, press, socket, unmount } = await openLooks();
  try {
    await press('[data-phone-look="v1"]');
    await serve({ type: "look_result", request_id: last("look_open").request_id as string, view_id: "v1", state: "closed" });
    expect(container.querySelector('[data-phone-look-state="closed"]')!.textContent).toContain("isn't open anymore");
    // Nothing was shown, so nothing is seen.
    expect(socket.sent.some((message) => message.type === "look_seen")).toBe(false);

    await press('[data-phone-back="looks"]');
    await press('[data-phone-look="v3"]');
    const first = last("look_open").request_id as string;
    await serve({ type: "look_result", request_id: first, view_id: "v3", state: "failed" });
    // Away from the Mac there is nothing to ask again: 다시 waits for the connection.
    const { patch } = await import("./store");
    await act(async () => patch({ connected: false }));
    expect(container.querySelector<HTMLButtonElement>("[data-phone-look-retry]")!.disabled).toBe(true);
    await act(async () => patch({ connected: true }));
    await press("[data-phone-look-retry]");
    const retried = last("look_open");
    expect(retried.view_id).toBe("v3");
    expect(retried.request_id).not.toBe(first);
    expect(container.querySelector('[data-phone-look-state="loading"]')).not.toBeNull();
  } finally {
    await unmount();
  }
});

it("names what the phone cannot show: a page's address, a binary, an image past its cap, an unreachable device", async () => {
  const { container, serve, last, press, socket, unmount } = await openLooks();
  try {
    const answer = async (id: string, state: ServerFrame & { type: "look_result" }) => {
      await press(`[data-phone-look="${id}"]`);
      await serve({ ...state, request_id: last("look_open").request_id as string });
    };
    const seen = () => socket.sent.filter((message) => message.type === "look_seen").map((message) => message.view_id);
    await answer("v1", { type: "look_result", request_id: "", view_id: "v1", state: "other" });
    expect(container.querySelector('[data-phone-look-state="other"]')!.textContent).toContain("Open it on your Mac");
    // A PDF or an image past its cap was opened and its name drawn: it is seen too (B24, D-15).
    expect(seen()).toEqual(["v1"]);
    await press('[data-phone-back="looks"]');
    await answer("v3", { type: "look_result", request_id: "", view_id: "v3", state: "too_large" });
    expect(container.querySelector('[data-phone-look-state="too_large"]')!.textContent).toContain("See it on your Mac");
    expect(seen()).toEqual(["v1", "v3"]);
    await press('[data-phone-back="looks"]');
    await answer("v2", { type: "look_result", request_id: "", view_id: "v2", state: "device_unreachable" });
    expect(container.querySelector('[data-phone-look-state="device_unreachable"]')!.textContent).toContain("isn't connected");
    // Nothing of it was drawn, so it stays unseen.
    expect(seen()).toEqual(["v1", "v3"]);
  } finally {
    await unmount();
  }
});
