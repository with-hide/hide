// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clientI18n, translate } from "../i18n/translator";
import { PhoneLanguageBoundary } from "./language";
import { SettingsSheet } from "./SettingsSheet";
import { DEFAULT_SETTINGS, choosePhoneSettings, usePhoneSettings } from "./settings";
import { patch } from "./store";

let root: Root;
let host: HTMLElement;

const option = (kind: "language" | "theme", value: string) => document.querySelector<HTMLElement>(`[data-phone-${kind}-option="${value}"]`)!;

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(window.navigator, "language", "get").mockReturnValue("en-US");
  localStorage.clear();
  host = document.body.appendChild(document.createElement("div"));
  root = createRoot(host);
  await act(async () => root.render(<><PhoneLanguageBoundary /><SettingsSheet defaultOpen /></>));
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  choosePhoneSettings(DEFAULT_SETTINGS);
  patch({ interfaceLanguage: null });
  vi.restoreAllMocks();
  await clientI18n.changeLanguage("en");
});

describe("the phone's settings sheet", () => {
  it("opens on the defaults: follow the Mac and follow the system", () => {
    expect(document.querySelector("[data-phone-settings-language]")?.getAttribute("data-phone-settings-language")).toBe("follow");
    expect(document.querySelector("[data-phone-settings-theme]")?.getAttribute("data-phone-settings-theme")).toBe("system");
    expect(document.body.textContent).toContain("Follow the Mac");
  });

  it("offers each language by its own name", () => {
    for (const name of ["한국어", "English", "简体中文", "日本語"]) expect(document.body.textContent).toContain(name);
  });

  it("changes the language at once, over the Mac's, and keeps it", async () => {
    await act(async () => patch({ interfaceLanguage: "ja" }));
    expect(translate("mobile.group.done")).toBe("未読");
    await act(async () => option("language", "ko").click());
    expect(translate("mobile.group.done")).toBe("읽을 것");
    expect(usePhoneSettings.getState().language).toBe("ko");
    expect(JSON.parse(localStorage.getItem("hide.phone.settings")!)).toEqual({ language: "ko", theme: "system" });
    expect(document.body.textContent).toContain("이 폰에만 저장돼요");
    await act(async () => option("language", "follow").click());
    expect(translate("mobile.group.done")).toBe("未読");
  });

  it("changes the appearance at once and keeps it", async () => {
    await act(async () => option("theme", "dark").click());
    expect(usePhoneSettings.getState().theme).toBe("dark");
    expect(JSON.parse(localStorage.getItem("hide.phone.settings")!).theme).toBe("dark");
  });

  it("says so when the browser cannot keep the choice", async () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
    await act(async () => option("theme", "light").click());
    expect(document.querySelector("[data-phone-settings-note]")?.getAttribute("data-phone-settings-note")).toBe("not-kept");
    expect(usePhoneSettings.getState().theme).toBe("light");
  });
});
