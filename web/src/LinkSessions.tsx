import { CopyIcon, LoaderCircleIcon, MessageSquareIcon, PlayIcon, CircleHelpIcon, SquareTerminalIcon, ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentMark } from "./AgentMark";
import { agentProductName } from "./agentAdapters";
import { markTone, rowLine } from "./agentRow";
import { statusText } from "./agentStatus";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { Elapsed } from "./components/elapsed";
import { MilestoneMark, SessionRow } from "./components/session-row";
import { StatusMark } from "./components/status-mark";
import { summariesEnabled } from "./hideAi";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { deviceLabel, failureKey, foldTimeline, resumeProvider, resumeBlock, resumeCheckout, sessionLines, spanText, timeText, viewBlock, type Blocked, type SessionLine } from "./linkPanel";
import { allAgents } from "./navigation";
import { timeline, type Milestone, type TimelineEntry } from "./prCardModel";
import { recordTitle } from "./sessionWork";
import { catalogOf, localDeviceId, type LinkPanel, type PullRequest, type Workspace } from "./snapshot";
import { startAnswer, startRequestId } from "./startAnswer";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

// The PR card's timeline (PRD link-graph B6-B25, B34; PRD pr-card-and-
// session-record-redesign D-03, D-05 to D-09, B3-B10): the record's session
// lines joined with what is live now and the pull request's milestones, one
// list oldest first, each row in the sidebar's agent-row shape, joined by one
// rail behind the marks (PRD record-card-readability D-09). A live row
// keeps the one button its state needs under its line; an ended row shows
// `View conversation` and `Resume` only while the pointer or the keyboard is
// on it. The record's failure, its first read and its empty line stand in
// the section's own place; the rest of the card never moves.

/**
 * The rail (PRD record-card-readability D-09, B16): one line behind the
 * marks, from the first row's mark to the last's, so the sessions and the
 * milestones read as one run. Each mark stands on the page's ground, so the
 * line joins the marks rather than crossing them.
 */
const RAIL = "before:absolute before:top-(--size-sidebar-line) before:bottom-(--size-sidebar-line) before:left-[calc(var(--spacing-xs)+var(--size-agent-mark)/2)] before:w-(--size-hairline) before:bg-border";
/** A timeline row's mark on the page's ground, over the rail. */
const ON_RAIL = "relative inline-flex rounded-full bg-background";

/** How long a resume waits for the core's answer before it says so; a device start can take a Home sync and a tab. */
const RESUME_TIMEOUT_MS = 90_000;

