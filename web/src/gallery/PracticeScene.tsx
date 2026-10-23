// The practice window's chapter player in the gallery, for design captures:
// one chapter on the desktop host's chords, in the gallery's theme, with no
// shell to keep its progress.

import { useLayoutEffect, useState } from "react";
import { CHAPTERS, nextChapter } from "../practice/chapters";
import { standInHost } from "../practice/mount";
import { PracticePlayer } from "../practice/PracticePlayer";
import { PALETTE_HIDE_CLASS } from "../theme";

export type PracticeSceneParams = { chapter: number; theme: "light" | "dark"; system: "mac" | "pc" };

export function practiceSceneParams(params: URLSearchParams): PracticeSceneParams {
  const chapter = Number(params.get("chapter") ?? "0");
  if (!CHAPTERS.some((row) => row.number === chapter)) throw new Error(`Unknown practice chapter ${chapter}`);
  return { chapter, theme: params.get("theme") === "light" ? "light" : "dark", system: params.get("sys") === "pc" ? "pc" : "mac" };
}

const NO_CHOICES = {};
const ignore = () => {};

export function PracticeScene({ chapter: first, theme, system }: PracticeSceneParams) {
  if (!window.hideHost) window.hideHost = standInHost(system === "mac" ? "darwin" : "win32");
  const [playing, setPlaying] = useState({ number: first, run: 0 });
  const chapter = CHAPTERS.find((row) => row.number === playing.number)!;
  const next = nextChapter(chapter);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.classList.toggle("light", theme === "light");
    root.classList.add(PALETTE_HIDE_CLASS);
    document.title = `hide · practice ${chapter.number}`;
  }, [chapter.number, theme]);

  const start = (target: typeof chapter) => setPlaying((before) => ({ number: target.number, run: before.run + 1 }));
  return (
    <div className="h-full" data-gallery-scene="practice">
      <PracticePlayer key={playing.run} chapter={chapter} next={next} resume={0} operator={NO_CHOICES} onStep={ignore} onStart={start} onHub={() => start(chapter)} />
    </div>
  );
}
