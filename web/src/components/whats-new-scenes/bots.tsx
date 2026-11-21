// What's new scene for bots (1.3.0): a program outside Hide becomes a bot
// the operator can see. A request comes up in Settings › 봇 with the owl the
// bot will get, 자기 공간 is chosen and approved, and the bot's owl lands on the
// device rail beside a 전체 bot already there, which wears the lime ring. Then
// the 전체 bot's agent asks the operator, and its row in 내 차례 wears that bot's
// owl on its logo, the line saying who started it. It is drawn from the
// screens' own parts and words (`BotFace`, `AgentMark`, `externalTools.*`,
// `devices.rail.*`, `common.bot.allStarted`), and the bots and their work are
// examples, never the operator's.

import { LaptopIcon } from "lucide-react";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { BotFace } from "../bot-mark";
import { Badge } from "../ui/badge";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const BOTS_LOOP_MS = 11_000;
/** The request comes up; the pointer rests on 자기 공간, already chosen, and presses it. */
const ASKED = 300;
const CHOOSE_POINT = 1_300;
const CHOOSE = 1_900;
/** It moves to the approve button and presses it; the request leaves and the bot is connected. */
const APPROVE_POINT = 2_600;
const APPROVE = 3_300;
const CONNECTED = 3_600;
/** The owl lands on the rail. */
const LANDED = 4_100;
/** The 전체 bot's agent asks, and its row comes up in 내 차례 wearing the bot's owl. */
const ASKS = 5_600;
/** The moment a still picture shows: both owls on the rail, both bots listed, the row in 내 차례. */
export const BOTS_STILL_MS = 7_500;

/** The bot that asks in the scene, and the 전체 bot already connected. */
const ASKING = "nightly";
const FULL = "hermes";

export function BotsScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const pending = at >= ASKED && at < CONNECTED;
  const connected = at >= CONNECTED;
  const pointer = at < APPROVE_POINT
    ? { left: 22, top: 28, shown: at >= CHOOSE_POINT, pressed: at >= CHOOSE }
    : { left: 87, top: 38, shown: at < CONNECTED + 600, pressed: at >= APPROVE };
  return (
    <div className="relative flex aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="bots" data-scene-beat={at >= ASKS ? "asks" : connected ? "connected" : "request"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex w-(--size-rail) shrink-0 flex-col items-center gap-md border-r border-border bg-sidebar py-md">
        <span className="flex size-(--size-icon-button-standard) items-center justify-center rounded-md border border-border bg-muted"><LaptopIcon className="size-(--size-icon)" /></span>
        <span className="h-(--size-hairline) w-(--size-icon) bg-border" />
        <BotFace name={FULL} size="tile" mode="all" />
        {at >= LANDED ? <BotFace name={ASKING} size="tile" mode="own" className="wns-pop" /> : null}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-sm p-md">
        <div className="flex min-h-0 flex-col rounded-md border border-border">
          <span className="border-b border-border px-md py-xs text-caption font-medium text-muted-foreground">{t("settings.tabs.externalTools")}</span>
          {pending ? <Request at={at} /> : null}
          {connected ? <Connected name={ASKING} mode="own" rise /> : null}
          <Connected name={FULL} mode="all" />
        </div>
        {at >= ASKS ? (
          <div className="wns-rise flex min-h-0 flex-col rounded-md border border-border">
            <span className="px-md py-xs text-caption font-medium text-warning">{t("devices.rail.needsYou", { count: 1 })}</span>
            <AskingRow />
          </div>
        ) : null}
      </div>
      <ScenePointer {...pointer} />
    </div>
  );
}

/** The waiting request as Settings › 봇 draws it: owl, name, the two modes with its own space chosen, and Approve beside Refuse. */
function Request({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="wns-rise flex flex-col gap-sm border-b border-border px-md py-sm">
      <span className="flex items-center gap-sm">
        <BotFace name={ASKING} size="row" mode="own" />
        <span className="text-caption font-medium">{ASKING}</span>
        <span className="text-micro text-muted-foreground">{t("externalTools.pending.title")}</span>
      </span>
      <span className="grid grid-cols-2 gap-sm">
        {(["own", "all"] as const).map((mode) => (
          <span key={mode} className={cn("flex items-center gap-xs rounded-md border px-sm py-xxs text-micro", mode === "own" ? "border-primary" : "border-border")}>
            <span className={cn("size-(--size-icon-sm) shrink-0 rounded-full border", mode === "own" ? "border-primary bg-primary" : "border-border")} />
            {t(`externalTools.mode.${mode}`)}
          </span>
        ))}
      </span>
      <span className="flex justify-end gap-sm text-micro">
        <span className="px-sm py-xxs text-muted-foreground">{t("externalTools.refuse")}</span>
        <span className={cn("rounded-md px-sm py-xxs font-medium transition-colors", at >= APPROVE ? "bg-primary text-primary-foreground" : "bg-secondary text-secondary-foreground")}>{t("externalTools.approve.own")}</span>
      </span>
    </div>
  );
}

/** A connected bot's row: owl, name and its mode as a pill. */
function Connected({ name, mode, rise = false }: { name: string; mode: "own" | "all"; rise?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className={cn("flex items-center gap-sm border-b border-border px-md py-xs last:border-b-0", rise && "wns-rise")}>
      <BotFace name={name} size="row" mode={mode} />
      <span className="text-caption font-medium">{name}</span>
      <Badge variant="secondary">{t(`externalTools.mode.${mode}`)}</Badge>
    </span>
  );
}

/** The 전체 bot's agent as its 내 차례 row: the logo wearing the bot's owl, the question, and who started it. */
function AskingRow() {
  const { t } = useInterfaceTranslation();
  return (
    <span className="wns-rise flex items-start gap-sm px-md py-xs">
      <AgentMark kind="claude" bot={FULL} />
      <span className="flex min-w-0 flex-col">
        <span className="truncate text-caption font-medium">{t("whatsNewScene.bots.agent")}</span>
        <span className="truncate text-micro text-warning">{t("whatsNewScene.bots.question")}</span>
        <span className="truncate text-micro text-muted-foreground">{t("common.bot.allStarted", { name: FULL })}</span>
      </span>
    </span>
  );
}
