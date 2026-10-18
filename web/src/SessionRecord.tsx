import { ArrowLeftIcon, CheckIcon, ChevronRightIcon, CircleDotIcon, CircleHelpIcon, FlagIcon, HouseIcon, LoaderCircleIcon, SquareTerminalIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { deviceConnected, readsHere } from "./devices";
import { branchChip, markTone } from "./agentRow";
import { Button } from "./components/ui/button";
import { Elapsed } from "./components/elapsed";
import { PrMark } from "./components/pr-mark";
import { SessionRow } from "./components/session-row";
import { StateBox, type StateBoxTone } from "./components/state-box";
import { StatusMark } from "./components/status-mark";
import { shownTaskState, TaskMark, TaskName, TaskWord } from "./components/task-progress";
import { Hint } from "./components/ui/tooltip";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { checklist, childOutputs, orderedOutputs, outputPrState, recordTitle, recordTree, sessionOfAgentOn, sessionPlace, titleOutputs, workKey, type ChecklistTask } from "./sessionWork";
import { boardProjects } from "./navigation";
import { cardLine, cardOfPane, cardQuestion, everyAgent } from "./overviewTasks";
import { destinationOf } from "./overviewBacklog";
import { folderName } from "./linkPanel";
import { machineName } from "./screenMachine";
import { localDeviceId } from "./snapshot";
import type { AgentRow, AgentStatusCode, MissedWork, RowMark, SessionRecord as RecordValue, WorkEnd, WorkOutput, WorkSession } from "./snapshot";
import { statusText } from "./agentStatus";
import { useShellStore } from "./store";
import { summariesEnabled } from "./hideAi";
import { PR_LOOK } from "./prMark";
import { archivedChildrenOf, archivedMemberOf } from "./archive";
import type { ArchivedMember, ArchivedSession } from "./snapshot";

const ROW = "min-w-0 rounded-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring";
/** Under a task's line: the icon column's width and its gap, so what hangs from a task lines up with its name. */
const LANE = "pl-[calc(var(--size-icon-sm)+var(--spacing-xs))]";
const FIRST_WORDS = 4;

/**
 * Where a record is drawn (PRD overview-device-tasks D-07, B17-B21): the
 * lane's 보기 slot, under the slot's head, or the Overview's panel, whose head the
 * panel gives (✕ and the ways to the work). Both draw the same record in the
 * same order and the same loading, failed and disconnected states.
 */
export type RecordFrame = {
  kind: "lane" | "overview";
  /** The Overview panel's head; the 보기 slot draws its own. */
  head?: ReactNode;
  /** A line above the record (the dashed card's B14 note). */
  note?: string | null;
  /** The children drilled into, outermost first (B20). */
  trail: readonly WorkSession[];
  onTrail: (trail: readonly WorkSession[]) => void;
  /** Reads the record again: a retry, or the device answering again. */
  onRead: () => void;
  /** The record leaves the screen. */
  onLeave: () => void;
};

/**
 * The 보기 slot's record (PRD sidebar-task-lens D-44, B16): the slot's head
 * is the panel's, a child opens as the slot's next item so ← comes back to
 * its parent, and the core reads what the slot shows.
 */
export function SlotSessionRecord({ actions, target }: { actions: Actions; target: WorkSession }) {
  const frame = useMemo<RecordFrame>(() => ({
    kind: "lane",
    trail: [],
    onTrail: (next) => {
      const child = next.at(-1);
      if (child) actions.openChildRecord(child);
    },
    onRead: () => actions.openChildRecord(target),
    // The core starts and stops the slot's read with the slot itself.
    onLeave: () => undefined,
  }), [actions, target]);
  return <SessionRecord actions={actions} target={target} frame={frame} />;
}

export function SessionRecord({ actions, target, frame }: { actions: Actions; target: WorkSession; frame: RecordFrame }) {
  const { t } = useInterfaceTranslation();
  const section = useShellStore((state) => state.sessionRecord);
  const rest = useShellStore((state) => state.rest);
  const summaries = summariesEnabled(rest?.status?.background_ai);
  const connected = deviceConnected(rest, target.device_id);
  const own = section?.session && workKey(section.session) === workKey(target) ? section : null;
  const records = useMemo(() => own?.record ? recordTree(own.record) : [], [own?.record]);
  // The drilled child, when it is still in the tree; a vanished one leaves its parent shown.
  const shown = useMemo(() => {
    for (let depth = frame.trail.length - 1; depth >= 0; depth -= 1) {
      const found = records.find((record) => workKey(record.session) === workKey(frame.trail[depth]!));
      if (found) return found;
    }
    return own?.record ?? null;
  }, [frame.trail, records, own?.record]);
  const parent = frame.trail.length > 0 ? (records.find((record) => record.children.some((child) => shown && workKey(child.session) === workKey(shown.session))) ?? null) : null;
  // A child opened as its own item carries no parent in its record, so its live row's parent pane names who asked (B24); a string, so an unrelated snapshot leaves it alone.
  const liveParent = useShellStore((state) => {
    if (!shown || parent) return null;
    const key = workKey(shown.session);
    const localId = localDeviceId(state.rest);
    const rows = everyAgent(state.rest, state.agents);
    for (const row of rows.values()) {
      const session = sessionOfAgentOn(row, localId);
      if (session && workKey(session) === key) return (row.lineage_parent_pane_id ? rows.get(row.lineage_parent_pane_id)?.identity_label : null) || null;
    }
    return null;
  });
  const descendants = useMemo(() => (shown ? recordTree(shown).slice(1) : []), [shown]);
  const wasConnected = useRef(connected);
  const asked = useRef(false);
  const leave = useRef(frame.onLeave);
  leave.current = frame.onLeave;
  useEffect(() => () => leave.current(), []);
  // Read again on a reconnect, and once when the slot draws a record with no
  // read behind it: one restored after a relaunch was opened by no event. The
  // Overview's panel starts its own read when it opens.
  useEffect(() => {
    const unread = frame.kind === "lane" && !own && !asked.current;
    if (connected && (!wasConnected.current || unread)) {
      asked.current = true;
      frame.onRead();
    }
    wasConnected.current = connected;
  }, [frame, connected, own]);
  const up = () => frame.onTrail(frame.trail.slice(0, -1));
  const open = (child: RecordValue) => frame.onTrail([...frame.trail, child.session]);
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-session-record={frame.kind}>
    {frame.kind === "lane" ? null : <>
      {frame.head}
      {parent ? <Button size="sm" variant="ghost" className="min-w-0 self-start px-md" onClick={up} data-record-up="true"><ArrowLeftIcon /><span className="truncate">{t("record.backTo", { parent: recordTitle(parent, summaries) })}</span></Button> : null}
    </>}
    {frame.note ? <p className="px-md pb-xs text-caption text-muted-foreground" data-record-note="true">{frame.note}</p> : null}
    {!own || own.loading ? <div className="space-y-md px-md py-sm" aria-busy="true" data-record-loading="true">{[0, 1, 2].map((key) => <div key={key} className="h-(--size-control-lg) rounded-sm bg-muted" />)}</div> : own.failure ? <div role="status" className="px-md py-sm text-caption"><p>{t("sessionWork.failed")}</p><Button size="sm" variant="ghost" onClick={frame.onRead}>{t("common.retry")}</Button></div> : shown ? <div className={cn("min-h-0 flex-1 overflow-y-auto px-md pb-md text-caption [word-break:keep-all]", !connected && "opacity-(--opacity-disabled)")} data-record-content="true">
      <h2 className="flex min-w-0 items-center gap-xs py-sm text-subhead font-semibold"><AgentMark kind={shown.session.agent} /><span className="min-w-0 truncate" title={recordTitle(shown, summaries)}>{recordTitle(shown, summaries)}</span><OutputMarks outputs={titleOutputs(shown, summaries)} actions={actions} /></h2>
      <RecordPlace record={shown} actions={actions} />
      <Work key={`work:${workKey(shown.session)}`} record={shown} descendants={descendants} summaries={summaries} onOpen={open} actions={actions} />
      <Words key={`words:${workKey(shown.session)}`} record={shown} parent={parent ? recordTitle(parent, summaries) : liveParent} />
    </div> : null}
  </div>;
}

/**
 * The record's place line, then the 지금 box (PRD overview-device-tasks B17;
 * PRD pr-card-and-session-record-redesign B14, B17; PRD
 * record-card-readability B5): device · project · ⌂ every checkout of the
 * task, a removed one as `(없어짐)`; the children stand under their tasks. The
 * task's line and question are the root's; a child's box reads its own
 * agent's state and, once its pane is gone, its record's end.
 */
function RecordPlace({ record, actions }: { record: RecordValue; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const projects = useMemo(() => boardProjects(rest, agents), [rest, agents]);
  const rows = useMemo(() => everyAgent(rest, agents), [rest, agents]);
  const place = useMemo(() => sessionPlace(projects, record.session, record.cwd, rest), [projects, record.session, record.cwd, rest]);
  const card = place?.agent ? cardOfPane(projects, place.agent.pane_id) : null;
  const root = card && place?.agent?.pane_id === card.card.root ? card.card : null;
  const device = machineName(rest?.navigator?.devices, record.session.device_id, t);
  // An archived session's record says it is archived, and nothing runs (PRD session-archive B19); a live
  // root whose close left members archived lists them to resume (B24).
  const member = place?.agent ? null : archivedMemberOf(rest, record.session);
  const archived = member && (member.row.listed || member.index > 0) ? member : null;
  const left = archivedChildrenOf(rest, record.session);
  const checkouts = card && root
    ? root.checkouts.map((id) => card.project.checkouts.find((row) => row.id === id)).filter((row): row is NonNullable<typeof row> => row !== undefined)
    : place?.checkout ? [place.checkout] : [];
  return <>
    <p className="mb-sm flex min-w-0 flex-wrap items-center gap-x-xs text-micro text-muted-foreground" data-record-place="true">
      {[device, place?.project.label ?? null].filter(Boolean).join(" · ")}
      {checkouts.map((checkout) => <span key={checkout.id} className={cn("inline-flex min-w-0 items-center gap-xxs", !checkout.exists && "text-muted-foreground/70")} data-record-checkout={checkout.id} data-removed={checkout.exists ? undefined : "true"}>· <HouseIcon aria-hidden="true" className="size-(--size-icon-sm)" /><span className="truncate">{checkout.exists ? checkout.label : t("record.worktreeGone", { label: checkout.label })}</span></span>)}
    </p>
    {archived ? <ArchivedNow row={archived.row} member={archived.row.members[archived.index]!} connected={deviceConnected(rest, archived.row.device_id)} actions={actions} />
      : root || place?.agent || record.ended_at_unix_ms !== null ? <NowBox record={record} agent={place?.agent ?? null} line={root ? cardLine(root, rows) : null} question={root ? cardQuestion(root, rows) : null} actions={actions} /> : null}
    {left ? <ArchivedChildren row={left.row} members={left.members} connected={deviceConnected(rest, left.row.device_id)} actions={actions} /> : null}
  </>;
}

/** What the agent last reported, its newest line, while its pane is still known. */
function lastWords(agent: AgentRow | null): string | null {
  return agent?.message?.trim().split(/\r?\n/).filter((line) => line.trim()).at(-1) ?? null;
}

/**
 * The 지금 box (B14): the PR card's decision box part, edged in the state's
 * colour. Working: `작업 중 · <경과>`, the last words, `pane으로`. A
 * question: `질문 · <경과>`, its first line, `답하러 가기`. Blocked: the
 * reason and `pane 보기`. Ended: `끝남 · <경과>` and the last words, no button.
 */
function NowBox({ record, agent, line, question, actions }: { record: RecordValue; agent: AgentRow | null; line: ReturnType<typeof cardLine> | null; question: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const kind: "question" | "blocked" | "working" | "ended" = line?.kind === "needs_you" || agent?.status_code === "question" || agent?.status_code === "approval" ? "question" : line?.kind === "blocked" ? "blocked" : agent?.status_code === "working" ? "working" : "ended";
  const tone: StateBoxTone = kind === "working" ? "working" : kind === "question" ? "attention" : kind === "blocked" ? "failed" : "done";
  const since = kind === "ended" ? record.ended_at_unix_ms ?? agent?.changed_at_unix_ms ?? null : agent?.changed_at_unix_ms ?? null;
  const word = kind === "working" ? t("agents.status.working") : kind === "question" ? t("agents.status.question") : kind === "blocked" ? t("agents.status.error") : agent && agent.status_code !== "done" ? statusText(t, agent.status_code) : t("sessionWork.done");
  const said = kind === "question" ? question ?? agent?.state.ask?.what ?? null : kind === "blocked" ? (line?.kind === "blocked" ? line.what : null) ?? lastWords(agent) : lastWords(agent);
  const pane = agent?.pane_id ?? null;
  const button = pane === null || kind === "ended" ? null : kind === "question" ? (
    <Button size="sm" onClick={() => actions.openAgent(pane)} data-record-now-button="answer"><CircleHelpIcon aria-hidden="true" />{t("links.answer")}</Button>
  ) : (
    <Button size="sm" variant="secondary" onClick={() => actions.openAgent(pane)} data-record-now-button="pane"><SquareTerminalIcon aria-hidden="true" />{t(kind === "blocked" ? "work.card.viewPane" : "record.toPane")}</Button>
  );
  return <StateBox tone={tone} className="mb-sm" aria-label={t("sessionWork.now")} data-record-now={kind}>
    <div className="flex min-w-0 flex-wrap items-center gap-sm">
      <p className={cn("flex min-w-0 flex-1 items-center gap-xs font-medium", kind === "working" ? "text-agent-working" : kind === "question" ? "text-warning" : kind === "blocked" ? "text-destructive" : "text-foreground")}>
        <span>{word}</span>
        {since !== null ? <Elapsed since={since} className="font-mono text-caption font-normal text-muted-foreground" data-record-now-elapsed="true" /> : null}
      </p>
      {button}
    </div>
    {said ? <p className={cn("whitespace-normal text-caption [overflow-wrap:anywhere]", kind === "question" ? "text-warning" : kind === "blocked" ? "text-destructive" : "text-subtle-foreground")} title={said} data-record-now-words="true">{said}</p> : null}
  </StateBox>;
}

/**
 * The 지금 box of an archived session (PRD session-archive B19-B23, B34):
 * `보관됨 · <경과>` and its last line, with 이어가기 as the one button;
 * 이어가는 중… while it resumes, the reason and 이어가기 again when it could
 * not, 상태 확인 when Hide cannot say. A device that is gone dims it and
 * waits for the device.
 */
function ArchivedNow({ row, member, connected, actions }: { row: ArchivedSession; member: ArchivedMember; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const resuming = row.members.some((one) => one.resume === "queued" || one.resume === "starting");
  const failed = member.resume === "failed" ? member : row.members.find((one) => one.resume === "failed") ?? null;
  const unknown = row.phase === "unknown" || member.resume === "unknown";
  const busy = row.phase === "archiving" || row.phase === "deleting";
  const word = row.phase === "deleting" ? t("archive.deleting") : row.phase === "archiving" ? t("archive.archiving") : unknown ? t("archive.unknown") : resuming ? t("archive.resuming") : failed ? t("archive.resumeFailed") : t("archive.state");
  const button = unknown ? (
    <Button size="sm" variant="secondary" disabled={!connected} onClick={() => actions.checkArchived(row.id)} data-record-now-button="check">{t("workspace.checkStatus")}</Button>
  ) : (
    <Button size="sm" disabled={!connected || resuming || busy} onClick={() => actions.resumeArchived(row.id)} data-record-now-button="resume">
      {resuming ? <LoaderCircleIcon aria-hidden="true" className="animate-spin" /> : null}{resuming ? t("archive.resuming") : t("archive.resume")}
    </Button>
  );
  const reason = failed?.reason ?? row.reason;
  return <StateBox tone={failed ? "failed" : "done"} className={cn("mb-sm", !connected && "opacity-(--opacity-disabled)")} aria-label={t("sessionWork.now")} data-record-now="archived" data-archived-phase={row.phase}>
    <div className="flex min-w-0 flex-wrap items-center gap-sm">
      <p className={cn("flex min-w-0 flex-1 items-center gap-xs font-medium", failed ? "text-destructive" : "text-foreground")}>
        <span>{word}</span>
        <Elapsed since={row.archived_at_unix_ms} className="font-mono text-caption font-normal text-muted-foreground" data-record-now-elapsed="true" />
      </p>
      {button}
    </div>
    {failed && reason ? <p className="whitespace-normal text-caption text-destructive [overflow-wrap:anywhere]" data-record-now-reason="true">{reason}</p> : null}
    {member.last_line ? <p className="whitespace-normal text-caption text-subtle-foreground [overflow-wrap:anywhere]" title={member.last_line} data-record-now-words="true">{member.last_line}</p> : null}
  </StateBox>;
}

/** The members a live root's close or resume left archived (B24): each with its state and its own 이어가기. */
function ArchivedChildren({ row, members, connected, actions }: { row: ArchivedSession; members: readonly ArchivedMember[]; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return <ul className={cn("mb-sm flex min-w-0 flex-col gap-xxs", !connected && "opacity-(--opacity-disabled)")} data-record-archived-children={row.id}>
    {members.map((member) => {
      const resuming = member.resume === "queued" || member.resume === "starting";
      const word = resuming ? t("archive.resuming") : member.resume === "failed" ? t("archive.resumeFailed") : member.resume === "unknown" ? t("archive.unknown") : t("archive.state");
      return <li key={member.session.session_id} className="flex min-w-0 items-center gap-xs" data-record-archived-child={member.session.session_id} data-archived-resume={member.resume}>
        <AgentMark kind={member.agent_kind} />
        <span className="min-w-0 flex-1 truncate" title={[member.title, member.reason].filter(Boolean).join("\n")}>{member.title}</span>
        <span className={cn("shrink-0 text-micro", member.resume === "failed" ? "text-destructive" : "text-muted-foreground")}>{word}</span>
        {member.resume === "unknown" ? (
          <Button size="sm" variant="ghost" disabled={!connected} onClick={() => actions.checkArchived(row.id)} data-record-archived-check={member.session.session_id}>{t("workspace.checkStatus")}</Button>
        ) : (
          <Button size="sm" variant="ghost" disabled={!connected || resuming} onClick={() => actions.resumeArchived(row.id, member.session.session_id)} data-record-archived-resume={member.session.session_id}>{t("archive.resume")}</Button>
        )}
      </li>;
    })}
  </ul>;
}

/** A section's head (PRD record-card-readability D-01): a small quiet label, a strong count, and what the count leaves out in words. */
function SectionHead({ label, count, summary, className }: { label: string; count?: string | number; summary?: string | null; className?: string }) {
  return <h3 className={cn("mb-xs flex min-w-0 flex-wrap items-baseline gap-x-xs text-micro font-medium text-muted-foreground", className)}>
    <span>{label}</span>
    {count !== undefined ? <span className="text-caption font-semibold tabular-nums text-foreground">{count}</span> : null}
    {summary ? <span className="font-normal">· {summary}</span> : null}
  </h3>;
}

function staleHint(output: WorkOutput, unavailable: string): string | null {
  return output.stale ? output.last_read_at_unix_ms ? new Date(output.last_read_at_unix_ms).toLocaleString() : unavailable : null;
}

/** One recorded output as a mark that opens its card (B15): a PR mark, or an issue's dot and number. */
function OutputMark({ output, actions }: { output: WorkOutput; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const state = outputPrState(output.state);
  const word = output.kind === "pr" ? t(PR_LOOK[state].word) : t(output.state === "closed" ? "sessionWork.closed" : "sessionWork.open");
  const stale = staleHint(output, t("common.unavailable"));
  return <button type="button" data-record-output={output.kind} data-record-output-number={output.number} className={cn(ROW, "shrink-0 px-xxs hover:bg-accent")} aria-label={[`#${output.number} ${word}`, output.title, stale].filter(Boolean).join(" · ")} title={output.title} onClick={() => actions.openRecordedOutput(output)}>
    {output.kind === "pr" ? <PrMark state={state} number={output.number} stale={output.stale} /> : <span className={cn("inline-flex shrink-0 items-center gap-xxs font-mono text-caption", output.state === "closed" ? "text-muted-foreground" : "text-success", output.stale && "opacity-(--opacity-dimmed)")}><CircleDotIcon aria-hidden="true" className="size-(--size-pr-icon)" /><span className="text-subtle-foreground">#{output.number}</span></span>}
  </button>;
}

/** The marks of several outputs, worst first; nothing without any. */
function OutputMarks({ outputs, actions }: { outputs: readonly WorkOutput[]; actions: Actions }) {
  if (!outputs.length) return null;
  return <span className="flex shrink-0 flex-wrap items-center gap-xxs" data-record-output-marks={outputs.length}>{orderedOutputs(outputs).map((output) => <OutputMark key={`${output.kind}:${output.repository}:${output.number}`} output={output} actions={actions} />)}</span>;
}

/**
 * What the session was asked and what it handed on (PRD
 * record-card-readability D-01 to D-07, B1-B14): with Agent summaries on, the
 * checklist under `시킨 일 <끝남>/<전체> · 맡긴 일 <N>`, each task with the
 * children it delegated hanging under it by `↳`, the children no task holds
 * under a small `맡긴 일` at its end; with summaries off, which keep no task,
 * the children as one list. Then, with summaries on, `놓친 것 N`, what is
 * still open of what the session and its descendants missed.
 */
const NO_RECORDS: readonly RecordValue[] = [];
function Work({ record, descendants = NO_RECORDS, summaries, onOpen, actions }: { record: RecordValue; descendants?: readonly RecordValue[]; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const list = useMemo(() => checklist(record, descendants), [record, descendants]);
  // The live row's mark, while the session has one: a task in progress is drawn so only while its row draws a ring (B34), and only a live session asks with `!`.
  // A primitive, so a snapshot that moves nothing of this mark leaves the checklist alone.
  const mark = useShellStore((state) => {
    const key = workKey(record.session);
    const localId = localDeviceId(state.rest);
    for (const row of everyAgent(state.rest, state.agents).values()) {
      const session = sessionOfAgentOn(row, localId);
      if (session && workKey(session) === key) return row.mark;
    }
    return null;
  });
  const children = record.children.length;
  const rows = (items: readonly RecordValue[], hang: boolean) => <ChildRows parent={record} items={items} hang={hang} summaries={summaries} onOpen={onOpen} actions={actions} />;
  return <>
    {summaries ? <section className="mt-md" data-record-work="true">
      <SectionHead label={t("sessionWork.tasks")} count={list.total ? `${list.done}/${list.total}` : undefined} summary={children ? t("sessionWork.delegatedCount", { count: children }) : null} />
      {!list.tasks.length && !list.delegated.length && !list.latest ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
      {list.tasks.map((item) => <TaskRow key={item.task.id} item={item} mark={mark} actions={actions}>{item.children.length ? rows(item.children, true) : null}</TaskRow>)}
      {list.latest ? <div className="flex min-w-0 items-start gap-xs py-xs" data-record-latest-request={list.latest.offset}><span className="shrink-0 font-medium text-agent-working">{t("sessionWork.justAsked")}</span><span className="min-w-0 flex-1 whitespace-normal [overflow-wrap:anywhere]" title={list.latest.text}>{list.latest.text.split(/\r?\n/)[0]}</span></div> : null}
      {list.delegated.length ? <div data-record-delegated={list.delegated.length}><h4 className="mt-sm mb-xxs text-micro font-medium text-muted-foreground">{t("sessionWork.delegated")}</h4>{rows(list.delegated, false)}</div> : null}
    </section> : children ? <section className="mt-md" data-record-delegated={children}>
      <SectionHead label={t("sessionWork.delegated")} count={children} />
      {rows(record.children, false)}
    </section> : null}
    {summaries && list.missed.length ? <section className="mt-md" data-record-missed={list.missed.length}>
      <SectionHead label={t("sessionWork.missed")} count={list.missed.length} />
      {list.missed.map(({ owner, missed, task, named }) => <MissedLine key={`${workKey(owner.session)}:${missed.id}`} missed={missed} session={owner.session} cwd={owner.cwd} note={named ? owner.title : task} actions={actions} />)}
    </section> : null}
  </>;
}

/** A task: its box, its name with the request that asked for it in the tooltip (B25), the words its box leaves out, its pull requests, and the children it delegated under it. */
function TaskRow({ item, mark, actions, children }: { item: ChecklistTask; mark: RowMark | null; actions: Actions; children: ReactNode }) {
  const { task } = item;
  const state = shownTaskState(item.state, mark);
  const live = mark !== null;
  const name = <TaskName state={state} name={task.name} className={state === "done" ? undefined : "font-medium"} data-record-task-name="true" />;
  return <div className="py-xxs" data-record-task={task.id}>
    <div className="flex min-w-0 items-start gap-xs">
      <TaskMark state={state} live={live} />
      {item.request ? <Hint label={item.request.text}>{name}</Hint> : name}
      <TaskWord state={state} live={live} prShown={item.prs.length > 0} />
      {item.prs.map((output) => <span key={output.url} data-record-task-pr={output.number}><OutputMark output={output} actions={actions} /></span>)}
    </div>
    {children}
  </div>;
}

/** A child's words when its pane is gone, by its last END; its mark is quiet, since no pane is left to work, answer or read (status-marks D-06, B24). */
const END_STATUS: Record<WorkEnd, AgentStatusCode> = { done: "done", working: "working", waiting: "waiting", question: "question", blocked: "error", unfinished: "unfinished" };
const FIRST_CHILDREN = 5;

/**
 * Children as agent rows (PRD record-card-readability D-06, B10, B11): the
 * sidebar's status mark, the agent's logo and the title, a short state word
 * while it is not done, its pull requests and issues as marks; a finished
 * child is a ✓ and a dim title. The branch it worked on and its device are in
 * the tooltip. `기록 보기` stands at the right while the pointer or the
 * keyboard is on it, and pressing the row opens the child's record in this
 * place. Under a task (`hang`) each row starts with `↳`; a list of its own
 * shows five and `N개 더 보기`.
 */
function ChildRows({ parent, items, hang, summaries, onOpen, actions }: { parent: RecordValue; items: readonly RecordValue[]; hang: boolean; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [visible, setVisible] = useState(FIRST_CHILDREN);
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const rows = useMemo(() => everyAgent(rest, agents), [rest, agents]);
  const shown = hang ? items : items.slice(0, visible);
  return <ol className={cn("min-w-0", hang && LANE)} data-record-children={items.length}>
    {shown.map((child) => {
      const live = [...rows.values()].find((row) => row.session_id === child.session.session_id) ?? null;
      const title = recordTitle(child, summaries);
      const agent = agentProductName(child.session.agent);
      const done = live ? live.status_code === "done" : child.last_end === "done" || (child.tasks.length > 0 && child.tasks.every((task) => task.done));
      const state = done ? null : live ? statusText(t, live.status_code) : child.state ? statusText(t, child.state as AgentStatusCode) : child.last_end ? statusText(t, END_STATUS[child.last_end]) : null;
      const status = live && !done ? <StatusMark mark={live.mark} className={markTone(live)} data-agent-status-mark={live.mark} />
        : done ? <span className="inline-flex size-(--size-agent-mark) items-center justify-center text-success" data-agent-status-mark="done-check"><CheckIcon className="size-full" /></span>
          : <StatusMark mark="quiet" data-agent-status-mark="quiet" />;
      const branch = live ? branchChip(live) : child.cwd && child.cwd !== parent.cwd ? folderName(child.cwd) : null;
      const device = child.session.device_id !== parent.session.device_id ? machineName(rest?.navigator?.devices, child.session.device_id, t) : null;
      const below = recordTree(child).length - 1;
      return <SessionRow
        key={workKey(child.session)}
        mark={<>{hang ? <span className="text-micro text-muted-foreground" data-record-child-arrow="true">↳</span> : null}{status}<AgentMark kind={child.session.agent} /></>}
        title={title}
        titleClassName={done ? "text-subtle-foreground" : undefined}
        hint={[title, agent, branch, device, below ? t("sessionWork.delegatedCount", { count: below }) : null].filter(Boolean).join(" · ")}
        chips={<>
          {state ? <span className="shrink-0 text-caption text-muted-foreground" data-record-child-state="true">{state}</span> : null}
          <OutputMarks outputs={childOutputs(parent, child)} actions={actions} />
        </>}
        reveal={<span className="inline-flex items-center gap-xxs text-caption text-subtle-foreground" data-record-child-open="true">{t("sessionWork.openRecord")}<ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" /></span>}
        onPress={() => onOpen(child)}
        pressLabel={[title, agent, state].filter(Boolean).join(", ")}
        data-record-child={child.session.session_id}
      />;
    })}
    {!hang && items.length > visible ? <li><Button size="sm" variant="ghost" onClick={() => setVisible((count) => count + FIRST_CHILDREN)}>{t("sessionWork.moreChildren", { count: items.length - visible })}</Button></li> : null}
  </ol>;
}

/**
 * An open missed item (PRD overview-device-tasks B43-B46; PRD
 * record-card-readability D-07, B12-B14): one warning line, the task it came
 * from or the descendant that found it quiet after it, `백로그로 · 무시` at its
 * end. `백로그로` makes the item at once in the project the session works in
 * and shows `만드는 중` meanwhile; the line then leaves, as one dismissed does,
 * since what is handled is no longer the record's. A failure keeps the line
 * with its reason and a retry that converges on one item.
 */
function MissedLine({ missed, session, cwd, note, actions, className }: { missed: MissedWork; session: WorkSession; cwd: string | null; note: string | null; actions: Actions; className?: string }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const connected = deviceConnected(rest, session.device_id);
  const project = useMemo(() => {
    const place = sessionPlace(boardProjects(rest, agents, session.device_id), session, cwd, rest);
    // A device the core dials keeps no backlog (B46).
    return place && readsHere(place.project, rest?.navigator?.devices) && place.project.tasks?.source ? place.project : null;
  }, [rest, agents, session, cwd]);
  const item = useMemo(() => [...(rest?.issue_work?.backlog ?? [])].reverse().find((row) => row.missed && workKey(row.missed.session) === workKey(session) && row.missed.id === missed.id) ?? null, [rest?.issue_work?.backlog, session, missed.id]);
  const making = missed.creating || item?.phase === "waiting" || item?.phase === "creating";
  const failed = item?.phase === "failed" ? item : null;
  const destination = destinationOf(project?.tasks?.source ?? null);
  const toBacklog = () => {
    if (project) actions.addBacklog(project.id, missed.text, { session, id: missed.id });
  };
  const small = "h-auto shrink-0 px-xs py-0 text-micro";
  return <div className={cn("flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs py-xxs", className)} data-missed-work={missed.id}>
    <FlagIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-warning" />
    <span className="min-w-0 flex-1 basis-1/2 whitespace-normal text-warning [overflow-wrap:anywhere]" title={missed.text}>{missed.text}{note ? <span className="text-muted-foreground" data-missed-note="true"> · {note}</span> : null}</span>
    {making ? <span className="flex shrink-0 items-center gap-xxs text-micro text-muted-foreground" data-missed-making="true"><LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />{t("backlog.creating")}</span> : failed ? <span role="status" className="flex shrink-0 flex-wrap items-center gap-xs text-micro text-destructive" data-missed-failed={failed.failure ?? "refused"}>
      <span>{t(`backlog.failure.${failed.failure ?? "refused"}`)}</span>
      {failed.retry ? <Button size="sm" variant="ghost" className={small} disabled={!connected} onClick={() => actions.retryBacklog(failed.intent)}>{t("common.retry")}</Button>
        : project ? <Button size="sm" variant="ghost" className={small} disabled={!connected} onClick={toBacklog} data-missed-backlog-add={missed.id}>{t("missed.toBacklog")}</Button> : null}
      <Button size="sm" variant="ghost" className={small} onClick={() => actions.dismissBacklog(failed.intent)} data-missed-failed-dismiss={missed.id}>{t("sessionWork.dismiss")}</Button>
    </span> : <span className="flex shrink-0 items-center gap-xxs">
      {project ? <Hint label={!connected ? t("devices.rail.notConnected") : t(`quickAdd.dest.${destination}`)}><span><Button size="sm" variant="outline" className={small} disabled={!connected} data-missed-backlog-add={missed.id} onClick={toBacklog}>
        {t("missed.toBacklog")}
        {destination === "public" ? <span className="text-micro text-warning" data-missed-public="true">{t("missed.public")}</span> : destination === "unknown" ? <span className="text-micro text-warning" data-missed-public="unknown">{t("missed.maybePublic")}</span> : null}
      </Button></span></Hint> : null}
      <Button size="sm" variant="ghost" className={small} onClick={() => actions.setMissedStatus(session, missed.id, "dismissed")}>{t("sessionWork.dismiss")}</Button>
    </span>}
  </div>;
}

/** The Overview backlog's missed group (D-52): an open missed item with its session's name and kind, where it was found. */
export function MissedRow({ missed, record, actions }: { missed: MissedWork; record: Pick<RecordValue, "session" | "title" | "cwd">; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return <li className="min-w-0 py-xs">
    <MissedLine missed={missed} session={record.session} cwd={record.cwd} note={null} actions={actions} className={LANE} />
    <button type="button" className={cn(ROW, "mt-xxs block w-full truncate text-micro text-muted-foreground", LANE)} title={record.title} onClick={() => actions.openSessionRecord(record.session)}>{record.title} · {t(`sessionWork.kind.${missed.kind}`)}</button>
  </li>;
}

/**
 * The requests the session received, at the record's end (PRD
 * record-card-readability D-15, B24, B25): `내 말 N`, or `받은 요청 N` when a
 * parent asked, each request's first line after who said it (`나`, or the
 * parent) and its time at the right, oldest first with the newest four open
 * and `이전 N턴` before them. The full words stay in the line's tooltip.
 */
function Words({ record, parent }: { record: RecordValue; parent: string | null }) {
  const { t, i18n } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const requests = useMemo(() => [...record.requests].sort((a, b) => a.at_unix_ms - b.at_unix_ms || a.offset - b.offset), [record.requests]);
  const hidden = all ? 0 : Math.max(0, requests.length - FIRST_WORDS);
  const received = requests.some((request) => request.author === "parent");
  const language = requireInterfaceLanguage(i18n.language);
  return <section className="mt-md" data-record-words={requests.length}>
    <SectionHead label={t(received ? "sessionWork.receivedWords" : "sessionWork.myWords")} count={requests.length} />
    {!requests.length ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
    {hidden ? <button type="button" className={cn(ROW, "flex items-center gap-xxs py-xxs text-micro text-muted-foreground hover:text-foreground")} aria-expanded="false" onClick={() => setAll(true)} data-record-words-earlier={hidden}>{t("sessionWork.earlierTurns", { count: hidden })}<ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" /></button> : null}
    <ol className="min-w-0">
      {requests.slice(hidden).map((request) => {
        const who = request.author === "operator" ? t("sessionWork.me") : parent ?? t("sessionWork.parent");
        return <li key={request.offset} className="flex min-w-0 items-baseline gap-sm py-xxs" data-record-request={request.offset}>
          <span className="w-(--size-control) shrink-0 truncate text-micro text-muted-foreground" title={who}>{who}</span>
          <span className="min-w-0 flex-1 truncate text-subtle-foreground" title={request.text}>{request.text.split(/\r?\n/)[0]}</span>
          <time className="shrink-0 font-mono text-micro text-muted-foreground" dateTime={new Date(request.at_unix_ms).toISOString()}>{formatDateTime(language, request.at_unix_ms, { hour: "2-digit", minute: "2-digit", hour12: false })}</time>
        </li>;
      })}
    </ol>
  </section>;
}
