// What's new scene for automations (1.2.11): the sidebar's 자동화 row and the
// automation screen at their real size, playing one automation from its making
// to its first run. A starting point is pressed and becomes the list's row; the
// night passes and the clock reaches its time; the row runs, its terminal
// reading the issues; the run ends with its ✓, the unseen dot, the sidebar's
// count and the corner notice. It is drawn from the screen's own parts and
// words (`AutomationScreen.tsx`, `sidebar-header.tsx`), so it reads as the
// screen the operator will open, and plays example work, never theirs.

import { CheckIcon, MoonIcon, MousePointer2Icon, RepeatIcon, SunIcon, WaypointsIcon } from "lucide-react";
import { agentProductName } from "../../agentAdapters";
import { AUTOMATION_EXAMPLE_KEYS, AUTOMATION_EXAMPLES } from "../../automationDraft";
import { EXAMPLE_WORDS } from "../../AutomationForm";
import { scheduleText } from "../../automationWords";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { AgentLogo } from "../agent-logo";
import { formatElapsed } from "../elapsed";
import { Badge } from "../ui/badge";
import { Toast } from "../ui/toast";
import type { SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const AUTOMATION_LOOP_MS = 11_000;
const PRESS = 1_100;
const MADE = 1_800;
const NIGHT = 3_000;
const MORNING = 4_000;
const LINE_EVERY = 650;
const DONE = 7_700;
const LINES = ["line1", "line2", "line3", "line4", "line5"] as const;
/** How long the example run took, as its row and notice say it. */
const TOOK_MS = 12 * 60_000;
/** The agent the example automation runs. */
const AGENT = "claude";
/** When the pointer sets off toward the first starting point. */
const POINT = PRESS - 700;

// Each part comes in as the screen would draw it: a row or a line rising into
// place, a count or a notice popping in. Lengths are in em so they follow the
// interface scale; reduced motion draws each part in place.
const CSS = `
@keyframes wns-rise { from { opacity: 0; transform: translateY(0.6em); } }
@keyframes wns-pop { from { opacity: 0; transform: scale(0.4); } }
@keyframes wns-press { from { transform: scale(0.3); opacity: 0.9; } to { transform: scale(1.6); opacity: 0; } }
.wns-rise { animation: wns-rise 0.4s cubic-bezier(0.2, 1, 0.3, 1) both; }
.wns-pop { animation: wns-pop 0.45s cubic-bezier(0.3, 1.6, 0.5, 1) both; }
.wns-press { animation: wns-press 0.5s ease-out both; }
.wns-pointer { transition: left 0.7s cubic-bezier(0.4, 0, 0.2, 1), top 0.7s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.3s; }
@media (prefers-reduced-motion: reduce) { .wns-rise, .wns-pop, .wns-press { animation: none; } .wns-pointer { transition: none; } }
`;

export function AutomationScene({ at }: SceneProps) {
  const { t, i18n } = useInterfaceTranslation();
  const language = i18n.language as Parameters<typeof scheduleText>[1];
  const example = AUTOMATION_EXAMPLE_KEYS[0]!;
  const name = t(EXAMPLE_WORDS[example].name);
  const made = at >= MADE;
  const night = at >= NIGHT && at < MORNING;
  const morning = at >= MORNING;
  const running = morning && at < DONE;
  const done = at >= DONE;
  const lines = morning ? LINES.slice(0, Math.min(LINES.length, Math.floor((at - MORNING) / LINE_EVERY) + 1)) : [];
  const took = formatElapsed(language, TOOK_MS);
  return (
    <div className="relative grid aspect-[2/1] grid-cols-[minmax(0,2fr)_minmax(0,5fr)] overflow-hidden bg-background text-foreground" data-whats-new-scene="automation" data-scene-beat={done ? "done" : running ? "run" : night ? "night" : made ? "made" : "pick"} aria-hidden>
      <style>{CSS}</style>
      <div className="flex flex-col border-r border-border bg-sidebar pt-md">
        <div className="flex h-(--size-tab-strip) items-center gap-sm px-md text-body text-subtle-foreground">
          <WaypointsIcon className="size-(--size-icon) shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("missionScreen.title")}</span>
        </div>
        <div className="flex h-(--size-tab-strip) items-center gap-sm bg-selected px-md text-body text-foreground">
          <RepeatIcon className="size-(--size-icon) shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("automationScreen.title")}</span>
          {done ? <span className="wns-pop text-caption tabular-nums text-foreground">1</span> : null}
        </div>
      </div>
      <div className="relative flex min-w-0 flex-col gap-md p-md">
        <div className="flex items-start gap-sm">
          <div className="flex min-w-0 flex-1 flex-col gap-xxs">
            <span className="text-subhead font-semibold">{t("automationScreen.title")}</span>
            <span className="truncate text-caption text-muted-foreground">{t("automationScreen.subtitle")}</span>
          </div>
          <span className={cn("flex shrink-0 items-center gap-xs rounded-md border px-sm py-xxs font-mono text-caption tabular-nums transition-colors", morning ? "border-primary text-foreground" : "border-border text-muted-foreground")}>
            {morning ? <SunIcon className="size-(--size-icon-sm)" /> : <MoonIcon className="size-(--size-icon-sm)" />}
            {morning ? "09:00" : "08:59"}
          </span>
        </div>
        {made ? (
          <div className={cn("wns-rise flex flex-col rounded-md border transition-colors", running ? "border-agent-working" : "border-border")}>
            <div className="flex items-center gap-sm px-md py-sm">
              <AgentLogo agent={AGENT} label={agentProductName(AGENT)} />
              <div className="flex min-w-0 flex-1 flex-col gap-xxs">
                <span className="flex min-w-0 items-center gap-xs">
                  <span className="min-w-0 truncate text-body font-semibold">{name}</span>
                  {done ? <span className="wns-pop size-(--size-tab-status-dot) shrink-0 rounded-full bg-primary" /> : null}
                  {running ? (
                    <Badge variant="outline" className="wns-pop text-agent-working">
                      <span className="size-(--size-tab-status-dot) rounded-full bg-agent-working" />
                      {t("automation.next.running")}
                    </Badge>
                  ) : null}
                </span>
                <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground">
                  <RepeatIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                  <span className="shrink-0">{scheduleText(AUTOMATION_EXAMPLES[example], language, t)}</span>
                  <span className="text-muted-foreground">·</span>
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">{t(EXAMPLE_WORDS[example].task)}</span>
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
            {morning ? (
              <div className="wns-rise flex flex-col gap-xxs border-t border-border bg-background px-md py-sm font-mono text-caption text-subtle-foreground" data-scene-terminal>
                {lines.map((line, index) => (
                  <span key={line} className={cn("wns-rise truncate", index === LINES.length - 1 && "text-primary")}>{t(`whatsNewScene.automation.${line}`)}</span>
                ))}
              </div>
            ) : null}
          </div>
        ) : (
          <div className="flex flex-col overflow-hidden rounded-md border border-border">
            {AUTOMATION_EXAMPLE_KEYS.map((key, index) => (
              <div key={key} className={cn("flex items-center gap-sm px-md py-sm transition-colors", index > 0 && "border-t border-border", index === 0 && at >= PRESS && "bg-accent")}>
                <RepeatIcon className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-body">{t(EXAMPLE_WORDS[key].name)}</span>
                <span className="shrink-0 text-caption text-subtle-foreground">{scheduleText(AUTOMATION_EXAMPLES[key], language, t)}</span>
              </div>
            ))}
          </div>
        )}
        {!made ? (
          <span className="wns-pointer pointer-events-none absolute flex text-foreground" style={{ left: at >= POINT ? "42%" : "88%", top: at >= POINT ? "30%" : "92%", opacity: at >= POINT ? 1 : 0 }}>
            {at >= PRESS ? <span className="wns-press absolute size-(--size-icon) rounded-full border border-primary" /> : null}
            <MousePointer2Icon className="size-(--size-icon) fill-foreground" />
          </span>
        ) : null}
        {done ? (
          <div className="wns-rise absolute right-md bottom-md max-w-(--size-onboarding-dialog-w)">
            <Toast level="info">{t("whatsNewScene.automation.finished", { name, time: took })}</Toast>
          </div>
        ) : null}
      </div>
      <div className={cn("pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-sm bg-scrim text-body text-foreground transition-opacity", night ? "opacity-100" : "opacity-0")}>
        <span className="flex items-center gap-sm"><MoonIcon className="size-(--size-icon)" />→<SunIcon className="size-(--size-icon) text-primary" /></span>
        <span>{t("whatsNewScene.automation.night")}</span>
      </div>
    </div>
  );
}
