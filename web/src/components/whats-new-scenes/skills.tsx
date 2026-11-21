// What's new scene for Skills & MCP (1.2.11): not the settings page itself but
// what it does, drawn as a picture. An address goes into Add; the skill it
// names comes up as a card; lines run from it to every agent on this machine,
// each logo taking its ✓ in turn, the page's own promise (`store.subtitle`)
// under them; then another machine comes in and the same skill is copied to
// it, its agents taking their ✓ too. Agents are their real logos
// (`AgentLogo`), and the skill is an example, never one the operator has.

import { CheckIcon, MonitorIcon, SparklesIcon } from "lucide-react";
import { agentProductName } from "../../agentAdapters";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { AgentLogo } from "../agent-logo";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const SKILLS_LOOP_MS = 11_000;
const TYPE_FROM = 300;
const TYPE_EVERY = 45;
const POINT = 1_000;
const PRESS = 1_700;
const CARD = 2_000;
const FAN = 2_500;
const FAN_EVERY = 220;
const DEVICE = 5_600;
const COPY = 6_300;
const LANDED = 7_100;
const THERE_EVERY = 220;

/** The agents of this machine, and of the other, with where each logo stands (percent of the scene). */
const HERE = ["claude", "codex", "opencode", "antigravity", "cursor", "grok"].map((kind, index) => ({ kind, x: 8 + index * 10.4, y: 80 }));
const THERE = ["claude", "codex", "opencode"].map((kind, index) => ({ kind, x: 78 + index * 8, y: 80 }));
const CARD_HERE = { x: 34, y: 47 };
const CARD_THERE = { x: 86, y: 47 };
const centered = (x: number, y: number) => ({ left: `${x}%`, top: `${y}%`, transform: "translate(-50%, -50%)" });

