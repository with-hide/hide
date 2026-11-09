// The phone list's rows (PRD mobile-companion-ui B1-B7): a group's top rows,
// a parent's children under it with a rail, and the folds. hided decides the
// tree, the order and what is folded (`hide-core` `phone/tree.rs`); this
// draws it and keeps only which parents and folds this phone opened.

import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useElapsed } from "../components/elapsed";
import { StatusMark } from "../components/status-mark";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { agentProductName } from "../agentAdapters";
import { lineText, markWords, rowDimmed } from "../agentRow";
import { useInterfaceTranslation } from "../i18n/translator";
import { sleepingCaption } from "../sleeping-session";
import { actOnSleepingSession, openDetail } from "./connection";
import { AgentHead, LIST_INSET, PhoneHumanTurn, Place } from "./parts";
import { keyTag, type AgentGroup, type GroupRow, type PhoneAgent, type PhoneSleepingSession } from "./protocol";
import { foldKey, openFold, setChildrenOpen, usePhone } from "./store";

/** A group's rows: the first few, then the folded ones once "더 보기" opened them (B7). */
export function GroupRows({ group }: { group: AgentGroup }) {
  const { t } = useInterfaceTranslation();
  const key = foldKey.group(group.group);
  const opened = usePhone((s) => s.foldsOpen[key] === true);
  const more = group.more ?? [];
  return (
    <ul className="divide-y divide-border border-b border-border">
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
function FoldButton({ label, note, onOpen, ...rest }: { label: string; note: string | null; onOpen: () => void } & Record<`data-${string}`, string>) {
  return (
    <Button variant="text" size="inline" className="min-h-(--size-touch-target) w-full justify-start gap-sm text-body" onClick={onOpen} {...rest}>
      <ChevronDownIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      <span>{label}</span>
      {note ? <span className="text-muted-foreground">· {note}</span> : null}
    </Button>
  );
}

/** An agent's row with its children under it, opened as the core chose until this phone chose otherwise (B2, B3). */
function AgentItem({ agent }: { agent: PhoneAgent }) {
  const { t } = useInterfaceTranslation();
  const tag = keyTag(agent);
  const kids = agent.children ?? [];
  const more = agent.more_children ?? [];
  const hasKids = kids.length + more.length > 0;
  const open = usePhone((s) => s.childrenOpen[tag]) ?? agent.children_open === true;
  const foldKeyOf = foldKey.children(tag);
  const moreOpened = usePhone((s) => s.foldsOpen[foldKeyOf] === true);
  return (
    <li data-phone-item={tag}>
      {/* Only the row itself dims: its children keep their own state. */}
      <div className={rowDimmed(agent) ? "opacity-(--opacity-dimmed)" : undefined} data-phone-row={tag}>
        <AgentRow agent={agent} />
        <PhoneHumanTurn agent={agent} inset={`${LIST_INSET} pb-md`} />
        <Descendants agent={agent} count={kids.length + more.length} open={open} onToggle={hasKids ? () => setChildrenOpen(agent, !open) : null} />
      </div>
      {hasKids && open ? (
        <ul className="ml-[calc(var(--size-agent-mark)/2)] border-l border-border pl-md" data-phone-children={tag}>
          {[...kids, ...(moreOpened ? more : [])].map((child) => <AgentItem key={keyTag(child)} agent={child} />)}
          {more.length > 0 && !moreOpened ? (
            <li><FoldButton label={t("mobile.more", { count: more.length })} note={null} data-phone-fold={foldKeyOf} onOpen={() => openFold(foldKeyOf)} /></li>
          ) : null}
        </ul>
      ) : null}
    </li>
  );
}

/** A parent's pills (asking, working, finished) and `자식 N ⌄`, which opens or closes its children (B2). */
function Descendants({ agent, count, open, onToggle }: { agent: PhoneAgent; count: number; open: boolean; onToggle: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  const counts = agent.descendants;
  if (!counts && !onToggle) return null;
  const pills = (
    <span className="flex min-w-0 items-center gap-md text-body text-subtle-foreground">
      {counts?.asking ? <Pill mark="turn" count={counts.asking} words={t("mobile.descendants.asking", { count: counts.asking })} /> : null}
      {counts?.working ? <Pill mark="going" count={counts.working} words={t("mobile.descendants.working", { count: counts.working })} /> : null}
      {counts?.done ? <Pill mark="done" count={counts.done} words={t("mobile.descendants.done", { count: counts.done })} /> : null}
    </span>
  );
  if (!onToggle) return <div className={`flex min-h-(--size-touch-target) items-center ${LIST_INSET}`}>{pills}</div>;
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <Button
      variant="ghost"
      aria-expanded={open}
      className={`h-auto min-h-(--size-touch-target) w-full justify-between gap-sm rounded-none py-xs pr-none text-body font-normal ${LIST_INSET}`}
      data-phone-children-toggle={keyTag(agent)}
      onClick={onToggle}
    >
      {pills}
      <span className="flex shrink-0 items-center gap-xs text-subtle-foreground">
        {t("mobile.children", { count })}
        <Chevron aria-hidden="true" className="size-(--size-icon) text-muted-foreground" />
      </span>
    </Button>
  );
}

function Pill({ mark, count, words }: { mark: "turn" | "going" | "done"; count: number; words: string }) {
  return (
    <span role="img" aria-label={words} className="flex items-center gap-xs">
      <StatusMark mark={mark} />
      <span aria-hidden="true" className="font-mono">{count}</span>
    </span>
  );
}

const LINE_TONE = { warning: "text-warning", news: "text-foreground", subtle: "text-subtle-foreground" } as const;

/** The row's own button: it opens the agent, never its children (B2). */
function AgentRow({ agent }: { agent: PhoneAgent }) {
  const { t } = useInterfaceTranslation();
  const elapsed = useElapsed(agent.changed_at_unix_ms);
  // Opening a row on the phone does not wake it, so the desktop's "wakes when opened" is not said here.
  const words = agent.status_code === "sleeping" && agent.mark === "asleep" ? t("panes.sleep.sleeping") : markWords(t, agent);
  const label = [words, agent.title, agentProductName(agent.agent_kind), agent.place, agent.device_label, elapsed].filter(Boolean).join(", ");
  return (
    <RowButton
      aria-label={label}
      data-phone-agent={`${agent.device_id}|${agent.pane_id}`}
      className="min-h-(--size-touch-target) flex-col items-stretch gap-xs rounded-none px-none py-md hover:bg-transparent active:bg-accent"
      // The eye row opens on 볼 것; a question, an approval or any other row on the conversation as before (B11).
      onClick={() => openDetail({ device_id: agent.device_id, pane_id: agent.pane_id }, agent.mark === "look" ? "looks" : "conversation")}
    >
      <span className="flex w-full items-center gap-sm">
        <span className="min-w-0 flex-1">
          <AgentHead agent={agent} large={false} />
        </span>
        <span className="shrink-0 font-mono text-body text-muted-foreground">{elapsed}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </span>
      <Place agent={agent} inset={LIST_INSET} />
      {/* A person's turn says itself under the row, with Done (PRD ask-human B19). */}
      {agent.line && !agent.human_turn ? (
        <span className={`line-clamp-2 ${LIST_INSET} text-body ${LINE_TONE[agent.line.tone]}`} data-phone-line={agent.line.mode}>
          {lineText(t, { wait: agent.wait, status_code: agent.status_code, descendant_counts: { working: agent.descendants_working }, background_left: agent.background_left, children_left: agent.children_left }, agent.line)}
        </span>
      ) : null}
    </RowButton>
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
      <div className={`flex min-w-0 items-center justify-between gap-sm ${LIST_INSET}`}>
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
