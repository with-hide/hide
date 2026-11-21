// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PhoneMessage } from "./protocol";

const sent: PhoneMessage[] = [];
let room: string | null = "r".repeat(43);
vi.mock("./connection", () => ({
  send: (message: PhoneMessage) => { sent.push(message); return true; },
  currentHost: () => (room ? { room } : null),
}));
vi.mock("./hosts", () => ({ hostStore: () => ({ list: async () => [{ room: "r".repeat(43) }, { room: "s".repeat(43) }] }) }));

const { appPushRow, leaveHostPush, refreshAppPush, syncAppPush, turnOffAppPush, turnOnAppPush, useAppPush } = await import("./appPush");
const { patch } = await import("./store");

const REGISTRATION = { found: true, token: "ab".repeat(32), environment: "sandbox", p256dh: "P".repeat(87), auth: "A".repeat(22) };
/** What the Android app's `HidePush` answers: Google's token, named by its service, and no environment. */
const FCM_REGISTRATION = { found: true, service: "fcm", token: "c".repeat(26) + ":APA91b" + "d".repeat(120), p256dh: "P".repeat(87), auth: "A".repeat(22) };
type Native = Record<string, (options?: Record<string, unknown>) => unknown>;
let native: Native;
const calls: string[] = [];

function install(methods: Partial<Native>) {
  native = {
    state: () => ({ permission: "granted" }),
    enable: () => REGISTRATION,
    registration: () => REGISTRATION,
    forget: () => ({}),
    demoSample: () => ({}),
    ...methods,
  } as Native;
  vi.stubGlobal("Capacitor", {
    isPluginAvailable: (name: string) => name === "HidePush",
    nativePromise: async (plugin: string, method: string, options?: Record<string, unknown>) => {
      calls.push(`${plugin}.${method}`);
      const result = native[method]!(options);
      if (result instanceof Error) throw result;
      return result;
    },
    addListener: () => ({ remove: () => undefined }),
  });
}
const rejection = (code: string) => Object.assign(new Error(code), { code });

beforeEach(() => {
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:" });
  localStorage.clear();
  sent.length = 0;
  calls.length = 0;
  room = "r".repeat(43);
  useAppPush.setState({ pref: false, demo: false, permission: "unknown", asking: false, failed: false, outdated: false });
  patch({ demo: false, notifications: "unasked" });
  install({});
});
afterEach(() => vi.unstubAllGlobals());

describe("the app's notification row", () => {
  const base = { pref: false, demo: false, permission: "unknown" as const, asking: false, failed: false };

  it("is off until the person turned it on and iOS allows it", () => {
    expect(appPushRow(base, false)).toBe("off");
    expect(appPushRow({ ...base, pref: true }, false)).toBe("off");
    expect(appPushRow({ ...base, pref: true, permission: "granted" }, false)).toBe("on");
  });

  it("tells a refusal by iOS over a kept preference, and the question while it is open", () => {
    expect(appPushRow({ ...base, pref: true, permission: "denied" }, false)).toBe("refused");
    expect(appPushRow({ ...base, permission: "denied", asking: true }, false)).toBe("asking");
    expect(appPushRow({ ...base, failed: true }, false)).toBe("failed");
  });

  it("keeps the demo's switch apart from the real preference", () => {
    expect(appPushRow({ ...base, pref: true, permission: "granted" }, true)).toBe("off");
    expect(appPushRow({ ...base, demo: true, permission: "granted" }, true)).toBe("on");
  });
});

