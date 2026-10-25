// The practice window (web `src/practice/`): the shell's own page on the
// daemon's origin, in a window of its own with no preload. The page stands in
// for this host from its address, so it needs nothing from the main process;
// it is only ever this one address, and the window neither navigates nor opens
// another, here or in the system's browser.

import { isInterfaceLanguage } from "../../../web/src/i18n/locale";

/** A chapter id as the core keeps progress for one (`practice_chapter_id_valid`). */
const CHAPTER_ID = /^[a-z0-9-]{1,32}$/;

/**
 * The practice page on `origin` in `language`, opening on `chapter` when one
 * is asked for and on the chapter list otherwise, or why a request for it is
 * refused.
 */
export function practiceUrl(origin: string, language: unknown, platform: string, chapter?: unknown): string | { refused: "language" | "chapter" } {
  if (!isInterfaceLanguage(language)) return { refused: "language" };
  if (chapter !== undefined && (typeof chapter !== "string" || !CHAPTER_ID.test(chapter))) return { refused: "chapter" };
  const url = new URL("/", origin);
  url.search = new URLSearchParams({ practice: "1", lang: language, ...(chapter === undefined ? {} : { chapter }), host: "electron", platform }).toString();
  return url.href;
}
