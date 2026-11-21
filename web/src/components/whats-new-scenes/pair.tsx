// What's new scene for pair view (1.2.11): a tab of three agents at work, and
// the one press that puts an agent's terminal beside what it opened. ◧ on the
// first pane's head (or its chord) clears the others away, the terminal glides
// to the left and the Desk comes in from the right with the page the agent
// opened; the agent changes the page and it changes beside it. Choosing another
// agent in the sidebar swaps both halves to that agent and its diff; the chord
// again brings the three panes back. It is drawn from pair view's own parts and
// words (`PairLayer.tsx`: the strip, its mark and its way back), so it reads as
// the mode the operator will enter, and plays example work, never theirs.

import { ArrowLeftIcon, ArrowRightIcon, Columns2Icon, GlobeIcon, LockIcon, PresentationIcon, RotateCwIcon } from "lucide-react";
import { agentProductName } from "../../agentAdapters";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { commandLabel } from "../../shortcutLabels";
import { AgentLogo } from "../agent-logo";
import { StatusMark } from "../status-mark";
import { Kbd } from "../ui/kbd";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const PAIR_LOOP_MS = 13_000;
const POINT = 500;
const PRESS = 1_500;
const PAIRED = 1_900;
const CHANGED = 4_200;
/** When the second agent's chart lands on its slide. */
const CHART = 8_300;
const SWITCH_POINT = 6_200;
const SWITCH = 7_000;
const BACK = 10_600;
const UNPAIRED = 11_000;

type Agent = "a" | "b" | "c";
const AGENTS = [
  { id: "a", kind: "claude", lines: ["a1", "a2", "a3"] },
  { id: "b", kind: "codex", lines: ["b1", "b2", "b3"] },
  { id: "c", kind: "opencode", lines: ["c1", "c2"] },
] as const satisfies readonly { id: Agent; kind: string; lines: readonly string[] }[];
/** Where each pane stands, in percent of the body: the tab's own layout, and the left half of pair view under the strip. */
const GRID: Record<Agent, Rect> = { a: { l: 0, t: 0, w: 50, h: 100 }, b: { l: 50, t: 0, w: 50, h: 50 }, c: { l: 50, t: 50, w: 50, h: 50 } };
const STRIP = 13;
const LEFT: Rect = { l: 0, t: STRIP, w: 45, h: 100 - STRIP };
const DESK: Rect = { l: 45, t: STRIP, w: 55, h: 100 - STRIP };
const DESK_OUT: Rect = { l: 100, t: STRIP, w: 55, h: 100 - STRIP };
type Rect = { l: number; t: number; w: number; h: number };
const place = (rect: Rect) => ({ left: `${rect.l}%`, top: `${rect.t}%`, width: `${rect.w}%`, height: `${rect.h}%` });

