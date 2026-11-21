// What's new scene for automations (1.2.11): the sidebar and the automation
// screen at their real size, playing one automation from its making to the
// operator reading what it found. The starting points are read first; the first,
// a morning briefing, is pressed and becomes the
// list's row; the night passes and the clock reaches its time; the row runs,
// its terminal reading the issues; the run ends with its ✓ and the corner
// notice, and what it wrote stands in Needs You as To look at; pressing that
// row opens the report beside the list and takes the row away. It is drawn
// from the screen's own parts and words (`AutomationScreen.tsx`,
// `sidebar-header.tsx`, the Needs You row), so it reads as the screen the
// operator will open, and plays example work, never theirs.

import { CheckIcon, FileTextIcon, MoonIcon, RepeatIcon, SunIcon, WaypointsIcon } from "lucide-react";
import { agentProductName } from "../../agentAdapters";
import { scheduleText } from "../../automationWords";
import type { AutomationSchedule } from "../../snapshot";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { AgentLogo } from "../agent-logo";
import { formatElapsed } from "../elapsed";
import { StatusMark } from "../status-mark";
import { Badge } from "../ui/badge";
import { Toast } from "../ui/toast";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const AUTOMATION_LOOP_MS = 17_500;
/** The starting points are read first; then the pointer sets off, rests on the first one, and presses it. */
const POINT = 1_600;
const HOVER = 2_400;
const PRESS = 3_200;
const MADE = 3_900;
const NIGHT = 5_100;
const MORNING = 6_100;
const LINE_EVERY = 650;
const DONE = 9_800;
/** When the pointer sets off toward the Needs You row, presses it, and the report opens. */
const LOOK_POINT = 11_400;
const LOOK_PRESS = 12_300;
const LOOKED = 12_700;
const LINES = ["line1", "line2", "line3", "line4", "line5"] as const;
/** How long the example run took, as its row and notice say it. */
const TOOK_MS = 12 * 60_000;
/** The agent the example automation runs. */
const AGENT = "claude";
/** The starting points the scene offers, everyday work rather than code; the first is the one made. */
const EXAMPLES: { key: "briefing" | "sales" | "mail"; schedule: AutomationSchedule }[] = [
  { key: "briefing", schedule: { kind: "weekdays", time: "08:00" } },
  { key: "sales", schedule: { kind: "weekly", days: ["mon"], time: "09:00" } },
  { key: "mail", schedule: { kind: "daily", time: "18:00" } },
];