export function LinkTimeline({
  panel,
  project,
  pr,
  milestones,
  branchOf,
  onRetry,
  actions,
}: {
  panel: LinkPanel;
  project: Workspace;
  /** The list's row for the pull request, or none while it is unlisted (then only sessions are drawn). */
  pr: PullRequest | null;
  milestones: readonly Milestone[];
  /** The branch of a pull request the line belongs to, for where it resumes when its folder is gone. */
  branchOf: (pr: number) => string | null;
  onRetry: () => void;
  actions: Actions;
}) {
  const { t } = useInterfaceTranslation();
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const navigatorAgents = useShellStore((s) => s.rest?.navigator?.agents);
  const agentScope = useShellStore((s) => s.rest?.navigator?.agent_scope);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const filling = useShellStore((s) => s.linkSummaries?.filling === true);
  const [unfolded, setUnfolded] = useState(false);
  const agents = useMemo(() => allAgents(remote, navigatorAgents ?? [], agentScope, devices).map((entry) => entry.agent), [remote, navigatorAgents, agentScope, devices]);
  const lines = useMemo(() => sessionLines(panel.sessions, agents), [panel.sessions, agents]);
  const entries = useMemo<TimelineEntry<SessionLine>[]>(() => {
    const sessions = lines.map((session) => ({ session, at: session.line.started_at_unix_ms }));
    return pr ? timeline(sessions, milestones, pr) : sessions.map(({ session, at }) => ({ kind: "session", session, at }));
  }, [lines, milestones, pr]);
  const { shown, earlier } = foldTimeline(entries, (entry) => entry.kind === "session", (entry) => entry.kind === "session" && entry.session.live !== null, unfolded);
  const finding = panel.loading || filling;
  const empty = lines.length === 0 && !panel.failure && !panel.loading;
  return (
    <div className="flex flex-col gap-xs" data-link-sessions={panel.total} data-link-loading={finding ? "true" : undefined}>
      {finding ? (
        <p role="status" className="flex items-center gap-xs text-caption text-muted-foreground" data-link-finding="true">
          <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />
          {t("links.finding")}
        </p>
      ) : null}
      {panel.failure ? (
        <p role="alert" className="flex items-center gap-sm text-caption text-warning" data-link-failure={panel.failure}>
          {t(failureKey(panel.failure))}
          <Button variant="ghost" size="sm" onClick={onRetry} data-link-retry="true">
            {t("common.retry")}
          </Button>
        </p>
      ) : null}
      {earlier > 0 ? (
        <button
          type="button"
          aria-expanded={unfolded}
          className="inline-flex items-center gap-xs self-start rounded-xs text-caption text-subtle-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
          onClick={() => setUnfolded(!unfolded)}
          data-link-earlier={earlier}
        >
          {unfolded ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
          {t("links.earlier", { count: earlier })}
        </button>
      ) : null}
      {empty ? (
        <p className="flex min-w-0 flex-wrap items-center gap-sm text-caption text-muted-foreground" data-link-none="true">
          {t("links.noSessions")}
          {pr && pr.state !== "merged" && pr.state !== "closed" ? (
            <Button variant="secondary" size="sm" onClick={() => useUiStore.getState().setWorkspaceDialog({ kind: "pr_delegate", workspaceId: project.id, prNumber: pr.number })} data-link-delegate={pr.number}>
              <PlayIcon aria-hidden="true" />
              {t("prList.delegate")}
            </Button>
          ) : null}
        </p>
      ) : null}
      {shown.length > 0 ? (
        <ol className={cn("relative flex flex-col", RAIL)} aria-label={t("work.card.timeline")} data-pr-card-timeline-rows={shown.length}>
          {shown.map((entry) => {
            if (entry.kind === "session") return <SessionLineView key={entry.session.line.id} entry={entry.session} project={project} checkoutBranch={branchOf(entry.session.line.pr)} opener={lines.length > 1} actions={actions} />;
            if (entry.kind === "milestone") return <MilestoneRow key={`${entry.milestone.kind}:${entry.milestone.at ?? ""}`} milestone={entry.milestone} />;
            return <NowRow key="now" running={entry.running} />;
          })}
        </ol>
      ) : null}
    </div>
  );
}

/** A milestone's word and colour (B5): what happened, in the colour the PR marks use for it. */
function milestoneLook(milestone: Milestone, t: ReturnType<typeof useInterfaceTranslation>["t"]): { text: string; tone: string } {
  switch (milestone.kind) {
    case "opened":
      return { text: t("work.card.milestone.opened"), tone: "text-subtle-foreground" };
    case "checks_passed":
      return { text: t("work.card.milestone.checksPassed"), tone: "text-pr-mergeable" };
    case "checks_failed":
      return { text: t("work.card.milestone.checksFailed"), tone: "text-pr-failed" };
    case "review_requested":
      return { text: t("work.card.milestone.reviewRequested", { who: milestone.who.join(", ") }), tone: "text-warning" };
    case "approved":
      return { text: t("work.card.milestone.approved"), tone: "text-pr-mergeable" };
    case "changes":
      return { text: t("work.card.milestone.changes"), tone: "text-pr-failed" };
    case "merged":
      return { text: t("work.card.milestone.merged"), tone: "text-pr-merged" };
  }
}

/** One milestone of the pull request (D-05, B5): the square in its colour, the word, the time at the right. */
function MilestoneRow({ milestone }: { milestone: Milestone }) {
  const { t } = useInterfaceTranslation();
  const look = milestoneLook(milestone, t);
  return (
    <SessionRow mark={<span className={ON_RAIL}><MilestoneMark className={look.tone} /></span>} title={look.text} right={milestone.at !== null ? <span className="font-mono text-caption text-muted-foreground">{timeText(milestone.at)}</span> : null} data-timeline-milestone={milestone.kind} data-timeline-at={milestone.at ?? undefined} />
  );
}

/** `지금` at the end of an open pull request (B6; PRD record-card-readability D-12, B21): one line, `지금 · 검사 N개 도는 중` with the dashed ring of proven work it waits on while checks run (status-marks D-06, D-25), otherwise `지금` and an empty slot. */
function NowRow({ running }: { running: number }) {
  const { t } = useInterfaceTranslation();
  return <SessionRow mark={<span className={ON_RAIL}><StatusMark mark={running > 0 ? "waiting" : null} /></span>} title={running > 0 ? `${t("work.card.now")} · ${t("work.card.why.ci", { count: running })}` : t("work.card.now")} data-timeline-now={running} />;
}

/** A resume in flight: the request it sent, then the core's refusal if it came. */
type Resume = { request: string | null; failure: string | null };

/**
 * Follows one resume to its answer (B12, B14): a tab that opened moves the
 * screen to its pane, a refusal or a failure stays on the line with Retry.
 */
function useResume(): { resume: Resume; start: (send: (request: string) => void) => void } {
  const { t } = useInterfaceTranslation();
  const [resume, setResume] = useState<Resume>({ request: null, failure: null });
  const request = resume.request;
  const phase = useShellStore((s) => (request ? startAnswer(s.rest, request).phase : null));
  useEffect(() => {
    if (!request || !phase || phase === "pending") return;
    const answer = startAnswer(useShellStore.getState().rest, request);
    if (answer.phase === "ready") {
      const ui = useUiStore.getState();
      if (answer.agentPhase) ui.setWatchedTask(answer.taskId);
      if (answer.paneId) ui.setFocusWhenListed(answer.paneId);
      setResume({ request: null, failure: answer.agentPhase === "failed" ? (answer.agentMessage ?? "") : null });
    } else if (answer.phase === "refused" || answer.phase === "failed") {
      setResume({ request: null, failure: answer.message ?? "" });
    }
  }, [request, phase]);
  useEffect(() => {
    if (!request) return undefined;
    const timer = window.setTimeout(() => setResume({ request: null, failure: t("shell.startTimeout") }), RESUME_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [request, t]);
  return {
    resume,
    start: (send) => {
      const id = startRequestId();
      setResume({ request: id, failure: null });
      send(id);
    },
  };
}

/**
 * The title of a session row (B7): the live agent's label, else the session
 * record's title as the Sessions list shows it, else the agent's product
 * name. The request stays in the tooltip.
 */
function useSessionTitle(line: SessionLine["line"], live: SessionLine["live"], workspaceId: string): string {
  const summaries = useShellStore((s) => summariesEnabled(s.rest?.status?.background_ai));
  const work = useShellStore((s) => s.linkSummaries?.projects[workspaceId]?.work);
  if (live) return live.agent.identity_label;
  const ids = new Set([line.id, ...line.ids]);
  const row = work?.find((summary) => ids.has(summary.session.session_id));
  return row ? recordTitle(row, summaries) : agentProductName(line.agent);
}

function SessionLineView({ entry, project, checkoutBranch, opener, actions }: { entry: SessionLine; project: Workspace; checkoutBranch: string | null; opener: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const workspaces = useShellStore((s) => s.rest?.navigator?.workspaces);
  const remote = useShellStore((s) => s.rest?.status?.remote);
  const node = useShellStore((s) => localDeviceId(s.rest));
  const { line, live } = entry;
  const title = useSessionTitle(line, live, project.id);
  const checkout = resumeCheckout(line, catalogOf(workspaces, remote), checkoutBranch, node);
  const viewOff = viewBlock(line, devices, node);
  const resumeOff = resumeBlock(line, checkout, devices, node);
  const { resume, start } = useResume();
  const gone = line.file === "missing";
  const asked = live?.kind === "question" ? rowLine(live.agent)?.text : null;
  const view = () => actions.openSessionConversation({ device_id: line.device_id, agent: line.agent, session_id: line.id });
  const resumeLine = () => {
    const provider = resumeProvider(line.agent);
    if (!checkout || !provider) return;
    start((requestId) => actions.startAgent({ target: { checkoutPath: checkout.path }, deviceId: line.device_id === node ? undefined : line.device_id, provider, resumeSessionId: line.id, resumeSessionPath: line.path ?? undefined, requestId }));
  };
  const agent = agentProductName(line.agent);
  // The tooltip carries what the row leaves out (B9): the request, who delegated it, how many sessions it joins, which device.
  const hint = [
    line.request ?? t("links.noRequest"),
    line.parent ? t("links.delegatedBy", { name: line.parent.name }) : null,
    line.ids.length > 1 ? t("links.continued", { count: line.ids.length }) : null,
    line.device_id !== node ? deviceLabel(devices, line.device_id) : null,
  ].filter((part): part is string => part !== null).join(" · ");
  // The status mark on the rail, then the agent's logo, as a sidebar row (D-09, B17).
  const mark = <><span className={ON_RAIL}>{live ? <StatusMark mark={live.agent.mark} className={markTone(live.agent)} data-agent-status-mark={live.agent.mark} /> : <StatusMark mark={null} data-agent-status-mark="ended" />}</span><AgentMark kind={line.agent} /></>;
  const caption = asked ? { text: asked, className: "text-warning", wrap: true } : { text: `${agent} · ${live ? statusText(t, live.agent.status_code) : t("sessionWork.done")}` };
  const right = live ? <Elapsed since={line.started_at_unix_ms} className="font-mono text-caption text-muted-foreground" data-link-elapsed={line.id} /> : (() => {
    const span = spanText(line.started_at_unix_ms, line.ended_at_unix_ms);
    return span ? <span className="font-mono text-caption text-muted-foreground" data-link-span={line.id}>{span}</span> : null;
  })();
  const paneButton = live ? (
    <Button variant="secondary" size="sm" onClick={() => actions.openAgent(live.agent.pane_id)} data-link-pane={line.id}>
      <SquareTerminalIcon aria-hidden="true" />
      {t("links.goToPane")}
    </Button>
  ) : null;
  const viewButton = <LineButton icon={<MessageSquareIcon aria-hidden="true" />} label={t("links.view")} blocked={viewOff} onClick={view} data="view" id={line.id} />;
  const resumeButton = <LineButton icon={resume.request ? <LoaderCircleIcon aria-hidden="true" className="animate-spin" /> : <PlayIcon aria-hidden="true" />} label={resume.request ? t("links.resuming") : t("links.resume")} blocked={resumeOff} busy={resume.request !== null} onClick={resumeLine} data="resume" id={line.id} />;
  // A live row keeps its one button under the line (B8); an ended row reveals its two in the time's place.
  const below = live?.kind === "question" ? (
    <Button size="sm" onClick={() => actions.openAgent(live.agent.pane_id)} data-link-answer={line.id}>
      <CircleHelpIcon aria-hidden="true" />
      {t("links.answer")}
    </Button>
  ) : live?.kind === "working" ? paneButton : null;
  const reveal = live ? (live.kind === "idle" ? <>{viewButton}{paneButton}</> : null) : <>{viewButton}{resumeButton}</>;
  const busy = resume.request !== null || resume.failure !== null;
  return (
    <SessionRow
      mark={mark}
      title={title}
      titleClassName={gone ? "text-muted-foreground" : undefined}
      hint={hint}
      chips={opener && line.role === "created" ? <span className="inline-flex h-(--size-control-sm) shrink-0 items-center rounded-xs border border-border px-xs text-micro text-pr-merged" data-link-chip="created">{t("links.created")}</span> : null}
      right={right}
      reveal={busy && !live ? undefined : reveal}
      line={caption}
      below={busy && !live ? <>{viewButton}{resumeButton}</> : below}
      aria-label={[title, agent].join(", ")}
      data-link-session={line.id}
      data-link-live={live?.kind}
      data-link-role={line.role}
      data-link-file={line.file}
      data-link-request={line.request ?? undefined}
    >
      {gone ? <MissingFile path={line.path} id={line.id} /> : null}
      {resume.failure !== null ? (
        <span role="alert" className="flex items-center gap-xs text-caption text-warning" data-link-resume-failed={line.id}>
          <span className="min-w-0 break-words">{resume.failure ? t("links.resumeFailed", { reason: resume.failure }) : t("links.resumeFailedPlain")}</span>
          <Button variant="ghost" size="sm" onClick={resumeLine} data-link-resume-retry={line.id}>
            {t("common.retry")}
          </Button>
        </span>
      ) : null}
    </SessionRow>
  );
}

/** A button under an ended line; off, it stays reachable so its tooltip says why (B10). */
function LineButton({ icon, label, blocked, busy = false, onClick, data, id }: { icon: ReactNode; label: string; blocked: Blocked; busy?: boolean; onClick: () => void; data: string; id: string }) {
  const { t } = useInterfaceTranslation();
  const off = blocked !== null || busy;
  const button = (
    <Button
      variant="ghost"
      size="sm"
      aria-disabled={off ? "true" : undefined}
      aria-busy={busy ? "true" : undefined}
      className={cn("h-(--size-sidebar-line) px-xs text-caption", off && "opacity-(--opacity-disabled) hover:bg-transparent")}
      onClick={() => {
        if (!off) onClick();
      }}
      data-link-button={data}
      data-link-line={id}
      data-link-blocked={blocked ? blocked.key : undefined}
    >
      {icon}
      {label}
    </Button>
  );
  if (!blocked) return button;
  return (
    <Hint label={"device" in blocked ? t(blocked.key, { device: blocked.device }) : t(blocked.key)} reveals>
      {button}
    </Hint>
  );
}

/** `Conversation file gone` and the copy of where it was (B15). */
function MissingFile({ path, id }: { path: string | null; id: string }) {
  const { t } = useInterfaceTranslation();
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | null>(null);
  useEffect(() => () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
  }, []);
  return (
    <span className="inline-flex items-center gap-xs text-caption text-muted-foreground" data-link-missing={id}>
      {t("links.fileMissing")}
      {path ? (
        <Hint label={copied ? t("links.copied") : t("links.copyPath")}>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => {
              void navigator.clipboard?.writeText(path).then(() => {
                setCopied(true);
                timer.current = window.setTimeout(() => setCopied(false), 1500);
              });
            }}
            data-link-copy={id}
          >
            <CopyIcon aria-hidden="true" />
          </Button>
        </Hint>
      ) : null}
    </span>
  );
}

const NO_CHIPS: readonly { number: number; created: boolean }[] = [];

/**
 * The pull requests a session made (`#N created`) or worked on (`#N`), on
 * its Sessions row and its open head (PRD link-graph B36); each opens that
 * PR's panel. A session the record has not linked has none, and a chip that
 * arrives while the record fills just appears.
 */
export function SessionPrChips({ workspaceId, sessionId, onPr, className }: { workspaceId: string; sessionId: string; onPr: (number: number) => void; className?: string }) {
  const { t } = useInterfaceTranslation();
  const chips = useShellStore((s) => s.linkSummaries?.projects[workspaceId]?.sessions?.[sessionId] ?? NO_CHIPS);
  if (chips.length === 0) return null;
  return (
    <span className={cn("flex flex-wrap items-center gap-xxs", className)} data-session-prs={sessionId}>
      {chips.map((chip) => (
        <Hint key={chip.number} label={t("links.chip", { number: chip.number })} reveals>
          <button
            type="button"
            className={cn("inline-flex h-(--size-control-sm) items-center rounded-xs border border-border px-xs text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", chip.created ? "text-pr-merged" : "text-subtle-foreground")}
            onClick={() => onPr(chip.number)}
            data-session-pr={chip.number}
            data-session-pr-created={chip.created ? "true" : undefined}
          >
            {chip.created ? t("links.createdNumber", { number: chip.number }) : t("links.workedNumber", { number: chip.number })}
          </button>
        </Hint>
      ))}
    </span>
  );
}
