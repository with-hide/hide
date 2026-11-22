// What's new scene for connecting two computers that run Hide (1.3.0): the
// link another computer shared is pasted in Settings › Devices and Connect is
// pressed; the pairing window shows the four words both screens show, They
// match is pressed and the two are connected. The Mac mini's tile lands on
// the rail under This Mac, its agent's question comes into this computer's
// Needs You with the Mac mini's name and a count on its tile, and pressing the
// tile shows the Mac mini's own Hide screen in the same window. It is drawn
// from the screens' own parts and words (`PairWords`, the rail tile's look,
// `DeviceChip`, `StatusMark`, `pairing.*`, `devices.*`), and the computers
// and their work are examples, never the operator's.

import { LaptopIcon } from "lucide-react";
import { AgentMark } from "../../AgentMark";
import { tileMonogram } from "../../devices";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { PairWords } from "../../settings/PairingRows";
import { DeviceChip } from "../device-chip";
import { StatusMark } from "../status-mark";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const CONNECT_LOOP_MS = 14_000;
/** The link is typed into the field, then the pointer presses Connect. */
const TYPING = 400;
const TYPED = 1_900;
const CONNECT_POINT = 2_300;
const CONNECT = 2_900;
/** The pairing window with its four words; the pointer presses They match and the two are connected. */
const WORDS = 3_300;
const SAME_POINT = 4_600;
const SAME = 5_300;
const PAIRED = 5_600;
/** The window and Settings close; the Mac mini's tile lands on the rail. */
const LANDED = 6_800;
/** The Mac mini's agent asks: its row comes into Needs You and a count onto its tile. */
const ASKS = 8_000;
/** The pointer presses the Mac mini's tile and the window shows that computer's own Hide screen. */
const TILE_POINT = 9_600;
const TILE = 10_200;
/** The moment a still picture shows: the tile chosen, its row in Needs You, its own screen. */
export const CONNECT_STILL_MS = 12_000;

const LINK = "hide://pair#mac-mini.4f7c2a9e1b";
const MINI = "Mac mini";
/** The tile's look on the rail (`device-rail.tsx`): a square, the name's letters or This Mac's laptop. */
const TILE_LOOK = "relative flex size-(--size-icon-button-standard) shrink-0 items-center justify-center rounded-md bg-secondary text-body font-semibold";

export function ConnectScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const settings = at < LANDED;
  const comparing = at >= WORDS && at < LANDED;
  const landed = at >= LANDED;
  const asks = at >= ASKS;
  const remote = at >= TILE;
  const pointer = at < SAME_POINT
    ? { left: 74, top: 36, shown: at >= CONNECT_POINT && at < WORDS, pressed: at >= CONNECT }
    : at < TILE_POINT
      ? { left: 71, top: 78, shown: at < PAIRED + 500, pressed: at >= SAME }
      : { left: 5, top: 30, shown: at < TILE + 900, pressed: at >= TILE };
  const typed = at < TYPING ? "" : LINK.slice(0, Math.ceil((LINK.length * Math.min(at - TYPING, TYPED - TYPING)) / (TYPED - TYPING)));
  return (
    <div className="relative flex aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="connect" data-scene-beat={remote ? "screen" : asks ? "asks" : landed ? "landed" : comparing ? "words" : "link"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex w-(--size-rail) shrink-0 flex-col items-center gap-md border-r border-border bg-sidebar py-md">
        <span className={cn(TILE_LOOK, remote ? "text-subtle-foreground" : "text-foreground ring-2 ring-foreground ring-offset-2 ring-offset-sidebar")}>
          <LaptopIcon className="size-(--size-icon-lg)" />
        </span>
        {landed ? (
          <span className={cn(TILE_LOOK, "wns-pop transition-shadow", remote ? "text-foreground ring-2 ring-foreground ring-offset-2 ring-offset-sidebar" : "text-subtle-foreground")}>
            {tileMonogram(MINI)}
            {asks ? <span className="wns-pop absolute -top-xs -right-xs flex h-(--size-rail-badge) min-w-(--size-rail-badge) items-center justify-center rounded-full bg-warning px-xxs text-(length:--size-rail-badge-text) leading-none font-semibold text-status-foreground tabular-nums ring-2 ring-sidebar">1</span> : null}
          </span>
        ) : null}
      </div>
      {remote ? <MiniScreen /> : (
        <>
          <div className="flex w-2/7 min-w-0 shrink-0 flex-col border-r border-border bg-sidebar pt-md">
            {asks ? (
              <div className="wns-rise flex flex-col gap-xxs">
                <span className="px-md text-micro font-medium text-warning">{t("agents.group.needs_you")}</span>
                <MiniRow />
              </div>
            ) : null}
          </div>
          <div className="relative flex min-w-0 flex-1 flex-col gap-sm p-md">
            {settings ? <Devices typed={typed} at={at} /> : <Work />}
            {comparing ? <Pairing at={at} /> : null}
          </div>
        </>
      )}
      <ScenePointer {...pointer} />
    </div>
  );
}