export function AutomationScene({ at }: SceneProps) {
  const { t, i18n } = useInterfaceTranslation();
  const language = i18n.language as Parameters<typeof scheduleText>[1];
  const example = EXAMPLES[0]!;
  const name = t(`whatsNewScene.automation.${example.key}.name`);
  const made = at >= MADE;
  const night = at >= NIGHT && at < MORNING;
  const morning = at >= MORNING;
  const running = morning && at < DONE;
  const done = at >= DONE;
  const looked = at >= LOOKED;
  const lines = morning ? LINES.slice(0, Math.min(LINES.length, Math.floor((at - MORNING) / LINE_EVERY) + 1)) : [];
  const took = formatElapsed(language, TOOK_MS);
  const pointer = at < MADE
    ? { left: at >= POINT ? 58 : 92, top: at >= POINT ? 30 : 92, shown: at >= POINT, pressed: at >= PRESS }
    : { left: at >= LOOK_POINT ? 15 : 60, top: at >= LOOK_POINT ? 39 : 92, shown: at >= LOOK_POINT && at < LOOKED + 500, pressed: at >= LOOK_PRESS };
  return (
    <div className="relative grid aspect-[2/1] grid-cols-[minmax(0,2fr)_minmax(0,5fr)] overflow-hidden bg-background text-foreground" data-whats-new-scene="automation" data-scene-beat={looked ? "looked" : done ? "done" : running ? "run" : night ? "night" : made ? "made" : "pick"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex min-w-0 flex-col border-r border-border bg-sidebar pt-md">
        <div className="flex h-(--size-tab-strip) items-center gap-sm px-md text-body text-subtle-foreground">
          <WaypointsIcon className="size-(--size-icon) shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("missionScreen.title")}</span>
        </div>
        <div className={cn("flex h-(--size-tab-strip) items-center gap-sm px-md text-body", looked ? "text-subtle-foreground" : "bg-selected text-foreground")}>
          <RepeatIcon className="size-(--size-icon) shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("automationScreen.title")}</span>
          {done && !looked ? <span className="wns-pop text-caption tabular-nums text-foreground">1</span> : null}
        </div>
        {done && !looked ? (
          <div className="wns-rise mt-md flex flex-col gap-xxs">
            <span className="px-md text-micro font-medium text-warning">{t("agents.group.needs_you")}</span>
            <div className={cn("flex min-w-0 items-center gap-xs px-md py-xs transition-colors", at >= LOOK_PRESS && "bg-accent")}>
              <StatusMark mark="look" />
              <span className="min-w-0 flex-1 truncate text-caption text-foreground">{t("hideAi.task.automation", { name })}</span>
            </div>
            <span className="truncate px-md pl-xl text-micro text-muted-foreground">{t("agents.mark.look")}</span>
          </div>
        ) : null}
      </div>
      <div className="relative flex min-w-0 flex-col gap-md p-md">
        <div className="flex items-start gap-sm">
          <div className="flex min-w-0 flex-1 flex-col gap-xxs">
            <span className="text-subhead font-semibold">{t("automationScreen.title")}</span>
            <span className="truncate text-caption text-muted-foreground">{t("automationScreen.subtitle")}</span>
          </div>
          <span className={cn("flex shrink-0 items-center gap-xs rounded-md border px-sm py-xxs font-mono text-caption tabular-nums transition-colors", morning ? "border-primary text-foreground" : "border-border text-muted-foreground")}>
            {morning ? <SunIcon className="size-(--size-icon-sm)" /> : <MoonIcon className="size-(--size-icon-sm)" />}
            {morning ? "08:00" : "07:59"}
          </span>
        </div>
        {made ? (
          <div className={cn("wns-rise flex flex-col rounded-md border transition-colors", running ? "border-agent-working" : "border-border")}>
            <div className="flex items-center gap-sm px-md py-sm">
              <AgentLogo agent={AGENT} label={agentProductName(AGENT)} />
              <div className="flex min-w-0 flex-1 flex-col gap-xxs">
                <span className="flex min-w-0 items-center gap-xs">
                  <span className="min-w-0 truncate text-body font-semibold">{name}</span>
                  {done && !looked ? <span className="wns-pop size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary" /> : null}
                  {running ? (
                    <Badge variant="outline" className="wns-pop text-agent-working">
                      <span className="size-(--size-tab-status-dot) rounded-full bg-agent-working" />
                      {t("automation.next.running")}
                    </Badge>
                  ) : null}
                </span>
                <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground">
                  <RepeatIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                  <span className="shrink-0">{scheduleText(example.schedule, language, t)}</span>
                  <span className="text-muted-foreground">·</span>
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">{t("whatsNewScene.automation.briefing.task")}</span>
                </span>
              </div>
              <div className="flex shrink-0 flex-col items-end gap-xxs">
                <span className="text-micro text-muted-foreground">{done ? t("automationScreen.recent") : t("automationScreen.col.next")}</span>
                {done ? (
                  <span className="wns-pop flex items-center gap-xxs text-caption text-subtle-foreground"><CheckIcon className="size-(--size-icon-sm) text-primary" />{took}</span>
                ) : (
                  <span className="text-caption tabular-nums text-foreground">{t("whatsNewScene.automation.next")}</span>
                )}
              </div>
            </div>
            {morning && !looked ? (
              <div className="wns-rise flex flex-col gap-xxs border-t border-border bg-background px-md py-sm font-mono text-caption text-subtle-foreground" data-scene-terminal>
                {lines.map((line, index) => (
                  <span key={line} className={cn("wns-rise truncate", index === LINES.length - 1 && "text-primary")}>{t(`whatsNewScene.automation.${line}`)}</span>
                ))}
              </div>
            ) : null}
          </div>
        ) : (
          <div className="flex flex-col overflow-hidden rounded-md border border-border">
            {EXAMPLES.map(({ key, schedule }, index) => (
              <div key={key} className={cn("wns-rise flex items-center gap-sm px-md py-sm transition-colors", index > 0 && "border-t border-border", index === 0 && at >= HOVER && (at >= PRESS ? "bg-selected" : "bg-accent"))} style={{ animationDelay: `${index * 180}ms` }}>
                <RepeatIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-body">{t(`whatsNewScene.automation.${key}.name`)}</span>
                <span className="shrink-0 text-caption text-subtle-foreground">{scheduleText(schedule, language, t)}</span>
              </div>
            ))}
          </div>
        )}
        {done && !looked ? (
          <div className="wns-rise absolute right-md bottom-md max-w-(--size-onboarding-dialog-w)">
            <Toast level="info">{t("whatsNewScene.automation.finished", { name, time: took })}</Toast>
          </div>
        ) : null}
        {looked ? <Report /> : null}
      </div>
      <ScenePointer {...pointer} />
      <div className={cn("pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-sm bg-scrim text-body text-foreground transition-opacity", night ? "opacity-100" : "opacity-0")}>
        <span className="flex items-center gap-sm"><MoonIcon className="size-(--size-icon)" />→<SunIcon className="size-(--size-icon) text-primary" /></span>
        <span>{t("whatsNewScene.automation.night")}</span>
      </div>
    </div>
  );
}

/** What the run wrote, opened beside the list as the View it opened: a briefing laid out as a document, the one thing to look at first, the news, the day's calendar and the to-dos. */
function Report() {
  const { t } = useInterfaceTranslation();
  const rise = (order: number) => ({ animationDelay: `${120 + order * 110}ms` });
  const news = [["news1", "source1"], ["news2", "source2"], ["news3", "source3"]] as const;
  const day = [["10:00", "s1", false], ["15:00", "s2", true], ["19:00", "s3", false]] as const;
  return (
    <div className="wns-rise absolute inset-y-sm right-sm left-md flex flex-col overflow-hidden rounded-md border border-border bg-popover shadow-overlay">
      <div className="flex items-center gap-xs border-b border-border px-md py-xxs text-micro text-subtle-foreground">
        <FileTextIcon className="size-(--size-icon-sm) shrink-0" />
        <span className="truncate font-mono">{t("whatsNewScene.automation.reportFile")}</span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-xs px-md py-sm">
        <div className="wns-rise flex items-baseline gap-sm" style={rise(0)}>
          <span className="text-body font-semibold">{t("whatsNewScene.automation.reportTitle")}</span>
          <span className="text-micro text-muted-foreground">{t("whatsNewScene.automation.report.date")}</span>
        </div>
        <div className="wns-rise flex items-center gap-sm rounded-md border-l-2 border-warning bg-warning/10 px-sm py-xs" style={rise(1)}>
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="text-micro font-medium text-warning">{t("whatsNewScene.automation.report.mustTitle")}</span>
            <span className="truncate text-caption font-medium">{t("whatsNewScene.automation.report.must")}</span>
          </span>
          <span className="shrink-0 text-micro text-primary">{t("whatsNewScene.automation.report.mustAction")}</span>
        </div>
        <div className="flex min-h-0 flex-1 gap-md">
          <div className="flex min-w-0 flex-1 flex-col gap-xxs">
            <span className="wns-rise text-micro font-semibold text-muted-foreground" style={rise(2)}>{t("whatsNewScene.automation.report.newsTitle")}</span>
            {news.map(([line, source], index) => (
              <span key={line} className="wns-rise flex min-w-0 items-center gap-xs text-micro" style={rise(3 + index)}>
                <span className="shrink-0 rounded-sm bg-muted px-xs text-muted-foreground">{t(`whatsNewScene.automation.report.${source}`)}</span>
                <span className="truncate text-foreground">{t(`whatsNewScene.automation.report.${line}`)}</span>
              </span>
            ))}
            <span className="wns-rise mt-xs text-micro font-semibold text-muted-foreground" style={rise(6)}>{t("whatsNewScene.automation.report.priceTitle")}</span>
            {([["us", 100, false], ["rival", 90, true]] as const).map(([who, width, cut], index) => (
              <span key={who} className="wns-rise flex items-center gap-xs text-micro" style={rise(7 + index)}>
                <span className="w-1/5 shrink-0 truncate text-subtle-foreground">{t(`whatsNewScene.automation.report.${who}`)}</span>
                <span className="flex min-w-0 flex-1 items-center gap-xs">
                  <span className={cn("h-(--spacing-sm) rounded-sm", cut ? "bg-warning" : "bg-primary")} style={{ width: `${width * 0.8}%` }} />
                  {cut ? <span className="shrink-0 font-medium text-warning">-10%</span> : null}
                </span>
              </span>
            ))}
          </div>
          <div className="flex w-2/5 shrink-0 flex-col gap-xxs">
            <span className="wns-rise text-micro font-semibold text-muted-foreground" style={rise(2)}>{t("whatsNewScene.automation.report.dayTitle")}</span>
            {day.map(([time, line, key], index) => (
              <span key={line} className={cn("wns-rise flex min-w-0 items-center gap-xs text-micro", key ? "font-medium text-foreground" : "text-subtle-foreground")} style={rise(3 + index)}>
                <span className={cn("size-(--size-tab-status-dot) shrink-0 rounded-full", key ? "bg-warning" : "bg-border")} />
                <span className="shrink-0 font-mono tabular-nums">{time}</span>
                <span className="truncate">{t(`whatsNewScene.automation.report.${line}`)}</span>
              </span>
            ))}
            <span className="wns-rise mt-xxs text-micro font-semibold text-muted-foreground" style={rise(6)}>{t("whatsNewScene.automation.report.todoTitle")}</span>
            {(["todo1", "todo2"] as const).map((line, index) => (
              <span key={line} className="wns-rise flex min-w-0 items-center gap-xs text-micro text-foreground" style={rise(7 + index)}>
                <span className="size-(--size-icon-sm) shrink-0 rounded-sm border border-border" />
                <span className="truncate">{t(`whatsNewScene.automation.report.${line}`)}</span>
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
