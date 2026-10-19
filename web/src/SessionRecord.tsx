import { ArrowLeftIcon, CheckIcon, ChevronRightIcon, CircleDotIcon, CircleHelpIcon, Clock3Icon, SquareIcon, GitPullRequestIcon, InfoIcon, LightbulbIcon, MessageSquareIcon, TriangleAlertIcon, HouseIcon, LoaderCircleIcon, SquareTerminalIcon, RotateCcwIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { deviceConnected, readsHere } from "./devices";
import { branchChip, markTone } from "./agentRow";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs";
import { Button } from "./components/ui/button";
import { Elapsed } from "./components/elapsed";
import { PrMark } from "./components/pr-mark";
import { StateBox } from "./components/state-box";
import { StatusMark } from "./components/status-mark";
import { shownTaskState, TaskMark, TaskName, TaskWord } from "./components/task-progress";
import { Hint } from "./components/ui/tooltip";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { checklist, childOutputs, orderedOutputs, outputPrState, recordTitle, recordTree, sessionOfAgentOn, sessionPlace, workKey, type ChecklistTask } from "./sessionWork";
import { boardProjects } from "./navigation";
import { cardLine, cardOfPane, cardQuestion, everyAgent } from "./overviewTasks";
import { destinationOf } from "./overviewBacklog";
import { folderName, resumeBlock, resumeProvider } from "./linkPanel";
import { useSessionResume } from "./useSessionResume";
import { recordDate } from "./sessionRecordTime";
import { machineName } from "./screenMachine";
import { localDeviceId } from "./snapshot";
import type { AgentRow, AgentStatusCode, Checkout, MissedWork, RowMark, SessionRecord as RecordValue, WorkEnd, WorkOutput, WorkSession } from "./snapshot";
import { statusText } from "./agentStatus";
import { useShellStore } from "./store";
import { summariesEnabled } from "./hideAi";
import { PR_LOOK } from "./prMark";
import { archivedChildrenOf, archivedMemberOf } from "./archive";
import type { ArchivedMember, ArchivedSession } from "./snapshot";
import { HighlightedRequest } from "./RequestHighlight";

type RequestLocator = { request_offset: number; query?: string };

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
  /** A retained History hit, scoped by the target's full session identity. */
  locator?: RequestLocator;
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
  const wasConnected = useRef(connected);
  // The connection this window is on: a new self-contained snapshot is a new core session (a reconnect, or a core that moved), which has read nothing.
  const generation = useShellStore((state) => state.viewGeneration);
  const askedIn = useRef<number | null>(null);
  const leave = useRef(frame.onLeave);
  leave.current = frame.onLeave;
  useEffect(() => () => leave.current(), []);
  // Read again on a reconnect, and once per connection when the slot draws a
  // record with no read behind it: one restored after a relaunch, or kept
  // across a core that moved, was opened by no event. The Overview's panel
  // starts its own read when it opens.
  useEffect(() => {
    const unread = frame.kind === "lane" && !own && askedIn.current !== generation;
    if (connected && (!wasConnected.current || unread)) {
      askedIn.current = generation;
      frame.onRead();
    }
    wasConnected.current = connected;
  }, [frame, connected, own, generation]);
  const up = () => frame.onTrail(frame.trail.slice(0, -1));
  const open = (child: RecordValue) => frame.onTrail([...frame.trail, child.session]);
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-session-record={frame.kind}>
    {frame.kind === "lane" ? null : <>
      {frame.head}
      {parent ? <Button size="sm" variant="ghost" className="min-w-0 self-start px-md" onClick={up} data-record-up="true"><ArrowLeftIcon /><span className="truncate">{t("record.backTo", { parent: recordTitle(parent, summaries) })}</span></Button> : null}
    </>}
    {frame.note ? <p className="px-md pb-xs text-caption text-muted-foreground" data-record-note="true">{frame.note}</p> : null}
    {!own || own.loading ? <div className="space-y-md px-md py-sm" aria-busy="true" data-record-loading="true">{[0, 1, 2].map((key) => <div key={key} className="h-(--size-control-lg) rounded-sm bg-muted" />)}</div> : own.failure ? <div role="status" className="px-md py-sm text-caption"><p>{t("sessionWork.failed")}</p><Button size="sm" variant="ghost" onClick={frame.onRead}>{t("common.retry")}</Button></div> : shown ? <div className={cn("min-h-0 flex-1 overflow-y-auto px-lg pb-lg text-caption [word-break:keep-all]", !connected && "opacity-(--opacity-disabled)")} data-record-content="true">
      <h2 className="flex min-w-0 items-center gap-xs py-sm text-subhead font-semibold"><AgentMark kind={shown.session.agent} /><span className="min-w-0 whitespace-normal [overflow-wrap:anywhere]" title={recordTitle(shown, summaries)}>{recordTitle(shown, summaries)}</span></h2>
      <RecordPlace key={workKey(shown.session)} record={shown} actions={actions} />
      <Work key={`work:${workKey(shown.session)}`} record={shown} summaries={summaries} onOpen={open} actions={actions} parent={parent ? recordTitle(parent, summaries) : liveParent} locator={workKey(shown.session) === workKey(target) ? frame.locator : undefined} />
    </div> : null}
  </div>;
}