/** Settings › Devices with Add device open at Paste a link: the field, then Connect. */
function Devices({ typed, at }: { typed: string; at: number }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className={cn("flex min-h-0 flex-col rounded-md border border-border transition-opacity", at >= WORDS && "opacity-(--opacity-dimmed)")}>
      <span className="border-b border-border px-md py-xs text-caption font-medium text-muted-foreground">{t("settings.tabs.devices")} · {t("devices.addTitle")}</span>
      <div className="flex flex-col gap-sm px-md py-sm">
        <span className="text-caption font-semibold text-subtle-foreground">{t("pairing.add.linkTitle")}</span>
        <span className="flex items-center gap-sm">
          <span className={cn("min-w-0 flex-1 truncate rounded-md border px-sm py-xxs font-mono text-micro", typed ? "border-ring text-foreground" : "border-border text-muted-foreground")}>{typed || t("pairing.add.linkPlaceholder")}</span>
          <span className={cn("shrink-0 rounded-md px-sm py-xxs text-micro font-medium transition-colors", at >= TYPED ? "bg-secondary text-secondary-foreground" : "bg-secondary text-muted-foreground", at >= CONNECT && "bg-accent text-foreground")}>{t("pairing.add.connect")}</span>
        </span>
      </div>
    </div>
  );
}

/** The pairing window: the four words, then They match pressed and Connected. */
function Pairing({ at }: { at: number }) {
  const { t } = useInterfaceTranslation();
  const paired = at >= PAIRED;
  return (
    <div className="wns-rise absolute inset-x-xl top-xl flex flex-col gap-sm rounded-lg border border-border bg-dialog p-md shadow-modal">
      <span className="text-body font-semibold">{t("pairing.join.title", { name: MINI })}</span>
      <span className="text-micro text-muted-foreground">{t("pairing.join.compare")}</span>
      <PairWords words={t("whatsNewScene.connect.words").split(" ")} />
      <span className="flex items-center justify-end gap-sm text-micro">
        {paired ? (
          <span className="wns-pop mr-auto font-medium text-success">{t("pairing.join.paired")}</span>
        ) : (
          <span className="px-sm py-xxs text-muted-foreground">{t("pairing.join.different")}</span>
        )}
        <span className={cn("rounded-md px-sm py-xxs font-medium transition-colors", at >= SAME ? "bg-primary text-primary-foreground" : "bg-secondary text-secondary-foreground")}>{t("pairing.join.same")}</span>
      </span>
    </div>
  );
}

/** This computer's own work, as it was before the two connected. */
function Work() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="wns-fade flex min-h-0 flex-col rounded-md border border-border">
      <span className="flex items-center gap-sm px-md py-xs">
        <AgentMark kind="claude" />
        <span className="min-w-0 truncate text-caption font-medium">{t("whatsNewScene.connect.here")}</span>
      </span>
    </div>
  );
}

/** The Mac mini's agent as a row of this computer's Needs You: its question, with the Mac mini's name on it. */
function MiniRow() {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex items-start gap-xs px-md py-xs">
      <StatusMark mark="turn" />
      <span className="flex min-w-0 flex-col">
        <span className="flex min-w-0 items-center gap-xs">
          <span className="min-w-0 truncate text-caption text-foreground">{t("whatsNewScene.connect.task")}</span>
          <DeviceChip label={MINI} variant="remote" className="max-w-2/5" />
        </span>
        <span className="truncate text-micro text-muted-foreground">{t("whatsNewScene.connect.question")}</span>
      </span>
    </span>
  );
}

/** The Mac mini's own Hide screen in this window: its project, its agent and the question it asks there. */
function MiniScreen() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="wns-fade flex min-w-0 flex-1">
      <div className="flex w-2/7 min-w-0 shrink-0 flex-col gap-xxs border-r border-border bg-sidebar pt-md">
        <span className="truncate px-md text-caption font-semibold text-subtle-foreground">{t("whatsNewScene.connect.place")}</span>
        <span className="flex items-center gap-xs bg-selected px-md py-xs">
          <StatusMark mark="turn" />
          <span className="min-w-0 truncate text-caption text-foreground">{t("whatsNewScene.connect.task")}</span>
        </span>
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-sm p-md">
        <span className="flex items-center gap-sm">
          <AgentMark kind="codex" />
          <span className="min-w-0 truncate text-caption font-medium">{t("whatsNewScene.connect.task")}</span>
        </span>
        <div className="flex flex-col gap-xxs rounded-md border border-border bg-card px-md py-sm font-mono text-micro">
          <span className="text-muted-foreground">{t("whatsNewScene.connect.log")}</span>
          <span className="font-semibold text-foreground">{t("whatsNewScene.connect.question")}</span>
          <span className="text-warning">› 1. {t("whatsNewScene.connect.yes")}</span>
          <span className="text-subtle-foreground">  2. {t("whatsNewScene.connect.no")}</span>
        </div>
      </div>
    </div>
  );
}
