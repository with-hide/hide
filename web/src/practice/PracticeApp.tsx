// The practice window's page (`/?practice`): the chapter list, and the chapter
// being played. It holds no daemon connection; the operator's language,
// chords, theme and progress come from the shell over `channel.ts`, and each
// step reached goes back the same way for the core to keep.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { clientI18n, useInterfaceTranslation } from "../i18n/translator";
import { isInterfaceLanguage } from "../i18n/locale";
import { resetRecent } from "../recent";
import { useShellStore } from "../store";
import { applyTerminalTheme } from "../terminals";
import { PALETTE_HIDE_CLASS, readColorTheme, readTheme, resolveTheme } from "../theme";
import { useUiStore } from "../ui";
import type { PracticeChapter } from "./chapter";
import { CHAPTERS, nextChapter } from "./chapters";
import { PRACTICE_CHANNEL, type ChapterProgress, type PracticeMessage, type PracticeProgress, type ShellAnswer } from "./channel";
import { holdPageClipboard } from "./clipboard";
import { HUB_CSS } from "./coachStyle";
import { PracticePlayer, type OperatorChoices } from "./PracticePlayer";

export type ChapterStatus = "new" | "paused" | "done" | "changed";

/** Where a learner stands in `chapter`: a progress kept for older steps reads as changed. */
export function chapterStatus(chapter: PracticeChapter, progress: ChapterProgress | undefined): ChapterStatus {
  if (!progress) return "new";
  if (progress.version !== chapter.version) return "changed";
  if (progress.finished) return "done";
  return progress.step > 0 ? "paused" : "new";
}

/** How long the page waits for a shell to answer before it says progress is not being kept. */
const ANSWER_WAIT_MS = 1500;

type Playing = { chapter: PracticeChapter; resume: number; run: number };

/**
 * The shell's page state as the page loaded it. Its stores and its recent
 * order are the page's, not the chapter's, so without this a chapter started
 * over, or after another, would keep what the last run left: a relationship
 * focus still waiting for a world that is gone, an open overlay, a focus, a
 * copied file. The player then fills the stores from its new world.
 */
export function freshShell() {
  useShellStore.setState(useShellStore.getInitialState(), true);
  useUiStore.setState(useUiStore.getInitialState(), true);
  resetRecent();
}

/** The chapter whose id is `id`, or null for an id no chapter has (the window then opens on the list). */
function chapterById(id: string | null | undefined): PracticeChapter | null {
  return CHAPTERS.find((chapter) => chapter.id === id) ?? null;
}

/**
 * Back to Hide: the practice window closes, and the desktop host brings the main window forward
 * (`openPractice` in `desktop/src/main/host.ts`); a browser tab the shell opened closes back to the shell's tab.
 */
function leave() {
  window.close();
}

/** `opening` is the chapter the address asks for (`practiceAddress`): the window plays it at once instead of listing the chapters. */
export function PracticeApp({ opening = null }: { opening?: string | null }) {
  const { t } = useInterfaceTranslation();
  const [answer, setAnswer] = useState<ShellAnswer | null>(null);
  const [unanswered, setUnanswered] = useState(false);
  const [playing, setPlaying] = useState<Playing | null>(() => {
    const chapter = chapterById(opening);
    return chapter ? { chapter, resume: 0, run: 1 } : null;
  });
  const channel = useRef<BroadcastChannel | null>(null);
  // A chapter keeps the page's copy and paste on its own clipboard; the chapter list holds them with none.
  const listing = playing === null;
  useEffect(() => (listing ? holdPageClipboard(null) : undefined), [listing]);

  // Every start goes through here: a chapter from the list, from its start again, the next one, or one the shell asks for.
  const start = useCallback((chapter: PracticeChapter, resume: number) => {
    freshShell();
    setPlaying((before) => ({ chapter, resume, run: (before?.run ?? 0) + 1 }));
  }, []);

  useEffect(() => {
    const opened = new BroadcastChannel(PRACTICE_CHANNEL);
    channel.current = opened;
    opened.onmessage = (event: MessageEvent<ShellAnswer | PracticeMessage>) => {
      if (event.data.kind === "state") setAnswer(event.data);
      else if (event.data.kind === "play") {
        // The shell brings this window forward and asks for a chapter from the start (the sidebar's invitation).
        const chapter = chapterById(event.data.chapter);
        if (chapter) start(chapter, 0);
      }
    };
    opened.postMessage({ kind: "hello" } satisfies PracticeMessage);
    const timer = window.setTimeout(() => setUnanswered(true), ANSWER_WAIT_MS);
    return () => {
      window.clearTimeout(timer);
      opened.close();
      channel.current = null;
    };
  }, [start]);

  useLayoutEffect(() => {
    const language = answer?.language;
    if (isInterfaceLanguage(language) && clientI18n.language !== language) void clientI18n.changeLanguage(language);
    if (isInterfaceLanguage(language)) document.documentElement.lang = language;
  }, [answer?.language]);

  useLayoutEffect(() => {
    const theme = resolveTheme(readTheme(answer?.theme).choice, window.matchMedia("(prefers-color-scheme: dark)").matches);
    const root = document.documentElement.classList;
    root.toggle("dark", theme === "dark");
    root.toggle("light", theme === "light");
    root.toggle(PALETTE_HIDE_CLASS, readColorTheme(answer?.color_theme).choice === "hide");
    applyTerminalTheme();
  }, [answer?.theme, answer?.color_theme]);

  // The window wears the practice's own name, playing or listing, so it is never taken for the main window.
  useEffect(() => {
    document.title = t("practice.hubTitle");
  }, [t]);

  const progress: PracticeProgress = useMemo(() => answer?.progress ?? {}, [answer?.progress]);
  const operator: OperatorChoices = useMemo(() => ({ shortcut_bindings: answer?.shortcut_bindings, browser_shortcut_bindings: answer?.browser_shortcut_bindings }), [answer?.shortcut_bindings, answer?.browser_shortcut_bindings]);

  const chapter = playing?.chapter;
  const onStep = useCallback((step: number, finished: boolean) => {
    if (!chapter) return;
    const before = progress[chapter.id];
    // Seeing a chapter again keeps it seen: a learner who finished it once has finished it.
    const kept = before?.version === chapter.version && before.finished;
    channel.current?.postMessage({ kind: "progress", chapter: chapter.id, version: chapter.version, step, finished: finished || kept } satisfies PracticeMessage);
  }, [chapter, progress]);

  if (playing) {
    const next = nextChapter(playing.chapter);
    return <PracticePlayer key={playing.run} chapter={playing.chapter} next={next} resume={playing.resume} operator={operator} onStep={onStep} onStart={start} onHub={() => setPlaying(null)} onLeave={leave} />;
  }
  return <PracticeHub progress={progress} unsaved={unanswered && !answer} onStart={start} onLeave={leave} />;
}

