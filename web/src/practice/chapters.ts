// The practice chapters: every file in `chapters/`, each default-exporting one
// chapter named like its file, listed by number, with their words. A chapter
// is added by adding its file; nothing here names one.

import { clientI18n } from "../i18n/translator";
import type { PracticeChapter } from "./chapter";
import { addChapterWords } from "./words";

// A chapter's words load with it, so wherever a chapter plays its words are there.
addChapterWords(clientI18n);

const files = import.meta.glob<PracticeChapter>("./chapters/*.ts", { eager: true, import: "default" });

function listed(modules: Record<string, PracticeChapter>): PracticeChapter[] {
  const chapters = Object.entries(modules).map(([path, chapter]) => {
    const file = path.slice(path.lastIndexOf("/") + 1, -".ts".length);
    if (chapter.id !== file) throw new Error(`Practice chapter ${chapter.id} is in ${path}`);
    return chapter;
  });
  chapters.sort((left, right) => left.number - right.number);
  chapters.forEach((chapter, index) => {
    const before = chapters[index - 1];
    if (before?.number === chapter.number) throw new Error(`Practice chapters ${before.id} and ${chapter.id} share number ${chapter.number}`);
  });
  return chapters;
}

export const CHAPTERS: readonly PracticeChapter[] = listed(files);

/** The chapter after `chapter` in the list, or null after the last. */
export function nextChapter(chapter: PracticeChapter): PracticeChapter | null {
  return CHAPTERS[CHAPTERS.indexOf(chapter) + 1] ?? null;
}
