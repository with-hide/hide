import type { TFunction } from "i18next";
import type { MessageKey } from "./i18n/catalogs";
import { githubReasonsEnglish } from "./i18n/resources/githubReasons";

/** A `gh` lookup's failure category in the viewer's language; one this build does not word reads as the general line. */
export function githubFailureText(category: string, t: TFunction<"translation">): string {
  const key = `githubReason.${category}`;
  return t(key in githubReasonsEnglish ? (key as MessageKey) : "githubReason.other");
}