export function SkillsScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const address = t("whatsNewScene.skills.add");
  const typed = address.slice(0, Math.max(0, Math.floor((at - TYPE_FROM) / TYPE_EVERY)));
  const card = at >= CARD;
  const device = at >= DEVICE;
  const landed = at >= LANDED;
  const hereDone = (index: number) => at >= FAN + 350 + index * FAN_EVERY;
  const thereDone = (index: number) => at >= LANDED + 300 + index * THERE_EVERY;
  return (
    <div className="relative aspect-[2/1] overflow-hidden bg-background text-foreground" data-whats-new-scene="skills" data-scene-beat={landed ? "copied" : device ? "device" : card ? "fan" : "add"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="absolute inset-y-md left-md flex flex-col rounded-lg border border-dashed border-border p-sm" style={{ right: "31%" }}>
        <span className="flex items-center gap-xs text-micro text-muted-foreground"><MonitorIcon className="size-(--size-icon-sm)" />{t("whatsNewScene.skills.here")}</span>
      </div>
      {device ? (
        <div className="wns-fade absolute inset-y-md right-md flex flex-col rounded-lg border border-dashed border-border p-sm" style={{ left: "71%" }}>
          <span className="flex items-center gap-xs text-micro text-muted-foreground"><MonitorIcon className="size-(--size-icon-sm)" />{t("whatsNewScene.skills.there")}</span>
        </div>
      ) : null}
      <svg className="pointer-events-none absolute inset-0 size-full" viewBox="0 0 100 100" preserveAspectRatio="none">
        {at >= FAN ? HERE.map((agent, index) => (
          <line key={agent.kind} x1={CARD_HERE.x} y1={CARD_HERE.y + 9} x2={agent.x} y2={agent.y - 7} vectorEffect="non-scaling-stroke"
            className={cn("wns-fade", hereDone(index) ? "stroke-primary" : "stroke-border")} strokeWidth={1.5} style={{ animationDelay: `${index * FAN_EVERY}ms` }} />
        )) : null}
        {landed ? THERE.map((agent, index) => (
          <line key={agent.kind} x1={CARD_THERE.x} y1={CARD_THERE.y + 9} x2={agent.x} y2={agent.y - 7} vectorEffect="non-scaling-stroke"
            className={cn("wns-fade", thereDone(index) ? "stroke-primary" : "stroke-border")} strokeWidth={1.5} style={{ animationDelay: `${index * THERE_EVERY}ms` }} />
        )) : null}
      </svg>
      {!card ? (
        <div className="absolute flex items-center gap-xs" style={{ ...centered(34, 30), width: "50%" }}>
          <span className="min-w-0 flex-1 truncate rounded-md border border-border bg-popover px-sm py-xs font-mono text-caption text-foreground">{typed}<span className="text-primary">▏</span></span>
          <span className={cn("shrink-0 rounded-md px-sm py-xs text-caption font-medium transition-colors", at >= PRESS ? "bg-primary text-primary-foreground" : "bg-muted text-foreground")}>{t("store.add")}</span>
        </div>
      ) : null}
      {card ? <SkillCard x={CARD_HERE.x} y={CARD_HERE.y} /> : null}
      {at >= COPY ? (
        <div className="wns-glide absolute" style={{ ...centered(landed ? CARD_THERE.x : CARD_HERE.x, CARD_HERE.y), opacity: landed ? 1 : 0.85 }}>
          <SkillCardBody small />
        </div>
      ) : null}
      {at >= COPY && !landed ? (
        <span className="wns-fade absolute text-micro text-primary" style={centered(60, 33)}>{t("store.copyFrom", { device: t("whatsNewScene.skills.here") })}</span>
      ) : null}
      {HERE.map((agent, index) => <Logo key={agent.kind} kind={agent.kind} x={agent.x} y={agent.y} checked={hereDone(index)} />)}
      {device ? THERE.map((agent, index) => <Logo key={agent.kind} kind={agent.kind} x={agent.x} y={agent.y} checked={thereDone(index)} />) : null}
      {at >= FAN + 350 + HERE.length * FAN_EVERY ? (
        <span className="wns-rise absolute text-caption font-medium text-foreground" style={centered(34, 93)}>{t("store.subtitle", { device: t("whatsNewScene.skills.here") })}</span>
      ) : null}
      <ScenePointer left={at >= POINT ? 57 : 70} top={at >= POINT ? 31 : 90} shown={at >= POINT && at < CARD + 300} pressed={at >= PRESS} />
    </div>
  );
}

function SkillCard({ x, y }: { x: number; y: number }) {
  return (
    <div className="wns-pop absolute" style={centered(x, y)}>
      <SkillCardBody />
    </div>
  );
}

/** The skill as the page's row draws it: its tile, name and one line. */
function SkillCardBody({ small = false }: { small?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex items-center gap-sm rounded-lg border border-border bg-popover px-sm py-xs shadow-overlay">
      <span className="flex size-(--size-icon-lg) shrink-0 items-center justify-center rounded-md bg-muted"><SparklesIcon className="size-(--size-icon-sm) text-primary" /></span>
      <span className="flex min-w-0 flex-col">
        <span className="whitespace-nowrap text-caption font-semibold">{t("whatsNewScene.skills.name")}</span>
        {small ? null : <span className="text-micro text-muted-foreground">{t("whatsNewScene.skills.desc")}</span>}
      </span>
    </div>
  );
}

function Logo({ kind, x, y, checked }: { kind: string; x: number; y: number; checked: boolean }) {
  return (
    <span className="wns-pop absolute" style={centered(x, y)}>
      <span className={cn("relative flex rounded-md p-xxs transition-colors", checked ? "bg-accent" : "")}>
        <AgentLogo agent={kind} label={agentProductName(kind)} />
        {checked ? (
          <span className="wns-pop absolute -top-xs -right-xs flex size-(--size-icon-sm) items-center justify-center rounded-full bg-primary text-primary-foreground">
            <CheckIcon className="size-full p-px" />
          </span>
        ) : null}
      </span>
    </span>
  );
}
