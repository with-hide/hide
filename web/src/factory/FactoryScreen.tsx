import { BrandOwl } from "../components/brand-owl";
import { useEffect, useMemo } from "react";
import { CircleXIcon, ClockIcon, MessageSquareIcon, PauseIcon, PlayIcon, PlusIcon, SparklesIcon } from "lucide-react";
import type { Actions } from "../actions";
import { ScreenIntroLine, useScreenIntro } from "../ScreenIntro";
import { useElapsed } from "../components/elapsed";
import { Button } from "../components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { Hint } from "../components/ui/tooltip";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { useShellStore } from "../store";
import { FACTORY_TABS, useUiStore, type FactoryPlace, type FactoryTab } from "../ui";
import { CreateSheet } from "./CreateSheet";
import { FactoryBoard } from "./FactoryBoard";
import { FactoryGraph } from "./FactoryGraph";
import { FactorySettings } from "./FactorySettings";
import { Refusal } from "./Decisions";
import { Line, ReadingTable } from "./Line";
import type { FactorySummary, FactoryView } from "./model";
import { useFactoryRequest } from "./request";
import { TaskPage } from "./TaskPage";
import { outsideRead, shownFactories } from "./view";

export const TAB_LABEL = { line: "factory.tab.line", board: "factory.tab.board", graph: "factory.tab.graph", settings: "factory.tab.settings" } as const;

/**
 * The Factory screen (PRD factory-human-loop D-34, B24-B26, B31): 라인 ·
 * 보드 · 그래프 · 설정 under a header of the project filter and 비서에게
 * 묻기, with the Factory's state at the end of the tabs row, or one Task's
 * page over them. Everything it shows is the engine's summary; before the
 * first one arrives it draws its frame, and with no Factory it offers only
 * + Factory 만들기.
 */
export function FactoryScreen({ actions }: { actions: Actions }) {
  const place = useUiStore((s) => (s.screen?.kind === "factory" ? s.screen.place : null));
  const summary = useShellStore((s) => s.factory?.summary ?? null);
  // The first visit's line (PRD first-run-onboarding D-18) sits under the title, whichever body is drawn.
  const intro = useScreenIntro("factory");
  if (!place) return null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background" data-factory-screen={summary === null ? "loading" : "ready"}>
      {summary === null ? <FactorySkeleton /> : <FactoryBody summary={summary} place={place} intro={intro} actions={actions} />}
      <CreateSheet actions={actions} />
    </div>
  );
}