function PracticeHub({ progress, unsaved, onStart, onLeave }: { progress: PracticeProgress; unsaved: boolean; onStart: (chapter: PracticeChapter, resume: number) => void; onLeave: () => void }) {
  const { t } = useInterfaceTranslation();
  const rows = CHAPTERS.map((chapter) => ({ chapter, status: chapterStatus(chapter, progress[chapter.id]), step: progress[chapter.id]?.step ?? 0 }));
  const firstOpen = rows.findIndex((row) => row.status !== "done");
  const [chosen, setChosen] = useState(firstOpen === -1 ? 0 : firstOpen);
  const play = useCallback((at: number, fromStart: boolean) => {
    const row = rows[at]!;
    onStart(row.chapter, !fromStart && row.status === "paused" ? row.step : 0);
  }, [onStart, rows]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return;
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setChosen((at) => Math.max(0, Math.min(rows.length - 1, at + (event.key === "ArrowDown" ? 1 : -1))));
      } else if (event.key === "Enter" && !(event.target instanceof HTMLButtonElement)) {
        event.preventDefault();
        play(chosen, false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [chosen, play, rows.length]);

  return (
    <div className="ph-page" data-practice-hub>
      <style>{HUB_CSS}</style>
      <main className="ph-col">
        <div className="ph-head">
          <h1>{t("practice.hubTitle")}</h1>
          <button type="button" className="ph-leave" onClick={onLeave} data-practice-leave>{t("practice.leave")}</button>
        </div>
        <p className="ph-lead">{t("practice.hubLead")}</p>
        <ol className="ph-list">
          {rows.map((row, at) => {
            const total = row.chapter.steps.length;
            const id = row.chapter.id;
            return (
              <li key={id} className={`ph-row${at === chosen ? " on" : ""}`} data-practice-hub-chapter={id} data-status={row.status} onMouseEnter={() => setChosen(at)}>
                <span className={`ph-num ${row.status}`}>{row.status === "done" ? "✓" : row.chapter.number}</span>
                <div className="ph-text">
                  <small>{t("practice.chapter", { number: row.chapter.number })} · {t("practice.steps", { count: total })}</small>
                  <b>{t(`practice.${id}.title` as never)}</b>
                  <span>{t(`practice.${id}.summary` as never)}</span>
                </div>
                <span className={`ph-chip ${row.status}`}>{row.status === "paused" ? t("practice.status.paused", { index: row.step + 1, total }) : t(`practice.status.${row.status}`)}</span>
                <div className="ph-acts">
                  {row.status === "paused" ? <button type="button" className="ph-sub" onClick={() => play(at, true)} data-practice-from-start>{t("practice.fromStart")}</button> : null}
                  <button type="button" className="ph-go" onClick={() => play(at, false)} data-practice-play>
                    {row.status === "paused" ? t("practice.continue") : row.status === "done" ? t("practice.replay") : t("practice.start")}
                  </button>
                </div>
              </li>
            );
          })}
        </ol>
        <p className="ph-keys">{t("practice.hubKeys")}</p>
        {unsaved ? <p className="ph-note" data-practice-unsaved>{t("practice.hubUnsaved")}</p> : null}
      </main>
    </div>
  );
}