describe("turning notifications on", () => {
  it("asks iOS, keeps the choice and gives the computer the token and the key, never the private half", async () => {
    await turnOnAppPush();
    expect(sent).toEqual([{ type: "apns_registration", token: REGISTRATION.token, environment: "sandbox", key: { p256dh: REGISTRATION.p256dh, auth: REGISTRATION.auth } }]);
    expect(localStorage.getItem("hide.phone.app-push")).toBe("on");
    expect(useAppPush.getState()).toMatchObject({ pref: true, permission: "granted", asking: false, failed: false });
  });

  it("gives the computer Google's token and the key when the Android app answers with an FCM registration", async () => {
    install({ enable: () => FCM_REGISTRATION });
    await turnOnAppPush();
    expect(sent).toEqual([{ type: "fcm_registration", token: FCM_REGISTRATION.token, key: { p256dh: FCM_REGISTRATION.p256dh, auth: FCM_REGISTRATION.auth } }]);
    expect(useAppPush.getState()).toMatchObject({ pref: true, permission: "granted", failed: false });
  });

  it("sends nothing and offers another try when the app's answer has no shape it knows", async () => {
    for (const answer of [{ ...FCM_REGISTRATION, service: "gcm" }, { ...REGISTRATION, environment: undefined }, { ...FCM_REGISTRATION, token: undefined }]) {
      install({ enable: () => answer });
      await turnOnAppPush();
      expect(sent).toEqual([]);
      expect(useAppPush.getState()).toMatchObject({ failed: true });
    }
  });

  it("stays off and says iOS refused when the person declines the question", async () => {
    install({ enable: () => rejection("denied") });
    await turnOnAppPush();
    expect(sent).toEqual([]);
    expect(localStorage.getItem("hide.phone.app-push")).toBeNull();
    expect(useAppPush.getState()).toMatchObject({ pref: false, permission: "denied", asking: false });
  });

  it("stays off and offers another try when Apple gives no token", async () => {
    install({ enable: () => rejection("token_timeout") });
    await turnOnAppPush();
    expect(localStorage.getItem("hide.phone.app-push")).toBeNull();
    expect(useAppPush.getState()).toMatchObject({ pref: false, failed: true, asking: false });
  });

  it("posts only the local sample in the demo: no key, no registration, nothing sent", async () => {
    let words: unknown;
    install({ demoSample: (options) => { words = options?.words; return {}; } });
    patch({ demo: true });
    await turnOnAppPush();
    expect(calls).toEqual(["HidePush.demoSample"]);
    // The sample carries a real notice's fields for the first sample agent (title, status, question, project); the phone words the body.
    const { translate } = await import("../i18n/translator");
    expect(words).toEqual({ title: translate("mobile.demo.a1.title"), status_code: "question", state: "needs_you", detail: translate("mobile.demo.a1.ask"), place: "demo · fix/review" });
    expect(sent).toEqual([]);
    expect(localStorage.getItem("hide.phone.app-push")).toBeNull();
    expect(useAppPush.getState().demo).toBe(true);
  });
});

describe("a computer that connects", () => {
  it("registers again on every connect while the switch is on", async () => {
    localStorage.setItem("hide.phone.app-push", "on");
    useAppPush.setState({ pref: true });
    await syncAppPush();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ type: "apns_registration", token: REGISTRATION.token });
  });

  it("registers the Android app's FCM token again on every connect", async () => {
    localStorage.setItem("hide.phone.app-push", "on");
    useAppPush.setState({ pref: true });
    install({ enable: () => FCM_REGISTRATION });
    await syncAppPush();
    expect(sent).toEqual([{ type: "fcm_registration", token: FCM_REGISTRATION.token, key: { p256dh: FCM_REGISTRATION.p256dh, auth: FCM_REGISTRATION.auth } }]);
  });

  it("unregisters when the switch is off but the computer still holds a registration", async () => {
    patch({ notifications: "on" });
    await syncAppPush();
    expect(sent).toEqual([{ type: "push_permission", state: "denied" }]);
    expect(calls).toContain("HidePush.forget");
  });

  it("says nothing to a computer that holds no registration while the switch is off", async () => {
    await syncAppPush();
    expect(sent).toEqual([]);
  });

  it("tells the computer the permission went away in iOS and keeps the preference", async () => {
    localStorage.setItem("hide.phone.app-push", "on");
    useAppPush.setState({ pref: true });
    patch({ notifications: "on" });
    install({ state: () => ({ permission: "denied" }) });
    await syncAppPush();
    expect(sent).toEqual([{ type: "push_permission", state: "denied" }]);
    expect(localStorage.getItem("hide.phone.app-push")).toBe("on");
    expect(useAppPush.getState().permission).toBe("denied");
  });

  it("does nothing outside the app or in the demo", async () => {
    patch({ demo: true });
    await syncAppPush();
    vi.stubGlobal("location", { ...window.location, protocol: "https:" });
    patch({ demo: false });
    await syncAppPush();
    expect(sent).toEqual([]);
    expect(calls).toEqual([]);
  });
});

describe("turning notifications off and removing a computer", () => {
  it("tells the connected computer first and deletes every computer's key", async () => {
    localStorage.setItem("hide.phone.app-push", "on");
    useAppPush.setState({ pref: true });
    const forgotten: unknown[] = [];
    install({ forget: (options) => { forgotten.push(options?.room); return {}; } });
    await turnOffAppPush();
    expect(sent).toEqual([{ type: "push_permission", state: "denied" }]);
    expect(forgotten).toEqual(["r".repeat(43), "s".repeat(43)]);
    expect(localStorage.getItem("hide.phone.app-push")).toBeNull();
  });

  it("unregisters the computer being removed only when it is the connected one", async () => {
    await leaveHostPush("s".repeat(43));
    expect(sent).toEqual([]);
    await leaveHostPush("r".repeat(43));
    expect(sent).toEqual([{ type: "push_permission", state: "denied" }]);
    expect(calls.filter((call) => call === "HidePush.forget")).toHaveLength(2);
  });

  it("reads what iOS allows without asking", async () => {
    install({ state: () => ({ permission: "denied" }) });
    await refreshAppPush();
    expect(useAppPush.getState().permission).toBe("denied");
    expect(calls).toEqual(["HidePush.state"]);
  });
});
