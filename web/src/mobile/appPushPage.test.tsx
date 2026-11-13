// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

// The page inside the app with only what the app's host provides substituted: Capacitor's bridge with a stand-in of
// the Android app's `HidePush` and `HideKeychain`, and the encrypted socket to the computer (the Noise relay is proved
// by the phone e2e). The page's own modules run as they ship: the paired computer read from the app's store, the
// connection, the Settings switch, the registration and the answer to it. A Playwright page cannot be the app here:
// Chromium refuses to load the app's `capacitor://` origin, which is what tells the page it is inside the app.
class FakeSocket {
  static all: FakeSocket[] = [];
  readyState = 0;
  sent: Record<string, unknown>[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(readonly host: { room: string }) {
    FakeSocket.all.push(this);
  }
  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.closed = true;
    this.readyState = 3;
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.();
  }
  receive(frame: object): void {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}
vi.mock("./relaySocket", () => ({ RelaySocket: FakeSocket }));

const ROOM = "r".repeat(43);
/** What the Android app's `HidePush` answers: Google's token, named by its service, and no environment. */
const FCM = { found: true, service: "fcm", token: "c".repeat(26) + ":APA91b" + "d".repeat(120), p256dh: "P".repeat(87), auth: "A".repeat(22) };
const HELLO = { type: "hello", mac_name: "Mac", phone_id: "phone-one", name: "Phone", vapid_public_key: "k", notifications: "unasked" };
let permission = "unknown";

let root: Root;
let host: HTMLElement;

beforeEach(() => {
  FakeSocket.all = [];
  permission = "unknown";
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(window.navigator, "language", "get").mockReturnValue("en-US");
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:", pathname: "/p/5/", search: `?host=${ROOM}`, hash: "", replace: vi.fn() });
  const native: Record<string, (options?: Record<string, unknown>) => unknown> = {
    "HideKeychain.list": () => ({ rooms: [ROOM] }),
    "HideKeychain.read": () => ({ found: true, noiseKey: "n".repeat(43), credential: "test-phone-credential" }),
    "HideKeychain.save": () => ({}),
    "HidePush.state": () => ({ permission }),
    "HidePush.enable": () => { permission = "granted"; return FCM; },
    "HidePush.registration": () => FCM,
    "HidePush.forget": () => ({}),
  };
  vi.stubGlobal("Capacitor", {
    nativePromise: async (plugin: string, method: string, options?: Record<string, unknown>) => {
      const call = native[`${plugin}.${method}`];
      if (!call) throw Object.assign(new Error(`${plugin}.${method}`), { code: "unimplemented" });
      return call(options);
    },
    addListener: () => ({ remove: () => undefined }),
  });
  localStorage.clear();
  localStorage.setItem("hide.phone.hosts.app.v1", JSON.stringify([{ room: ROOM, name: "Mac", protocol: 5, lastSeen: null }]));
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
  vi.resetModules();
});

/** A fresh page (its modules keep the socket and the stores), connected to the paired computer, on Settings. */
async function settingsOfConnectedApp() {
  vi.resetModules();
  const connection = await import("./connection");
  const { usePhone } = await import("./store");
  const { useAppPush } = await import("./appPush");
  const { SettingsScreen } = await import("./SettingsScreen");
  const { PhoneLanguageBoundary } = await import("./language");
  connection.start();
  await vi.waitFor(() => expect(FakeSocket.all).toHaveLength(1));
  const socket = FakeSocket.all[0]!;
  expect(socket.host.room).toBe(ROOM);
  socket.open();
  socket.receive(HELLO);
  socket.receive({ type: "meta", push_mode: "app_closed", other_phones: 0 });
  expect(usePhone.getState().connected).toBe(true);
  await act(async () => root.render(<><PhoneLanguageBoundary /><SettingsScreen /></>));
  const row = () => document.querySelector<HTMLElement>("[data-phone-notifications]")!;
  const turnOn = () => act(async () => row().querySelector<HTMLElement>('[role="switch"]')!.click());
  const registrations = () => socket.sent.filter((message) => String(message.type).endsWith("_registration"));
  return { connection, socket, usePhone, useAppPush, row, turnOn, registrations };
}

it("turns the Android app's notifications on by giving the computer Google's token and the key, and shows them on", async () => {
  const { socket, row, turnOn, registrations } = await settingsOfConnectedApp();
  expect(row().getAttribute("data-phone-notifications")).toBe("off");
  await turnOn();
  await vi.waitFor(() => expect(registrations()).toHaveLength(1));
  expect(registrations()[0]).toEqual({ type: "fcm_registration", token: FCM.token, key: { p256dh: FCM.p256dh, auth: FCM.auth } });
  await act(async () => socket.receive({ type: "push_state", notifications: "on" }));
  expect(row().getAttribute("data-phone-notifications")).toBe("on");
  expect(row().textContent).not.toContain("Update Hide on the computer");
});

it("keeps the session with a computer too old to know the registration and says that computer needs updating", async () => {
  const { socket, usePhone, useAppPush, row, turnOn, registrations } = await settingsOfConnectedApp();
  await turnOn();
  await vi.waitFor(() => expect(registrations()).toHaveLength(1));
  await act(async () => socket.receive({ type: "refused_request", request: "fcm_registration", reason: "not_allowed" }));
  expect(useAppPush.getState().outdated).toBe(true);
  expect(row().textContent).toContain("Update Hide on the computer");
  expect(socket.closed).toBe(false);
  expect(usePhone.getState().connected).toBe(true);
  expect(FakeSocket.all).toHaveLength(1);
});
