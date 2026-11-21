// What's new scene for Skills & MCP (1.2.11): adding once and managing in one
// place, drawn as a picture of the page rather than the page itself. A GitHub
// address goes into Add and comes up as a card marked Skill; a command goes in
// and comes up marked MCP; each drops into the table and every agent's cell
// takes its ✓ in turn, the page's own promise (`store.subtitle`) above them.
// Then the management: one switch turns the MCP server off for every agent at
// once, the skill's update is taken with one press, and the switch puts the
// server back. Agents are their real logos (`AgentLogo`) and the words are the
// page's own (`store.*`); the skill and the server are examples, never the
// operator's.

import { CheckIcon, PlugIcon, SparklesIcon } from "lucide-react";
import type { ReactNode } from "react";
import { agentProductName } from "../../agentAdapters";
import { useInterfaceTranslation } from "../../i18n/translator";
import { cn } from "../../lib/utils";
import { AgentLogo } from "../agent-logo";
import { ScenePointer } from "./pointer";
import { SCENE_CSS, type SceneProps } from "./scene";

/** One loop, in ms: the beats below fall inside it. */
export const SKILLS_LOOP_MS = 14_000;
const TYPE_EVERY = 40;
/** The skill: typed, Add pressed, its card shown, its row landed. */
const SKILL_TYPE = 300;
const SKILL_PRESS = 1_400;
const SKILL_CARD = 1_600;
const SKILL_ROW = 2_700;
/** The MCP server, the same way. */
const MCP_TYPE = 3_100;
const MCP_PRESS = 4_100;
const MCP_CARD = 4_300;
const MCP_ROW = 5_400;
/** Each agent's cell taking its ✓ after a row lands. */
const CELL_EVERY = 120;
/** The management: the server switched off for everyone, the skill's update taken, the server back on. */
const OFF_POINT = 6_900;
const OFF = 7_600;
const UPDATE_SHOWN = 8_600;
const UPDATE_POINT = 9_300;
const UPDATE = 10_000;
const ON_POINT = 11_000;
const ON = 11_600;

const AGENTS = ["claude", "codex", "opencode", "antigravity", "cursor", "grok"];

