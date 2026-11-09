// The practice window in the gallery, for design captures: the chapter list
// (`view=hub`) or one chapter playing, on the desktop host's chords, in the
// gallery's theme, with no shell to keep its progress.

import { useLayoutEffect, useState } from "react";
import { requireInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { CHAPTERS } from "../practice/chapters";
import { GUIDES } from "../practice/guides";
import { freshShell, PracticeHub } from "../practice/PracticeApp";
import { FIRST_CHAPTER, type PracticeProgress } from "../practice/channel";
import { standInHost } from "../practice/mount";
import { PracticePlayer } from "../practice/PracticePlayer";
import { PALETTE_HIDE_CLASS } from "../theme";

export type PracticeSceneParams = { chapter: number; hub: boolean; guide: string | null; theme: "light" | "dark"; system: "mac" | "pc"; language: InterfaceLanguage };

export function practiceSceneParams(params: URLSearchParams): PracticeSceneParams {
  const chapter = Number(params.get("chapter") ?? CHAPTERS.find((row) => row.id === FIRST_CHAPTER)!.number);
  if (!CHAPTERS.some((row) => row.number === chapter)) throw new Error(`Unknown practice chapter ${chapter}`);
  const guide = params.get("guide");
  if (guide !== null && !GUIDES.some((row) => row.id === guide)) throw new Error(`Unknown practice guide ${guide}`);
  return { chapter, hub: params.get("view") === "hub" || guide !== null, guide, theme: params.get("theme") === "light" ? "light" : "dark", system: params.get("sys") === "pc" ? "pc" : "mac", language: requireInterfaceLanguage(params.get("lang") ?? "en") };
}

const NO_CHOICES = {};
const ignore = () => {};

/** The list as a learner who finished the first flow on an older version sees it. */
function sampleProgress(): PracticeProgress {
  const first = CHAPTERS.find((row) => row.id === FIRST_CHAPTER)!;
  return { [first.id]: { version: first.version - 1, step: first.steps.length - 1, finished: true } };
}

export function PracticeScene({ chapter: first, hub, guide, theme, system, language }: PracticeSceneParams) {
  useFollowInterfaceLanguage(language);
  if (!window.hideHost) window.hideHost = standInHost(system === "mac" ? "darwin" : "win32");
  const [playing, setPlaying] = useState({ number: first, run: 0 });
  const chapter = CHAPTERS.find((row) => row.number === playing.number)!;

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.add(PALETTE_HIDE_CLASS);
    document.title = `hide · practice ${chapter.number}`;
  }, [chapter.number, theme]);

  // Every start begins on the shell as the page loaded it, as the practice window's does.
  const start = () => {
    freshShell();
    setPlaying((before) => ({ number: before.number, run: before.run + 1 }));
  };
  if (hub) {
    return (
      <div className="h-full" data-gallery-scene="practice">
        <PracticeHub progress={sampleProgress()} operator={NO_CHOICES} unsaved={false} opened={guide ?? undefined} onStart={ignore} onLeave={ignore} />
      </div>
    );
  }
  return (
    <div className="h-full" data-gallery-scene="practice">
      <PracticePlayer key={playing.run} chapter={chapter} operator={NO_CHOICES} onStep={ignore} onHub={start} onLeave={ignore} />
    </div>
  );
}
