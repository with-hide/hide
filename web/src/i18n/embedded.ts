import { createInstance, type FormatterModule, type i18n } from "i18next";
import { formatNumber } from "./format";
import { INTERFACE_LANGUAGES, requireInterfaceLanguage, type InterfaceLanguage } from "./locale";
import type { Catalogs, MessageSchema } from "./schema";

/** Failures carry no inserted values, user text, or unknown caller keys. */
export class TranslationError extends Error {
  constructor(readonly kind: "missing_translation" | "missing_interpolation" | "invalid_interpolation") {
    super(kind);
    this.name = "TranslationError";
  }
}

/** i18next's formatter extension reuses our four bounded Intl bundles. */
const interfaceFormatter: FormatterModule = {
  type: "formatter",
  // The formatter is stateless; initialization allocates no resources.
  init: () => {},
  add: () => { throw new Error("unsupported_interface_formatter_registration"); },
  addCached: () => { throw new Error("unsupported_interface_formatter_registration"); },
  format: (value: unknown, _format, locale) => {
    if (typeof value === "number") return formatNumber(requireInterfaceLanguage(locale), value);
    if (typeof value === "string") return value;
    if (value === undefined) throw new TranslationError("missing_interpolation");
    throw new TranslationError("invalid_interpolation");
  },
};

/** Embedded resources initialize synchronously, before the first render. */
export function initializeEmbeddedI18n(selected: InterfaceLanguage, catalogs: Catalogs<MessageSchema>): i18n {
  const language = requireInterfaceLanguage(selected);
  const instance = createInstance().use(interfaceFormatter);
  // Also guard callers of the library's public language-changing API.
  instance.on("languageChanging", requireInterfaceLanguage);
  void instance.init({
    lng: language,
    supportedLngs: [...INTERFACE_LANGUAGES],
    load: "currentOnly",
    fallbackLng: false,
    ns: ["translation"],
    defaultNS: "translation",
    keySeparator: false,
    nsSeparator: false,
    resources: {
      en: { translation: catalogs.en },
      ko: { translation: catalogs.ko },
      "zh-CN": { translation: catalogs["zh-CN"] },
      ja: { translation: catalogs.ja },
    },
    initAsync: false,
    returnNull: false,
    returnEmptyString: false,
    returnObjects: false,
    saveMissing: false,
    parseMissingKeyHandler: () => {
      throw new TranslationError("missing_translation");
    },
    missingInterpolationHandler: () => {
      throw new TranslationError("missing_interpolation");
    },
    interpolation: {
      // Consumers render text; React escapes at its DOM boundary.
      escapeValue: false,
      // A user name containing {{...}} or $t(...) remains literal data.
      skipOnVariables: true,
      alwaysFormat: true,
    },
  });
  if (!instance.isInitialized) throw new Error("interface_i18n_not_initialized");
  return instance;
}