/** Agent product, device, project and checkouts, followed by this session's native state. */
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
      {[agentProductName(record.session.agent), device, place?.project.label ?? null].filter(Boolean).join(" · ")}
      {checkouts.map((checkout) => <span key={checkout.id} className={cn("inline-flex min-w-0 items-center gap-xxs", !checkout.exists && "text-muted-foreground/70")} data-record-checkout={checkout.id} data-removed={checkout.exists ? undefined : "true"}>· <HouseIcon aria-hidden="true" className="size-(--size-icon-sm)" /><span className="truncate">{checkout.exists ? checkout.label : t("record.worktreeGone", { label: checkout.label })}</span></span>)}
    </p>
    {archived ? <ArchivedNow row={archived.row} member={archived.row.members[archived.index]!} own={archived.index > 0} connected={deviceConnected(rest, archived.row.device_id)} actions={actions} />
      : <NowBox record={record} checkout={place?.checkout ?? null} agent={place?.agent ?? null} line={root ? cardLine(root, rows) : null} question={root ? cardQuestion(root, rows) : null} actions={actions} />}
    {left ? <ArchivedChildren row={left.row} members={left.members} connected={deviceConnected(rest, left.row.device_id)} actions={actions} /> : null}
  </>;
}

/** Uses the same device, checkout and native resume boundary as a PR's session line. */
function RestoreSession({ record, checkout: original, actions }: { record: RecordValue; checkout: Checkout | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const rest = useShellStore((state) => state.rest);
  const connected = useShellStore((state) => state.connection === "live");
  const node = localDeviceId(rest);
  const checkout = original?.exists ? original : null;
  const blocked = resumeBlock({ ...record.session, file: record.path ? "present" : "missing" }, checkout, rest?.navigator?.devices, node);
  const { resume, start } = useSessionResume();
  const busy = resume.request !== null && !resume.uncertain;
  const off = blocked !== null || !connected;
  const reason = blocked ? "device" in blocked ? t(blocked.key, { device: blocked.device }) : t(blocked.key) : !connected ? t("devices.rail.notConnected") : null;
  const restore = () => {
    const provider = resumeProvider(record.session.agent);
    if (off || busy || !provider || !checkout) return;
    start((requestId, confirmOnly) => actions.startAgent({ target: { checkoutPath: checkout.path }, deviceId: record.session.device_id === node ? undefined : record.session.device_id, provider, resumeSessionId: record.session.session_id, resumeSessionPath: record.path ?? undefined, requestId, confirmOnly }));
  };
  return <div className="flex min-w-0 flex-col items-end gap-xs" data-record-restore="true">
    <Button size="sm" disabled={off || busy} aria-busy={busy || undefined} onClick={restore} data-record-restore-button="true">
      {busy ? <LoaderCircleIcon aria-hidden="true" className="animate-spin" /> : <RotateCcwIcon aria-hidden="true" />}{t(resume.uncertain ? "groups.confirm" : busy ? "recordCard.restoring" : "recordCard.restore")}
    </Button>
    {resume.uncertain ? <p role="status" className="text-micro text-muted-foreground [overflow-wrap:anywhere]">{t("links.resumeUnknown")}</p> : null}
    {reason ? <p className="text-micro text-muted-foreground [overflow-wrap:anywhere]" data-record-restore-unavailable="true">{reason}</p> : null}
    {resume.failure !== null ? <div role="alert" className="flex min-w-0 flex-wrap items-center gap-xs text-caption text-warning" data-record-restore-failed="true">
      <span className="min-w-0 [overflow-wrap:anywhere]">{resume.uncertain ? resume.failure : resume.failure ? t("links.resumeFailed", { reason: resume.failure }) : t("links.resumeFailedPlain")}</span>
      {!resume.uncertain ? <Button size="sm" variant="ghost" disabled={off || busy} onClick={restore}>{t("common.retry")}</Button> : null}
    </div> : null}
  </div>;
}

/** What the agent last reported, its newest line, while its pane is still known. */
function lastWords(agent: AgentRow | null): string | null {
  return agent?.message?.trim().split(/\r?\n/).filter((line) => line.trim()).at(-1) ?? null;
}

/** Native state and the action for this session: visit its pane or restore its conversation. */
function NowBox({ record, checkout, agent, line, question, actions }: { record: RecordValue; checkout: Checkout | null; agent: AgentRow | null; line: ReturnType<typeof cardLine> | null; question: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const kind: "question" | "blocked" | "working" | "ended" = line?.kind === "needs_you" || agent?.status_code === "question" || agent?.status_code === "approval" ? "question" : line?.kind === "blocked" ? "blocked" : agent?.status_code === "working" ? "working" : "ended";
  const since = kind === "ended" ? record.ended_at_unix_ms ?? agent?.changed_at_unix_ms ?? null : agent?.changed_at_unix_ms ?? null;
  const word = kind === "working" ? t("agents.status.working") : kind === "question" ? t("agents.status.question") : kind === "blocked" ? t("agents.status.error") : agent && agent.status_code !== "done" ? statusText(t, agent.status_code) : t("sessionWork.done");
  const said = kind === "question" ? question ?? agent?.state.ask?.what ?? null : kind === "blocked" ? (line?.kind === "blocked" ? line.what : null) ?? lastWords(agent) : lastWords(agent);
  const pane = agent?.pane_id ?? null;
  const button = pane === null ? <RestoreSession record={record} checkout={checkout} actions={actions} /> : kind === "question" ? (
    <Button size="sm" onClick={() => actions.openAgent(pane)} data-record-now-button="answer"><CircleHelpIcon aria-hidden="true" />{t("links.answer")}</Button>
  ) : (
    <Button size="sm" variant="secondary" onClick={() => actions.openAgent(pane)} data-record-now-button="pane"><SquareTerminalIcon aria-hidden="true" />{t(kind === "blocked" ? "work.card.viewPane" : "record.toPane")}</Button>
  );
  return <div className="mb-sm flex min-w-0 flex-col gap-xs" aria-label={t("sessionWork.now")} data-record-now={kind}>
    <div className="flex min-w-0 flex-wrap items-center gap-sm">
      <p className={cn("flex min-w-0 flex-1 flex-wrap items-center gap-xs font-medium", kind === "working" ? "text-agent-working" : kind === "question" ? "text-warning" : kind === "blocked" ? "text-destructive" : "text-foreground")}>
        <span>{word}</span>
        {since !== null ? <Elapsed since={since} className="font-mono text-caption font-normal text-muted-foreground" data-record-now-elapsed="true" /> : null}
        <span className="rounded-xs border border-border px-xs text-micro font-normal text-muted-foreground" data-record-pane={pane ? "open" : "closed"}>{t(pane ? "recordCard.paneOpen" : "recordCard.paneClosed")}</span>
      </p>
      {button}
    </div>
    {said && kind !== "working" ? <p className={cn("whitespace-normal text-caption [overflow-wrap:anywhere]", kind === "question" ? "text-warning" : kind === "blocked" ? "text-destructive" : "text-subtle-foreground")} title={said} data-record-now-words="true">{said}</p> : null}
  </div>;
}

/**
 * The 지금 box of an archived session (PRD session-archive B19-B23, B34):
 * `보관됨 · <경과>` and its last line, with 이어가기 as the one button;
 * 이어가는 중… while it resumes, the reason and 이어가기 again when it could
 * not, 상태 확인 when Hide cannot say. A device that is gone dims it and
 * waits for the device.
 */
function ArchivedNow({ row, member, own, connected, actions }: { row: ArchivedSession; member: ArchivedMember; own: boolean; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // A child's own record speaks for that child; the root's for the whole session.
  const mine = own ? [member] : row.members;
  const resuming = mine.some((one) => one.resume === "queued" || one.resume === "starting");
  const failed = mine.find((one) => one.resume === "failed") ?? null;
  const unknown = row.phase === "unknown" || member.resume === "unknown";
  const busy = row.phase === "archiving" || row.phase === "deleting";
  const word = row.phase === "deleting" ? t("archive.deleting") : row.phase === "archiving" ? t("archive.archiving") : unknown ? t("archive.unknown") : resuming ? t("archive.resuming") : failed ? t("archive.resumeFailed") : t("archive.state");
  const button = unknown ? (
    <Button size="sm" variant="secondary" disabled={!connected} onClick={() => actions.checkArchived(row.id)} data-record-now-button="check">{t("workspace.checkStatus")}</Button>
  ) : (
    <Button size="sm" disabled={!connected || resuming || busy} onClick={() => actions.resumeArchived(row.id, own ? member.session.session_id : undefined)} data-record-now-button="resume">
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

/** An archived member's state in one word. */
function archivedWord(t: ReturnType<typeof useInterfaceTranslation>["t"], resume: ArchivedMember["resume"]): string {
  return resume === "queued" || resume === "starting" ? t("archive.resuming") : resume === "failed" ? t("archive.resumeFailed") : resume === "unknown" ? t("archive.unknown") : t("archive.state");
}

/** The members a live root's close or resume left archived (B24): each with its state and its own 이어가기. */
function ArchivedChildren({ row, members, connected, actions }: { row: ArchivedSession; members: readonly ArchivedMember[]; connected: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return <ul className={cn("mb-sm flex min-w-0 flex-col gap-xxs", !connected && "opacity-(--opacity-disabled)")} data-record-archived-children={row.id}>
    {members.map((member) => {
      const resuming = member.resume === "queued" || member.resume === "starting";
      const word = archivedWord(t, member.resume);
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
  return <h3 className={cn("mb-xs flex min-w-0 flex-wrap items-baseline gap-x-xs text-caption font-semibold text-foreground", className)}>
    <span>{label}</span>
    {count !== undefined ? <span className="text-caption font-semibold tabular-nums text-foreground">{count}</span> : null}
    {summary ? <span className="text-micro font-normal text-muted-foreground">· {summary}</span> : null}
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

/** The current session owns the requested work and tabs. Children expand without replacing it. */
function Work({ record, summaries, onOpen, actions, parent, locator }: { record: RecordValue; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions; parent: string | null; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  return <>
    <RequestedWork record={record} summaries={summaries} />
    <RecordTabs record={record} summaries={summaries} actions={actions} parent={parent} locator={locator} />
    {record.children.length ? <section className="mt-lg min-w-0" data-record-delegated={record.children.length}>
      <SectionHead label={t("recordCard.children")} count={record.children.length} />
      <ChildRows parent={record} summaries={summaries} onOpen={onOpen} actions={actions} />
    </section> : null}
  </>;
}

function useRecordAgent(record: RecordValue) {
  return useShellStore((state) => {
    const key = workKey(record.session);
    for (const row of everyAgent(state.rest, state.agents).values()) {
      const session = sessionOfAgentOn(row, localDeviceId(state.rest));
      if (session && workKey(session) === key) return row;
    }
    return null;
  });
}

function RequestTime({ at, full = false }: { at: number; full?: boolean }) {
  const { i18n } = useInterfaceTranslation();
  return <time className="shrink-0 font-mono text-micro text-muted-foreground" dateTime={new Date(at).toISOString()}>{full ? `${recordDate(at)} ` : null}{formatDateTime(requireInterfaceLanguage(i18n.language), at, { hour: "2-digit", minute: "2-digit", hour12: false })}</time>;
}

function RequestedWork({ record, summaries }: { record: RecordValue; summaries: boolean }) {
  const { t } = useInterfaceTranslation();
  const list = useMemo(() => checklist(record), [record]);
  const agent = useRecordAgent(record);
  const latest = [...record.requests].sort((a, b) => b.at_unix_ms - a.at_unix_ms || b.offset - a.offset)[0];
  const said = lastWords(agent);
  return <section className="mt-lg min-w-0" data-record-work={summaries && list.tasks.length ? "true" : undefined}>
    <SectionHead label={t(summaries && list.tasks.length ? "sessionWork.tasks" : "recordCard.latestRequest")} summary={summaries && list.total ? t("recordCard.completed", { done: list.done, total: list.total }) : null} />
    {summaries ? list.tasks.map((item) => <TaskRow key={item.task.id} item={item} mark={agent?.mark ?? null} />) : null}
    {(!summaries || !list.tasks.length) && latest ? <div className="whitespace-pre-wrap text-caption [overflow-wrap:anywhere]" data-record-raw-request="true"><p className="mb-xs text-micro text-muted-foreground">{t(summaries ? latest.judged ? "recordCard.request" : "recordCard.pending" : "recordCard.aiOff")} · <RequestTime at={latest.at_unix_ms} full /></p>{latest.text}</div> : null}
    {!latest && !list.tasks.length ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
    {summaries && list.tasks.length && list.latest ? <p className="mt-sm whitespace-pre-wrap text-caption [overflow-wrap:anywhere]" data-record-latest-request={list.latest.offset}><span className="mr-xs text-micro text-muted-foreground">{t("recordCard.pending")} · <RequestTime at={list.latest.at_unix_ms} full /></span>{list.latest.text}</p> : null}
    {said && agent?.status_code === "working" ? <p className="mt-sm flex min-w-0 items-baseline gap-xs border-t border-border pt-sm text-caption" data-record-current="true"><span className="shrink-0 text-micro text-agent-working">{t("sessionWork.now")}</span><span className="min-w-0 whitespace-normal text-subtle-foreground [overflow-wrap:anywhere]">{said}</span></p> : null}
  </section>;
}

/** The original request is visible beneath the task, including its actual timestamp. */
function TaskRow({ item, mark }: { item: ChecklistTask; mark: RowMark | null }) {
  const { t } = useInterfaceTranslation();
  const state = shownTaskState(item.state, mark);
  return <div className="min-w-0 py-sm" data-record-task={item.task.id}>
    <div className="flex min-w-0 items-start gap-xs">
      <TaskMark state={state} live={mark !== null} question={item.task.end === "question"} />
      <TaskName state={state} name={item.task.name} className="font-medium" data-record-task-name="true" />
      {state === "working" || state === "done" ? <span className={cn("shrink-0 text-micro leading-(--size-sidebar-line)", state === "working" ? "text-agent-working" : "text-muted-foreground")}>{t(state === "done" ? "sessionWork.done" : "agents.status.working")}</span> : <TaskWord state={state} live={mark !== null} prShown={false} />}
    </div>
    {item.request ? <p className={cn("mt-xs whitespace-pre-wrap text-caption text-subtle-foreground [overflow-wrap:anywhere]", LANE)} data-record-task-request={item.request.offset}><span className="mr-xs text-micro text-muted-foreground">{t("recordCard.request")} · <RequestTime at={item.request.at_unix_ms} /></span>{item.request.text}</p> : null}
  </div>;
}

function RecordTabs({ record, summaries, actions, parent, locator }: { record: RecordValue; summaries: boolean; actions: Actions; parent: string | null; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  const [selected, setSelected] = useState(locator ? "requests" : record.missed.some((item) => item.status === "open") && summaries ? "findings" : "prs");
  useEffect(() => { if (locator) setSelected("requests"); }, [locator?.request_offset, locator?.query]);
  const own = workKey(record.session);
  const outputs = orderedOutputs(record.outputs.filter((output) => workKey(output.source) === own));
  const found = summaries ? record.missed.filter((item) => item.status === "open") : [];
  const primary = outputs.filter((output) => output.state !== "merged" && output.state !== "closed");
  const settled = outputs.filter((output) => output.state === "merged" || output.state === "closed");
  const outputRows = (items: WorkOutput[]) => items.map((output) => <div key={`${output.kind}:${output.repository}:${output.number}`} className="flex min-w-0 items-start gap-sm py-xs" data-record-pr-row={output.number}><OutputMark output={output} actions={actions} /><button type="button" className={cn(ROW, "min-w-0 flex-1 whitespace-normal text-caption hover:underline [overflow-wrap:anywhere]", output.stale && "opacity-(--opacity-dimmed)")} onClick={() => actions.openRecordedOutput(output)} title={staleHint(output, t("common.unavailable")) ?? undefined}>{output.title}</button></div>);
  const tab = "h-auto min-w-0 max-w-full shrink-0 rounded-none border-b-2 border-transparent bg-transparent px-none py-sm text-caption font-medium data-[state=active]:border-foreground data-[state=active]:bg-transparent";
  return <Tabs value={!summaries && selected === "findings" ? "prs" : selected} onValueChange={setSelected} className="mt-lg min-w-0 gap-sm border-t border-border pt-sm" data-record-tabs="true">
    <TabsList className="w-full flex-wrap justify-start gap-sm rounded-none border-b border-border bg-transparent p-none">
      <TabsTrigger value="prs" className={tab}><GitPullRequestIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t(outputs.some((output) => output.kind === "issue") ? "recordCard.results" : "recordCard.prs")} <span className="text-micro text-muted-foreground">{outputs.length}</span></TabsTrigger>
      {summaries ? <TabsTrigger value="findings" className={tab}><TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("recordCard.findings")} <span className="text-micro text-muted-foreground">{found.length}</span></TabsTrigger> : null}
      <TabsTrigger value="requests" className={tab}><MessageSquareIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("recordCard.history")} <span className="text-micro text-muted-foreground">{record.requests.length}</span></TabsTrigger>
    </TabsList>
    <TabsContent value="prs" className="min-w-0">
      {!outputs.length ? <p className="py-xs text-micro text-muted-foreground">{t("recordCard.noPrs")}</p> : null}
      {outputRows(primary)}
      {settled.length ? <details className="min-w-0"><summary className="cursor-pointer py-xs text-micro text-muted-foreground">{t("recordCard.settled", { count: settled.length })}</summary>{outputRows(settled)}</details> : null}
    </TabsContent>
    {summaries ? <TabsContent value="findings" className="min-w-0" data-record-missed={found.length}>
      <p className="mb-sm text-micro text-muted-foreground">{t(found.length ? "recordCard.unassigned" : "recordCard.noFindings")}</p>
      {found.map((missed) => <MissedLine key={missed.id} missed={missed} session={record.session} cwd={record.cwd} note={null} actions={actions} />)}
    </TabsContent> : null}
    <TabsContent value="requests" className="min-w-0"><Words record={record} parent={parent} locator={locator} /></TabsContent>
  </Tabs>;
}

/** One flat list, with one child expanded at a time and no row of unbounded PR marks. */
const END_STATUS: Record<WorkEnd, AgentStatusCode> = { done: "done", working: "working", waiting: "waiting", question: "question", blocked: "error", unfinished: "unfinished" };
const FIRST_CHILDREN = 5;
function ChildRows({ parent, summaries, onOpen, actions }: { parent: RecordValue; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [visible, setVisible] = useState(FIRST_CHILDREN);
  const [expanded, setExpanded] = useState<string | null>(null);
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const rows = useMemo(() => [...everyAgent(rest, agents).values()], [rest, agents]);
  return <ol className="min-w-0" data-record-children={parent.children.length}>
    {parent.children.slice(0, visible).map((child, index) => {
      const key = workKey(child.session);
      const live = rows.find((row) => { const session = sessionOfAgentOn(row, localDeviceId(rest)); return session && workKey(session) === key; }) ?? null;
      const held = live ? null : archivedMemberOf(rest, child.session);
      const away = held ? held.row.members[held.index]!.resume : null;
      // The core also knows report completion and whether the newest turn was judged.
      const progress = summaries ? parent.child_progress?.children[index] : undefined;
      const done = live?.status_code === "working" ? false : progress !== undefined ? !away && progress === "done" : live ? live.status_code === "done" : summaries && !away && (child.tasks.length > 0 ? child.tasks.every((task) => task.done) : child.last_end === "done");
      const state = away && away !== "resumed" ? archivedWord(t, away) : done ? t("sessionWork.done") : live ? statusText(t, live.status_code) : progress === "unjudged" ? t("common.unavailable") : summaries && child.last_end ? (child.last_end === "done" && (progress === "other" || child.tasks.some((task) => !task.done)) ? t("sessionWork.left") : statusText(t, END_STATUS[child.last_end])) : t("common.unavailable");
      const title = recordTitle(child, summaries);
      const outputs = childOutputs(parent, child).filter((output) => workKey(output.source) === key);
      const prs = outputs.filter((output) => output.kind === "pr").length;
      const findings = summaries ? child.missed.filter((item) => item.status === "open").length : 0;
      const branch = live ? branchChip(live) : child.cwd && child.cwd !== parent.cwd ? folderName(child.cwd) : null;
      const device = child.session.device_id !== parent.session.device_id ? machineName(rest?.navigator?.devices, child.session.device_id, t) : null;
      return <li key={key} className="min-w-0 border-b border-border last:border-b-0" data-record-child={child.session.session_id}>
        <button type="button" className={cn(ROW, "flex w-full items-start gap-sm py-sm hover:bg-accent")} aria-expanded={expanded === key} title={[branch, device].filter(Boolean).join(" · ")} onClick={() => setExpanded(expanded === key ? null : key)}>
          {done ? <CheckIcon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-success" /> : live ? <StatusMark mark={live.mark} className={cn("mt-xs shrink-0", markTone(live))} /> : summaries && progress !== "unjudged" && child.last_end === "waiting" ? <Clock3Icon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-muted-foreground" /> : <SquareIcon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-muted-foreground" />}
          <span className="min-w-0 flex-1"><span className={cn("block whitespace-normal text-caption font-medium [overflow-wrap:anywhere]", done && "text-subtle-foreground")}>{title}</span><span className="mt-xs flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs text-micro text-muted-foreground"><AgentMark kind={child.session.agent} />{agentProductName(child.session.agent)} · <span data-record-child-state="true">{state}</span>{prs ? <span>· {t("recordCard.prs")} {prs}</span> : null}{findings ? <span>· {t("recordCard.findings")} {findings}</span> : null}</span></span>
          <ChevronRightIcon aria-hidden="true" className={cn("mt-xs size-(--size-icon-sm) shrink-0 text-muted-foreground", expanded === key && "rotate-90")} />
        </button>
        {expanded === key ? <div className="mb-sm ml-md min-w-0 border-l-2 border-border bg-card px-sm pb-sm" data-record-child-detail={child.session.session_id}>
          <div className="flex min-w-0 flex-wrap gap-xs pt-sm"><Button size="sm" variant="ghost" onClick={() => onOpen(child)}>{t("sessionWork.openRecord")}<ChevronRightIcon aria-hidden="true" /></Button>{live ? <Button size="sm" variant="ghost" onClick={() => actions.openAgent(live.pane_id)}><SquareTerminalIcon aria-hidden="true" />{t("record.toPane")}</Button> : null}</div>
          <RequestedWork record={child} summaries={summaries} />
          <RecordTabs record={{ ...child, outputs }} summaries={summaries} actions={actions} parent={recordTitle(parent, summaries)} />
        </div> : null}
      </li>;
    })}
    {parent.children.length > visible ? <li><Button size="sm" variant="ghost" onClick={() => setVisible((count) => count + FIRST_CHILDREN)}>{t("sessionWork.moreChildren", { count: parent.children.length - visible })}</Button></li> : null}
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
  return <div className={cn("flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xs border-b border-border py-sm last:border-b-0", className)} data-missed-work={missed.id}>
    <span className={cn("inline-flex shrink-0 items-center gap-xxs text-micro", missed.kind === "problem" ? "text-warning" : "text-muted-foreground")}>
      {missed.kind === "problem" ? <TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <LightbulbIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
      {t(`sessionWork.kind.${missed.kind}`)}
    </span>
    <span className="min-w-0 flex-1 basis-1/2 whitespace-normal text-foreground [overflow-wrap:anywhere]" title={missed.text}>{missed.text}{note ? <span className="text-muted-foreground" data-missed-note="true"> · {note}</span> : null}</span>
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

/** Full request history, author and timestamp, with the latest four initially visible. */
function Words({ record, parent, locator }: { record: RecordValue; parent: string | null; locator?: RequestLocator }) {
  const { t, i18n } = useInterfaceTranslation();
  const [all, setAll] = useState(false);
  const requests = useMemo(() => [...record.requests].sort((a, b) => a.at_unix_ms - b.at_unix_ms || a.offset - b.offset), [record.requests]);
  const hidden = all || locator ? 0 : Math.max(0, requests.length - FIRST_WORDS);
  const target = useRef<HTMLDivElement>(null);
  const located = locator && requests.some((request) => request.offset === locator.request_offset);
  useEffect(() => {
    if (located && target.current) {
      target.current.focus({ preventScroll: true });
      target.current.scrollIntoView({ block: "center" });
    }
  }, [located, locator?.request_offset, locator?.query]);
  const language = requireInterfaceLanguage(i18n.language);
  return <section className="min-w-0" data-record-words={requests.length}>
    {locator ? <Hint label={t("overviewHistory.limit")}><button type="button" className="inline-flex items-center gap-xs rounded-xs py-xs text-micro text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-record-retention="true"><InfoIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("overviewHistory.limitLabel")}</button></Hint> : null}
    {locator && !located ? <p role="status" className="py-xs text-caption text-muted-foreground" data-record-request-unavailable="true">{t("overviewHistory.requestUnavailable")}</p> : null}
    {!requests.length ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
    {hidden ? <button type="button" className={cn(ROW, "flex items-center gap-xxs py-xxs text-micro text-muted-foreground hover:text-foreground")} aria-expanded="false" onClick={() => setAll(true)} data-record-words-earlier={hidden}>{t("sessionWork.earlierTurns", { count: hidden })}<ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" /></button> : null}
    <ol className="min-w-0">
      {requests.slice(hidden).map((request, index, shown) => {
        const who = request.author === "operator" ? t("sessionWork.me") : parent ?? t("sessionWork.parent");
        const matched = locator?.request_offset === request.offset;
        const day = recordDate(request.at_unix_ms);
        const first = index === 0 || recordDate(shown[index - 1]!.at_unix_ms) !== day;
        return <li key={request.offset} className="min-w-0">
          {first ? <div className="flex items-center gap-sm pt-sm pb-xs" data-record-date={day}><time dateTime={day} className="shrink-0 font-mono text-micro text-muted-foreground">{day}</time><span className="h-px min-w-0 flex-1 bg-border" aria-hidden="true" /></div> : null}
          <div ref={matched ? target : undefined} tabIndex={matched ? -1 : undefined} className={cn("flex min-w-0 flex-wrap items-baseline gap-sm py-sm", matched && "rounded-xs bg-accent/40 px-xs outline-none focus:ring-1 focus:ring-ring")} data-record-request={request.offset} data-record-request-match={matched ? "true" : undefined}>
          <span className="w-(--size-control) shrink-0 truncate text-micro text-muted-foreground" title={who}>{who}</span>
          <span className="min-w-0 flex-1 whitespace-pre-wrap text-subtle-foreground [overflow-wrap:anywhere]" title={request.text}>{matched ? <HighlightedRequest text={request.text} query={locator.query ?? ""} /> : request.text}</span>
          <time className="shrink-0 font-mono text-micro text-muted-foreground" dateTime={new Date(request.at_unix_ms).toISOString()}>{formatDateTime(language, request.at_unix_ms, { hour: "2-digit", minute: "2-digit", hour12: false })}</time>
          </div>
        </li>;
      })}
    </ol>
  </section>;
}
