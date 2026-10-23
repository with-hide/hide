// The practice window's page (`/?practice`): the chapter list, and the chapter
// being played. It holds no daemon connection; the operator's language,
// chords, theme and progress come from the shell over `channel.ts`, and each
// step reached goes back the same way for the core to keep.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { clientI18n, useInterfaceTranslation } from "../i18n/translator";
import { isInterfaceLanguage } from "../i18n/locale";
import { applyTerminalTheme } from "../terminals";
import { PALETTE_HIDE_CLASS, readColorTheme, readTheme, resolveTheme } from "../theme";
import { CHAPTERS, type PracticeChapter } from "./chapters";
import { PRACTICE_CHANNEL, type ChapterProgress, type PracticeMessage, type PracticeProgress, type ShellAnswer } from "./channel";
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

export function PracticeApp() {
  const [answer, setAnswer] = useState<ShellAnswer | null>(null);
  const [unanswered, setUnanswered] = useState(false);
  const [playing, setPlaying] = useState<Playing | null>(null);
  const channel = useRef<BroadcastChannel | null>(null);

  useEffect(() => {
    const opened = new BroadcastChannel(PRACTICE_CHANNEL);
    channel.current = opened;
    opened.onmessage = (event: MessageEvent<ShellAnswer | PracticeMessage>) => {
      if (event.data.kind === "state") setAnswer(event.data);
    };
    opened.postMessage({ kind: "hello" } satisfies PracticeMessage);
    const timer = window.setTimeout(() => setUnanswered(true), ANSWER_WAIT_MS);
    return () => {
      window.clearTimeout(timer);
      opened.close();
      channel.current = null;
    };
  }, []);

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

  const progress: PracticeProgress = useMemo(() => answer?.progress ?? {}, [answer?.progress]);
  const operator: OperatorChoices = useMemo(() => ({ shortcut_bindings: answer?.shortcut_bindings, browser_shortcut_bindings: answer?.browser_shortcut_bindings }), [answer?.shortcut_bindings, answer?.browser_shortcut_bindings]);

  const start = useCallback((chapter: PracticeChapter, resume: number) => setPlaying((before) => ({ chapter, resume, run: (before?.run ?? 0) + 1 })), []);
  const chapter = playing?.chapter;
  const onStep = useCallback((step: number, finished: boolean) => {
    if (!chapter) return;
    const before = progress[chapter.id];
    // Seeing a chapter again keeps it seen: a learner who finished it once has finished it.
    const kept = before?.version === chapter.version && before.finished;
    channel.current?.postMessage({ kind: "progress", chapter: chapter.id, version: chapter.version, step, finished: finished || kept } satisfies PracticeMessage);
  }, [chapter, progress]);

  if (playing) {
    const next = CHAPTERS.find((row) => row.number === playing.chapter.number + 1) ?? null;
    return <PracticePlayer key={playing.run} chapter={playing.chapter} next={next} resume={playing.resume} operator={operator} onStep={onStep} onStart={(target) => start(target, 0)} onHub={() => setPlaying(null)} />;
  }
  return <PracticeHub progress={progress} unsaved={unanswered && !answer} onStart={start} />;
}

function PracticeHub({ progress, unsaved, onStart }: { progress: PracticeProgress; unsaved: boolean; onStart: (chapter: PracticeChapter, resume: number) => void }) {
  const { t } = useInterfaceTranslation();
  const rows = CHAPTERS.map((chapter) => ({ chapter, status: chapterStatus(chapter, progress[chapter.id]), step: progress[chapter.id]?.step ?? 0 }));
  const firstOpen = rows.findIndex((row) => row.status !== "done");
  const [chosen, setChosen] = useState(firstOpen === -1 ? 0 : firstOpen);
  const play = useCallback((at: number, fromStart: boolean) => {
    const row = rows[at]!;
    onStart(row.chapter, !fromStart && row.status === "paused" ? row.step : 0);
  }, [onStart, rows]);

  useEffect(() => {
    document.title = t("practice.hubTitle");
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
  }, [chosen, play, rows.length, t]);

  return (
    <div className="ph-page" data-practice-hub>
      <style>{HUB_CSS}</style>
      <main className="ph-col">
        <h1>{t("practice.hubTitle")}</h1>
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
                  {row.status === "paused" ? <button type="button" className="ph-sub" onClick={() => play(at, true)}>{t("practice.fromStart")}</button> : null}
                  <button type="button" className="ph-go" onClick={() => play(at, false)}>
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
