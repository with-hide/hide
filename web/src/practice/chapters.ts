// The practice chapters: every file in `chapters/`, each default-exporting one
// chapter named like its file, listed by number. A chapter is added by adding
// its file; nothing here names one.

import type { PracticeChapter } from "./chapter";

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
