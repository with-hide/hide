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
import { CHAPTERS } from "./chapters";
import { PRACTICE_CHANNEL, type ChapterProgress, type PracticeMessage, type PracticeProgress, type ShellAnswer } from "./channel";
import { holdPageClipboard } from "./clipboard";
import { HUB_CSS } from "./coachStyle";
import { keyParts } from "./PracticeCoach";
import { GUIDES, type PracticeGuide } from "./guides";
import { PracticeFrames } from "./PracticeFrames";
import { PracticePreview, PREVIEW_CSS } from "./PracticePreview";
import { Button } from "../components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "../components/ui/dialog";
import { Kbd, KbdGroup } from "../components/ui/kbd";
import { RowButton } from "../components/ui/row-button";
import { hostKind, keySystem } from "../host";
import { commandLabel } from "../shortcutLabels";
import { hostChord, hostRegistry, type CommandId } from "../shortcuts";
import { PracticePlayer, type OperatorChoices } from "./PracticePlayer";
import { composingKey } from "../keys";

export type ChapterStatus = "new" | "done" | "changed";

/** Where a learner stands in `chapter`: a progress kept for older steps reads as changed. */
export function chapterStatus(chapter: PracticeChapter, progress: ChapterProgress | undefined): ChapterStatus {
  if (!progress) return "new";
  if (progress.version !== chapter.version) return "changed";
  return progress.finished ? "done" : "new";
}

/** How long the page waits for a shell to answer before it says progress is not being kept. */
const ANSWER_WAIT_MS = 1500;

