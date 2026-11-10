import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { initializeEmbeddedI18n } from "../i18n/embedded";
import { resolveInterfaceLanguage, INTERFACE_LANGUAGES } from "../i18n/locale";
import { validateCatalogs, type Catalogs } from "../i18n/schema";
import { mobileCatalogs, mobileEnglish } from "../i18n/resources/mobile";
import { mobileRelayCatalogs, mobileRelayEnglish } from "../i18n/resources/mobileRelay";
import { mobileDemoCatalogs, mobileDemoEnglish } from "../i18n/resources/mobileDemo";
import { commonCatalogs, commonEnglish } from "../i18n/resources/common";
import { usePhoneSettings } from "./settings";

// The loader uses the same messages and translation policy, without shipping desktop catalogs.
const english = { ...commonEnglish, ...mobileEnglish, ...mobileRelayEnglish, ...mobileDemoEnglish };
const catalogs = Object.fromEntries(INTERFACE_LANGUAGES.map((language) => [language,
  { ...commonCatalogs[language], ...mobileCatalogs[language], ...mobileRelayCatalogs[language], ...mobileDemoCatalogs[language] },
])) as Catalogs<typeof english>;
validateCatalogs(english, catalogs);
// The start page has no Mac yet: this phone's own choice, else its language.
const language = () => resolveInterfaceLanguage(usePhoneSettings.getState().language, navigator.language).language;
const instance = initializeEmbeddedI18n(language(), catalogs);

export function useLoaderTranslation() {
  useEffect(() => {
    const follow = () => {
      const selected = language();
      document.documentElement.lang = selected;
      if (instance.language !== selected) void instance.changeLanguage(selected);
    };
    follow();
    window.addEventListener("languagechange", follow);
    return () => window.removeEventListener("languagechange", follow);
  }, []);
  return useTranslation("translation", { i18n: instance, useSuspense: false });
}
