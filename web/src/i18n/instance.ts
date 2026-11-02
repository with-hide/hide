import type { i18n } from "i18next";
import { catalogs, english } from "./catalogs";
import { initializeEmbeddedI18n } from "./embedded";
import type { InterfaceLanguage } from "./locale";
import { validateCatalogs } from "./schema";
export { TranslationError } from "./embedded";

// Validate once per bundle, never per mounted screen.
validateCatalogs(english, catalogs);
export function initializeInterfaceI18n(selected: InterfaceLanguage): i18n {
  return initializeEmbeddedI18n(selected, catalogs);
}
export async function createInterfaceI18n(selected: InterfaceLanguage): Promise<i18n> {
  return initializeInterfaceI18n(selected);
}
