import { ChevronRightIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { AgentLogo } from "./components/agent-logo";
import { Elapsed } from "./components/elapsed";
import { HideAiExplainer } from "./components/hide-ai-explainer";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { sourceTime } from "./memoryList";
import { allAgents } from "./navigation";
import { catalogWorkspaces, type ProjectSuggestions, type SuggestionEvidence, type SuggestionRow } from "./snapshot";
import { suggestionSteps } from "./hideAiSteps";
import { useShellStore } from "./store";
import { applyOffer, appliedSummary, attemptLine, localLine, suggestionGroups, suggestionMark, suggestionTitle, type SuggestionMark } from "./suggestionList";
import { useUiStore } from "./ui";

/**
 * The Hide AI tool's 제안 tab (PRD hide-ai-suggestions D-05..D-07, D-37,
 * D-44, B10-B18): one line for the tab's state, then 고를 것, 반영 중 and the
 * folded 반영됨, each row as the core sent it. A row opens in place to its
 * fix and evidence; every press is one `suggestion_action` and the core
 * moves the row. The open row's evidence is read again masked while it is
 * open (PRD memory-self-improve B7), and the Hide global instruction shows
 * only when sessions are not given it (B5).
 */
export function SuggestionsTab({ project, actions }: { project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const groups = suggestionGroups(project.rows);
  const [folded, setFolded] = useState(true);
  const applied = appliedSummary(groups.applied);
  const line = statusLine(project, groups.choose.length + groups.applying.length);
  const open = useUiStore((state) => state.openSuggestion);
  const workspace = project.workspace_id;
  useEffect(() => {
    if (!open) return;
    actions.suggestionEvidence(workspace, open);
    return () => actions.suggestionEvidence(workspace, null);
  }, [actions, workspace, open]);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto px-md pb-lg" data-suggestions={project.status}>
      <GlobalInstruction project={project} actions={actions} />
      {line ? (
        <HideAiExplainer title={t(line)} steps={suggestionSteps(t)} data-suggestions-status={project.status}>
          {project.status === "off" ? (
            <Button size="sm" variant="outline" className="self-start" onClick={() => actions.suggestionSettings(project.workspace_id, { suggestions_enabled: true })} data-suggestions-turn-on="true">
              {t("suggestions.turnOn")}
            </Button>
          ) : null}
        </HideAiExplainer>
      ) : null}
      {project.status === "off" ? null : (
        <>
          <Group name={t("suggestions.group.choose")} rows={groups.choose} project={project} actions={actions} kind="choose" />
          <Group name={t("suggestions.group.applying")} rows={groups.applying} project={project} actions={actions} kind="applying" />
          {applied.count > 0 ? (
            <section className="flex min-w-0 flex-col" data-suggestions-group="applied">
              <button type="button" aria-expanded={!folded} className="flex items-center gap-xs self-start py-sm text-caption text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={() => setFolded(!folded)} data-suggestions-fold={applied.count}>
                <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", !folded && "rotate-90")} />
                <span>{t("suggestions.group.applied")} {applied.count}</span>
                {applied.effective > 0 ? <span>· {t("suggestions.appliedEffective", { count: applied.effective })}</span> : null}
              </button>
              {folded ? null : groups.applied.map((row) => <Row key={row.id} row={row} project={project} actions={actions} />)}
            </section>
          ) : null}
        </>
      )}
    </div>
  );
}

/** The tab's one line (D-44), the owl's title over what the tab gives while nothing is open or it is off; `조사 중` and `조사 못 함` sit in the head, and the rows stay as they were. */
function statusLine(project: ProjectSuggestions, open: number): "suggestions.status.firstRun" | "suggestions.status.noneToday" | "suggestions.status.off" | null {
  switch (project.status) {
    case "first_run_pending": return "suggestions.status.firstRun";
    case "none_today": return open === 0 ? "suggestions.status.noneToday" : null;
    case "off": return "suggestions.status.off";
    case "investigating":
    case "not_investigated":
    case "idle": return null;
  }
}

/** The Hide global instruction (B5): one line with `파일 열기` while sessions are not given it, nothing while they are. */
function GlobalInstruction({ project, actions }: { project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const global = project.global;
  if (!global?.unusable) return null;
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs border-b border-border py-sm text-caption" data-suggestions-global={global.unusable}>
      <span className="font-medium text-foreground">{t("suggestions.global.title")}</span>
      <span className="text-warning">{t(global.unusable === "too_large" ? "suggestions.global.too_large" : "suggestions.global.unreadable")}</span>
      <OpenFile path={global.path} actions={actions} />
    </div>
  );
}

/** `파일 열기`: the Hide global instruction in the system's editor, where the operator fixes what Hide cannot (B5, B9, B10). */
function OpenFile({ path, actions }: { path: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <button type="button" className="pointer-events-auto shrink-0 text-subtle-foreground underline-offset-4 hover:text-foreground hover:underline" title={path} onClick={() => actions.openOwnedFile(path)} data-suggestion-open-file={path}>
      {t("suggestions.local.openFile")}
    </button>
  );
}

function Group({ name, rows, project, actions, kind }: { name: string; rows: SuggestionRow[]; project: ProjectSuggestions; actions: Actions; kind: "choose" | "applying" }) {
  if (rows.length === 0) return null;
  return (
    <section className="flex min-w-0 flex-col" aria-label={name} data-suggestions-group={kind}>
      <h3 className="flex items-baseline gap-xs pt-md pb-xs text-caption font-medium text-muted-foreground">
        {name}
        <span className="text-subtle-foreground">{rows.length}</span>
      </h3>
      {rows.map((row) => <Row key={row.id} row={row} project={project} actions={actions} />)}
    </section>
  );
}

/** A row's mark (D-37): the green open circle, the yellow dot, the blue turning ring; an empty slot keeps the titles in line. */
export function SuggestionMarkIcon({ mark }: { mark: SuggestionMark }) {
  const { t } = useInterfaceTranslation();
  const label = mark === "new" ? t("suggestions.mark.new") : mark === "my_turn" ? t("suggestions.mark.myTurn") : mark === "working" ? t("suggestions.mark.working") : undefined;
  return (
    <span {...(label ? { role: "img", "aria-label": label } : { "aria-hidden": true })} className="mt-xxs inline-flex size-(--size-agent-mark) shrink-0 items-center justify-center" data-suggestion-mark={mark ?? undefined}>
      {mark === "new" ? (
        <svg viewBox="0 0 12 12" className="size-full"><circle cx="6" cy="6" r="4.2" fill="none" strokeWidth="1.6" className="stroke-success" /></svg>
      ) : mark === "my_turn" ? (
        <span className="size-(--size-status-mark) rounded-full bg-warning" />
      ) : mark === "working" ? (
        <svg viewBox="0 0 12 12" className="size-full motion-safe:animate-spin"><path d="M10.05 6A4.05 4.05 0 1 1 6 1.95" fill="none" strokeWidth="1.6" className="stroke-agent-working" /></svg>
      ) : null}
    </span>
  );
}

function Row({ row, project, actions }: { row: SuggestionRow; project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const open = useUiStore((state) => state.openSuggestion === row.id);
  const self = useRef<HTMLDivElement>(null);
  const title = suggestionTitle(t, row);
  const mark = suggestionMark(row.state);
  const working = row.state === "hide_ai_working";
  useEffect(() => {
    if (open) self.current?.scrollIntoView({ block: "nearest" });
  }, [open]);
  const toggle = () => useUiStore.getState().showHideAiTab("suggestions", open ? null : row.id);
  return (
    <div ref={self} className={cn("relative min-w-0 border-b border-border", open && "bg-card")} data-suggestion={row.id} data-suggestion-state={row.state} data-suggestion-source={row.source} data-suggestion-open={open ? "true" : undefined}>
      <button type="button" className="absolute inset-0 rounded-xs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" aria-label={t(open ? "suggestions.closeRow" : "suggestions.openRow", { row: title })} aria-expanded={open} onClick={toggle} data-suggestion-toggle={row.id} />
      <div className="pointer-events-none relative grid min-w-0 grid-cols-[auto_1fr] gap-x-sm gap-y-xs px-xs py-sm">
        <SuggestionMarkIcon mark={mark} />
        <div className={cn("flex min-w-0 items-start gap-xs", working && "opacity-(--opacity-dimmed)")}>
          {row.source === "memory" ? <span className="mt-xxs shrink-0 font-mono text-caption text-subtle-foreground" data-suggestion-memory={row.id}>{row.id}</span> : null}
          <span className={cn("min-w-0 flex-1 break-words break-keep text-body", row.group === "applied" ? "text-subtle-foreground" : "text-foreground", !open && "line-clamp-2")} title={title}>{title}</span>
          {row.auto ? <span className="mt-xxs shrink-0 rounded-sm bg-secondary px-xs text-micro font-medium text-subtle-foreground" data-suggestion-auto="true">{t("suggestions.auto")}</span> : null}
        </div>
        <Lines row={row} open={open} project={project} actions={actions} />
        <Fix row={row} open={open} />
        {open ? <Detail row={row} project={project} actions={actions} /> : null}
        <Buttons row={row} project={project} actions={actions} />
      </div>
    </div>
  );
}

/** The row's second line: 놓친 것 for a choice (with `문서로는 안 됐어요` when it came back after a merge, `넘어갔지만 다시 생겼어요` after a skip), the attempt while it is applied, the watch once merged. */
function Lines({ row, open, project, actions }: { row: SuggestionRow; open: boolean; project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const working = row.state === "hide_ai_working";
  const attempt = attemptLine(t, row);
  const local = localLine(t, row, project.global?.unusable ?? null);
  return (
    <div className={cn("col-start-2 flex min-w-0 flex-col gap-xxs text-caption", working && "opacity-(--opacity-dimmed)")}>
      {local ? (
        <span className="flex min-w-0 flex-wrap items-center gap-x-sm" data-suggestion-local={row.local?.failure ?? (row.local?.recurred ? "recurred" : row.local?.present ? "present" : "held")}>
          <span className={cn("min-w-0 break-words", local.tone === "warning" ? "text-warning" : local.tone === "success" ? "text-success" : "text-muted-foreground")}>{local.text}</span>
          {local.openFile && project.global ? <OpenFile path={project.global.path} actions={actions} /> : null}
        </span>
      ) : null}
      {row.escalated ? (
        <span className="flex min-w-0 items-center gap-xs" data-suggestion-escalated={row.escalated.level}>
          <span className="shrink-0 text-warning">{t("suggestions.escalated")}</span>
          {row.escalated.previous_pr_url ? <PrLink project={project} actions={actions} url={row.escalated.previous_pr_url} label={previousPrLabel(t, row.escalated.previous_pr_url)} /> : null}
        </span>
      ) : null}
      {row.returned_after_skip ? <span className="text-warning" data-suggestion-returned="true">{t("suggestions.returnedAfterSkip")}</span> : null}
      {row.group === "choose" && row.cause ? (
        <span className={cn("min-w-0 text-muted-foreground", !open && "truncate")} title={row.cause} data-suggestion-cause="true">
          <span className="font-medium text-subtle-foreground">{t("suggestions.cause")}</span> · {row.cause}
        </span>
      ) : null}
      {row.group === "choose" && row.hold === "auto_pr_unreviewed" ? <span className="text-muted-foreground" data-suggestion-hold="auto_pr_unreviewed">{t("suggestions.waitsForAutoPr")}</span> : null}
      {row.group !== "choose" && attempt ? (
        <span className="flex min-w-0 items-center gap-sm" data-suggestion-attempt={row.attempt?.phase}>
          {row.attempt?.phase === "pr_open" && row.attempt.url ? (
            <PrLink project={project} actions={actions} url={row.attempt.url} label={attempt.text} className="min-w-0 truncate text-warning hover:text-warning" />
          ) : (
            <span className={cn("min-w-0 truncate", attempt.tone === "warning" ? "text-warning" : attempt.tone === "destructive" ? "text-destructive" : "text-muted-foreground")} title={attempt.text}>{attempt.text}</span>
          )}
          {row.attempt && (row.attempt.phase === "starting" || row.attempt.phase === "working") ? <Elapsed since={row.attempt.started_at_unix_ms} className="shrink-0 text-muted-foreground" /> : null}
          {row.attempt?.url && row.attempt.phase !== "pr_open" ? <PrLink project={project} actions={actions} url={row.attempt.url} label={row.outcome === "issue" && row.attempt.pr_number === null ? t("suggestions.openIssue") : t("suggestions.openPr", { number: String(row.attempt.pr_number ?? "") })} /> : null}
          <AgentLink agentId={row.attempt?.agent_id ?? null} actions={actions} />
        </span>
      ) : null}
      {row.group === "applying" && row.attempt?.phase === "failed" && row.cause ? (
        <span className={cn("min-w-0 break-words text-muted-foreground", !open && "truncate")} title={row.cause} data-suggestion-failure-detail="true">{row.cause}</span>
      ) : null}
      {row.group === "applied" && row.state === "watching" && row.watch ? (
        <span className="text-muted-foreground" data-suggestion-watch={`${row.watch.sessions_seen}/${row.watch.sessions_needed}`}>{t("suggestions.watching", { seen: row.watch.sessions_seen, needed: row.watch.sessions_needed })}</span>
      ) : null}
      {row.group === "applied" && row.state === "effective" ? <span className="text-success" data-suggestion-effective="true">{t("suggestions.effective")}</span> : null}
    </div>
  );
}

function previousPrLabel(t: ReturnType<typeof useInterfaceTranslation>["t"], url: string): string {
  const number = /\/pull\/(\d+)/.exec(url)?.[1];
  return number ? t("suggestions.previousPr", { number }) : url;
}

/** A pull request or issue the row names: a pull request opens in the PR panel, an issue in the browser. */
function PrLink({ project, actions, url, label, className }: { project: ProjectSuggestions; actions: Actions; url: string; label: string; className?: string }) {
  const number = Number(/\/pull\/(\d+)/.exec(url)?.[1] ?? Number.NaN);
  return (
    <button type="button" className={cn("pointer-events-auto shrink-0 text-subtle-foreground underline-offset-4 hover:text-foreground hover:underline", className)} title={label} onClick={() => (Number.isFinite(number) ? actions.openSessionPullRequest({ workspace_id: project.workspace_id, number, url }) : actions.openLink(url, false))} data-suggestion-link={url}>
      {label}
    </button>
  );
}

/** `에이전트 ↗` while the applying agent lives: it opens that agent's pane. */
function AgentLink({ agentId, actions }: { agentId: string | null; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pane = useShellStore((state) => {
    if (!agentId) return null;
    const rest = state.rest;
    return allAgents(rest?.status?.remote, state.agents, rest?.navigator?.agent_scope, rest?.navigator?.devices).find((row) => row.agent.id === agentId)?.agent.pane_id ?? null;
  });
  if (!pane) return null;
  return (
    <button type="button" className="pointer-events-auto ml-auto shrink-0 text-subtle-foreground hover:text-foreground" aria-label={t("suggestions.openAgent")} onClick={() => actions.openAgent(pane)} data-suggestion-agent={agentId}>
      {t("suggestions.agent")}
    </button>
  );
}

/**
 * 고칠 것 above a choice's buttons (D-51, B10, B14), so what a press does is
 * read before it: the core's fix, or for an issue without one why it becomes
 * an issue.
 */
function Fix({ row, open }: { row: SuggestionRow; open: boolean }) {
  const { t } = useInterfaceTranslation();
  if (row.group !== "choose") return null;
  if (row.local) return <LocalFix row={row} line={row.local.line} open={open} />;
  const text = row.fix ?? (row.outcome === "issue" ? t("suggestions.issueFix") : null);
  if (!text) return null;
  return (
    <div className="col-start-2 flex min-w-0 flex-col gap-xxs" data-suggestion-fix={row.id}>
      <span className="text-micro text-muted-foreground">{t("suggestions.fix")}</span>
      <p className={cn("whitespace-pre-wrap break-words break-keep text-caption text-foreground", !open && "line-clamp-2")}>{text}</p>
    </div>
  );
}

/** The one line a global instruction row writes, as it will read in the file (PRD memory-self-improve B6). */
function LocalFix({ row, line, open }: { row: SuggestionRow; line: string; open: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="col-start-2 flex min-w-0 flex-col gap-xxs" data-suggestion-fix={row.id} data-suggestion-local-line="true">
      <span className="text-micro text-muted-foreground">{t("suggestions.local.target")}</span>
      <p className={cn("whitespace-pre-wrap break-words rounded-xs bg-secondary px-xs py-xxs text-caption text-foreground", !open && "line-clamp-2")}>{line}</p>
    </div>
  );
}

/** A row in full (B10): the evidence it rests on, and 고칠 것 once it left the choice; evidence that is gone stays, dimmed, and opens nothing. */
function Detail({ row, project, actions }: { row: SuggestionRow; project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="col-start-2 flex min-w-0 flex-col gap-sm pt-xs" data-suggestion-detail={row.id}>
      {row.local && row.group !== "choose" ? <LocalFix row={row} line={row.local.line} open /> : null}
      {row.fix && !row.local && row.group !== "choose" ? (
        <div className="flex flex-col gap-xxs">
          <span className="text-micro text-muted-foreground">{t("suggestions.fix")}</span>
          <p className="whitespace-pre-wrap break-words break-keep text-caption text-foreground">{row.fix}</p>
        </div>
      ) : null}
      {row.evidence.length > 0 ? (
        <div className="flex flex-col gap-xxs">
          <span className="text-micro text-muted-foreground">{t("suggestions.evidence")}</span>
          {row.evidence.map((evidence, index) => <Evidence key={index} evidence={evidence} project={project} actions={actions} />)}
        </div>
      ) : null}
    </div>
  );
}

function Evidence({ evidence, project, actions }: { evidence: SuggestionEvidence; project: ProjectSuggestions; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const device = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === project.workspace_id)?.device_id ?? null);
  const signal = t(`suggestions.signal.${evidence.signal}`);
  const session = evidence.kind === "session" && evidence.agent && evidence.session_id && device ? { device_id: device, agent: evidence.agent, session_id: evidence.session_id } : null;
  const pull = evidence.kind === "pr" && evidence.url && evidence.pr_number !== null ? { workspace_id: project.workspace_id, number: evidence.pr_number, url: evidence.url } : null;
  const openable = !evidence.gone && (session !== null || pull !== null);
  const name = evidence.kind === "pr" && evidence.pr_number !== null ? t("suggestions.openPr", { number: String(evidence.pr_number) }) : t("suggestions.evidence.session");
  const said = evidence.excerpt ? (
    <p className="min-w-0 whitespace-pre-wrap break-words border-l-2 border-border pl-sm text-caption text-muted-foreground" data-suggestion-excerpt="true">{evidence.excerpt}</p>
  ) : evidence.unreadable ? (
    <span className="text-caption text-warning" data-suggestion-excerpt-unreadable="true">{t("suggestions.evidence.unreadable")}</span>
  ) : null;
  return (
    <div className="flex min-w-0 flex-col gap-xxs">
      <button
        type="button"
        disabled={!openable}
        className={cn("pointer-events-auto grid min-w-0 grid-cols-[auto_1fr_auto] items-center gap-x-sm rounded-xs text-left text-caption outline-none enabled:hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring", evidence.gone && "opacity-(--opacity-dimmed)")}
        onClick={() => (session ? actions.openSessionRecord(session) : pull ? actions.openSessionPullRequest(pull) : undefined)}
        data-suggestion-evidence={evidence.kind}
        data-suggestion-evidence-gone={evidence.gone ? "true" : undefined}
      >
        {evidence.agent ? <AgentLogo agent={evidence.agent} label={evidence.agent} /> : <span aria-hidden="true" className="size-(--size-icon-lg)" />}
        {evidence.improvement ? (
          <span className="flex min-w-0 items-center gap-xs">
            <Badge data-suggestion-evidence-improvement={evidence.improvement}>{t(`suggestions.improvement.${evidence.improvement}`)}</Badge>
            <span className="min-w-0 truncate text-subtle-foreground">{name}{evidence.gone ? ` · ${t("suggestions.evidence.gone")}` : ""}</span>
          </span>
        ) : (
          <span className="min-w-0 truncate text-subtle-foreground">{name} · {signal}{evidence.gone ? ` · ${t("suggestions.evidence.gone")}` : ""}</span>
        )}
        <span className="shrink-0 font-mono text-muted-foreground">{sourceTime(evidence.at_unix_ms, language)}</span>
      </button>
      {said ? <div className="ml-sm pl-(--size-icon-lg)">{said}</div> : null}
    </div>
  );
}

/**
 * A choice's `문서 고치는 PR 열기` or `이슈로 남기기` (B10, B14), `GitHub 연결
 * 필요` in its place (B18), and a failed attempt's `다시 시도` (B12), each
 * with a quiet `넘어가기` beside it (D-51, B16). A global instruction row
 * offers `Hide 전역 지시에 넣기` with the preview it was shown, held while it
 * cannot be written, and `되돌리기` once it went in (PRD memory-self-improve
 * B8, B9).
 */
function Buttons({ row, project, actions }: { row: SuggestionRow; project: ProjectSuggestions; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const id = project.workspace_id;
  const offer = applyOffer(row);
  const retry = row.group === "applying" && attemptLine(t, row)?.retry === true;
  const revert = row.group === "applied" && row.local !== null && row.local !== undefined && row.local.failure !== "edited";
  if (revert) {
    return (
      <div className="col-start-2 flex flex-wrap items-center gap-xs pt-xxs">
        <Button className="pointer-events-auto" size="sm" variant="outline" onClick={() => actions.suggestionAction(id, row.id, "revert")} data-suggestion-revert={row.id}>{t("suggestions.local.revert")}</Button>
      </div>
    );
  }
  if (offer === null && !retry) return null;
  return (
    <div className="col-start-2 flex flex-wrap items-center gap-xs pt-xxs">
      {offer === "local" || offer === "local_held" ? (
        <Button className="pointer-events-auto" size="sm" disabled={offer === "local_held"} onClick={() => row.local?.preview != null && actions.suggestionAction(id, row.id, "apply", row.local.preview)} data-suggestion-apply={offer}>
          {t(row.local?.failure === "write_failed" ? "suggestions.retry" : "suggestions.makeLocal")}
        </Button>
      ) : offer === "github_needed" ? (
        <span className="text-caption text-muted-foreground" data-suggestion-github-needed="true">{t("suggestions.githubNeeded")}</span>
      ) : offer ? (
        <Button className="pointer-events-auto" size="sm" variant={offer === "issue" ? "secondary" : "default"} onClick={() => actions.suggestionAction(id, row.id, "apply")} data-suggestion-apply={offer}>
          {t(offer === "issue" ? "suggestions.makeIssue" : "suggestions.makePr")}
        </Button>
      ) : null}
      {retry ? <Button className="pointer-events-auto" size="sm" variant="outline" onClick={() => actions.suggestionAction(id, row.id, "retry")} data-suggestion-retry={row.id}>{t("suggestions.retry")}</Button> : null}
      <Button className="pointer-events-auto text-muted-foreground" size="sm" variant="ghost" onClick={() => actions.suggestionAction(id, row.id, "dismiss")} data-suggestion-dismiss={row.id}>{t("suggestions.dismiss")}</Button>
    </div>
  );
}
