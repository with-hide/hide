// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS, SETTINGS_KEY, choosePhoneSettings, followPhoneTheme, readSettings, usePhoneSettings, writeSettings } from "./settings";

const reader = (value: string | null) => ({ getItem: () => value });

describe("reading the phone's settings", () => {
  it("is the defaults when nothing is stored", () => {
    expect(readSettings(reader(null))).toEqual({ language: null, theme: "system" });
  });

  it("reads a stored choice back", () => {
    expect(readSettings(reader(JSON.stringify({ language: "ja", theme: "light" })))).toEqual({ language: "ja", theme: "light" });
  });

  it("falls back per field on an unknown value, and wholly on unreadable or throwing storage", () => {
    expect(readSettings(reader(JSON.stringify({ language: "fr", theme: "dark" })))).toEqual({ language: null, theme: "dark" });
    expect(readSettings(reader(JSON.stringify({ language: "ko", theme: "sepia" })))).toEqual({ language: "ko", theme: "system" });
    expect(readSettings(reader("not json"))).toEqual(DEFAULT_SETTINGS);
    expect(readSettings(reader("42"))).toEqual(DEFAULT_SETTINGS);
    expect(readSettings({ getItem: () => { throw new Error("denied"); } })).toEqual(DEFAULT_SETTINGS);
  });

  it("reports whether a write was kept", () => {
    const items = new Map<string, string>();
    expect(writeSettings({ setItem: (key, value) => void items.set(key, value) }, { language: "zh-CN", theme: "dark" })).toBe(true);
    expect(readSettings({ getItem: (key) => items.get(key) ?? null })).toEqual({ language: "zh-CN", theme: "dark" });
    expect(writeSettings({ setItem: () => { throw new Error("full"); } }, DEFAULT_SETTINGS)).toBe(false);
  });
});

describe("choosing", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    vi.restoreAllMocks();
    choosePhoneSettings(DEFAULT_SETTINGS);
  });

  it("applies at once and survives a reload of the page's storage read", () => {
    choosePhoneSettings({ language: "ko" });
    choosePhoneSettings({ theme: "dark" });
    expect(usePhoneSettings.getState()).toMatchObject({ language: "ko", theme: "dark", kept: true });
    expect(readSettings(localStorage)).toEqual({ language: "ko", theme: "dark" });
    expect(localStorage.getItem(SETTINGS_KEY)).not.toBeNull();
  });

  it("still applies for this visit when the browser refuses the write, and says it was not kept", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("blocked"); });
    choosePhoneSettings({ theme: "light" });
    expect(usePhoneSettings.getState()).toMatchObject({ theme: "light", kept: false });
  });
});

describe("the page's dark class", () => {
  let dark = false;
  let notify: () => void = () => {};
  beforeEach(() => {
    dark = false;
    vi.stubGlobal("matchMedia", () => ({
      get matches() { return dark; },
      addEventListener: (_: string, listener: () => void) => { notify = listener; },
    }));
    choosePhoneSettings(DEFAULT_SETTINGS);
    document.documentElement.classList.remove("dark");
    followPhoneTheme();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    choosePhoneSettings(DEFAULT_SETTINGS);
  });

  it("follows the system by default, in both directions", () => {
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    dark = true; notify();
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    dark = false; notify();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("holds an explicit choice against the system, and returns to following it", () => {
    choosePhoneSettings({ theme: "dark" });
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    dark = true; choosePhoneSettings({ theme: "light" });
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    choosePhoneSettings({ theme: "system" });
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});
