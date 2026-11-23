// The phone list's rows (PRD mobile-companion-ui B1-B7): a group's top rows,
// each a two-line row (Doop 01), a parent's children under it with a rail,
// opened by the children chip, and the folds. hided decides the
// tree, the order and what is folded (`hide-core` `phone/tree.rs`); this
// draws it and keeps only which parents and folds this phone opened.

import { pullLabel } from "../codeHost";
import { useMemo, type ReactNode } from "react";
import { ChevronDownIcon } from "lucide-react";
import { MissionChipFace, TONE_TEXT } from "../components/mission-tone";
import { chipWords, missionStalled, missionToneOf } from "../missions";
import { CLOSED_DIM, PR_LOOK, prCountsSummary, prMarkParts } from "../prMark";
import { cn } from "../lib/utils";
import { agentLinks, type AgentLinks } from "./missionModel";
import { useElapsed } from "../components/elapsed";
import { StatusMark } from "../components/status-mark";
import { SubagentLine, SubagentMoreLine, subagentsOf } from "../components/subagent-row";
import { Button } from "../components/ui/button";
import { RowButton, RowOverlayButton } from "../components/ui/row-button";
import { agentProductName } from "../agentAdapters";
import { lineText, markWords, rowDimmed } from "../agentRow";
import { useInterfaceTranslation } from "../i18n/translator";
import { sleepingCaption } from "../sleeping-session";
import { actOnSleepingSession, openDetail } from "./connection";
import type { GatheredRow } from "./gather";
import { hostLink } from "./hosts";
import { humanTurnText } from "../humanTurn";
import { AgentStatusMark, AgentTitle, DeviceTag, PhoneHumanTurn, PlaceParts } from "./parts";
import { ROW_INSET, TwoLineRow } from "./layout";
import { TreeChipFace } from "../components/tree-chip";
import { keyTag, type AgentGroup, type GroupRow, type PhoneAgent, type PhoneSleepingSession } from "./protocol";
import { foldKey, openFold, setChildrenOpen, usePhone } from "./store";

/** A group's rows: the first few, then the folded ones once "더 보기" opened them (B7). */
export function GroupRows({ group }: { group: AgentGroup }) {
  const { t } = useInterfaceTranslation();
  const key = foldKey.group(group.group);
  const opened = usePhone((s) => s.foldsOpen[key] === true);
  const more = group.more ?? [];
  return (
    <ul>
      {[...group.rows, ...(opened ? more : [])].map((row) => <TopRow key={rowKey(row)} row={row} />)}
      {more.length > 0 && !opened ? (
        <li>
          <FoldButton
            label={t("mobile.more", { count: more.length })}
            note={group.more_asleep ? t("mobile.moreAsleep", { count: group.more_asleep }) : null}
            data-phone-fold={key}
            onOpen={() => openFold(key)}
          />
        </li>
      ) : null}
    </ul>
  );
}

function rowKey(row: GroupRow): string {
  return row.kind === "agent" ? keyTag(row) : row.sleep_id;
}

function TopRow({ row }: { row: GroupRow }) {
  return row.kind === "agent" ? <AgentItem agent={row} /> : <SleepingRow session={row} />;
}

/** "더 보기 N", opened in place; the note says how many of the folded rows sleep. */
export function FoldButton({ label, note, onOpen, ...rest }: { label: string; note: string | null; onOpen: () => void } & Record<`data-${string}`, string>) {
  return (
    <Button variant="text" size="inline" className="min-h-(--size-touch-target) w-full justify-start gap-sm text-body" onClick={onOpen} {...rest}>
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      <span>{label}</span>
      {note ? <span className="text-muted-foreground">· {note}</span> : null}
    </Button>
  );
}

/**
 * An agent's row with its children under it, opened as the core chose until
 * this phone chose otherwise (B2, B3), and after them the subagents its agent
 * runs inside its own session, as the desktop's opened row lists them: the
 * first five, dimmed and with nothing to press, and the rest as one line
 * (docs/UI_BEHAVIOR.md, Subagents). `nested` is a child's row, whose project
 * is its parent's.
 */
