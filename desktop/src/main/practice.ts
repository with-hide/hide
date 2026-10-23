// The practice window (web `src/practice/`): the shell's own page on the
// daemon's origin, in a window of its own with no preload. The page stands in
// for this host from its address, so it needs nothing from the main process;
// it is only ever this one address, and the window neither navigates nor opens
// another, here or in the system's browser.

import { isInterfaceLanguage } from "../../../web/src/i18n/locale";

/** The practice page on `origin` in `language`, or why a request for it is refused. */
export function practiceUrl(origin: string, language: unknown, platform: string): string | { refused: "language" } {
  if (!isInterfaceLanguage(language)) return { refused: "language" };
  const url = new URL("/", origin);
  url.search = new URLSearchParams({ practice: "1", lang: language, host: "electron", platform }).toString();
  return url.href;
}
