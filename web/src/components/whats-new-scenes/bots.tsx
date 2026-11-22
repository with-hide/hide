// What's new scene for bots (1.3.0): an AI outside Hide becomes a bot the
// operator made. Settings › 봇 opens its make card with the name typed and the
// owl it gets, 봇 공간 chosen, and the pointer presses 만들기; the bot waits for
// its AI, then is connected, and its owl lands on the device rail. Beside it a
// 전체 bot already connected wears a person in its owl's corner and has no rail
// tile. Then the 전체 bot's agent asks the operator, and its row in 내 차례
// wears that bot's owl on its logo, the line saying who started it. It is drawn
// from the screens' own parts and words (`BotFace`, `AgentMark`,
// `externalTools.*`, `devices.rail.*`, `common.bot.allStarted`), and the bots
// and their work are examples, never the operator's.

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
/** The make card comes up with the name typed and 봇 공간 chosen; the pointer presses 만들기. */
const MAKING = 300;
const MAKE_POINT = 1_300;
const MAKE = 1_900;
/** The bot waits for its AI, which joins; the bot is connected. */
const WAITING = 2_300;
const CONNECTED = 3_900;
/** The owl lands on the rail. */
const LANDED = 4_400;
/** The 전체 bot's agent asks, and its row comes up in 내 차례 wearing the bot's owl. */
const ASKS = 5_900;
/** The moment a still picture shows: the made bot's owl on the rail, both bots listed, the row in 내 차례. */
export const BOTS_STILL_MS = 7_800;

/** The bot made in the scene, and the 전체 bot already connected. */
const MADE = "nightly";
const FULL = "hermes";

export function BotsScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const making = at >= MAKING && at < WAITING;
  const waiting = at >= WAITING && at < CONNECTED;
  const connected = at >= CONNECTED;
  const pointer = { left: 86, top: 44, shown: at >= MAKE_POINT && at < WAITING + 400, pressed: at >= MAKE };
  return (
    <div className="relative flex aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="bots" data-scene-beat={at >= ASKS ? "asks" : connected ? "connected" : waiting ? "waiting" : "make"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex w-(--size-rail) shrink-0 flex-col items-center gap-md border-r border-border bg-sidebar py-md">
        <span className="flex size-(--size-icon-button-standard) items-center justify-center rounded-md border border-border bg-muted"><LaptopIcon className="size-(--size-icon)" /></span>
        {at >= LANDED ? <span className="h-(--size-hairline) w-(--size-icon) bg-border" /> : null}
        {at >= LANDED ? <BotFace name={MADE} size="tile" className="wns-pop" /> : null}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-sm p-md">
        <div className="flex min-h-0 flex-col rounded-md border border-border">
          <span className="border-b border-border px-md py-xs text-caption font-medium text-muted-foreground">{t("settings.tabs.externalTools")}</span>
          {making ? <Make at={at} /> : null}
          {waiting ? <Waiting /> : null}
          {connected ? <Connected name={MADE} mode="own" rise /> : null}
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

/** The make card as Settings › 봇 draws it: the name with the owl it gets, the two modes with its own space chosen, and 만들기. */
function Make({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="wns-rise flex flex-col gap-sm border-b border-border px-md py-sm">
      <span className="text-micro text-muted-foreground">{t("externalTools.create.title")}</span>
      <span className="flex items-center gap-sm">
        <BotFace name={MADE} size="row" />
        <span className="flex-1 rounded-md border border-border px-sm py-xxs font-mono text-caption">{MADE}</span>
      </span>
      <span className="grid grid-cols-2 gap-sm">
        {(["own", "all"] as const).map((mode) => (
          <span key={mode} className={cn("flex items-center gap-xs rounded-md border px-sm py-xxs text-micro", mode === "own" ? "border-primary" : "border-border")}>
            <span className={cn("size-(--size-icon-sm) shrink-0 rounded-full border", mode === "own" ? "border-primary bg-primary" : "border-border")} />
            {t(`externalTools.mode.${mode}`)}
          </span>
        ))}
      </span>
      <span className="flex justify-end text-micro">
        <span className={cn("rounded-md px-sm py-xxs font-medium transition-colors", "bg-primary text-primary-foreground", at >= MAKE && "opacity-(--opacity-dimmed)")}>{t("externalTools.create.submit")}</span>
      </span>
    </div>
  );
}

/** The made bot waiting for its AI: the owl dimmed, under the group's title. */
function Waiting() {
  const { t } = useInterfaceTranslation();
  return (
    <span className="wns-rise flex items-center gap-sm border-b border-border px-md py-xs">
      <BotFace name={MADE} size="row" className="opacity-(--opacity-dimmed)" />
      <span className="text-caption font-medium">{MADE}</span>
      <Badge variant="secondary">{t("externalTools.mode.own")}</Badge>
      <span className="ml-auto text-micro text-muted-foreground">{t("externalTools.invites.title")}</span>
    </span>
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