export function PairScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const paired: Agent | null = at >= PAIRED && at < UNPAIRED ? (at >= SWITCH ? "b" : "a") : null;
  const chord = commandLabel("pair_view");
  const keycap = (at >= PRESS && at < PAIRED + 700) || (at >= BACK && at < UNPAIRED + 700);
  const pointer = at < PAIRED
    ? { left: 55, top: 3, shown: at >= POINT, pressed: at >= PRESS }
    : { left: at >= SWITCH_POINT ? 12 : 40, top: at >= SWITCH_POINT ? 17 : 95, shown: at >= SWITCH_POINT && at < SWITCH + 500, pressed: at >= SWITCH };
  return (
    <div className="relative grid aspect-[2/1] grid-cols-[minmax(0,2fr)_minmax(0,7fr)] overflow-hidden bg-background text-foreground" data-whats-new-scene="pair" data-scene-beat={paired ?? "tab"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex min-w-0 flex-col gap-xxs border-r border-border bg-sidebar pt-md">
        {AGENTS.map((agent) => (
          <div key={agent.id} className={cn("flex min-w-0 items-center gap-xs px-sm py-xs text-caption transition-colors", paired === agent.id ? "bg-selected text-foreground" : "text-subtle-foreground")}>
            <StatusMark mark="going" />
            <span className="min-w-0 flex-1 truncate">{t(`whatsNewScene.pair.${agent.id}`)}</span>
            {paired === agent.id ? <Columns2Icon className="wns-pop size-(--size-icon-sm) shrink-0 text-pair-view" /> : null}
          </div>
        ))}
      </div>
      <div className="relative min-w-0 overflow-hidden">
        {paired ? (
          <div key={paired} className="wns-rise absolute inset-x-0 top-0 flex items-center gap-sm border-b-2 border-pair-view bg-pair-view-band px-md text-caption" style={{ height: `${STRIP}%` }}>
            <span className="flex shrink-0 items-center gap-xxs rounded-full bg-pair-view px-sm font-medium text-primary-foreground">
              <Columns2Icon className="size-(--size-icon-sm)" />
              {t("panes.pair.strip")}
            </span>
            <span className="min-w-0 flex-1 truncate font-semibold">{t(`whatsNewScene.pair.${paired}`)}</span>
            <span className="flex shrink-0 items-center gap-xs rounded-md border border-border px-sm py-xxs">{t("panes.menu.pairOff")}{chord ? <Kbd>{chord}</Kbd> : null}</span>
          </div>
        ) : null}
        {AGENTS.map((agent) => {
          const shown = paired === null || paired === agent.id;
          const rect = paired === agent.id ? LEFT : GRID[agent.id];
          const count = linesShown(agent.id, at);
          return (
            <div key={agent.id} className="wns-glide absolute p-xxs" style={{ ...place(rect), opacity: shown ? 1 : 0, transform: shown ? "none" : "scale(0.92)" }}>
              <div className={cn("flex size-full flex-col overflow-hidden rounded-md border bg-background", paired === agent.id ? "border-pair-view" : "border-border")}>
                <div className="flex items-center gap-xs border-b border-border px-sm py-xxs text-caption">
                  <AgentLogo agent={agent.kind} label={agentProductName(agent.kind)} size="sm" />
                  <span className="min-w-0 flex-1 truncate">{t(`whatsNewScene.pair.${agent.id}`)}</span>
                  {agent.id === "a" ? <Columns2Icon className={cn("size-(--size-icon-sm) shrink-0", paired === "a" || (at >= PRESS && at < PAIRED) ? "text-pair-view" : "text-muted-foreground")} /> : null}
                </div>
                <div className="flex flex-col gap-xxs px-sm py-xs font-mono text-micro text-subtle-foreground">
                  {agent.lines.slice(0, count).map((line, index) => (
                    <span key={line} className={cn("wns-rise truncate", index === count - 1 && "text-foreground")}>{t(`whatsNewScene.pair.${line}`)}</span>
                  ))}
                </div>
              </div>
            </div>
          );
        })}
        <div className="wns-glide absolute p-xxs" style={place(paired ? DESK : DESK_OUT)}>
          <div className="flex size-full flex-col overflow-hidden rounded-md border border-border bg-popover">
            <div className="flex items-center gap-xs border-b border-border px-sm py-xxs text-caption text-subtle-foreground">
              {paired === "b" ? <PresentationIcon className="size-(--size-icon-sm) shrink-0" /> : <GlobeIcon className="size-(--size-icon-sm) shrink-0" />}
              <span className="truncate">{t(paired === "b" ? "whatsNewScene.pair.tabDeck" : "whatsNewScene.pair.siteBrand")}</span>
            </div>
            {paired === "b" ? <Deck charted={at >= CHART} /> : <Site changed={at >= CHANGED} />}
          </div>
        </div>
        {keycap && chord ? (
          <div key={at >= BACK ? "back" : "in"} className="wns-pop absolute inset-x-0 bottom-md flex justify-center">
            <Kbd className="px-md py-xs text-subhead shadow-overlay">{chord}</Kbd>
          </div>
        ) : null}
      </div>
      <ScenePointer {...pointer} />
    </div>
  );
}

/** How many of an agent's lines its terminal shows: the web agent's last one as it changes the page, the deck agent's as it adds the chart. */
function linesShown(agent: Agent, at: number): number {
  if (agent === "a") return 1 + (at >= 1_400 ? 1 : 0) + (at >= CHANGED - 600 ? 1 : 0);
  if (agent === "b") return 1 + (at >= SWITCH + 400 ? 1 : 0) + (at >= CHART + 900 ? 1 : 0);
  return 1 + (at >= 2_800 ? 1 : 0);
}

