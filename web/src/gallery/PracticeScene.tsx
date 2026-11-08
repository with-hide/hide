// The practice window in the gallery, for design captures: the chapter list
// (`view=hub`) or one chapter playing, on the desktop host's chords, in the
// gallery's theme, with no shell to keep its progress.

import { useLayoutEffect, useState } from "react";
import { requireInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { CHAPTERS, nextChapter } from "../practice/chapters";
import { freshShell, PracticeHub } from "../practice/PracticeApp";
import type { PracticeProgress } from "../practice/channel";
import { standInHost } from "../practice/mount";
import { PracticePlayer } from "../practice/PracticePlayer";
import { PALETTE_HIDE_CLASS } from "../theme";

export type PracticeSceneParams = { chapter: number; hub: boolean; theme: "light" | "dark"; system: "mac" | "pc"; language: InterfaceLanguage };

export function practiceSceneParams(params: URLSearchParams): PracticeSceneParams {
  const chapter = Number(params.get("chapter") ?? "0");
  if (!CHAPTERS.some((row) => row.number === chapter)) throw new Error(`Unknown practice chapter ${chapter}`);
  return { chapter, hub: params.get("view") === "hub", theme: params.get("theme") === "light" ? "light" : "dark", system: params.get("sys") === "pc" ? "pc" : "mac", language: requireInterfaceLanguage(params.get("lang") ?? "en") };
}

const NO_CHOICES = {};
const ignore = () => {};

/** The list as a learner partway through sees it: chapter 0 seen, chapter 4 stopped partway, chapter 2 seen on older steps. */
function sampleProgress(): PracticeProgress {
  const at = (number: number) => CHAPTERS.find((row) => row.number === number)!;
  return {
    [at(0).id]: { version: at(0).version, step: at(0).steps.length - 1, finished: true },
    [at(4).id]: { version: at(4).version, step: 4, finished: false },
    [at(2).id]: { version: at(2).version - 1, step: 3, finished: true },
  };
}

export function PracticeScene({ chapter: first, hub, theme, system, language }: PracticeSceneParams) {
  useFollowInterfaceLanguage(language);
  if (!window.hideHost) window.hideHost = standInHost(system === "mac" ? "darwin" : "win32");
  const [playing, setPlaying] = useState({ number: first, resume: 0, run: 0 });
  const chapter = CHAPTERS.find((row) => row.number === playing.number)!;
  const next = nextChapter(chapter);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.add(PALETTE_HIDE_CLASS);
    document.title = `hide · practice ${chapter.number}`;
  }, [chapter.number, theme]);

  // Every start begins on the shell as the page loaded it, as the practice window's does.
  const start = (target: typeof chapter, resume = 0) => {
    freshShell();
    setPlaying((before) => ({ number: target.number, resume, run: before.run + 1 }));
  };
  if (hub) {
    return (
      <div className="h-full" data-gallery-scene="practice">
        <PracticeHub progress={sampleProgress()} operator={NO_CHOICES} unsaved={false} onStart={ignore} onLeave={ignore} />
      </div>
    );
  }
  return (
    <div className="h-full" data-gallery-scene="practice">
      <PracticePlayer key={playing.run} chapter={chapter} next={next} resume={playing.resume} operator={NO_CHOICES} onStep={ignore} onStart={start} onHub={() => start(chapter)} onLeave={ignore} />
    </div>
  );
}
