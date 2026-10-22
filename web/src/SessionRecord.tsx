import { ArrowLeftIcon, CheckIcon, ChevronRightIcon, CircleDotIcon, CircleHelpIcon, Clock3Icon, SquareIcon, ImageIcon, InfoIcon, LightbulbIcon, MailIcon, TriangleAlertIcon, HouseIcon, LoaderCircleIcon, SquareTerminalIcon, RotateCcwIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { deviceConnected, readsHere } from "./devices";
import { branchChip, markTone } from "./agentRow";
import { Button } from "./components/ui/button";
import { Elapsed } from "./components/elapsed";
import { PrMark } from "./components/pr-mark";
import { StateBox } from "./components/state-box";
import { StatusMark } from "./components/status-mark";
import { Hint } from "./components/ui/tooltip";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { holdsRequest, orderedOutputs, requestRuns, outputPrState, recordTitle, recordTree, requestPreview, sessionOfAgentOn, sessionPlace, settledOutput, workKey } from "./sessionWork";
import { boardProjects } from "./navigation";
import { cardLine, cardOfPane, cardQuestion, everyAgent } from "./overviewTasks";
import { destinationOf } from "./overviewBacklog";
import { folderName, resumeBlock, resumeProvider } from "./linkPanel";
import { useSessionResume } from "./useSessionResume";
import { recordDate } from "./sessionRecordTime";
import { machineName } from "./screenMachine";
import { localDeviceId } from "./snapshot";
import type { AgentRow, Checkout, FlowResult, MissedWork, PlanView, PrSettlement, SessionRecord as RecordValue, WorkEnd, WorkOutput, WorkRequest, WorkSession } from "./snapshot";
import { PlanGraph } from "./PlanGraph";
import { planCount, planOfSession, planWorkLine } from "./plans";
import { statusText } from "./agentStatus";
import { useShellStore } from "./store";
import { summariesEnabled } from "./hideAi";
import { PR_LOOK } from "./prMark";
import { archivedChildrenOf, archivedMemberOf } from "./archive";
import type { ArchivedMember, ArchivedSession } from "./snapshot";
import { HighlightedRequest } from "./RequestHighlight";

type RequestLocator = { request_offset: number; query?: string };

const ROW = "min-w-0 rounded-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring";
/** Under a line with an icon: the icon column's width and its gap, so what hangs from it lines up with its words. */
const LANE = "pl-[calc(var(--size-icon-sm)+var(--spacing-xs))]";
/** The command's own name, which no language translates. */
const GOAL_MARK = "/goal";

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
      <Work key={`work:${workKey(shown.session)}`} record={shown} summaries={summaries} onOpen={open} actions={actions} locator={workKey(shown.session) === workKey(target) ? frame.locator : undefined} />
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
      : <NowBox record={record} summaries={summariesEnabled(rest?.status?.background_ai)} checkout={place?.checkout ?? null} agent={place?.agent ?? null} line={root ? cardLine(root, rows) : null} question={root ? cardQuestion(root, rows) : null} actions={actions} />}
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

/**
 * Native state and the action for this session: visit its pane or restore
 * its conversation. While the agent neither works nor asks, the state is the
 * record's last real turn, a usage limit skipped and shown as a badge
 * (D-10, B10); a pane drawing that limit as a block defers to the record.
 */
