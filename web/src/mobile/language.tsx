import { useEffect } from "react";
import { useFollowInterfaceLanguage, useInterfaceTranslation } from "../i18n/translator";
import { postWords } from "./push";
import { rememberMacLanguage, usePhoneSettings } from "./settings";
import { usePhone } from "./store";

/**
 * The phone's language is the one chosen in its own settings; while that
 * follows the Mac, the core's explicit choice from the last `agents` frame, or
 * the phone's own language before that frame and while it is unset.
 * That Mac's choice is kept for the start page (`MAC_LANGUAGE_KEY`), so adding a computer speaks the same language.
 * The service worker is told the notification words on load and on every
 * language change; a new subscription tells it again (push.ts).
 */
export function PhoneLanguageBoundary() {
  const own = usePhoneSettings((state) => state.language);
  const core = usePhone((state) => state.interfaceLanguage);
  // Known once the Mac's first list arrived; the sample host's is not a Mac's.
  const known = usePhone((state) => state.groups !== null && !state.demo);
  useFollowInterfaceLanguage(own ?? core);
  useEffect(() => {
    if (!known) return;
    try {
      rememberMacLanguage(localStorage, core);
    } catch (error) {
      console.warn(JSON.stringify({ event: "phone.mac_language_unsaved", error: String(error) }));
    }
  }, [known, core]);
  const { i18n } = useInterfaceTranslation();
  useEffect(() => {
    void postWords();
  }, [i18n.language]);
  return null;
}
