import { useEffect } from "react";
import { useFollowInterfaceLanguage, useInterfaceTranslation } from "../i18n/translator";
import { postWords } from "./push";
import { usePhoneSettings } from "./settings";
import { usePhone } from "./store";

/**
 * The phone's language is the one chosen in its own settings; while that
 * follows the Mac, the core's explicit choice from the last `agents` frame, or
 * the phone's own language before that frame and while it is unset.
 * The service worker is told the notification words on load and on every
 * language change; a new subscription tells it again (push.ts).
 */
export function PhoneLanguageBoundary() {
  const own = usePhoneSettings((state) => state.language);
  const core = usePhone((state) => state.interfaceLanguage);
  useFollowInterfaceLanguage(own ?? core);
  const { i18n } = useInterfaceTranslation();
  useEffect(() => {
    void postWords();
  }, [i18n.language]);
  return null;
}