export function SkillsScene({ at }: SceneProps) {
  const { t } = useInterfaceTranslation();
  const adding = at < MCP_TYPE ? "skill" : at < MCP_ROW ? "mcp" : null;
  const text = adding === "skill" ? t("whatsNewScene.skills.add") : t("whatsNewScene.skills.mcpAdd");
  const typed = adding ? text.slice(0, Math.max(0, Math.floor((at - (adding === "skill" ? SKILL_TYPE : MCP_TYPE)) / TYPE_EVERY))) : "";
  const pressed = adding === "skill" ? at >= SKILL_PRESS : at >= MCP_PRESS;
  const card = adding === "skill" ? at >= SKILL_CARD && at < SKILL_ROW : adding === "mcp" && at >= MCP_CARD && at < MCP_ROW;
  const mcpOn = !(at >= OFF && at < ON);
  const updated = at >= UPDATE;
  const pointer = at < SKILL_CARD
    ? { left: 95, top: 7, shown: at >= SKILL_PRESS - 600, pressed: at >= SKILL_PRESS }
    : at < MCP_CARD
      ? { left: 95, top: 7, shown: at >= MCP_PRESS - 600, pressed: at >= MCP_PRESS }
      : at < UPDATE_POINT
        ? { left: 95, top: 46, shown: at >= OFF_POINT, pressed: at >= OFF }
        : at < ON_POINT
          ? { left: 21, top: 34, shown: true, pressed: at >= UPDATE }
          : { left: 95, top: 46, shown: at < ON + 900, pressed: at >= ON };
  return (
    <div className="relative flex aspect-[2/1] flex-col gap-sm overflow-hidden bg-background p-md text-foreground" data-whats-new-scene="skills" data-scene-beat={at >= OFF ? "manage" : adding ?? "added"} aria-hidden>
      <style>{SCENE_CSS}</style>
      <div className="flex h-(--size-control) shrink-0 items-center gap-sm">
        {adding ? (
          <>
            <span key={adding} className="wns-fade flex min-w-0 flex-1 items-center rounded-md border border-border bg-popover px-sm py-xs font-mono text-caption">
              <span className="truncate">{typed}</span><span className="text-primary">▏</span>
            </span>
            <span className={cn("shrink-0 rounded-md px-md py-xs text-caption font-medium transition-colors", pressed ? "bg-primary text-primary-foreground" : "bg-muted text-foreground")}>{t("store.add")}</span>
          </>
        ) : (
          <span className="wns-rise flex-1 truncate text-center text-body font-medium">{t("store.subtitle", { device: t("whatsNewScene.skills.here") })}</span>
        )}
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col rounded-md border border-border">
        <div className="flex items-center gap-sm border-b border-border px-sm py-xs">
          <span className="min-w-0 flex-1" />
          {AGENTS.map((kind) => <span key={kind} className="flex w-(--size-control) justify-center"><AgentLogo agent={kind} label={agentProductName(kind)} size="sm" /></span>)}
          <span className="w-(--size-control)" />
        </div>
        {at >= SKILL_ROW ? (
          <Row
            landed={SKILL_ROW}
            at={at}
            tile={<SparklesIcon className="size-(--size-icon-sm) text-primary" />}
            name={t("whatsNewScene.skills.name")}
            kind={t("store.section.skill")}
            version={updated ? t("whatsNewScene.skills.versionNew") : t("whatsNewScene.skills.versionOld")}
            versionChanged={updated}
            dot={at >= UPDATE_SHOWN && !updated}
            on
            update={at >= UPDATE_SHOWN && at < UPDATE + 500 ? t("store.action.update") : null}
            updatePressed={at >= UPDATE}
          />
        ) : null}
        {at >= MCP_ROW ? (
          <Row
            landed={MCP_ROW}
            at={at}
            tile={<PlugIcon className="size-(--size-icon-sm) text-agent-working" />}
            name={t("whatsNewScene.skills.mcpName")}
            kind={t("store.section.mcp")}
            version={t("whatsNewScene.skills.mcpVersion")}
            on={mcpOn}
            switched={at >= OFF}
          />
        ) : null}
        {at >= OFF && at < ON + 600 ? (
          <span key={mcpOn ? "on" : "off"} className="wns-pop absolute right-sm bottom-sm rounded-md border border-border bg-popover px-sm py-xxs text-micro text-subtle-foreground shadow-overlay">
            {t("store.switch.all", { name: t("whatsNewScene.skills.mcpName") })} · {mcpOn ? t("store.legend.on") : t("store.legend.off")}
          </span>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center justify-end gap-md text-micro text-muted-foreground">
        <span className="flex items-center gap-xxs"><span className="flex size-(--size-icon-sm) items-center justify-center rounded-full bg-primary/15 text-primary"><CheckIcon className="size-full p-px" /></span>{t("store.legend.on")}</span>
        <span className="flex items-center gap-xxs"><span className="size-(--size-icon-sm) rounded-full border border-border" />{t("store.legend.off")}</span>
      </div>
      {card ? <AddedCard kind={adding!} /> : null}
      <ScenePointer {...pointer} />
    </div>
  );
}

/** What Add read, as the confirm shows it: the kind first, big, then the tile, name and one line. */
function AddedCard({ kind }: { kind: "skill" | "mcp" }) {
  const { t } = useInterfaceTranslation();
  const skill = kind === "skill";
  return (
    <div key={kind} className="wns-pop absolute inset-x-0 flex justify-center" style={{ top: "34%" }}>
      <div className="flex items-center gap-sm rounded-lg border border-primary bg-popover px-md py-sm shadow-overlay">
        <span className={cn("rounded-md px-sm py-xxs text-caption font-semibold text-primary-foreground", skill ? "bg-primary" : "bg-agent-working")}>{t(skill ? "store.section.skill" : "store.section.mcp")}</span>
        <span className="flex size-(--size-icon-lg) shrink-0 items-center justify-center rounded-md bg-muted">
          {skill ? <SparklesIcon className="size-(--size-icon-sm) text-primary" /> : <PlugIcon className="size-(--size-icon-sm) text-agent-working" />}
        </span>
        <span className="flex min-w-0 flex-col">
          <span className="whitespace-nowrap text-body font-semibold">{t(skill ? "whatsNewScene.skills.name" : "whatsNewScene.skills.mcpName")}</span>
          <span className="whitespace-nowrap text-caption text-muted-foreground">{t(skill ? "whatsNewScene.skills.desc" : "whatsNewScene.skills.mcpDesc")}</span>
        </span>
      </div>
    </div>
  );
}

/** One item as the page's table draws it: tile, name, kind and version, a state per agent, and the one switch for every agent. */
function Row({ landed, at, tile, name, kind, version, versionChanged = false, dot = false, on, switched = false, update = null, updatePressed = false }: {
  landed: number; at: number; tile: ReactNode; name: string; kind: string; version: string; versionChanged?: boolean; dot?: boolean; on: boolean; switched?: boolean; update?: string | null; updatePressed?: boolean;
}) {
  return (
    <div className="wns-rise flex items-center gap-sm border-b border-border px-sm py-xs">
      <span className="flex min-w-0 flex-1 items-center gap-sm">
        <span className="flex size-(--size-icon-lg) shrink-0 items-center justify-center rounded-md bg-muted">{tile}</span>
        <span className="flex min-w-0 flex-col gap-xxs">
          <span className="flex min-w-0 items-center gap-xs">
            <span className="truncate text-caption font-semibold">{name}</span>
            <span className="shrink-0 rounded-sm border border-border px-xs text-micro text-muted-foreground">{kind}</span>
          </span>
          <span className="flex items-center gap-xs">
            <span key={version} className={cn("rounded-full bg-muted px-xs font-mono text-micro text-subtle-foreground", versionChanged && "wns-pop text-primary")}>{version}</span>
            {dot ? <span className="wns-pop size-(--size-tab-status-dot) rounded-full bg-primary" /> : null}
            {update ? <span className={cn("wns-pop rounded-md px-xs text-micro font-medium", updatePressed ? "bg-primary text-primary-foreground" : "bg-primary/15 text-primary")}>{update}</span> : null}
          </span>
        </span>
      </span>
      {AGENTS.map((agent, index) => {
        const applied = on && at >= landed + 300 + index * CELL_EVERY;
        return (
          <span key={agent} className="flex w-(--size-control) justify-center">
            {applied ? (
              <span key={switched ? "back" : "first"} className="wns-pop flex size-(--size-icon) items-center justify-center rounded-full bg-primary/15 text-primary" style={{ animationDelay: switched ? `${index * 60}ms` : "0ms" }}>
                <CheckIcon className="size-(--size-icon-sm)" />
              </span>
            ) : (
              <span className="size-(--size-icon) rounded-full border border-border" />
            )}
          </span>
        );
      })}
      <span className="flex w-(--size-control) justify-center">
        <span className={cn("inline-flex h-(--size-checkbox) w-(--size-control) items-center rounded-xl transition-colors", on ? "bg-primary" : "bg-input")}>
          <span className={cn("block size-(--size-icon) rounded-xl bg-background transition-transform", on ? "translate-x-(--size-icon-sm)" : "translate-x-(--spacing-none)")} />
        </span>
      </span>
    </div>
  );
}