function AgentItem({ agent, nested = false }: { agent: PhoneAgent; nested?: boolean }) {
  const { t } = useInterfaceTranslation();
  const tag = keyTag(agent);
  const kids = agent.children ?? [];
  const more = agent.more_children ?? [];
  const subagents = subagentsOf(agent);
  const opens = kids.length + more.length + (agent.subagents ?? 0) > 0;
  const open = usePhone((s) => s.childrenOpen[tag]) ?? agent.children_open === true;
  const foldKeyOf = foldKey.children(tag);
  const moreOpened = usePhone((s) => s.foldsOpen[foldKeyOf] === true);
  return (
    <li data-phone-item={tag}>
      {/* Only the row itself dims: its children keep their own state. */}
      <div className={rowDimmed(agent) ? "opacity-(--opacity-dimmed)" : undefined} data-phone-row={tag}>
        <AgentRow agent={agent} nested={nested} chip={<TreeChip agent={agent} open={open} onToggle={opens ? () => setChildrenOpen(agent, !open) : null} />} />
        <PhoneHumanTurn agent={agent} inset={`${ROW_INSET} pb-md`} />
      </div>
      {opens && open ? (
        <ul className="ml-[calc(var(--size-agent-mark)/2)] border-l border-border pl-md" data-phone-children={tag}>
          {[...kids, ...(moreOpened ? more : [])].map((child) => <AgentItem key={keyTag(child)} agent={child} nested />)}
          {more.length > 0 && !moreOpened ? (
            <li><FoldButton label={t("mobile.more", { count: more.length })} note={null} data-phone-fold={foldKeyOf} onOpen={() => openFold(foldKeyOf)} /></li>
          ) : null}
          {subagents.rows.map((row) => <li key={row.id} data-phone-subagent={row.id}><SubagentLine row={row} size="body" /></li>)}
          {subagents.more > 0 ? <li data-phone-subagent-more=""><SubagentMoreLine count={subagents.more} listed={subagents.rows.length} size="body" /></li> : null}
        </ul>
      ) : null}
    </li>
  );
}

/**
 * The desktop pane header's children chip at the head of a parent's second line (`≡ ● 2 · • 3 ⌄`, operator decision 2026-11-22,
 * D): the first child's mark, how many children, the subagents after a dot. It opens or closes the children and the subagents
 * (B2), and its name spells out every count; a parent whose children the list cannot open shows the chip with nothing to press.
 */
