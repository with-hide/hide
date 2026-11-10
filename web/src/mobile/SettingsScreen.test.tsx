// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clientI18n, translate } from "../i18n/translator";
import { PhoneLanguageBoundary } from "./language";
import { SettingsScreen } from "./SettingsScreen";
import { DEFAULT_SETTINGS, choosePhoneSettings, usePhoneSettings } from "./settings";
import { patch } from "./store";

let root: Root;
let host: HTMLElement;

const setting = (name: string) => document.querySelector<HTMLElement>(`[data-phone-setting="${name}"]`)!;
const option = (kind: "language" | "theme", value: string) => document.querySelector<HTMLElement>(`[data-phone-${kind}-option="${value}"]`)!;
/** A value row opens its sheet, whose rows are only in the page while it is open. */
const open = async (name: "language" | "theme") => { await act(async () => setting(name).click()); };

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(window.navigator, "language", "get").mockReturnValue("en-US");
  localStorage.clear();
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
  await act(async () => root.render(<><PhoneLanguageBoundary /><SettingsScreen /></>));
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  choosePhoneSettings(DEFAULT_SETTINGS);
  patch({ interfaceLanguage: null });
  vi.restoreAllMocks();
  await clientI18n.changeLanguage("en");
});

describe("the phone's Settings tab", () => {
  it("shows language and appearance as one-line values, on the defaults: follow the Mac and follow the system", async () => {
    expect(setting("language").textContent).toContain("Follow the Mac");
    expect(setting("theme").textContent).toContain("Follow the system");
    await open("language");
    expect(document.querySelector("[data-phone-settings-language]")?.getAttribute("data-phone-settings-language")).toBe("follow");
  });

  it("offers each language by its own name in the sheet the row opens", async () => {
    await open("language");
    for (const name of ["한국어", "English", "简体中文", "日本語"]) expect(document.body.textContent).toContain(name);
  });

  it("changes the language at once, over the Mac's, keeps it and closes the sheet", async () => {
    await act(async () => patch({ interfaceLanguage: "ja" }));
    expect(translate("mobile.group.done")).toBe("未読");
    await open("language");
    await act(async () => option("language", "ko").click());
    expect(document.querySelector("[data-phone-settings-sheet]")).toBeNull();
    expect(translate("mobile.group.done")).toBe("읽을 것");
    expect(usePhoneSettings.getState().language).toBe("ko");
    expect(JSON.parse(localStorage.getItem("hide.phone.settings")!)).toEqual({ language: "ko", theme: "system" });
    expect(document.body.textContent).toContain("이 폰에만 저장돼요");
    await open("language");
    await act(async () => option("language", "follow").click());
    expect(translate("mobile.group.done")).toBe("未読");
  });

  it("changes the appearance at once and keeps it", async () => {
    await open("theme");
    await act(async () => option("theme", "dark").click());
    expect(usePhoneSettings.getState().theme).toBe("dark");
    expect(JSON.parse(localStorage.getItem("hide.phone.settings")!).theme).toBe("dark");
  });

  it("says so when the browser cannot keep the choice", async () => {
    await open("theme");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
    await act(async () => option("theme", "light").click());
    expect(document.querySelector("[data-phone-settings-note]")?.getAttribute("data-phone-settings-note")).toBe("not-kept");
    expect(usePhoneSettings.getState().theme).toBe("light");
  });

  it("keeps the order: computers, notifications, display, about; and opens the pages outside the app", () => {
    const sections = [...document.querySelectorAll("[data-phone-settings-section]")].map((node) => node.getAttribute("data-phone-settings-section"));
    expect(sections).toEqual(["computers", "notifications", "display", "about"]);
    for (const [name, href] of [["privacy", "https://withhide.dev/privacy"], ["support", "https://withhide.dev/support"]] as const) {
      const link = setting(name) as HTMLAnchorElement;
      expect(link.href).toBe(href);
      expect(link.target).toBe("_blank");
    }
    expect(setting("protocol").textContent).toMatch(/\d+$/);
  });

  it("is the fourth place of the bottom bar", () => {
    const tabs = [...document.querySelectorAll("[data-phone-tab]")].map((node) => node.getAttribute("data-phone-tab"));
    expect(tabs).toEqual(["agents", "missions", "prs", "settings"]);
    expect(document.querySelector("[data-phone-tab=settings]")?.getAttribute("aria-current")).toBe("page");
  });
});
