// The chapters' words: every file in `web/src/i18n/resources/practice/`, each
// default-exporting one chapter's words in every language, every key under
// `practice.<file name>.`. They live outside the shared catalog because only
// this window shows them, and the desktop host bundles that catalog without
// Vite. A chapter's words are added by adding its file; nothing here names one.

import type { i18n } from "i18next";
import { INTERFACE_LANGUAGES, type InterfaceLanguage } from "../i18n/locale";
import { validateCatalogs, type Catalogs, type MessageSchema } from "../i18n/schema";

const files = import.meta.glob<Catalogs<MessageSchema>>("../i18n/resources/practice/*.ts", { eager: true, import: "default" });

/** Every chapter file's words joined, each file checked like the shared catalog; a key outside its file's chapter would overwrite another's. */
function joined(modules: Record<string, Catalogs<MessageSchema>>): Catalogs<MessageSchema> {
  const words = Object.fromEntries(INTERFACE_LANGUAGES.map((language) => [language, {}])) as Record<InterfaceLanguage, Record<string, string>>;
  for (const [path, catalogs] of Object.entries(modules)) {
    const prefix = `practice.${path.slice(path.lastIndexOf("/") + 1, -".ts".length)}.`;
    validateCatalogs(catalogs.en, catalogs);
    const stray = Object.keys(catalogs.en).find((key) => !key.startsWith(prefix));
    if (stray) throw new Error(`Practice words ${stray} are in ${path}`);
    for (const language of INTERFACE_LANGUAGES) Object.assign(words[language], catalogs[language]);
  }
  return words;
}

export const CHAPTER_WORDS: Catalogs<MessageSchema> = joined(files);

/** Gives `instance` every chapter's words in every language. */
export function addChapterWords(instance: i18n): void {
  for (const language of INTERFACE_LANGUAGES) instance.addResources(language, "translation", CHAPTER_WORDS[language]);
}