function TreeChip({ agent, open, onToggle }: { agent: PhoneAgent; open: boolean; onToggle: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const kids = agent.children ?? [];
  const listed = kids.length + (agent.more_children?.length ?? 0);
  const subagents = agent.subagents ?? 0;
  const counts = agent.descendants;
  // A row whose children the list does not hang under it (a second level folded flat, `tree.rs`) still counts its descendants.
  const count = listed > 0 ? listed : counts ? counts.asking + counts.working + counts.done : 0;
  if (count === 0 && subagents === 0) return null;
  const first = kids.find((child) => child.mark !== "unknown") ?? null;
  const label = [
    listed > 0 ? t("agentSessions.tree.children", { count: listed }) : null,
    counts?.asking ? t("mobile.descendants.asking", { count: counts.asking }) : null,
    counts?.working ? t("mobile.descendants.working", { count: counts.working }) : null,
    counts?.done ? t("mobile.descendants.done", { count: counts.done }) : null,
    subagents > 0 ? t("agents.subagents.working", { count: subagents }) : null,
  ].filter(Boolean).join(" · ");
  const face = (
    <span className={cn("flex items-center gap-xxs rounded-full border bg-secondary px-sm py-xxs font-normal text-subtle-foreground", counts?.asking ? "border-warning/60" : "border-transparent")}>
      <TreeChipFace mark={first ? <StatusMark mark={first.mark} human={first.human_turn != null} data-phone-tree-mark={first.mark} /> : null} count={count} subagents={subagents} icon="size-(--size-icon-sm)" chevron={onToggle !== null} />
    </span>
  );
  if (!onToggle) return <span role="img" aria-label={label} className="flex shrink-0" data-phone-tree-chip={keyTag(agent)}>{face}</span>;
  return (
    <Button
      variant="ghost"
      aria-label={label}
      aria-expanded={open}
      className="pointer-events-auto -my-sm h-auto min-h-(--size-touch-target) shrink-0 px-none py-none text-body hover:bg-transparent"
      data-phone-children-toggle={keyTag(agent)}
      onClick={onToggle}
    >
      {face}
    </Button>
  );
}

const LINE_TONE = { warning: "text-warning", news: "text-foreground", subtle: "text-subtle-foreground" } as const;

/** What the row's marks show: the open missions this agent conducts (a step's worker gets none, as on the desktop) and the pull requests that name it. */
function useRowLinks(agent: PhoneAgent): AgentLinks {
  const missions = usePhone((s) => s.missions);
  const prs = usePhone((s) => s.prs?.items ?? null);
  return useMemo(() => {
    const links = agentLinks({ device_id: agent.device_id, pane_id: agent.pane_id }, missions, prs);
    return { ...links, missions: links.missions.filter((link) => link.node === null) };
  }, [agent.device_id, agent.pane_id, missions, prs]);
}

/**
 * The desktop row's marks after the title, in its order (`sidebar-agent-row.tsx`): the pull requests (one as its state's mark
 * and `#number`, several as one mark, how many are still open) and the mission chip with `!` for the operator's turn and `k/n`. They
 * never shrink, so a narrow phone cuts the title first; the row opens the detail, whose chips open the pages.
 */
function RowMarks({ links }: { links: AgentLinks }) {
  const parts = prMarkParts(links.prs);
  if (!parts && links.missions.length === 0) return null;
  return (
    <span aria-hidden="true" className="flex shrink-0 items-center gap-sm font-mono text-body" data-phone-row-marks="true">
      {parts && "counts" in parts ? (() => {
        const { state, left } = prCountsSummary(parts.counts);
        const { icon: Icon, tone } = PR_LOOK[state];
        return <span className={cn("inline-flex items-center gap-xxs", tone, state === "closed" && CLOSED_DIM)} data-phone-row-pr={state}><Icon className="size-(--size-pr-icon) shrink-0" />{left > 0 ? left : null}</span>;
      })() : parts ? (() => {
        const { icon: Icon, tone } = PR_LOOK[parts.state];
        return <span className={cn("inline-flex items-center gap-xxs text-subtle-foreground", parts.state === "closed" && CLOSED_DIM)} data-phone-row-pr={parts.state}><Icon className={cn("size-(--size-pr-icon) shrink-0", tone)} />{parts.label}</span>;
      })() : null}
      {links.missions.map(({ mission, turn }) => (
        <span key={mission.id} className={cn("flex items-center gap-xxs rounded-xs bg-secondary px-xs", TONE_TEXT[missionToneOf(mission.status, turn, missionStalled(mission))])} data-phone-row-mission={mission.id}>
          <MissionChipFace turn={turn} delay={false} status={mission.status} counts={chipWords(mission).counts} icon="size-(--size-icon)" />
        </span>
      ))}
    </span>
  );
}

/**
 * The row (Doop 01): its mark, title, the desktop row's marks and the time on the first line; the children chip, the project
 * and the request or news on the second. The row's press opens the agent; the chip, in front of it, only opens its children (B2).
 */
function AgentRow({ agent, nested, chip }: { agent: PhoneAgent; nested: boolean; chip: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(agent.changed_at_unix_ms);
  // Opening a row on the phone does not wake it, so the desktop's "wakes when opened" is not said here.
  const words = agent.status_code === "sleeping" && agent.mark === "asleep" ? t("panes.sleep.sleeping") : markWords(t, agent);
  const links = useRowLinks(agent);
  const marks = [
    agent.starred ? t("taskLens.revisit.starred") : null,
    ...links.prs.map((pr) => pullLabel(pr)),
    ...links.missions.map(({ mission }) => `${t("mobile.tab.missions")} ${mission.title}`),
  ];
  const label = [words, agent.title, ...marks, agentProductName(agent.agent_kind), agent.tool, agent.place, agent.device_label, elapsed].filter(Boolean).join(", ");
  // A person's turn says itself under the row, with Done (PRD ask-human B19).
  const line = agent.line && !agent.human_turn ? agent.line : null;
  return (
    <div className="relative">
      <RowOverlayButton
        aria-label={label}
        hover={false}
        data-phone-agent={`${agent.device_id}|${agent.pane_id}`}
        className="active:bg-accent"
        // The eye row opens on 볼 것; a question, an approval or any other row on the conversation as before (B11).
        onClick={() => openDetail({ device_id: agent.device_id, pane_id: agent.pane_id }, agent.mark === "look" ? "looks" : "conversation")}
      />
      <div className="pointer-events-none relative flex min-h-(--size-touch-target) items-center py-md">
        <TwoLineRow
          lead={<AgentStatusMark agent={agent} />}
          head={<><AgentTitle agent={agent} /><RowMarks links={links} /></>}
          end={elapsed}
          line={
            <>
              {chip}
              <PlaceParts agent={nested ? { ...agent, place: null, project: undefined } : agent} project />
              {line ? (
                <span className={cn("min-w-0 flex-1 truncate", LINE_TONE[line.tone])} data-phone-line={line.mode}>
                  {/* A Mac that predates wait counts sends no `wait_count`; its working descendants are the children count. */}
                  {lineText(t, agent.wait === "children" && agent.wait_count === undefined ? { ...agent, wait_count: agent.descendants_working } : agent, line)}
                </span>
              ) : null}
            </>
          }
        />
      </div>
    </div>
  );
}

export function SleepingRow({ session }: { session: PhoneSleepingSession }) {
  const { t } = useInterfaceTranslation();
  const connected = usePhone((state) => state.connected);
  const action = usePhone((state) => state.sleepAction);
  const pending = action?.sleepId === session.sleep_id && !!action.requestId;
  const uncertain = session.phase === "close_unknown" || session.phase === "wake_unknown";
  const failed = (action?.sleepId === session.sleep_id && action.error) || session.attention;
  const caption = t(sleepingCaption(session));
  const lineShown = failed || session.phase !== "sleeping" || session.checking;
  return (
    <li className="flex min-h-(--size-touch-target) flex-col gap-xs py-md" data-phone-sleeping={session.sleep_id}>
      <div className="flex min-w-0 items-center gap-sm">
        {/* A grey moon at full strength; sleeping says itself there, waking or failing on the line below (status-marks D-21, B31). */}
        <span className="flex size-(--size-agent-mark) shrink-0 items-center justify-center"><StatusMark mark="asleep" data-phone-mark="asleep" /></span>
        <span className="min-w-0 flex-1 truncate text-title text-subtle-foreground">{session.title}<span className="sr-only">, {agentProductName(session.agent_kind)}{lineShown ? null : `, ${caption}`}</span></span>
      </div>
      <div className={`flex min-w-0 items-center justify-between gap-sm ${ROW_INSET}`}>
        <span className="min-w-0 break-words text-subhead text-muted-foreground" role={failed ? "status" : undefined}>
          {failed ? t("panes.sleep.requestFailed") : lineShown ? caption : null}
        </span>
        {session.wake_available || uncertain ? (
          <Button
            variant="ghost"
            className="min-h-(--size-touch-target) shrink-0 text-body"
            disabled={!connected || !!action?.requestId || session.checking}
            data-phone-sleep-action={session.wake_available ? "wake" : "check"}
            onClick={() => actOnSleepingSession(session.sleep_id, session.wake_available ? "wake_sleeping_session" : "check_sleeping_session")}
          >
            {session.checking ? t("agents.checking") : pending ? t(session.wake_available ? "panes.sleep.waking" : "agents.checking") : session.wake_available ? t("panes.sleep.wake") : t("workspace.checkStatus")}
          </Button>
        ) : null}
      </div>
    </li>
  );
}

/**
 * A row of the gathered view (`gather.ts`, PRD device-connect-iroh B55): the row's head, the computer it came from and its line,
 * read-only. A press opens it where it lives: the open computer's detail here, another computer's through the start page, which
 * opens that computer's bundle on the agent as a notification does. A sleeping conversation of another computer opens that
 * computer's own list, where its wake is. `dim` is the open computer's unreachable list, which dims only its own rows.
 */
export function GatheredRowItem({ item, dim }: { item: GatheredRow; dim: boolean }) {
  const { row } = item;
  if (row.kind === "sleeping" && item.here) return <SleepingRow session={row} />;
  return (
    <li className={item.here && dim ? "opacity-(--opacity-dimmed)" : undefined} data-phone-gathered={`${item.host.room}|${rowKey(row)}`}>
      {row.kind === "agent" ? <GatheredAgentRow agent={row} item={item} /> : <GatheredSleepingRow session={row} item={item} />}
    </li>
  );
}

function GatheredAgentRow({ agent, item }: { agent: PhoneAgent; item: GatheredRow }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(agent.changed_at_unix_ms);
  const words = agent.status_code === "sleeping" && agent.mark === "asleep" ? t("panes.sleep.sleeping") : markWords(t, agent);
  const turn = agent.human_turn ? humanTurnText(t, agent.human_turn) : null;
  const label = [words, agent.title, item.host.name, agentProductName(agent.agent_kind), agent.tool, agent.place, agent.device_label, elapsed].filter(Boolean).join(", ");
  const open = () => {
    if (item.here) openDetail({ device_id: agent.device_id, pane_id: agent.pane_id }, agent.mark === "look" ? "looks" : "conversation");
    else window.location.assign(hostLink(item.host, keyTag(agent)));
  };
  return (
    <RowButton
      aria-label={label}
      data-phone-agent={`${agent.device_id}|${agent.pane_id}`}
      className={cn("min-h-(--size-touch-target) rounded-none px-none py-md hover:bg-transparent active:bg-accent", rowDimmed(agent) && "opacity-(--opacity-dimmed)")}
      onClick={open}
    >
      <TwoLineRow
        lead={<AgentStatusMark agent={agent} />}
        head={<AgentTitle agent={agent} />}
        end={elapsed}
        line={
          <>
            <PlaceParts agent={agent} project computer={item.host.name} />
            {/* A person's turn says what it waits on; its Done is on the row's own computer. */}
            {turn ? (
              <span className="min-w-0 flex-1 truncate text-warning" data-phone-line="human_turn">{turn}</span>
            ) : agent.line ? (
              <span className={cn("min-w-0 flex-1 truncate", LINE_TONE[agent.line.tone])} data-phone-line={agent.line.mode}>
                {lineText(t, agent.wait === "children" && agent.wait_count === undefined ? { ...agent, wait_count: agent.descendants_working } : agent, agent.line)}
              </span>
            ) : null}
          </>
        }
      />
    </RowButton>
  );
}

function GatheredSleepingRow({ session, item }: { session: PhoneSleepingSession; item: GatheredRow }) {
  const { t } = useInterfaceTranslation();
  const caption = t(sleepingCaption(session));
  return (
    <RowButton
      aria-label={[session.title, item.host.name, agentProductName(session.agent_kind), caption].join(", ")}
      className="min-h-(--size-touch-target) rounded-none px-none py-md hover:bg-transparent active:bg-accent"
      onClick={() => window.location.assign(hostLink(item.host, undefined, true))}
    >
      <TwoLineRow
        lead={<StatusMark mark="asleep" data-phone-mark="asleep" />}
        head={<span className="min-w-0 flex-1 truncate text-title text-subtle-foreground">{session.title}</span>}
        line={<><DeviceTag label={item.host.name} computer /><span className="min-w-0 truncate text-muted-foreground">{caption}</span></>}
      />
    </RowButton>
  );
}