function FactoryBody({ summary, place, intro, actions }: { summary: FactorySummary; place: FactoryPlace; intro: boolean; actions: Actions }) {
  const open = summary.factories.filter((view) => !view.closed);
  // A filter naming a Factory that closed or went away shows them all.
  const factory = place.factory !== null && open.some((view) => view.id === place.factory) ? place.factory : null;
  const factories = useMemo(() => shownFactories(summary, factory), [summary, factory]);
  if (open.length === 0) return <NoFactory intro={intro} actions={actions} />;
  if (place.task) return <TaskPage summary={summary} place={place} actions={actions} />;
  return (
    <>
      <FactoryHeader factories={open} factory={factory} actions={actions} />
      {intro ? <ScreenIntroLine screen="factory" actions={actions} /> : null}
      <div className="flex shrink-0 items-center gap-md px-lg pb-md">
        <Tabs value={place.tab} onValueChange={(value) => useUiStore.getState().setFactoryPlace({ tab: value as FactoryTab })}>
          <TabsList data-factory-tabs="true">
            {FACTORY_TABS.map((tab) => (
              <TabsTrigger key={tab} value={tab} data-factory-tab={tab}>
                <TabLabel tab={tab} count={factory === null ? summary.my_turn : (open.find((view) => view.id === factory)?.my_turn ?? 0)} />
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <span className="flex-1" />
        <FactoryState factories={factories} />
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-auto" data-factory-body={place.tab}>
        {place.tab === "line" ? <Line summary={summary} factory={factory} actions={actions} /> : null}
        {place.tab === "board" ? <FactoryBoard factories={factories} place={place} actions={actions} inbox={summary.inbox} /> : null}
        {place.tab === "graph" ? <FactoryGraph factories={factories} filtered={factory !== null} /> : null}
        {place.tab === "settings" ? <FactorySettings factories={factories} filtered={factory !== null} actions={actions} /> : null}
      </div>
    </>
  );
}

/** 라인 carries the one person-facing number, the same the sidebar shows for the Factory row or the project row the filter keeps. */
function TabLabel({ tab, count }: { tab: FactoryTab; count: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex items-center gap-xs">
      {t(TAB_LABEL[tab])}
      {tab === "line" && count > 0 ? (
        <span className="text-warning" data-factory-line-count={count}>
          {count}
        </span>
      ) : null}
    </span>
  );
}

function FactoryHeader({ factories, factory, actions }: { factories: FactoryView[]; factory: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const view = factories.find((other) => other.id === factory) ?? null;
  return (
    <div className="flex shrink-0 items-center gap-md px-lg pt-lg pb-sm" data-factory-header="true">
      <h1 className="text-title font-semibold">{t("factory.title")}</h1>
      <Select value={factory ?? "all"} onValueChange={(value) => useUiStore.getState().setFactoryPlace({ factory: value === "all" ? null : value, column: null })}>
        <SelectTrigger size="sm" className="w-auto max-w-2/5" aria-label={t("factory.projectFilter")} data-factory-project-filter="true">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{t("factory.allProjects")}</SelectItem>
          {factories.map((view) => (
            <SelectItem key={view.id} value={view.id} data-factory-project-option={view.id}>
              {view.project_name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {view?.paused ? (
        <span className="flex shrink-0 items-center gap-xxs rounded-full bg-muted px-sm py-xxs text-caption text-subtle-foreground" data-factory-paused-chip="true">
          <PauseIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          {t("factory.pausedChip")}
        </span>
      ) : null}
      <span className="flex-1" />
      {view ? <PauseFactory view={view} actions={actions} /> : null}
      <Button variant="ghost" size="sm" data-factory-create-open="true" onClick={() => useUiStore.getState().setFactoryPlace({ create: true })}>
        <PlusIcon />
        {t("factory.create.open")}
      </Button>
      <Button variant="ghost" size="sm" data-factory-ask-secretary="true" onClick={() => actions.openSecretary()}>
        <MessageSquareIcon />
        {t("factory.secretary.ask")}
      </Button>
    </div>
  );
}

/**
 * Pauses or resumes the one Factory the header shows (D-48): a paused Factory
 * starts nothing, asks Factory AI nothing and keeps its workers asleep.
 */
function PauseFactory({ view, actions }: { view: FactoryView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const request = useFactoryRequest(actions);
  const sending = request.state.phase === "sending";
  return (
    <>
      <Refusal state={request.state} />
      {view.paused ? (
        <Button variant="outline" size="sm" disabled={sending} data-factory-pause="resume" onClick={() => request.send({ verb: "resume_factory", project: view.project })}>
          <PlayIcon />
          {t("factory.resume")}
        </Button>
      ) : (
        <Button variant="ghost" size="sm" disabled={sending} data-factory-pause="pause" onClick={() => request.send({ verb: "pause_factory", project: view.project })}>
          <PauseIcon />
          {t("factory.pause")}
        </Button>
      )}
    </>
  );
}

/**
 * The Factory's state at the end of the tabs row (B21, B24, B31): main
 * broken, today's Factory AI judgments used up, and when GitHub was last
 * read, dimmed once three reads in a row failed. Each shows only while true.
 */
function FactoryState({ factories }: { factories: FactoryView[] }) {
  const { t, i18n } = useInterfaceTranslation();
  const read = outsideRead(factories);
  const broken = factories.some((view) => view.main_broken);
  const capped = factories.find((view) => view.observer_capped) ?? null;
  return (
    <div className="flex min-w-0 shrink items-center gap-md text-caption" data-factory-state-row="true">
      {broken ? (
        <span className="flex shrink-0 items-center gap-xxs rounded-full border border-destructive px-sm py-xxs text-destructive" data-factory-main-broken="true">
          <CircleXIcon aria-hidden="true" className="size-(--size-icon-sm)" />
          {t("factory.header.mainBroken")}
        </span>
      ) : null}
      {capped ? (
        <span className="flex min-w-0 items-center gap-xxs text-warning" data-factory-ai-capped="true">
          <SparklesIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="truncate">{t("factory.header.aiCapped", { used: capped.observer_today, limit: capped.observer_limit })}</span>
        </span>
      ) : null}
      {read ? (
        <Hint label={read.stale ? t("factory.header.staleHint") : t("factory.header.readHint")}>
          <span className={cn("flex shrink-0 items-center gap-xxs text-muted-foreground", read.stale && "opacity-(--opacity-dimmed)")} data-factory-outside-read={read.stale ? "stale" : "fresh"} tabIndex={0}>
            {read.stale ? <ClockIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : null}
            {read.at === null ? t("factory.header.notRead") : read.stale ? t("factory.header.lastRead", { time: new Date(read.at).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }) }) : <ReadAgo at={read.at} />}
          </span>
        </Hint>
      ) : null}
    </div>
  );
}

/** `GitHub 읽음 3분 전`, counted on the window's one clock. */
function ReadAgo({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  const ago = useElapsed(at);
  return <>{t("factory.header.read", { time: ago ?? "" })}</>;
}

/** Before the core's first summary: the screen's frame, never an empty state that might be wrong. */
function FactorySkeleton() {
  return (
    <div className="flex flex-col gap-lg px-lg pt-lg" data-factory-skeleton="true">
      <div className="h-(--size-control) w-1/5 rounded-sm bg-muted" />
      <div className="h-(--size-control-sm) w-1/4 rounded-sm bg-muted" />
      <ReadingTable />
    </div>
  );
}

/** No Factory on this machine (B1): only the way to make one. */
function NoFactory({ intro, actions }: { intro: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <div className="flex shrink-0 items-center px-lg pt-lg pb-sm">
        <h1 className="text-title font-semibold">{t("factory.title")}</h1>
      </div>
      {intro ? <ScreenIntroLine screen="factory" actions={actions} /> : null}
      <NoFactoryButton owl={!intro} />
    </>
  );
}

function NoFactoryButton({ owl }: { owl: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-md" data-factory-empty="none">
      {owl ? <BrandOwl pose="factory" className="size-(--size-owl-empty)" /> : null}
      <Button data-factory-create-open="true" onClick={() => useUiStore.getState().setFactoryPlace({ create: true })}>
        <PlusIcon />
        {t("factory.create.open")}
      </Button>
    </div>
  );
}

/** Keeps the core's Task page open while the page shows it, and closes it when the page goes (B18). */
export function useTaskDetail(actions: Actions, task: FactoryPlace["task"]) {
  const factory = task?.factory ?? null;
  const id = task?.task ?? null;
  useEffect(() => {
    if (factory === null || id === null) return undefined;
    actions.factoryTaskOpen(factory, id);
    return () => actions.factoryTaskClose();
  }, [actions, factory, id]);
  return useShellStore((s) => (s.factoryTask && s.factoryTask.factory === factory && s.factoryTask.task === id ? s.factoryTask : null));
}
