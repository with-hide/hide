// The phone's own settings (PRD mobile-companion-ui, mission-34 node settings):
// the language and light or dark this phone shows, kept in this browser only.
// The Mac's language and the system's appearance stay the defaults ("follow");
// nothing here is sent to the Mac, and the desktop's settings are not read.
// The loader (start page) and the app share this module, so a choice is right
// on the first paint of either.

import { create } from "zustand";
import { isInterfaceLanguage, resolveInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";
import { resolveTheme, type ThemeChoice } from "../theme";
import { appHost } from "./appHost";
import { setAppearance } from "./appBridge";

/** One key for both choices; the loader and every protocol bundle of an origin read the same value. */
export const SETTINGS_KEY = "hide.phone.settings";

export type PhoneSettings = {
  /** An explicit language, or null to follow the Mac's (the core's `interface_language`, else this phone's own). */
  language: InterfaceLanguage | null;
  /** `system` follows `prefers-color-scheme`. */
  theme: ThemeChoice;
};

export const DEFAULT_SETTINGS: PhoneSettings = { language: null, theme: "system" };

function isThemeChoice(value: unknown): value is ThemeChoice {
  return value === "system" || value === "light" || value === "dark";
}

/**
 * What storage holds, field by field: a missing, unreadable or unknown value
 * is that field's default, so a bundle that does not know a newer value
 * (and a browser whose storage throws) still opens on a working page.
 */
export function readSettings(storage: Pick<Storage, "getItem">): PhoneSettings {
  let stored: unknown;
  try {
    const text = storage.getItem(SETTINGS_KEY);
    stored = text === null ? null : JSON.parse(text);
  } catch {
    return DEFAULT_SETTINGS;
  }
  const record = typeof stored === "object" && stored !== null ? (stored as Record<string, unknown>) : {};
  return {
    language: isInterfaceLanguage(record.language) ? record.language : DEFAULT_SETTINGS.language,
    theme: isThemeChoice(record.theme) ? record.theme : DEFAULT_SETTINGS.theme,
  };
}

/** Whether the browser kept the settings. */
export function writeSettings(storage: Pick<Storage, "setItem">, settings: PhoneSettings): boolean {
  try {
    storage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    return true;
  } catch {
    return false;
  }
}

function browserSettings(): PhoneSettings {
  try {
    return readSettings(localStorage);
  } catch {
    return DEFAULT_SETTINGS;
  }
}

type SettingsState = PhoneSettings & {
  /** False after a write the browser refused: the choice holds for this visit only. */
  kept: boolean;
};

export const usePhoneSettings = create<SettingsState>(() => ({ ...browserSettings(), kept: true }));

/** A choice applies at once to every subscriber and is written to this browser. */
export function choosePhoneSettings(change: Partial<PhoneSettings>): void {
  const { language, theme } = { ...usePhoneSettings.getState(), ...change };
  let kept = true;
  try {
    kept = writeSettings(localStorage, { language, theme });
  } catch {
    kept = false;
  }
  usePhoneSettings.setState({ language, theme, kept });
}

/**
 * The explicit language of the last Mac this phone showed (its core's `interface_language`), so the start page, which has no Mac
 * to ask, speaks the language the app last spoke: the phone's own choice, else this, else the phone's language. Absent while that
 * Mac follows its system, which the app reads as the phone's own language too.
 */
export const MAC_LANGUAGE_KEY = "hide.phone.macLanguage";

export function readMacLanguage(storage: Pick<Storage, "getItem">): InterfaceLanguage | null {
  try {
    const stored = storage.getItem(MAC_LANGUAGE_KEY);
    return isInterfaceLanguage(stored) ? stored : null;
  } catch {
    return null;
  }
}

/** Kept from each `agents` frame; written only when it changed, since a frame arrives with every change of the list. */
export function rememberMacLanguage(storage: Pick<Storage, "getItem" | "setItem" | "removeItem">, language: string | null): void {
  const next = isInterfaceLanguage(language) ? language : null;
  if (readMacLanguage(storage) === next) return;
  try {
    if (next === null) storage.removeItem(MAC_LANGUAGE_KEY);
    else storage.setItem(MAC_LANGUAGE_KEY, next);
  } catch (error) {
    // The start page then speaks the phone's language, as before this was kept.
    console.warn(JSON.stringify({ event: "phone.mac_language_unsaved", error: String(error) }));
  }
}

/** The start page's language, resolved as the app resolves its own (`PhoneLanguageBoundary`). */
export function startLanguage(storage: Pick<Storage, "getItem">, systemLanguage: string): InterfaceLanguage {
  return resolveInterfaceLanguage(usePhoneSettings.getState().language ?? readMacLanguage(storage), systemLanguage).language;
}

/**
 * Puts the `dark` class on the page from the choice and the system's
 * appearance, and keeps it right as either changes. The one theme function of
 * the loader and the app.
 */
export function followPhoneTheme(): void {
  const scheme = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    const { theme } = usePhoneSettings.getState();
    document.documentElement.classList.toggle("dark", resolveTheme(theme, scheme.matches) === "dark");
    // The app's status bar, keyboard and system colors follow the choice; `system` hands the decision back to the system.
    if (appHost()) void setAppearance(theme).catch((error: unknown) => console.warn("phone.appearance_failed", String(error)));
  };
  apply();
  scheme.addEventListener("change", apply);
  usePhoneSettings.subscribe(apply);
}