type Playing = { chapter: PracticeChapter; run: number };

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
    return chapter ? { chapter, run: 1 } : null;
  });
  const channel = useRef<BroadcastChannel | null>(null);
  // A chapter keeps the page's copy and paste on its own clipboard; the chapter list holds them with none.
  const listing = playing === null;
  useEffect(() => (listing ? holdPageClipboard(null) : undefined), [listing]);

  // Every start goes through here: the chapter from the list, or the one the shell asks for.
  const start = useCallback((chapter: PracticeChapter) => {
    freshShell();
    setPlaying((before) => ({ chapter, run: (before?.run ?? 0) + 1 }));
  }, []);

  // A real mission: the shell opens its Start panel with the sentence waiting, and the practice window closes onto it.
  const mission = useCallback(() => {
    channel.current?.postMessage({ kind: "mission" } satisfies PracticeMessage);
    leave();
  }, []);

  useEffect(() => {
    const opened = new BroadcastChannel(PRACTICE_CHANNEL);
    channel.current = opened;
    opened.onmessage = (event: MessageEvent<ShellAnswer | PracticeMessage>) => {
      if (event.data.kind === "state") setAnswer(event.data);
      else if (event.data.kind === "play") {
        // The shell brings this window forward and asks for a chapter from the start (the sidebar's invitation).
        const chapter = chapterById(event.data.chapter);
        if (chapter) start(chapter);
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

  if (playing) return <PracticePlayer key={playing.run} chapter={playing.chapter} operator={operator} onStep={onStep} onMission={mission} onHub={() => setPlaying(null)} onLeave={leave} />;
  return <PracticeHub progress={progress} operator={operator} unsaved={unanswered && !answer} onStart={start} onMission={mission} onLeave={leave} />;
}

type HubProps = { progress: PracticeProgress; operator: OperatorChoices; unsaved: boolean; /** The id of the card whose detail the list opens on, for a capture. */ opened?: string; onStart: (chapter: PracticeChapter) => void; onMission: () => void; onLeave: () => void };

/** A card on the list: the first flow, which plays on a practice screen, or a guide, which is looked at. */
type HubRow = { chapter: PracticeChapter; status: ChapterStatus } | { guide: PracticeGuide };
const rowId = (row: HubRow) => ("chapter" in row ? row.chapter.id : row.guide.id);
const rowNumber = (row: HubRow) => ("chapter" in row ? row.chapter.number : row.guide.number);
const rowPreview = (row: HubRow) => ("chapter" in row ? row.chapter.preview : row.guide.preview);

/** `id`'s chord on this host with the operator's bindings as keycap parts, or null where the host has none. */
function chordParts(id: CommandId, operator: OperatorChoices): string[] | null {
  const host = hostKind();
  const command = hostRegistry(operator, host, keySystem()).registry.find((row) => row.id === id);
  return command && hostChord(command, host) ? keyParts(commandLabel(id, operator)) : null;
}

/**
 * The flow list: a card per flow with the move it shows playing on it, and
 * pressing a card opens its detail. The first flow starts a practice there;
 * the others are pictures to look things up in. The cards hold still while a
 * detail is open.
 */
export function PracticeHub({ progress, operator, unsaved, opened: openedId, onStart, onMission, onLeave }: HubProps) {
  const { t } = useInterfaceTranslation();
  const rows: HubRow[] = [...CHAPTERS.map((chapter) => ({ chapter, status: chapterStatus(chapter, progress[chapter.id]) })), ...GUIDES.map((guide) => ({ guide }))].sort((a, b) => rowNumber(a) - rowNumber(b));
  const firstOpen = rows.findIndex((row) => "chapter" in row && row.status !== "done");
  const [chosen, setChosen] = useState(firstOpen === -1 ? 0 : firstOpen);
  const [opened, setOpened] = useState<number | null>(() => {
    const at = rows.findIndex((row) => rowId(row) === openedId);
    return at === -1 ? null : at;
  });
  const cards = useRef<(HTMLButtonElement | null)[]>([]);
  const choose = useCallback((at: number) => {
    setChosen(at);
    cards.current[at]?.focus();
  }, []);

  useEffect(() => {
    if (opened !== null) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey || composingKey(event)) return;
      const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key];
      if (step) {
        event.preventDefault();
        choose(Math.max(0, Math.min(rows.length - 1, chosen + step)));
      } else if (event.key === "Enter" && !(event.target instanceof HTMLButtonElement)) {
        event.preventDefault();
        setOpened(chosen);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [choose, chosen, opened, rows.length]);

  const open = opened === null ? null : rows[opened]!;
  return (
    <div className="ph-page" data-practice-hub>
      <style>{HUB_CSS}{PREVIEW_CSS}</style>
      <main className="ph-col">
        <div className="ph-head">
          <h1>{t("practice.hubTitle")}</h1>
          <Button variant="secondary" onClick={onLeave} data-practice-leave>{t("practice.leave")}</Button>
        </div>
        <p className="ph-lead">{t("practice.hubLead")}</p>
        <p className="ph-again">{t("practice.hubAgain")}</p>
        <ol className="ph-grid">
          {rows.map((row, at) => {
            const id = rowId(row);
            return (
              <li key={id} className="ph-cell" style={{ animationDelay: `${at * 60}ms` }}>
                <RowButton
                  ref={(node) => { cards.current[at] = node; }}
                  className={`ph-card${at === chosen ? " on" : ""}`}
                  data-practice-hub-chapter={id}
                  data-status={"chapter" in row ? row.status : undefined}
                  onMouseEnter={() => setChosen(at)}
                  onFocus={() => setChosen(at)}
                  onClick={() => setOpened(at)}
                >
                  <PracticePreview kind={rowPreview(row)} playing={opened === null} />
                  <span className="ph-meta">
                    <span className="ph-top">
                      <small>{t("practice.chapter", { number: rowNumber(row) })}</small>
                      {"chapter" in row ? <StatusChip status={row.status} /> : null}
                    </span>
                    <b>{t(`practice.${id}.title` as never)}</b>
                    <span className="ph-sum">{t(`practice.${id}.summary` as never)}</span>
                  </span>
                </RowButton>
              </li>
            );
          })}
        </ol>
        <p className="ph-keys">{t("practice.hubKeys")}</p>
        {unsaved ? <p className="ph-note" data-practice-unsaved>{t("practice.hubUnsaved")}</p> : null}
      </main>
      <Dialog open={open !== null} onOpenChange={(next) => { if (!next) setOpened(null); }}>
        {open && "chapter" in open ? <ChapterDetail row={open} onPlay={() => onStart(open.chapter)} returnTo={() => cards.current[opened!] ?? null} /> : null}
        {open && "guide" in open ? <GuideDetail guide={open.guide} operator={operator} onMission={onMission} returnTo={() => cards.current[opened!] ?? null} /> : null}
      </Dialog>
    </div>
  );
}

function StatusChip({ status }: { status: ChapterStatus }) {
  const { t } = useInterfaceTranslation();
  return <span className={`ph-chip ${status}`}>{t(`practice.status.${status}`)}</span>;
}

/** The first flow: what it shows, and the way to start it. */
function ChapterDetail({ row, onPlay, returnTo }: { row: { chapter: PracticeChapter; status: ChapterStatus }; onPlay: () => void; returnTo: () => HTMLElement | null }) {
  const { t } = useInterfaceTranslation();
  const { chapter, status } = row;
  const go = useRef<HTMLButtonElement>(null);
  return (
    <DialogContent
      className="ph-detail"
      showCloseButton
      returnFocusTo={returnTo}
      onOpenAutoFocus={(event) => { event.preventDefault(); go.current?.focus(); }}
      data-practice-detail={chapter.id}
    >
      <PracticePreview kind={chapter.preview} playing large />
      <div className="ph-body">
        <small>{t("practice.chapter", { number: chapter.number })} · {t("practice.steps", { count: chapter.steps.length })}</small>
        <DialogTitle className="ph-title">{t(`practice.${chapter.id}.title` as never)}</DialogTitle>
        <DialogDescription className="ph-desc">{t(`practice.${chapter.id}.summary` as never)}</DialogDescription>
      </div>
      <DialogFooter className="ph-foot">
        <Button ref={go} variant="practice" onClick={onPlay} data-practice-play>{status === "done" ? t("practice.replay") : t("practice.start")}</Button>
      </DialogFooter>
    </DialogContent>
  );
}

/** A guide: its moves drawn one after another, one line of what to know, and the chords it uses. */
function GuideDetail({ guide, operator, onMission, returnTo }: { guide: PracticeGuide; operator: OperatorChoices; onMission: () => void; returnTo: () => HTMLElement | null }) {
  const { t } = useInterfaceTranslation();
  const go = useRef<HTMLButtonElement>(null);
  const chords = guide.chords.flatMap((id) => {
    const keys = chordParts(id, operator);
    return keys ? [{ id, keys }] : [];
  });
  return (
    <DialogContent
      className="ph-detail ph-guide"
      showCloseButton
      returnFocusTo={returnTo}
      onOpenAutoFocus={(event) => { if (go.current) { event.preventDefault(); go.current.focus(); } }}
      data-practice-detail={guide.id}
    >
      <PracticeFrames guide={guide} />
      <div className="ph-body">
        <small>{t("practice.chapter", { number: guide.number })}</small>
        <DialogTitle className="ph-title">{t(`practice.${guide.id}.title` as never)}</DialogTitle>
        <DialogDescription className="ph-desc">{t(`practice.${guide.id}.line` as never)}</DialogDescription>
        {chords.length ? (
          <ul className="ph-learn" data-practice-chords>
            {chords.map(({ id, keys }) => (
              <li key={id}>
                <span>{t(`commands.${id}` as never)}</span>
                <KbdGroup>{keys.map((part, at) => <Kbd key={at}>{part}</Kbd>)}</KbdGroup>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
      {guide.mission ? (
        <DialogFooter className="ph-foot">
          <Button ref={go} variant="practice" onClick={onMission} data-practice-mission>{t("practice.tryMission")}</Button>
        </DialogFooter>
      ) : null}
    </DialogContent>
  );
}
