// @vitest-environment jsdom
// Files attached to a reply from the phone (docs/UI_BEHAVIOR.md, The phone app; Doop 08, 09): the `+` only on a computer whose hello
// says `attachments`, the reply's words and files leaving as one input once every file is in, and the sent files over the bubble.
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

const listed: PhoneAgent = { device_id: "local", pane_id: "w1:p1", root_pane_id: "w1:p1", group: "seen", mark: "going", wait: null, descendants_working: 0, agent_kind: "claude", title: "메모리 사용량 측정", place: null, device_label: null, changed_at_unix_ms: null, line: null, status_code: "idle", demand: "none", emphasized: false, holds_notification: false };

async function openDetail(hello: Partial<Extract<ServerFrame, { type: "hello" }>>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("WebSocket", FakeSocket);
  FakeSocket.instances = [];
  vi.resetModules();
  const { App } = await import("./App");
  const connection = await import("./connection");
  const { patch } = await import("./store");
  const uploads = await import("./uploads");
  const container = document.createElement("div");
  const root = createRoot(container);
  patch({ screen: "pair", pair: { v: 1, endpoint: "https://mac.example.ts.net", code: "code" } });
  connection.pairNow();
  const socket = FakeSocket.instances[0]!;
  const serve = (frame: ServerFrame) => act(async () => socket.onmessage?.({ data: JSON.stringify(frame) }));
  await act(async () => socket.onopen?.());
  await serve({ type: "hello", mac_name: "Mac", phone_id: "phone-a", name: null, vapid_public_key: "key", notifications: "off", answer: true, ...hello });
  await serve({ type: "agents", groups: [{ group: "seen", count: 1, rows: [{ kind: "agent", ...listed }] }], devices: [], interface_language: "en", color_theme: null });
  await act(async () => root.render(<App />));
  await act(async () => container.querySelector<HTMLElement>('[data-phone-agent="local|w1:p1"]')!.click());
  const field = () => container.querySelector<HTMLTextAreaElement>("[data-phone-reply]")!;
  const type = (text: string) =>
    act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(field(), text);
      field().dispatchEvent(new Event("input", { bubbles: true }));
    });
  const submit = () => act(async () => field().form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const of = (type: string) => socket.sent.filter((message) => message.type === type);
  const send = () => container.querySelector<HTMLButtonElement>("[data-phone-send]")!;
  return { container, serve, type, submit, of, send, uploads, unmount: () => act(async () => root.unmount()) };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

it("offers no + on a computer before attachments, whose reply bar stays as it was", async () => {
  const { container, unmount } = await openDetail({});
  try {
    expect(container.querySelector("[data-phone-reply]")).not.toBeNull();
    expect(container.querySelector("[data-phone-attach]")).toBeNull();
  } finally {
    await unmount();
  }
});

it("sends the words and the files as one input once every file is in, and draws the sent files over the bubble", async () => {
  const { container, serve, type, submit, of, send, uploads, unmount } = await openDetail({ attachments: true });
  try {
    expect(container.querySelector("[data-phone-attach]")!.getAttribute("aria-label")).toBe("Attach files");
    const photo = uploads.reserveFile("IMG_0001.jpg", "photo")!;
    await act(async () => uploads.addFile(photo, { name: "IMG_0001.jpg", bytes: new Uint8Array([1, 2, 3]), kind: "photo", thumb: "data:image/jpeg;base64,AA==" }));
    const log = uploads.reserveFile("holder.log", "file")!;
    await act(async () => uploads.addFile(log, { name: "holder.log", bytes: new TextEncoder().encode("busy 248 MB"), kind: "file", thumb: null }));
    expect(container.querySelectorAll("[data-phone-attached]")).toHaveLength(2);
    // Only the file in front uploads; send waits for both.
    const [shot] = of("attach_begin") as { stage: string }[];
    await type("어제 잰 화면이랑 오늘 holder 로그야.");
    expect(send().disabled).toBe(true);
    await serve({ type: "attach_ack", stage: shot!.stage, done: true });
    const logStage = (of("attach_begin")[1] as { stage: string }).stage;
    await serve({ type: "attach_ack", stage: logStage, done: true });
    expect(send().disabled).toBe(false);
    await submit();
    expect(of("input")).toEqual([expect.objectContaining({ text: "어제 잰 화면이랑 오늘 holder 로그야.", stages: [shot!.stage, logStage] })]);

    // A file picked while the reply is on its way is not the reply's: it stays on the bar once the reply lands.
    const later = uploads.reserveFile("later.log", "file")!;
    await act(async () => uploads.addFile(later, { name: "later.log", bytes: new Uint8Array([4]), kind: "file", thumb: null }));
    const requestId = (of("input")[0] as { request_id: string }).request_id;
    await serve({ type: "input_result", request_id: requestId, ok: true, reason: null });
    expect([...container.querySelectorAll("[data-phone-attached]")].map((file) => file.textContent)).toEqual([expect.stringContaining("later.log")]);
    const folder = "/Users/me/.hide/state/attachments";
    await serve({
      type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode: "reset", before: null, running: null,
      messages: [{ id: 1, who: "you", text: `"${folder}/${shot!.stage}-IMG_0001.jpg" "${folder}/${logStage}-holder.log" 어제 잰 화면이랑 오늘 holder 로그야.`, truncated: false, at_ms: Date.now() }],
    });
    const bubble = container.querySelector<HTMLElement>('[data-phone-message="you"]')!;
    expect(bubble.querySelector<HTMLImageElement>('[data-phone-sent-photo="IMG_0001.jpg"] img')!.src).toBe("data:image/jpeg;base64,AA==");
    expect(bubble.querySelector('[data-phone-sent-file="holder.log"]')!.textContent).toContain("holder.log");
    expect(bubble.querySelector('[data-phone-sent-file="later.log"]')).toBeNull();
    expect(bubble.querySelector("p")!.textContent).toBe("어제 잰 화면이랑 오늘 holder 로그야.");
  } finally {
    await unmount();
  }
});

it("draws a photo the agent took in as an image over the message that names its upload, an image with no path as a photo with no name, and a photo sent alone with no bubble", async () => {
  const { container, serve, type, submit, of, uploads, unmount } = await openDetail({ attachments: true });
  try {
    const at = Date.now();
    const said = (id: number, who: "you" | "agent", text: string, images = 0, files: string[] = []) => ({
      id, who, text, truncated: false, at_ms: at + id, ...(images > 0 ? { images } : {}), ...(files.length > 0 ? { files } : {}),
    });
    const conversation = (mode: "reset" | "append", ...messages: ReturnType<typeof said>[]) =>
      serve({ type: "conversation", device_id: "local", pane_id: "w1:p1", state: "ok", mode, before: null, running: null, messages });
    const reply = async (name: string, words: string) => {
      const photo = uploads.reserveFile(name, "photo")!;
      await act(async () => uploads.addFile(photo, { name, bytes: new Uint8Array([1, 2, 3]), kind: "photo", thumb: "data:image/jpeg;base64,AA==" }));
      const stage = (of("attach_begin").at(-1) as { stage: string }).stage;
      await serve({ type: "attach_ack", stage, done: true });
      await type(words);
      await submit();
      const requestId = (of("input").at(-1) as { request_id: string }).request_id;
      await serve({ type: "input_result", request_id: requestId, ok: true, reason: null });
      return stage;
    };
    await conversation("reset", said(100, "agent", "버퍼를 줄였어요"));
    const first = await reply("IMG_0001.jpg", "확인해줘");
    // The agent took the photo in as an image and recorded its path beside the words; later the same words come with a photo whose
    // path no record kept, and a photo pasted on the computer stands alone.
    await conversation("append", said(200, "you", "확인해줘", 1, [`${first}-IMG_0001.jpg`]), said(300, "agent", "봤어요"), said(400, "you", "확인해줘", 1), said(500, "you", "", 1));
    const mine = () => [...container.querySelectorAll<HTMLElement>('[data-phone-message="you"]')];
    const drawn = (message: HTMLElement) => [
      message.querySelector("p")?.textContent ?? null,
      message.querySelectorAll("[data-phone-sent-photo]").length,
      [...message.querySelectorAll("[data-phone-sent-image]")].map((chip) => chip.textContent),
    ];
    expect(mine().map(drawn)).toEqual([
      ["확인해줘", 1, []],
      ["확인해줘", 0, ["Photo"]],
      [null, 0, ["Photo"]],
    ]);

    const second = await reply("IMG_0002.jpg", "");
    await conversation("append", said(600, "you", "", 1, [`${second}-IMG_0002.jpg`]));
    const alone = mine().at(-1)!;
    expect(alone.querySelector<HTMLImageElement>('[data-phone-sent-photo="IMG_0002.jpg"] img')!.src).toBe("data:image/jpeg;base64,AA==");
    expect(alone.querySelector("p")).toBeNull();

    // A log the agent recorded beside no words and no image stands as its chip alone.
    await conversation("append", said(700, "you", "", 0, ["holder.log"]));
    const onlyLog = mine().at(-1)!;
    expect(onlyLog.querySelector('[data-phone-sent-file="holder.log"]')!.textContent).toContain("holder.log");
    expect(onlyLog.querySelector("p")).toBeNull();
  } finally {
    await unmount();
  }
});
