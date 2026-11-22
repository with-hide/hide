import { useLayoutEffect, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { initializeInterfaceI18n } from "./instance";
import { resolveInterfaceLanguage } from "./locale";

// One bounded, in-memory translator per page. Only the core owns a choice;
// neither the resolved system language nor the preference is cached locally.
// This module reads no shell state, so the phone page can import it alone.
export const clientI18n = initializeInterfaceI18n(resolveInterfaceLanguage(null, navigator.language).language);

/**
 * For text composed when an event happens, outside any render (a notice a
 * store or action module builds). The language is read at call time; prefer
 * keeping the key and its values in state and translating where it renders.
 */
export const translate: TFunction<"translation"> = clientI18n.getFixedT(null, "translation");

export function useInterfaceTranslation() {
  return useTranslation("translation", { i18n: clientI18n, useSuspense: false });
}

/**
 * Applies the core's explicit choice, or `fallback` (the language of the
 * window that shows another machine's screen), else the browser's primary
 * language while it is unset or unknown, to this page's translator and
 * document, and follows the browser's `languagechange`.
 */
export function useFollowInterfaceLanguage(preference: unknown, fallback: string | null = null): void {
  const [systemLanguage, setSystemLanguage] = useState(navigator.language);
  useLayoutEffect(() => {
    const changed = () => setSystemLanguage(navigator.language);
    window.addEventListener("languagechange", changed);
    return () => window.removeEventListener("languagechange", changed);
  }, []);
  const language = resolveInterfaceLanguage(preference, fallback ?? systemLanguage).language;
  useLayoutEffect(() => {
    if (clientI18n.language !== language) void clientI18n.changeLanguage(language);
    document.documentElement.lang = language;
  }, [language]);
}