/** The site the web agent runs, in the browser View: its address bar, its nav and its hero, whose button the agent changes while the operator watches. */
function Site({ changed }: { changed: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-1 flex-col bg-background">
      <div className="flex items-center gap-xs border-b border-border px-sm py-xxs text-muted-foreground">
        <ArrowLeftIcon className="size-(--size-icon-sm) shrink-0" />
        <ArrowRightIcon className="size-(--size-icon-sm) shrink-0" />
        <RotateCwIcon key={changed ? "again" : "first"} className={cn("size-(--size-icon-sm) shrink-0", changed && "wns-pop text-primary")} />
        <span className="flex min-w-0 flex-1 items-center gap-xxs rounded-full bg-muted px-sm font-mono text-micro text-subtle-foreground">
          <LockIcon className="size-(--size-icon-sm) shrink-0" />
          <span className="truncate">{t("whatsNewScene.pair.tabPage")}</span>
        </span>
      </div>
      <div className="flex items-center gap-sm border-b border-border px-md py-xs text-micro">
        <span className="flex items-center gap-xxs font-semibold text-foreground"><span className="size-(--size-tab-status-dot) rounded-full bg-primary" />{t("whatsNewScene.pair.siteBrand")}</span>
        <span className="min-w-0 flex-1" />
        <span className="text-muted-foreground">{t("whatsNewScene.pair.navProduct")}</span>
        <span className="text-muted-foreground">{t("whatsNewScene.pair.navPricing")}</span>
        <span className="rounded-sm border border-border px-xs text-foreground">{t("whatsNewScene.pair.navLogin")}</span>
      </div>
      <div className="flex flex-1 items-center gap-sm bg-primary/10 px-md">
        <div className="flex min-w-0 flex-1 flex-col items-start gap-xs">
          <span className="text-body font-semibold leading-tight">{t("whatsNewScene.pair.heroTitle")}</span>
          <span className="text-micro text-muted-foreground">{t("whatsNewScene.pair.heroSub")}</span>
          <span key={changed ? "new" : "old"} className={cn("mt-xxs rounded-md px-sm py-xxs text-micro font-medium", changed ? "wns-pop bg-primary text-primary-foreground" : "bg-muted text-foreground")}>
            {t(changed ? "whatsNewScene.pair.ctaNew" : "whatsNewScene.pair.ctaOld")}
          </span>
        </div>
        <div className="flex w-2/5 shrink-0 flex-col gap-xxs rounded-md border border-border bg-background p-xs shadow-overlay">
          <span className="h-(--spacing-xs) w-1/2 rounded-full bg-primary" />
          <span className="h-(--spacing-xs) w-full rounded-full bg-muted" />
          <span className="h-(--spacing-xs) w-4/5 rounded-full bg-muted" />
          <span className="h-(--spacing-xs) w-3/5 rounded-full bg-muted" />
        </div>
      </div>
    </div>
  );
}

/** The deck the second agent makes: its slides down the side and the open slide, where the agent's chart lands while the operator watches. */
function Deck({ charted }: { charted: boolean }) {
  const { t } = useInterfaceTranslation();
  const bars = [35, 50, 62, 88];
  return (
    <div className="flex flex-1 gap-xs bg-muted p-xs">
      <div className="flex w-1/6 shrink-0 flex-col gap-xxs">
        {[0, 1, 2, 3].map((index) => (
          <span key={index} className={cn("flex aspect-video flex-col gap-px overflow-hidden rounded-sm border bg-background p-px", index === 2 ? "border-primary" : "border-border")}>
            <span className="h-px w-full bg-primary" />
            <span className="h-px w-3/5 bg-subtle-foreground" />
            {index === 2 && charted ? <span className="wns-pop flex flex-1 items-end justify-end gap-px">{bars.map((bar) => <span key={bar} className="w-1/6 bg-primary" style={{ height: `${bar}%` }} />)}</span> : null}
          </span>
        ))}
      </div>
      <div className="flex min-w-0 flex-1 items-center">
        <div className="flex aspect-video w-full flex-col overflow-hidden rounded-sm bg-background shadow-overlay">
          <span className="h-(--spacing-xs) w-full shrink-0 bg-primary" />
          <div className="flex min-h-0 flex-1 flex-col gap-xxs p-sm">
            <span className="text-caption font-semibold">{t("whatsNewScene.pair.slideTitle")}</span>
            <div className="flex min-h-0 flex-1 gap-sm">
              <div className="flex min-w-0 flex-1 flex-col gap-xxs text-micro text-subtle-foreground">
                <span className="truncate">• {t("whatsNewScene.pair.slideBullet1")}</span>
                <span className="truncate">• {t("whatsNewScene.pair.slideBullet2")}</span>
              </div>
              <div className="flex w-2/5 shrink-0 items-end gap-xxs border-b border-l border-border px-xxs">
                {charted ? bars.map((bar, index) => (
                  <span key={bar} className="wns-rise flex-1 rounded-t-sm bg-primary" style={{ height: `${bar}%`, animationDelay: `${index * 120}ms` }} />
                )) : null}
              </div>
            </div>
            <span className="self-end font-mono text-micro text-muted-foreground">3 / 4</span>
          </div>
        </div>
      </div>
    </div>
  );
}