function NowBox({ record, summaries, checkout, agent, line, question, actions }: { record: RecordValue; summaries: boolean; checkout: Checkout | null; agent: AgentRow | null; line: ReturnType<typeof cardLine> | null; question: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const live: "question" | "blocked" | "working" | "ended" = line?.kind === "needs_you" || agent?.status_code === "question" || agent?.status_code === "approval" ? "question" : line?.kind === "blocked" ? "blocked" : agent?.status_code === "working" ? "working" : "ended";
  const status = summaries ? record.status : null;
  const recorded = status && (live === "ended" || (live === "blocked" && status.limited !== null)) ? status : null;
  const kind = recorded ? (recorded.end === "question" ? "question" : recorded.end === "blocked" || recorded.end === "limited" ? "blocked" : recorded.end === "working" ? "working" : "ended") : live;
  // The current flow's pull requests, all settled after that turn, stand for its line (fact-status B2).
  const settled = recorded?.settled ?? null;
  const since = settled ? settled.at_unix_ms : recorded ? recorded.at_unix_ms : kind === "ended" ? record.ended_at_unix_ms ?? agent?.changed_at_unix_ms ?? null : agent?.changed_at_unix_ms ?? null;
  const word = settled ? t(settled.state === "merged" ? "record.merged" : "record.closed") : recorded ? t(`requests.end.${recorded.end}`) : kind === "working" ? t("agents.status.working") : kind === "question" ? t("agents.status.question") : kind === "blocked" ? t("agents.status.error") : agent && agent.status_code !== "done" ? statusText(t, agent.status_code) : t("sessionWork.done");
  // While it works on a plan, the line is its step, as on the pane's second row (B7).
  const livePlan = useShellStore((s) => planOfSession(s.rest, record.session)) ?? record.plan ?? null;
  const step = kind === "working" && !settled && livePlan ? planWorkLine(livePlan) : null;
  const said = settled ? settledPrs(t, settled) : recorded ? recorded.line || null : kind === "question" ? question ?? agent?.state.ask?.what ?? null : kind === "blocked" ? (line?.kind === "blocked" ? line.what : null) ?? lastWords(agent) : lastWords(agent);
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
        {status && status.limited !== null ? <span className="min-w-0 max-w-full truncate rounded-xs bg-destructive/10 px-xs text-micro font-normal text-destructive" title={status.limited || t("record.limited")} aria-label={status.limited ? `${t("record.limited")} · ${status.limited}` : t("record.limited")} data-record-limited="true">{status.limited || t("record.limited")}</span> : null}
        <span className="rounded-xs border border-border px-xs text-micro font-normal text-muted-foreground" data-record-pane={pane ? "open" : "closed"}>{t(pane ? "recordCard.paneOpen" : "recordCard.paneClosed")}</span>
      </p>
      {button}
    </div>
    {step ? <p className="whitespace-normal text-caption text-agent-working [overflow-wrap:anywhere]" title={step} data-record-now-step="true">{step}</p> : null}
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

/** The session's own plan (PRD hide-plan B9, B21): its count, `끝남` once every step is done or dropped, and the map. */
function RecordPlan({ plan }: { plan: PlanView }) {
  const { t } = useInterfaceTranslation();
  return <section className="mt-lg min-w-0" data-record-plan={plan.progress.finished ? "finished" : plan.review}>
    <SectionHead label={t("plan.title")} count={planCount(plan)} summary={plan.progress.finished ? t("plan.finishedWord") : null} />
    <div className="min-w-0 overflow-x-auto"><PlanGraph plan={plan} size="compact" /></div>
  </section>;
}

/**
 * The record below its head (PRD session-record-goal-flow B2): what waits on
 * the operator, the goal flows, the pull requests and other outputs, then the
 * sessions that worked with it. With summaries off it is the raw request
 * list and the pull requests (B28).
 */
function Work({ record, summaries, onOpen, actions, locator }: { record: RecordValue; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  return <>
    {summaries ? <Asks record={record} actions={actions} /> : null}
    {summaries ? <Flows record={record} locator={locator} /> : <RawRequests record={record} locator={locator} />}
    {record.plan ? <RecordPlan plan={record.plan} /> : null}
    <Prs record={record} actions={actions} />
    {summaries && record.artifacts.length ? <section className="mt-lg min-w-0" data-record-artifacts={record.artifacts.length}>
      <SectionHead label={t("record.artifacts")} count={record.artifacts.length} />
      <ul className="flex min-w-0 flex-wrap gap-xs">
        {record.artifacts.map((name) => <li key={name} className="min-w-0 max-w-full truncate rounded-xs border border-border px-xs py-xxs text-micro text-subtle-foreground" title={name} data-record-artifact={name}>{name}</li>)}
      </ul>
    </section> : null}
    {record.children.length ? <section className="mt-lg min-w-0" data-record-delegated={record.children.length}>
      <SectionHead label={t("recordCard.children")} count={record.children.length} />
      <ChildRows parent={record} summaries={summaries} onOpen={onOpen} actions={actions} />
    </section> : null}
  </>;
}

function RequestTime({ at, full = false }: { at: number; full?: boolean }) {
  const { i18n } = useInterfaceTranslation();
  return <time className="shrink-0 font-mono text-micro text-muted-foreground" dateTime={new Date(at).toISOString()}>{full ? `${recordDate(at)} ` : null}{formatDateTime(requireInterfaceLanguage(i18n.language), at, { hour: "2-digit", minute: "2-digit", hour12: false })}</time>;
}

/**
 * 나를 기다리는 것 (D-12, B11, B12): the agent's open questions and decisions,
 * each with 넘기기; a line put away keeps its place with 되돌리기 while this
 * record stays open, and leaves that state when the core lists it as open
 * again after it was gone. No line at all when nothing waits.
 */
function Asks({ record, actions }: { record: RecordValue; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // `gone` is set once the core no longer lists the line; listed again after that, it is open again.
  const [away, setAway] = useState<readonly { ask: MissedWork; gone: boolean }[]>([]);
  useEffect(() => {
    const open = new Set(record.asks.map((ask) => ask.id));
    setAway((held) => {
      const next = held.filter((one) => !(one.gone && open.has(one.ask.id))).map((one) => (one.gone || open.has(one.ask.id) ? one : { ...one, gone: true }));
      return next.length === held.length && next.every((one, index) => one === held[index]) ? held : next;
    });
  }, [record.asks]);
  const lines = [...record.asks, ...away.map((one) => one.ask).filter((ask) => !record.asks.some((open) => open.id === ask.id))].sort((a, b) => a.at_unix_ms - b.at_unix_ms);
  if (!lines.length) return null;
  const small = "h-auto shrink-0 px-xs py-0 text-micro";
  return <section className="mt-lg min-w-0" aria-label={t("record.waiting")} data-record-asks={record.asks.length}>
    <SectionHead label={t("record.waiting")} count={record.asks.length} />
    <ul className="min-w-0">
      {lines.map((ask) => {
        const dismissed = away.some((one) => one.ask.id === ask.id);
        return <li key={ask.id} className="flex min-w-0 items-start gap-xs border-b border-border py-xs last:border-b-0" data-record-ask={ask.id} data-dismissed={dismissed ? "true" : undefined}>
          <CircleHelpIcon aria-hidden="true" className={cn("mt-xxs size-(--size-icon-sm) shrink-0", dismissed ? "text-muted-foreground" : "text-warning")} />
          <span className={cn("min-w-0 flex-1 whitespace-normal [overflow-wrap:anywhere]", dismissed ? "text-muted-foreground line-through" : "text-foreground")} title={ask.text}>{ask.text}</span>
          {dismissed ? (
            <Button size="sm" variant="ghost" className={small} onClick={() => { actions.setMissedStatus(record.session, ask.id, "open"); setAway((held) => held.filter((one) => one.ask.id !== ask.id)); }} data-record-ask-undo={ask.id}>{t("backlog.panel.undo")}</Button>
          ) : (
            <Button size="sm" variant="ghost" className={small} onClick={() => { actions.setMissedStatus(record.session, ask.id, "dismissed"); setAway((held) => [...held, { ask, gone: false }]); }} data-record-ask-dismiss={ask.id}>{t("sessionWork.dismiss")}</Button>
          )}
        </li>;
      })}
    </ul>
  </section>;
}

/** A recorded turn's end word; a usage limit takes the block's look. */
function EndMark({ end }: { end: WorkEnd }) {
  const { t } = useInterfaceTranslation();
  const tone = end === "done" ? "text-success" : end === "question" ? "text-warning" : end === "blocked" || end === "limited" ? "text-destructive" : end === "working" ? "text-agent-working" : "text-muted-foreground";
  return <span className={cn("shrink-0 text-micro font-medium", tone)} data-record-end={end}>{t(`requests.end.${end}`)}</span>;
}

/** `#n` for one pull request, `PR n개` for several (fact-status B1). */
function settledPrs(t: ReturnType<typeof useInterfaceTranslation>["t"], settled: PrSettlement): string {
  return settled.numbers.length === 1 ? `#${settled.numbers[0]}` : t("record.prCount", { count: settled.numbers.length });
}

/**
 * A past flow's result (fact-status B1-B5): Hide's PR fact with its time, a
 * question or wait it moved on from, the line its closing label gave, or the
 * end of its last turn.
 */
function FlowResultLine({ result }: { result: FlowResult }) {
  const { t } = useInterfaceTranslation();
  const line = (text: string) => text ? <span className="min-w-0 truncate text-micro text-subtle-foreground" title={text}>{text}</span> : null;
  return <span className="mt-xxs flex min-w-0 items-baseline gap-xs" data-record-flow-result={result.kind}>
    {result.kind === "settled" ? <>
      <span className={cn("shrink-0 text-micro font-medium", result.state === "merged" ? "text-pr-merged" : "text-pr-closed")} data-record-end={result.state}>{t(result.state === "merged" ? "record.merged" : "record.closed")}</span>
      <span className="min-w-0 truncate text-micro text-subtle-foreground">{settledPrs(t, result)}</span>
      <RequestTime at={result.at_unix_ms} />
    </> : result.kind === "moved_on" ? <>
      <span className="shrink-0 text-micro font-medium text-muted-foreground" data-record-end="moved_on">{t("record.movedOn")}</span>
      {line(result.line)}
    </> : result.kind === "closing" ? line(result.line) : <>
      <EndMark end={result.end} />
      {line(result.line)}
    </>}
  </span>;
}

/**
 * The goal flows (D-03, B3, B13): each with its name, when it began and how
 * many requests it holds. The current one is open; a past one is folded to
 * its result and opens on a click. A History hit opens the flow that holds it.
 */
function Flows({ record, locator }: { record: RecordValue; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  const current = record.flows.at(-1)?.id ?? null;
  const located = locator ? record.flows.find((flow) => flow.requests.some((request) => holdsRequest(request, locator.request_offset)))?.id ?? null : null;
  // Only a click overrides the default; a new current flow opens and the one it follows folds.
  const [toggled, setToggled] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  useEffect(() => { if (located) setToggled((ids) => { const next = new Map(ids); next.delete(located); return next; }); }, [located, locator?.query]);
  const isOpen = (id: string) => toggled.get(id) ?? (id === current || id === located);
  const toggle = (id: string) => setToggled((ids) => new Map(ids).set(id, !isOpen(id)));
  return <section className="mt-lg min-w-0" data-record-flows={record.flows.length}>
    <LocatorNotes locator={locator} located={located !== null} />
    {!record.flows.length ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
    <ol className="min-w-0">
      {record.flows.map((flow) => {
        const expanded = isOpen(flow.id);
        const isCurrent = flow.id === current;
        return <li key={flow.id} className="min-w-0 border-b border-border last:border-b-0" data-record-flow={flow.id} data-current={isCurrent ? "true" : undefined}>
          <button type="button" className={cn(ROW, "flex w-full items-start gap-xs py-sm hover:bg-accent")} aria-expanded={expanded} onClick={() => toggle(flow.id)} data-record-flow-toggle={flow.id}>
            <ChevronRightIcon aria-hidden="true" className={cn("mt-xxs size-(--size-icon-sm) shrink-0 text-muted-foreground", expanded && "rotate-90")} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-caption font-semibold text-foreground" title={flow.name} data-record-flow-name="true">{flow.name}</span>
              <span className="mt-xxs flex min-w-0 flex-wrap items-center gap-x-xs text-micro text-muted-foreground">
                <RequestTime at={flow.at_unix_ms} /><span>·</span><span>{isCurrent ? t("record.flowNow") : t("record.flowRequests", { count: flow.request_count })}</span>
              </span>
              {!isCurrent && flow.result ? <FlowResultLine result={flow.result} /> : null}
            </span>
          </button>
          {expanded ? <ol className={cn("min-w-0 pb-sm", LANE)} data-record-flow-requests={flow.requests.length}>
            {requestRuns(flow.requests).map((run) => run.kind === "request"
              ? <RequestRow key={run.request.offset} request={run.request} locator={locator} />
              : <LetterRun key={run.requests[0]!.offset} letters={run.requests} locator={locator} />)}
          </ol> : null}
        </li>;
      })}
    </ol>
    {record.omitted_requests ? <p className="pt-sm text-micro text-muted-foreground" data-record-omitted={record.omitted_requests}>{t("record.omitted", { count: record.omitted_requests })}</p> : null}
  </section>;
}

/** With summaries off (B28): every request in time order, its words as sent. */
function RawRequests({ record, locator }: { record: RecordValue; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  const requests = useMemo(() => record.flows.flatMap((flow) => flow.requests).sort((a, b) => a.at_unix_ms - b.at_unix_ms || a.offset - b.offset), [record.flows]);
  const located = locator !== undefined && requests.some((request) => holdsRequest(request, locator.request_offset));
  return <section className="mt-lg min-w-0" data-record-raw-requests={requests.length}>
    <SectionHead label={t("sessionWork.requests")} count={requests.length} summary={t("recordCard.aiOff")} />
    <LocatorNotes locator={locator} located={located} />
    {!requests.length ? <p className="text-micro text-muted-foreground">{t("sessionWork.empty")}</p> : null}
    <ol className="min-w-0">{requests.map((request) => <RequestRow key={request.offset} request={{ ...request, summary: null, answers: null }} locator={locator} />)}</ol>
    {record.omitted_requests ? <p className="pt-sm text-micro text-muted-foreground" data-record-omitted={record.omitted_requests}>{t("record.omitted", { count: record.omitted_requests })}</p> : null}
  </section>;
}

/**
 * Letters in a row (B24): one dim line with their count and senders, which
 * opens to each letter's own line. A History hit inside opens it.
 */
function LetterRun({ letters, locator }: { letters: readonly WorkRequest[]; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  const holds = locator !== undefined && letters.some((letter) => holdsRequest(letter, locator.request_offset));
  const [open, setOpen] = useState(holds);
  useEffect(() => { if (holds) setOpen(true); }, [holds, locator?.query]);
  const senders = [...new Set(letters.map((letter) => letter.letter_from!))];
  const count = letters.reduce((total, letter) => total + letter.count, 0);
  return <li className="min-w-0 py-xxs" data-record-letter-run={letters.length}>
    <button type="button" className={cn(ROW, "flex w-full min-w-0 items-baseline gap-sm py-xxs text-muted-foreground hover:bg-accent")} aria-expanded={open} onClick={() => setOpen(!open)}>
      <RequestTime at={letters[0]!.at_unix_ms} />
      <span className="flex min-w-0 flex-1 items-baseline gap-xs">
        <MailIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 self-center" />
        <span className="shrink-0 text-micro">{t("record.letterRun", { count })}</span>
        <span className="min-w-0 flex-1 truncate text-micro" title={senders.join(", ")}>{senders.join(", ")}</span>
      </span>
    </button>
    {open ? <ol className="min-w-0 pl-(--size-control)">{letters.map((letter) => <RequestRow key={letter.offset} request={letter} locator={locator} />)}</ol> : null}
  </li>;
}

/** A History hit's notes: how long requests are kept, and that this one is gone. */
function LocatorNotes({ locator, located }: { locator?: RequestLocator; located: boolean }) {
  const { t } = useInterfaceTranslation();
  if (!locator) return null;
  return <>
    <Hint label={t("overviewHistory.limit")}><button type="button" className="inline-flex items-center gap-xs rounded-xs py-xs text-micro text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-record-retention="true"><InfoIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("overviewHistory.limitLabel")}</button></Hint>
    {!located ? <p role="status" className="py-xs text-caption text-muted-foreground" data-record-request-unavailable="true">{t("overviewHistory.requestUnavailable")}</p> : null}
  </>;
}

/**
 * One request (D-06, D-07, D-18; B5, B7, B23-B25): its summary, or its first
 * words, and a click opens the words as sent in place. A reply names the
 * question it answers after `↩`; a letter is a dim line under its sender; a
 * repeat shows its count, an attachment its image count, a `/goal` its mark.
 */
function RequestRow({ request, locator }: { request: WorkRequest; locator?: RequestLocator }) {
  const { t } = useInterfaceTranslation();
  const matched = locator !== undefined && holdsRequest(request, locator.request_offset);
  const [raw, setRaw] = useState(matched);
  const target = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (!matched) return;
    setRaw(true);
    target.current?.focus({ preventScroll: true });
    target.current?.scrollIntoView({ block: "center" });
  }, [matched, locator?.query]);
  const preview = requestPreview(request);
  const letter = request.letter_from;
  // A History hit always shows the words it matched, marked.
  const opens = request.text.trim() !== preview.text || preview.cut || matched;
  return <li ref={target} tabIndex={matched ? -1 : undefined} className={cn("min-w-0 py-xxs", matched && "rounded-xs bg-accent/40 px-xs outline-none focus:ring-1 focus:ring-ring")} data-record-request={request.offset} data-record-request-match={matched ? "true" : undefined} data-record-letter={letter ?? undefined}>
    <button type="button" className={cn(ROW, "flex w-full min-w-0 items-baseline gap-sm py-xxs", opens && "hover:bg-accent")} aria-expanded={opens ? raw : undefined} disabled={!opens} onClick={() => setRaw(!raw)}>
      <RequestTime at={request.at_unix_ms} />
      <span className={cn("flex min-w-0 flex-1 items-baseline gap-xs", letter ? "text-muted-foreground" : "text-foreground")}>
        {letter ? <span className="inline-flex shrink-0 items-center gap-xxs text-micro" aria-label={t("record.letterFrom", { name: letter })}><MailIcon aria-hidden="true" className="size-(--size-icon-sm) self-center" />{letter}</span> : null}
        {request.goal_command ? <span className="shrink-0 rounded-xs border border-border px-xxs font-mono text-micro text-muted-foreground" aria-label={t("record.goalCommand")} data-record-goal="true">{GOAL_MARK}</span> : null}
        {request.answers ? <span className="min-w-0 shrink truncate text-micro text-muted-foreground" aria-label={t("record.answers", { question: request.answers })} data-record-answers="true">↩ {request.answers}:</span> : null}
        <span className={cn("min-w-0 flex-1 truncate", !preview.summarized && !letter && "text-subtle-foreground")} title={preview.text}>{preview.text}</span>
        {request.images ? <span className="inline-flex shrink-0 items-center gap-xxs text-micro text-muted-foreground" data-record-images={request.images}><ImageIcon aria-hidden="true" className="size-(--size-icon-sm) self-center" />{t("record.images", { count: request.images })}</span> : null}
        {request.count > 1 ? <span className="shrink-0 font-mono text-micro text-muted-foreground" aria-label={t("record.repeated", { count: request.count })} data-record-repeat={request.count}>×{request.count}</span> : null}
      </span>
    </button>
    {raw && opens ? <p className="whitespace-pre-wrap pb-xs pl-(--size-control) text-caption text-subtle-foreground [overflow-wrap:anywhere]" data-record-request-raw="true">{matched ? <HighlightedRequest text={request.text} query={locator?.query ?? ""} /> : request.text}</p> : null}
  </li>;
}

/**
 * The pull requests (D-14, B18, B19): this session's first, then one group
 * per direct child holding its whole subtree's. Open and failing ones show;
 * merged and closed ones fold into one count line that opens them.
 */
function Prs({ record, actions }: { record: RecordValue; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [unfolded, setUnfolded] = useState(false);
  const all = record.pr_groups.flatMap((group) => group.outputs);
  const settled = all.filter(settledOutput);
  const allMerged = all.length > 0 && all.every((output) => output.state === "merged");
  const rows = (items: readonly WorkOutput[]) => items.map((output) => <div key={`${output.kind}:${output.repository}:${output.number}`} className="flex min-w-0 items-start gap-sm py-xs" data-record-pr-row={output.number}><OutputMark output={output} actions={actions} /><button type="button" className={cn(ROW, "min-w-0 flex-1 truncate text-caption hover:underline", output.stale && "opacity-(--opacity-dimmed)")} onClick={() => actions.openRecordedOutput(output)} title={[output.title, staleHint(output, t("common.unavailable"))].filter(Boolean).join("\n")}>{output.title}</button></div>);
  return <section className="mt-lg min-w-0" data-record-prs={all.length}>
    <SectionHead label={t("recordCard.prs")} count={all.length} summary={allMerged ? t("record.allMerged") : null} />
    {!all.length ? <p className="py-xs text-micro text-muted-foreground">{t("recordCard.noPrs")}</p> : null}
    {record.pr_groups.map((group) => {
      const shown = unfolded ? group.outputs : group.outputs.filter((output) => !settledOutput(output));
      // A group whose PRs all sit in the fold draws no empty name line.
      if (!shown.length) return null;
      return <div key={group.child ? workKey(group.child) : "own"} className="min-w-0" data-record-pr-group={group.child ? group.child.session_id : "own"}>
        {record.pr_groups.length > 1 || group.child ? <p className="truncate pt-xs text-micro text-muted-foreground" title={group.name}>{group.child ? group.name : t("record.thisSession")}</p> : null}
        {rows(orderedOutputs(shown))}
      </div>;
    })}
    {settled.length && !unfolded ? <button type="button" className={cn(ROW, "flex items-center gap-xxs py-xs text-micro text-muted-foreground hover:text-foreground")} aria-expanded="false" onClick={() => setUnfolded(true)} data-record-settled={settled.length}>{t("record.settledPrs", { count: settled.length })}<ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" /></button> : null}
  </section>;
}

/**
 * The sessions that worked with this one (B20, B26): each with its job's
 * name and its state now, from its live pane, else its last real turn. One
 * opens at a time to its own flows and pull requests.
 */
const FIRST_CHILDREN = 5;
function ChildRows({ parent, summaries, onOpen, actions }: { parent: RecordValue; summaries: boolean; onOpen: (child: RecordValue) => void; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const [visible, setVisible] = useState(FIRST_CHILDREN);
  const [expanded, setExpanded] = useState<string | null>(null);
  const rest = useShellStore((state) => state.rest);
  const agents = useShellStore((state) => state.agents);
  const rows = useMemo(() => [...everyAgent(rest, agents).values()], [rest, agents]);
  return <ol className="min-w-0" data-record-children={parent.children.length}>
    {parent.children.slice(0, visible).map((child) => {
      const key = workKey(child.session);
      const live = rows.find((row) => { const session = sessionOfAgentOn(row, localDeviceId(rest)); return session && workKey(session) === key; }) ?? null;
      const held = live ? null : archivedMemberOf(rest, child.session);
      const away = held ? held.row.members[held.index]!.resume : null;
      const end = summaries ? child.status?.end ?? null : null;
      // A stopped child's pull requests that settled after its last turn stand for it, as on the parent's top state (fact-status B2).
      const settled = !live && !away && summaries ? child.status?.settled ?? null : null;
      const done = live ? live.status_code === "done" : !away && (settled ? settled.state === "merged" : end === "done");
      const state = away && away !== "resumed" ? archivedWord(t, away) : live ? statusText(t, live.status_code) : settled ? t(settled.state === "merged" ? "record.merged" : "record.closed") : end ? t(`requests.end.${end}`) : t("common.unavailable");
      const title = recordTitle(child, summaries);
      const prs = child.pr_groups.reduce((count, group) => count + group.outputs.length, 0);
      const branch = live ? branchChip(live) : child.cwd && child.cwd !== parent.cwd ? folderName(child.cwd) : null;
      const device = child.session.device_id !== parent.session.device_id ? machineName(rest?.navigator?.devices, child.session.device_id, t) : null;
      return <li key={key} className="min-w-0 border-b border-border last:border-b-0" data-record-child={child.session.session_id}>
        <button type="button" className={cn(ROW, "flex w-full items-start gap-sm py-sm hover:bg-accent")} aria-expanded={expanded === key} title={[branch, device].filter(Boolean).join(" · ")} onClick={() => setExpanded(expanded === key ? null : key)}>
          {done ? <CheckIcon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-success" /> : live ? <StatusMark mark={live.mark} className={cn("mt-xs shrink-0", markTone(live))} /> : end === "waiting" && !settled ? <Clock3Icon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-muted-foreground" /> : <SquareIcon aria-hidden="true" className="mt-xs size-(--size-agent-mark) shrink-0 text-muted-foreground" />}
          <span className="min-w-0 flex-1"><span className={cn("block truncate text-caption font-medium", done && "text-subtle-foreground")} title={title}>{title}</span><span className="mt-xs flex min-w-0 flex-wrap items-center gap-x-xs gap-y-xxs text-micro text-muted-foreground"><AgentMark kind={child.session.agent} />{agentProductName(child.session.agent)} · <span data-record-child-state="true">{state}</span>{prs ? <span>· {t("recordCard.prs")} {prs}</span> : null}</span></span>
          <ChevronRightIcon aria-hidden="true" className={cn("mt-xs size-(--size-icon-sm) shrink-0 text-muted-foreground", expanded === key && "rotate-90")} />
        </button>
        {expanded === key ? <div className="mb-sm ml-md min-w-0 border-l-2 border-border bg-card px-sm pb-sm" data-record-child-detail={child.session.session_id}>
          <div className="flex min-w-0 flex-wrap gap-xs pt-sm"><Button size="sm" variant="ghost" onClick={() => onOpen(child)}>{t("sessionWork.openRecord")}<ChevronRightIcon aria-hidden="true" /></Button>{live ? <Button size="sm" variant="ghost" onClick={() => actions.openAgent(live.pane_id)}><SquareTerminalIcon aria-hidden="true" />{t("record.toPane")}</Button> : null}</div>
          {summaries ? <Flows record={child} /> : <RawRequests record={child} />}
          <Prs record={child} actions={actions} />
        </div> : null}
      </li>;
    })}
    {parent.children.length > visible ? <li><Button size="sm" variant="ghost" onClick={() => setVisible((count) => count + FIRST_CHILDREN)}>{t("sessionWork.moreChildren", { count: parent.children.length - visible })}</Button></li> : null}
  </ol>;
}

/**
 * An open problem or follow-up (PRD overview-device-tasks B43-B46; PRD
 * session-record-goal-flow D-13): one warning line with `백로그로 · 무시` at its
 * end. `백로그로` makes the item at once in the project the session works in
 * and shows `만드는 중` meanwhile; the line then leaves, as one dismissed does.
 * A failure keeps the line with its reason and a retry that converges on one
 * item.
 */
function MissedLine({ missed, session, cwd, actions, className }: { missed: MissedWork; session: WorkSession; cwd: string | null; actions: Actions; className?: string }) {
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
    <span className="min-w-0 flex-1 basis-1/2 whitespace-normal text-foreground [overflow-wrap:anywhere]" title={missed.text}>{missed.text}</span>
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
    <MissedLine missed={missed} session={record.session} cwd={record.cwd} actions={actions} className={LANE} />
    <button type="button" className={cn(ROW, "mt-xxs block w-full truncate text-micro text-muted-foreground", LANE)} title={record.title} onClick={() => actions.openSessionRecord(record.session)}>{record.title} · {t(`sessionWork.kind.${missed.kind}`)}</button>
  </li>;
}
