// The window after an update and the update history (docs/UI_BEHAVIOR.md,
// What's new and Update history). The window opens once, on the first launch
// of a stable version whose release names main features, over the dimmed
// shell: a large drawing of the chosen feature (its own scene,
// `whats-new-scenes/`, else a practice card's move, `PracticePreview`), the features with a 체험하기 into the practice window
// where a chapter teaches one, the changes and fixes folded behind 더 보기,
// the history link and OK. The history lists every version the app knows up
// to the one in use, newest first. What decides and what is kept is
// `releases.ts` and the desktop host; this file draws it.

import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useShellStore } from "../store";
import { useInterfaceTranslation } from "../i18n/client";
import { formatDateTime } from "../i18n/format";
import { requireInterfaceLanguage, type InterfaceLanguage } from "../i18n/locale";
import { openPractice } from "../practice/channel";
import { CHAPTERS } from "../practice/chapters";
import type { PracticePreviewKind } from "../practice/chapter";
import { PracticePreview, PREVIEW_CSS } from "../practice/PracticePreview";
import { SCENES, type SceneName } from "./whats-new-scenes";
import { useSceneTime } from "./whats-new-scenes/scene";
import { historyVersions, inLanguage, RELEASE_DRAWINGS, unseen, type ReleaseDrawing, whatsNewOffer, type Change, type Offer, type OfferHighlight, type RecordUpdate, type ReleaseEntry, type WhatsNewRecord } from "../releases";
import { releaseNotesUrl, useUpdateStatus, readyVersion } from "../updates";
import { chooseUpdateHistoryVersion, closeUpdateHistory, openUpdateHistory, retryUpdateHistory, saveWhatsNew, useUpdateHistory, useWhatsNewState } from "../whatsNew";
import type { Actions } from "../actions";
import { cn } from "../lib/utils";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "./ui/dialog";
import { RowButton } from "./ui/row-button";

/** How long the window waits on a feature drawn by a practice move before it moves to the next, the move's own loop; a scene waits its own loop. */
const ADVANCE_MS = 6000;
/** After this long from launch the window is not opened in this launch; the next launch asks again (B11). */
export const OFFER_WINDOW_MS = 30_000;

// The release file's drawing names are exactly the scenes and the practice cards' moves.
type Drawn = PracticePreviewKind | SceneName;
type SameKinds = [Drawn] extends [ReleaseDrawing] ? ([ReleaseDrawing] extends [Drawn] ? true : false) : false;
const sameKinds: SameKinds = true;
void sameKinds;
const isDrawing = (name: string): name is ReleaseDrawing => (RELEASE_DRAWINGS as readonly string[]).includes(name);
const isScene = (name: string): name is SceneName => Object.hasOwn(SCENES, name);
const isMove = (name: string): name is PracticePreviewKind => isDrawing(name) && !isScene(name);
const advanceMs = (drawing: string) => (isScene(drawing) ? SCENES[drawing].loopMs : ADVANCE_MS);

function SceneDrawing({ name, playing }: { name: SceneName; playing: boolean }) {
  const scene = SCENES[name];
  const at = useSceneTime(scene, playing);
  return <scene.Draw at={at} />;
}

/** A highlight's drawing: its feature's scene, or the practice move it names; nothing for a name this app does not have. */
function Drawing({ drawing, playing }: { drawing: string; playing: boolean }) {
  if (isScene(drawing)) return <SceneDrawing name={drawing} playing={playing} />;
  return isMove(drawing) ? <PracticePreview kind={drawing} playing={playing} large /> : null;
}
const chapterKnown = (id: string | null): id is string => id !== null && CHAPTERS.some((chapter) => chapter.id === id);

/** A drawing or chapter the release names that this app does not have: shown without it, and noted for the log (B14). */
function noteMissing(highlight: { drawing: string; chapter: string | null }): void {
  if (isDrawing(highlight.drawing) && (highlight.chapter === null || chapterKnown(highlight.chapter))) return;
  const note = useShellStore.getState().noteDiagnostic;
  if (!isDrawing(highlight.drawing)) note(`whats_new: no drawing ${highlight.drawing}`);
  if (highlight.chapter !== null && !chapterKnown(highlight.chapter)) note(`whats_new: no practice chapter ${highlight.chapter}`);
}

