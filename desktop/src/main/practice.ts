// The practice window (web `src/practice/`): the shell's own page on the
// daemon's origin, in a window of its own with no preload. The page stands in
// for this host from its address, so it needs nothing from the main process;
// it is only ever this one address, and the window neither navigates nor opens
// another, here or in the system's browser.

import type { Rectangle } from "electron";
import { isInterfaceLanguage } from "../../../web/src/i18n/locale";
import { MIN_SIZE } from "./windowState";

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

/** How much of the main window's width and height the practice window takes. */
const PRACTICE_SHARE = 0.85;

/**
 * The narrowest practice window whose screen shows every part side by side:
 * chapter 1's device rail, sidebar, Agent Views at their 480 px and a docked
 * lane with its tool rail need about 1264 px, and a narrower window draws the
 * lane over the agents it is meant to sit beside.
 */
const PRACTICE_MIN_WIDTH = 1280;

/**
 * Where the practice window opens: smaller than the main window and against its lower right, so the
 * main window's title row and sidebar still show beside it and the learner sees the real Hide is there,
 * but never narrower than `PRACTICE_MIN_WIDTH` where the screen it opens on (`workArea`) has the room.
 * With no main window it takes the size the main window would have, centred by the system.
 */
export function practiceBounds(main: Rectangle | undefined, workArea: Rectangle): Partial<Rectangle> & { width: number; height: number } {
  if (!main) return { width: Math.min(PRACTICE_MIN_WIDTH, workArea.width), height: Math.min(800, workArea.height) };
  const width = Math.min(workArea.width, Math.max(MIN_SIZE.width, PRACTICE_MIN_WIDTH, Math.round(main.width * PRACTICE_SHARE)));
  const height = Math.max(MIN_SIZE.height, Math.round(main.height * PRACTICE_SHARE));
  const right = Math.min(main.x + main.width, workArea.x + workArea.width);
  return { x: Math.max(workArea.x, right - width), y: main.y + main.height - height, width, height };
}
