// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clientI18n } from "../i18n/translator";
import { PhoneLanguageBoundary } from "./language";
import { SettingsScreen } from "./SettingsScreen";
import { useAppPush } from "./appPush";
import { patch } from "./store";

let root: Root;
let host: HTMLElement;
const opened: string[] = [];
let permission = "unknown";

const row = () => document.querySelector<HTMLElement>("[data-phone-notifications]")!;
const toggle = () => row().querySelector<HTMLElement>('[role="switch"]')!;

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(window.navigator, "language", "get").mockReturnValue("en-US");
  vi.stubGlobal("location", { ...window.location, protocol: "capacitor:" });
  vi.stubGlobal("Capacitor", {
    nativePromise: async (_plugin: string, method: string) => {
      if (method === "state") return { permission };
      if (method === "openSettings") { opened.push("settings"); return {}; }
      if (method === "demoSample") return {};
      throw new Error("unexpected " + method);
    },
    addListener: () => ({ remove: () => undefined }),
  });
  localStorage.clear();
  opened.length = 0;
  permission = "unknown";
  useAppPush.setState({ pref: false, demo: false, permission: "unknown", asking: false, failed: false, outdated: false });
  patch({ demo: false });
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  await clientI18n.changeLanguage("en");
});

const render = () => act(async () => root.render(<><PhoneLanguageBoundary /><SettingsScreen /></>));

describe("the app's notification row in Settings", () => {
  it("is one switch, off, with a line saying what it is for, and no sentence that it cannot be turned on", async () => {
    await render();
    expect(row().getAttribute("data-phone-notifications")).toBe("off");
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(row().textContent).toContain("Hear when an agent needs your answer");
    expect(document.body.textContent).not.toContain("can't be turned on");
  });

  it("shows on when iOS allows it and the person turned it on", async () => {
    permission = "granted";
    useAppPush.setState({ pref: true });
    await render();
    expect(row().getAttribute("data-phone-notifications")).toBe("on");
    expect(toggle().getAttribute("aria-checked")).toBe("true");
  });

  it("tells a refusal by iOS, disables the switch and opens iOS Settings from a button", async () => {
    permission = "denied";
    await render();
    expect(row().getAttribute("data-phone-notifications")).toBe("refused");
    expect(toggle().hasAttribute("disabled")).toBe(true);
    expect(row().textContent).toContain("off in iOS");
    await act(async () => document.querySelector<HTMLElement>("[data-phone-notifications-settings]")!.click());
    expect(opened).toEqual(["settings"]);
  });

  it("says the computer is too old when it refused the registration", async () => {
    permission = "granted";
    useAppPush.setState({ pref: true, outdated: true });
    await render();
    expect(row().textContent).toContain("Update Hide on the computer");
  });

  it("is worded in Korean, and the iOS-Settings button too", async () => {
    permission = "denied";
    await clientI18n.changeLanguage("ko");
    await render();
    await act(async () => patch({ interfaceLanguage: "ko" }));
    expect(row().textContent).toContain("iOS에서 Hide 알림이 꺼져 있어요");
    expect(document.querySelector("[data-phone-notifications-settings]")?.textContent).toBe("iOS 설정 열기");
  });
});