function releaseDate(language: InterfaceLanguage, date: string): string {
  return formatDateTime(language, Date.parse(`${date}T00:00:00Z`), { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
}

/** "2 changes · 3 fixes", leaving out a kind with none. */
function useChangeCount() {
  const { t } = useInterfaceTranslation();
  return (changes: readonly Change[]) => {
    const changed = changes.filter((row) => row.kind === "change").length;
    const fixed = changes.length - changed;
    return [changed ? t("whatsNew.changes", { count: changed }) : null, fixed ? t("whatsNew.fixes", { count: fixed }) : null].filter(Boolean).join(" · ");
  };
}

function ChangeList({ changes, language }: { changes: readonly Change[]; language: InterfaceLanguage }) {
  const { t } = useInterfaceTranslation();
  return (
    <ul className="flex flex-col gap-xs" data-whats-new-changes>
      {changes.map((row, at) => (
        <li key={at} className="grid grid-cols-[auto_1fr] items-start gap-sm text-caption text-subtle-foreground" data-change-kind={row.kind}>
          <Badge variant={row.kind === "fix" ? "outline" : "secondary"} className="justify-center">{t(`whatsNew.kind.${row.kind}`)}</Badge>
          <span className="min-w-0 break-keep">{inLanguage(row.text, language)}</span>
        </li>
      ))}
    </ul>
  );
}

function Stage({ drawing, playing }: { drawing: string; playing: boolean }) {
  return isDrawing(drawing) ? (
    <div className="shrink-0 overflow-hidden rounded-md border border-border" data-whats-new-stage={drawing}>
      <Drawing drawing={drawing} playing={playing} />
    </div>
  ) : null;
}

/**
 * The window's words and actions for `offer`, from values a caller holds (the
 * gallery draws it this way). `auto` moves the drawing to the next feature
 * every loop until a feature is pressed.
 */
export function WhatsNewDialogBody({ offer, language, auto = true, open: openMore = false, onTry, onHistory, onClose }: {
  offer: Offer;
  language: InterfaceLanguage;
  auto?: boolean;
  open?: boolean;
  onTry: (chapter: string) => void;
  onHistory: () => void;
  onClose: () => void;
}) {
  const { t } = useInterfaceTranslation();
  const count = useChangeCount();
  const [chosen, setChosen] = useState(0);
  const [advancing, setAdvancing] = useState(auto && offer.highlights.length > 1);
  const [more, setMore] = useState(openMore);
  const ok = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  useEffect(() => {
    if (!advancing) return;
    const timer = window.setTimeout(() => setChosen((at) => (at + 1) % offer.highlights.length), advanceMs(offer.highlights[chosen]!.drawing));
    return () => window.clearTimeout(timer);
  }, [advancing, chosen, offer.highlights]);
  useEffect(() => {
    for (const row of offer.highlights) noteMissing(row);
  }, [offer.highlights]);
  const current = offer.highlights[chosen]!;
  const several = offer.from !== null;
  return (
    <DialogContent
      className="w-(--size-onboarding-dialog-w)"
      showCloseButton
      aria-labelledby={titleId}
      onOpenAutoFocus={(event) => { event.preventDefault(); ok.current?.focus(); }}
      data-whats-new={offer.to}
    >
      <div className="flex items-baseline gap-sm px-lg pt-lg pr-xxl">
        <DialogTitle id={titleId} className="text-subhead">{t("whatsNew.title")}</DialogTitle>
        <span className="font-mono text-caption text-muted-foreground" data-whats-new-range>{several ? `${offer.from} → ${offer.to}` : offer.to}</span>
      </div>
      <DialogDescription className="sr-only">{offer.highlights.map((row) => inLanguage(row.text, language).title).join(", ")}</DialogDescription>
      <div className="flex min-h-0 flex-1 flex-col gap-sm overflow-auto px-lg pt-md">
        <style>{PREVIEW_CSS}</style>
        <Stage key={`${chosen}-${current.drawing}`} drawing={current.drawing} playing />
        <ol className="flex flex-col gap-xxs" data-whats-new-features>
          {offer.highlights.map((row, at) => {
            const words = inLanguage(row.text, language);
            const tryable = chapterKnown(row.chapter);
            return (
              <li key={`${row.version}-${at}`} className={cn("flex items-center gap-sm rounded-md pr-sm", at === chosen && "bg-accent")}>
                <RowButton
                  className="grid min-w-0 flex-1 grid-cols-[auto_1fr] items-start gap-sm rounded-md px-sm py-xs"
                  aria-current={at === chosen ? "true" : undefined}
                  onClick={() => { setAdvancing(false); setChosen(at); }}
                  data-whats-new-feature={at}
                >
                  <span className={cn("mt-xxs flex size-(--size-icon) items-center justify-center rounded-full text-micro", at === chosen ? "bg-primary text-primary-foreground" : "bg-secondary text-subtle-foreground")}>{at + 1}</span>
                  <span className="flex min-w-0 flex-col">
                    <span className="text-body font-medium text-foreground">
                      {words.title}
                      {several ? <span className="ml-xs font-mono text-micro text-muted-foreground">{row.version}</span> : null}
                    </span>
                    <span className="break-keep text-caption text-muted-foreground">{words.line}</span>
                  </span>
                </RowButton>
                {tryable ? (
                  <Button variant="practice" size="sm" className="shrink-0" onClick={() => onTry(row.chapter!)} data-whats-new-try={row.chapter}>
                    {t("whatsNew.try")}
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ol>
        {offer.changes.length ? (
          <div className="border-t border-border pt-xs" data-whats-new-more={more}>
            <div className="flex items-center gap-sm py-xxs text-caption text-subtle-foreground">
              <span>{count(offer.changes)}</span>
              <Button variant="text" size="inline" className="ml-auto text-caption" aria-expanded={more} onClick={() => setMore((was) => !was)} data-whats-new-more-toggle>
                {t(more ? "whatsNew.less" : "whatsNew.more")}
              </Button>
            </div>
            {more ? <div className="pb-sm"><ChangeList changes={offer.changes} language={language} /></div> : null}
          </div>
        ) : null}
      </div>
      <DialogFooter className="justify-between pt-md">
        <Button variant="text-link" size="inline" className="text-caption" onClick={onHistory} data-whats-new-history>{t("whatsNew.history")}</Button>
        <Button ref={ok} onClick={onClose} data-whats-new-ok>{t("whatsNew.ok")}</Button>
      </DialogFooter>
    </DialogContent>
  );
}

type HistoryRow = ReleaseEntry & { isNew: boolean; isReady: boolean };

/**
 * The history's words and actions, from values a caller holds (the gallery
 * draws it this way): the versions newest first on the left, the chosen
 * one's features and its changes and fixes on the right.
 */
export function UpdateHistoryBody({ rows, appVersion, chosen, language, state, onChoose, onTry, onRetry, onLink }: {
  rows: readonly HistoryRow[];
  appVersion: string;
  chosen: string | null;
  language: InterfaceLanguage;
  state: "ready" | "loading" | "failed";
  onChoose: (version: string) => void;
  onTry: (chapter: string) => void;
  onRetry: () => void;
  onLink: (url: string) => void;
}) {
  const { t } = useInterfaceTranslation();
  const count = useChangeCount();
  const list = useRef<(HTMLButtonElement | null)[]>([]);
  const entry = rows.find((row) => row.version === chosen) ?? rows[0] ?? null;
  const at = entry ? rows.indexOf(entry) : -1;
  useEffect(() => {
    for (const row of entry?.highlights ?? []) noteMissing(row);
  }, [entry]);
  if (state === "failed" && rows.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-md p-xl text-center" data-update-history-failed>
        <DialogTitle className="text-body font-normal">{t("whatsNew.failed")}</DialogTitle>
        <div className="flex items-center gap-md">
          <Button onClick={onRetry} data-update-history-retry>{t("whatsNew.retry")}</Button>
          <Button variant="text-link" size="inline" onClick={() => onLink("https://github.com/with-hide/hide/releases")}>{t("whatsNew.viewOnGithub")}</Button>
        </div>
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <div className="grid h-full grid-cols-[var(--size-settings-nav-w)_1fr]" aria-busy="true" data-update-history-loading>
        <DialogTitle className="sr-only">{t("whatsNew.loading")}</DialogTitle>
        <div className="flex flex-col gap-sm border-r border-border p-md">
          {[0, 1, 2, 3].map((row) => <div key={row} className="h-(--size-control) animate-pulse rounded-md bg-secondary" />)}
        </div>
        <div className="flex flex-col gap-md p-lg">
          <div className="h-(--size-control) w-1/4 animate-pulse rounded-md bg-secondary" />
          <div className="aspect-[2/1] w-1/2 animate-pulse rounded-md bg-secondary" />
        </div>
      </div>
    );
  }
  return (
    <div className="grid h-full min-h-0 grid-cols-[var(--size-settings-nav-w)_1fr]" data-update-history>
      <nav aria-label={t("whatsNew.versions")} className="flex min-h-0 flex-col gap-xxs overflow-auto border-r border-border bg-card p-sm">
        <DialogTitle className="px-xs pb-xs text-body">{t("whatsNew.history")}</DialogTitle>
        {rows.map((row, index) => {
          const major = row.highlights.length > 0;
          return (
            <RowButton
              key={row.version}
              ref={(node) => { list.current[index] = node; }}
              className={cn("grid grid-cols-[auto_1fr] items-start gap-sm rounded-md px-xs py-xs", index === at && "bg-accent")}
              aria-current={index === at ? "true" : undefined}
              onClick={() => onChoose(row.version)}
              onKeyDown={(event) => {
                const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
                if (!step) return;
                event.preventDefault();
                const next = rows[Math.max(0, Math.min(rows.length - 1, index + step))]!;
                onChoose(next.version);
                list.current[rows.indexOf(next)]?.focus();
              }}
              data-update-history-version={row.version}
              data-major={major}
            >
              <span className="mt-xxs flex size-sm items-center justify-center">
                <span className={cn("rounded-full", major ? "size-sm" : "size-xs", row.isReady ? "border border-dashed border-primary" : index === at ? "bg-primary" : "bg-muted-foreground")} />
              </span>
              <span className="flex min-w-0 flex-col">
                <span className="flex items-center gap-xs font-mono text-caption text-foreground">
                  {row.version}
                  {row.isReady ? <Badge variant="outline" className="font-sans">{t("whatsNew.ready")}</Badge> : row.isNew ? <Badge variant="default" className="font-sans" data-update-history-new>{t("whatsNew.new")}</Badge> : null}
                </span>
                <span className="text-micro text-muted-foreground">
                  {releaseDate(language, row.date)} · {major ? t("whatsNew.features", { count: row.highlights.length }) : count(row.changes)}
                </span>
              </span>
            </RowButton>
          );
        })}
      </nav>
      {entry ? (
        <section className="flex min-h-0 flex-col gap-md overflow-auto p-lg" data-update-history-entry={entry.version}>
          <style>{PREVIEW_CSS}</style>
          <div className="flex items-baseline gap-sm pr-xl">
            <h3 className="font-mono text-title text-foreground">{entry.version}</h3>
            <span className="text-caption text-muted-foreground">
              {releaseDate(language, entry.date)}
              {entry.isReady ? ` · ${t("whatsNew.ready")}` : entry.version === appVersion ? ` · ${t("whatsNew.current")}` : ""}
            </span>
          </div>
          {entry.highlights.length ? (
            <ul className="grid grid-cols-[repeat(auto-fill,minmax(var(--size-picker-popover),1fr))] gap-md">
              {entry.highlights.map((row, index) => {
                const words = inLanguage(row.text, language);
                return (
                  <li key={index} className="flex flex-col overflow-hidden rounded-lg border border-border" data-update-history-feature={index}>
                    <Drawing drawing={row.drawing} playing />
                    <div className="flex flex-1 flex-col gap-xxs p-sm">
                      <span className="text-body font-medium text-foreground">{words.title}</span>
                      <span className="flex-1 break-keep text-caption text-muted-foreground">{words.line}</span>
                      {chapterKnown(row.chapter) ? (
                        <Button variant="practice" size="sm" className="mt-xs self-start" onClick={() => onTry(row.chapter!)} data-update-history-try={row.chapter}>{t("whatsNew.try")}</Button>
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : null}
          {entry.changes.length ? (
            <div className="flex flex-col gap-sm">
              <h4 className="text-caption text-muted-foreground">{t("whatsNew.listHead")}</h4>
              <ChangeList changes={entry.changes} language={language} />
            </div>
          ) : null}
          <Button variant="text-link" size="inline" className="self-start text-caption" onClick={() => onLink(releaseNotesUrl(entry.version))} data-update-history-github>{t("whatsNew.github")}</Button>
        </section>
      ) : null}
    </div>
  );
}

/** The history rows for an app at `appVersion` with `ready` downloaded, each marked NEW or ready. */
export function historyRows(entries: readonly ReleaseEntry[], record: WhatsNewRecord | null, ready: string | null): HistoryRow[] {
  return entries.map((row) => ({ ...row, isReady: row.version === ready, isNew: row.version !== ready && unseen(row.version, record) }));
}

/**
 * The window after an update and the history, connected to the host. The
 * decision is taken once per launch, when the host's file and the core's
 * snapshot are both in and no later than `OFFER_WINDOW_MS` after launch.
 */
export function WhatsNewHost({ actions }: { actions: Actions }) {
  const { i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const state = useWhatsNewState();
  const restIn = useShellStore((s) => s.rest !== null);
  const history = useUpdateHistory();
  const updates = useUpdateStatus();
  const ready = readyVersion(updates);
  const [offer, setOffer] = useState<{ offer: Offer; save: RecordUpdate } | null>(null);
  const decided = useRef(false);

  useEffect(() => {
    if (decided.current || !state || !restIn) return;
    if (Date.now() - state.launchedAt > OFFER_WINDOW_MS) {
      decided.current = true;
      return;
    }
    if (!state.feed) return;
    const result = whatsNewOffer(state.feed, state.appVersion, state.record);
    if (result.kind === "none") return;
    decided.current = true;
    if (result.kind === "quiet") saveWhatsNew(result.save);
    else setOffer({ offer: result.offer, save: result.save });
  }, [state, restIn]);

  const close = () => {
    if (!offer) return;
    saveWhatsNew(offer.save);
    setOffer(null);
  };
  const tryChapter = (chapter: string) => {
    close();
    closeUpdateHistory();
    openPractice(chapter, true);
  };

  const rows = useMemo(() => (state?.feed ? historyRows(historyVersions(state.feed, state.appVersion, ready), state.record, ready) : []), [state, ready]);
  const chosen = history.version ?? rows.find((row) => row.version === state?.appVersion)?.version ?? rows.find((row) => !row.isReady)?.version ?? null;
  // Opening a version in the history is what takes its NEW mark away (B17).
  const shownVersion = rows.find((row) => row.version === chosen) ?? null;
  useEffect(() => {
    if (history.open && shownVersion && !shownVersion.isReady && shownVersion.isNew) saveWhatsNew({ opened: shownVersion.version });
  }, [history.open, shownVersion]);

  return (
    <>
      <Dialog open={offer !== null} onOpenChange={(next) => { if (!next) close(); }}>
        {offer ? (
          <WhatsNewDialogBody
            offer={offer.offer}
            language={language}
            onTry={tryChapter}
            onHistory={() => { close(); openUpdateHistory(); }}
            onClose={close}
          />
        ) : null}
      </Dialog>
      <Dialog open={history.open} onOpenChange={(next) => { if (!next) closeUpdateHistory(); }}>
        {history.open && state ? (
          <DialogContent className="h-(--size-settings-sheet-h) w-(--size-settings-sheet-w) p-0" showCloseButton data-update-history-dialog>
            <DialogDescription className="sr-only">{state.appVersion}</DialogDescription>
            <UpdateHistoryBody
              rows={rows}
              appVersion={state.appVersion}
              chosen={chosen}
              language={language}
              state={rows.length ? "ready" : state.fetching ? "loading" : "failed"}
              onChoose={chooseUpdateHistoryVersion}
              onTry={tryChapter}
              onRetry={retryUpdateHistory}
              onLink={(url) => actions.openLink(url, true)}
            />
          </DialogContent>
        ) : null}
      </Dialog>
    </>
  );
}

export type { HistoryRow, OfferHighlight };
