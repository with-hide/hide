// What's new scene for connecting computers that run Hide (1.2.12): many
// computers, one team of agents. This Mac stands in the middle with its own
// agent at work; a Mac mini, a Windows PC and a Linux server join around it,
// one after another; an agent works on each of them; the Linux server's
// agent hands its result to the Mac mini's agent as a letter; and the Windows
// PC's agent asks a question that lands in this Mac's Needs You with that
// computer's name on it. Each computer is drawn from the screen's own parts
// (`AgentMark`, `StatusMark`, `DeviceChip`, the rail's computer icons, the
// Needs You heading), and the computers and their work are examples, never
// the operator's; no system's logo is drawn, since the app draws none.

import { LaptopIcon, MonitorIcon, ServerIcon, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { AgentMark } from "../../AgentMark";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { DeviceChip } from "../device-chip";
import { StatusMark } from "../status-mark";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it, one movement at a time. */
export const CONNECT_LOOP_MS = 9_500;
/** The other computers join, one after another, each with a quiet line to This Mac. */
const JOINS = [500, 900, 1_300] as const;
/** An agent starts on each of them. */
const WORKING = 2_100;
/** The Linux server's agent sends the Mac mini's agent a letter: a dot that slides along the lines, through This Mac. */
const LETTER_SENT = 3_000;
const LETTER_THROUGH = 3_100;
const LETTER_ARRIVES = 3_700;
const LETTER_GONE = 4_300;
/** The Windows PC's agent asks; its question slides to This Mac and lands in Needs You. */
const ASKS = 4_800;
const QUESTION_SENT = 5_300;
const QUESTION_MOVES = 5_400;
const LANDS = 6_000;
/** The moment a still picture shows: every computer at work and the question in Needs You. */
export const CONNECT_STILL_MS = 8_000;

/** Where each computer stands, in percent of the scene, and the point its line meets. */
const PLACES = {
  home: { left: 30, top: 38, width: 40, x: 100, y: 52 },
  mini: { left: 3, top: 8, width: 27, x: 30, y: 22 },
  windows: { left: 70, top: 8, width: 27, x: 170, y: 22 },
  linux: { left: 3, top: 68, width: 27, x: 30, y: 80 },
} as const;
type Place = keyof typeof PLACES;

export function ConnectScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const joined = (index: number) => at >= JOINS[index]!;
  const working = at >= WORKING;
  const asks = at >= ASKS;
  const landed = at >= LANDS;
  const windows = t("whatsNewScene.connect.windows");
  const others: { place: Exclude<Place, "home">; name: string; icon: LucideIcon; agent: string; task: string }[] = [
    { place: "mini", name: t("whatsNewScene.connect.mini"), icon: MonitorIcon, agent: "claude", task: t("whatsNewScene.connect.miniTask") },
    { place: "windows", name: windows, icon: MonitorIcon, agent: "codex", task: t("whatsNewScene.connect.windowsTask") },
    { place: "linux", name: t("whatsNewScene.connect.linux"), icon: ServerIcon, agent: "codex", task: t("whatsNewScene.connect.linuxTask") },
  ];
  const beat = landed ? "landed" : asks ? "asks" : at >= LETTER_SENT ? "letter" : working ? "working" : joined(0) ? "joins" : "alone";
  return (
    <div className="relative aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="connect" data-scene-beat={beat} aria-hidden>
      <style>{SCENE_CSS}</style>
      <svg className="absolute inset-0 size-full" viewBox="0 0 200 100" preserveAspectRatio="none">
        {others.map((other, index) =>
          joined(index) ? (
            <path key={other.place} className="wns-fade stroke-border" d={`M${PLACES.home.x} ${PLACES.home.y} L${PLACES[other.place].x} ${PLACES[other.place].y}`} fill="none" strokeWidth={0.35} />
          ) : null,
        )}
      </svg>
      {/* Drawn under the computers, so a dot shows only on the lines and slips into a computer as it arrives. */}
      <Dot shown={at >= LETTER_SENT && at < LETTER_GONE} place={at >= LETTER_ARRIVES ? "mini" : at >= LETTER_THROUGH ? "home" : "linux"} />
      <Dot shown={at >= QUESTION_SENT && at < LANDS} place={at >= QUESTION_MOVES ? "home" : "windows"} />
      {others.map((other, index) =>
        joined(index) ? (
          <Computer key={other.place} place={other.place} name={other.name} icon={other.icon} className="wns-fade">
            {working ? (
              <Row className="wns-fade">
                <AgentMark kind={other.agent} />
                <span className="min-w-0 flex-1 truncate">{other.task}</span>
                <StatusMark mark={other.place === "windows" && asks ? "turn" : "going"} />
              </Row>
            ) : null}
            {other.place === "windows" && asks ? (
              <span className="wns-fade truncate px-xs text-micro text-muted-foreground">{t("whatsNewScene.connect.question")}</span>
            ) : null}
          </Computer>
        ) : null,
      )}
      <Computer place="home" name={t("common.thisMac")} icon={LaptopIcon} className="border-subtle-foreground bg-dialog">
        <Row>
          <AgentMark kind="claude" />
          <span className="min-w-0 flex-1 truncate">{t("whatsNewScene.connect.here")}</span>
          <StatusMark mark="going" />
        </Row>
        {landed ? (
          <div className="wns-fade flex flex-col gap-xxs">
            <span className="px-xs text-micro font-medium text-warning">{t("agents.group.needs_you")}</span>
            <Row>
              <StatusMark mark="turn" />
              <span className="min-w-0 flex-1 truncate">{t("whatsNewScene.connect.question")}</span>
              <DeviceChip label={windows} variant="remote" className="shrink-0" />
            </Row>
          </div>
        ) : null}
      </Computer>
    </div>
  );
}

/** One computer: its icon and name, then what runs there. */
function Computer({ place, name, icon: Icon, className, children }: { place: Place; name: string; icon: LucideIcon; className?: string; children?: ReactNode }) {
  const { left, top, width } = PLACES[place];
  return (
    <div className={cn("absolute flex flex-col gap-xxs rounded-md border border-border bg-card p-xs", className)} style={{ left: `${left}%`, top: `${top}%`, width: `${width}%` }} data-scene-computer={place}>
      <span className="flex min-w-0 items-center gap-xs px-xs text-caption font-semibold text-subtle-foreground">
        <Icon className="size-(--size-icon) shrink-0 text-muted-foreground" />
        <span className="truncate">{name}</span>
      </span>
      {children}
    </div>
  );
}

/** An agent's row, as the sidebar draws one. */
function Row({ className, children }: { className?: string; children: ReactNode }) {
  return <span className={cn("flex min-w-0 items-center gap-xs rounded-sm bg-secondary px-xs py-xxs text-caption", className)}>{children}</span>;
}

/** A letter or a question on its way between two computers: a small dot sliding along the lines from point to point. */
function Dot({ shown, place }: { shown: boolean; place: Place }) {
  const at = PLACES[place];
  return <span className="wns-glide pointer-events-none absolute size-sm rounded-full bg-device-remote" style={{ left: `${(at.x / 200) * 100}%`, top: `${at.y}%`, transform: "translate(-50%, -50%)", opacity: shown ? 1 : 0 }} />;
}
