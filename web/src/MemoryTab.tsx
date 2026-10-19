import { ChevronDownIcon, ChevronRightIcon, ClockIcon, FileIcon, LoaderCircleIcon, MoreHorizontalIcon, SettingsIcon, SparklesIcon, Trash2Icon, Undo2Icon, UserIcon } from "lucide-react";
import { useUiStore } from "./ui";
import { sameGithubPullRequest } from "./pullRequestIdentity";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "./actions";
import { AgentLogo } from "./components/agent-logo";
import { PrMark } from "./components/pr-mark";
import { Elapsed } from "./components/elapsed";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "./components/ui/alert-dialog";
import { Button } from "./components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Checkbox } from "./components/ui/checkbox";
import { Input } from "./components/ui/input";
import { Switch } from "./components/ui/switch";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import type { MessageKey } from "./i18n/catalogs";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { cardName, memoryCards, memoryTime, sourceTime, visibleCards, memoryProgress, memoryProgressAgent, memoryProgressStatus, selectableMemoryQuestions, type MemoryProgress } from "./memoryList";
import { allAgents } from "./navigation";
import { catalogWorkspaces, frontCheckout, type AgentRow, type PullRequest, type MemoryCard, type MemoryCardKind, type MemoryQuestion, type MemoryStartFailure, type MemoryRow, type ProjectMemory } from "./snapshot";
import { useShellStore } from "./store";

const KIND: Record<MemoryCardKind, MessageKey> = { rule: "memory.kind.rule", decision: "memory.kind.decision", lesson: "memory.kind.lesson", fact: "memory.kind.fact", preference: "memory.kind.preference" };

/**
 * The lane's Memory tool (PRD project-memory-v2 D-17, B19-B32; PRD
 * right-lane D-01): the front checkout's Project, which the tool rail opens
 * (`useMemoryProject`); until the core answers for that Project, the tool reads.
 */
export function MemoryTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const projectId = useShellStore((state) => frontCheckout(state.rest)?.workspace_id ?? null);
  const memory = useShellStore((state) => state.memory?.project ?? null);
  const opened = memory && memory.workspace_id === projectId ? memory : null;
  if (!projectId) return <p className="p-lg text-caption text-muted-foreground">{t("history.openWorkspace")}</p>;
  if (!opened || (opened.loading && opened.rows.length === 0 && !opened.unavailable)) {
    return (
      <p role="status" className="p-lg text-caption text-muted-foreground">
        {t("memory.reading")}
      </p>
    );
  }
  return <MemoryBody project={opened} actions={actions} />;
}

/**
 * Opens the front checkout's Project's Memory while the tool rail shows,
 * whatever the lane holds, so the Memory icon's dot says when the Project has
 * something to ask (B28). The core keeps one opened Project per daemon: the
 * rail names its Project when it shows, when the front Project changes, and
 * when its window comes forward after another window named another one, so
 * two windows never take it from each other in a loop.
 */
export function useMemoryProject(actions: Actions, requested?: string | null) {
  const settingsOpen = useUiStore((state) => state.overlay === "project_settings");
  const frontId = useShellStore((state) => frontCheckout(state.rest)?.workspace_id ?? null);
  const projectId = requested !== undefined ? requested : settingsOpen ? null : frontId;
  const live = useShellStore((state) => state.connection === "live");
  const opened = useShellStore((state) => projectId !== null && state.memory?.project?.workspace_id === projectId);
  const openedRef = useRef(opened);
  openedRef.current = opened;
  useEffect(() => {
    if (!live || !projectId) return;
    actions.memoryOpen(projectId);
    const reclaim = () => {
      if (!openedRef.current) actions.memoryOpen(projectId);
    };
    window.addEventListener("focus", reclaim);
    return () => window.removeEventListener("focus", reclaim);
  }, [actions, live, projectId]);
}

function MemoryBody({ project, actions }: { project: ProjectMemory; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const id = project.workspace_id;
  const busy = project.pending_action !== null;
  const [deleting, setDeleting] = useState(false);
  const label = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === id)?.label ?? id);
  // Every agent is past the usage limit: Hide AI is on but asks nothing until one is under it (D-06).
  const pastLimit = useShellStore((state) => {
    const refusal = state.rest?.status?.background_ai?.refusal;
    return refusal?.reason === "past_limit" && !refusal.using;
  });
  const unavailable = project.unavailable;
  if (unavailable) {
    return (
      <div className="flex flex-col items-start gap-sm p-lg" data-memory={unavailable.reason}>
        <p className="break-keep text-caption text-subtle-foreground">{t(unavailable.reason === "unregistered" ? "memory.unregistered" : "memory.unreadable")}</p>
        {unavailable.reason === "unreadable" ? (
          <Button size="sm" variant="secondary" onClick={() => actions.memoryOpen(id)}>
            {t("memory.readAgain")}
          </Button>
        ) : null}
      </div>
    );
  }
  const count = project.rows.length;
  return (
    <div className="relative flex min-h-0 flex-1 flex-col" data-memory={project.enabled ? "on" : "off"}>
      <div className="flex min-h-0 flex-1 flex-col gap-lg overflow-auto p-lg">
        <Header project={project} actions={actions} busy={busy} />
        {project.enabled ? null : (
          <div className="-mt-sm flex flex-col items-start gap-sm">
            <p className="break-keep [overflow-wrap:anywhere] text-caption text-subtle-foreground">{t("memory.disclosure")}</p>
            {project.consent_required ? <p className="break-keep text-caption text-subtle-foreground" data-memory-consent="required">{t("memory.reconsent")}</p> : null}
            {count > 0 ? (
              <button type="button" className="text-caption text-muted-foreground underline-offset-4 hover:underline" onClick={() => setDeleting(true)} data-memory-delete="link">
                {t("memory.deleteData")}
              </button>
            ) : null}
          </div>
        )}
        {project.enabled && project.analysis === "ai_not_ready" ? (
          <div className="-mt-sm flex flex-col items-start gap-sm" data-memory-ai="not_ready">
            <p className="break-keep text-caption text-subtle-foreground">{t(pastLimit ? "memory.aiPastLimit" : "memory.aiNotReady")}</p>
            <Button size="sm" variant="outline" onClick={() => actions.openSettings("hideAi")}>
              <SettingsIcon aria-hidden="true" />
              {t("memory.openHideAi")}
            </Button>
          </div>
        ) : null}
        {project.enabled && project.capacity_reached ? <p className="-mt-sm break-keep text-caption text-warning">{t("memory.capacity")}</p> : null}
        {project.enabled && (project.questions.length > 0 || memoryProgress(project).length > 0) ? <Questions project={project} actions={actions} busy={busy} /> : null}
        {project.enabled ? <Cards project={project} actions={actions} busy={busy} onDelete={() => setDeleting(true)} /> : null}
      </div>
      {project.forgotten !== null ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex p-md">
          <div role="status" className="pointer-events-auto flex items-center gap-sm rounded-md border border-border bg-popover px-md py-xs text-caption text-foreground shadow-md" data-memory-forgotten={project.forgotten}>
            <span>{t("memory.forgotten", { card: cardName(project.forgotten) })}</span>
            <span aria-hidden="true" className="text-muted-foreground">·</span>
            <button type="button" disabled={busy} className="flex items-center gap-xxs font-medium hover:underline disabled:opacity-(--opacity-disabled)" onClick={() => actions.memoryAction(id, "undo", project.forgotten)}>
              <Undo2Icon aria-hidden="true" className="size-(--size-icon-sm)" />
              {t("memory.undo")}
            </button>
          </div>
        </div>
      ) : null}
      <AlertDialog open={deleting} onOpenChange={setDeleting}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("memory.delete.title", { project: label })}</AlertDialogTitle>
            <AlertDialogDescription>{t("memory.delete.body", { count })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("memory.delete.keep")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => actions.memoryAction(id, "delete")} data-memory-delete="confirm">
              {t("memory.delete.confirm", { count })}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** The switch, the analysis chip and the Hide AI chip. */
function Header({ project, actions, busy }: { project: ProjectMemory; actions: Actions; busy: boolean }) {
  const { t } = useInterfaceTranslation();
  const ai = useShellStore((state) => state.rest?.status?.background_ai ?? null);
  // The chip names the agent answering now: the listed one that stands in while Runs on cannot answer (B4).
  const answering = ai?.refusal?.using ?? ai?.provider ?? null;
  const provider = answering ? ai?.providers.find((row) => row.id === answering) ?? null : null;
  const switchId = `memory-switch-${project.workspace_id}`;
  const dim = project.enabled && project.analysis === "ai_not_ready";
  return (
    <div className="flex flex-col gap-sm">
      <div className="flex items-center gap-sm">
        <label htmlFor={switchId} className="text-body text-foreground">
          {t("memory.switch")}
        </label>
        {project.enabled && project.analysis === "analyzing" ? (
          <span role="status" className="flex items-center gap-xxs rounded-sm border border-border px-xs text-micro text-subtle-foreground" data-memory-analysis="analyzing">
            <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />
            {t("memory.analyzing")}
          </span>
        ) : null}
        <Switch
          id={switchId}
          className="ml-auto"
          checked={project.enabled}
          disabled={busy || project.unavailable !== null}
          onCheckedChange={(on) => actions.memoryAction(project.workspace_id, on ? "enable" : "disable")}
          data-memory-switch={project.enabled ? "on" : "off"}
        />
      </div>
      <span className={cn("flex w-fit items-center gap-xs rounded-sm bg-secondary px-xs py-xxs text-caption text-subtle-foreground", dim && "opacity-(--opacity-dimmed)")} data-memory-ai-chip={provider?.agent ?? "none"}>
        {provider ? <AgentLogo agent={provider.agent} label={provider.label} /> : null}
        {provider ? t("memory.hideAi", { agent: provider.label }) : t("memory.hideAiOnly")}
      </span>
    </div>
  );
}

/** Same action and consent as Tools; only the settings owner can claim this read slot. */
export function MemorySettings({ project, actions, disabled }: { project: ProjectMemory; actions: Actions; disabled: boolean }) {
  const { t } = useInterfaceTranslation();
  const failed = project.action_failure === "enable" || project.action_failure === "disable" ? project.action_failure : null;
  if (project.unavailable) return <div className="flex flex-col items-start gap-sm"><p role="alert" className="text-body text-muted-foreground">{t(project.unavailable.reason === "unregistered" ? "memory.unregistered" : "memory.unreadable")}</p><Button size="sm" variant="ghost" disabled={disabled} onClick={() => actions.memoryOpen(project.workspace_id)}>{t("memory.readAgain")}</Button></div>;
  if (project.loading) return <p role="status" className="text-body text-muted-foreground">{t("memory.reading")}</p>;
  return <div className="flex min-w-0 flex-col gap-sm"><Header project={project} actions={actions} busy={disabled || project.pending_action !== null} />{!project.enabled ? <p className="break-words text-body text-muted-foreground">{t("memory.disclosure")}</p> : null}{project.consent_required ? <p className="text-body text-muted-foreground">{t("memory.reconsent")}</p> : null}{project.pending_action !== null ? <p role="status" className="text-body text-muted-foreground">{t(project.preparing_hooks ? "projectSettings.memoryPreparing" : "projectSettings.pending")}</p> : null}{failed ? <div role="alert" className="flex flex-wrap items-center gap-sm"><p className="text-body text-destructive">{t("projectSettings.memoryFailed")}</p><Button size="sm" variant="ghost" disabled={disabled || project.pending_action !== null} onClick={() => actions.memoryAction(project.workspace_id, failed!)}>{t("projectSettings.retry")}</Button></div> : null}</div>;
}

function Questions({ project, actions, busy }: { project: ProjectMemory; actions: Actions; busy: boolean }) {
  const { t } = useInterfaceTranslation();
  const workspace = useShellStore((state) => catalogWorkspaces(state.rest).find((row) => row.id === project.workspace_id));
  const pulls = workspace?.pull_requests ?? [];
  const localAgents = useShellStore((state) => state.agents);
  const rest = useShellStore((state) => state.rest);
  const agents = allAgents(rest?.status?.remote, localAgents, rest?.navigator?.agent_scope, rest?.navigator?.devices).map((row) => row.agent);
  const eligible = selectableMemoryQuestions(project);
  const [selection, setSelection] = useState<{ project: string; numbers: number[] }>({ project: project.workspace_id, numbers: [] });
  const selected = selection.project === project.workspace_id ? selection.numbers.filter((number) => eligible.has(number)) : [];
  const toggle = (number: number) => setSelection({ project: project.workspace_id, numbers: selected.includes(number) ? selected.filter((card) => card !== number) : selected.length < 5 ? [...selected, number] : selected });
  const groups = memoryProgress(project);
  return (
    <section className="flex min-w-0 flex-col gap-sm" data-memory-questions="true">
      <h3 className="text-caption font-medium text-warning">{t("memory.questions")}</h3>
      {project.questions.map((question, index) => (
        <Question key={question.number} question={question} project={project} actions={actions} busy={busy} last={index === project.questions.length - 1} selected={selected.includes(question.number)} selecting={selected.length > 0} selectionDisabled={!eligible.has(question.number) || (!selected.includes(question.number) && selected.length >= 5)} onSelect={() => toggle(question.number)} />
      ))}
      {selected.length ? (
        <div className="flex flex-wrap items-center gap-sm border-t border-border pt-sm text-caption" data-memory-selection={selected.length}>
          <span className="min-w-0 flex-1 text-muted-foreground">{t("memory.batch.selected", { count: selected.length })}</span>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => setSelection({ project: project.workspace_id, numbers: [] })}>{t("memory.batch.clear")}</Button>
          <Button size="sm" disabled={busy} onClick={() => actions.memoryAction(project.workspace_id, "promote", null, selected)} data-memory-open-batch="true">{t("memory.batch.open")}</Button>
        </div>
      ) : null}
      {groups.map((group) => <ProgressRow key={group.key} group={group} pull={pulls.find((pull) => pull.number === group.pr && sameGithubPullRequest(pull.url, group.url))} agents={agents} project={project} actions={actions} />)}
    </section>
  );
}

/** A native button behind the read view makes the whole row one disclosure,
 * while sibling controls keep their own pointer and keyboard actions. */
function AskRow({ number, expanded, onToggle, children }: { number: number; expanded: boolean; onToggle: () => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  return <div className="relative min-w-0">
    <button type="button" className="absolute inset-0 rounded-xs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" aria-label={t(expanded ? "memory.collapse" : "memory.expand", { card: cardName(number) })} aria-expanded={expanded} onClick={onToggle} data-memory-row-toggle={number} />
    <div className="pointer-events-none relative grid min-w-0 grid-cols-[auto_1fr] gap-x-sm gap-y-xs py-sm">{children}</div>
  </div>;
}

function AskChevron({ expanded }: { expanded: boolean }) {
  return <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon-sm) shrink-0 text-muted-foreground", expanded && "rotate-90")} />;
}

function ProgressRow({ group, pull, agents, project, actions }: { group: MemoryProgress; pull: PullRequest | undefined; agents: AgentRow[]; project: ProjectMemory; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const first = group.cards[0];
  const promotion = first.promotion;
  const expanded = group.cards.some((row) => row.number === project.expanded?.number);
  const agent = memoryProgressAgent(group, pull, agents);
  const pane = agent?.pane_id ?? (group.pr === null ? promotion?.agent?.pane_id : null);
  const status = memoryProgressStatus(group, pull, agent);
  const reason = [t(status.key), promotion?.github_failure && promotion.github_failure !== "no_github_remote" ? t(`memory.github.${promotion.github_failure}`) : null].filter(Boolean).join(" · ");
  return <div className="min-w-0 border-b border-border" data-memory-card={first.number} data-memory-progress="true" data-memory-pr-group={group.pr ?? undefined}>
    <AskRow number={first.number} expanded={expanded} onToggle={() => actions.memoryExpand(project.workspace_id, expanded ? null : first.number)}>
      <PrMark className="mt-xxs" state={pull?.state ?? "pending"} compact />
      <div className="flex min-w-0 items-center gap-sm">
        <span className="min-w-0 flex-1 truncate text-body" title={first.title}>{first.title}</span>
        {group.cards.length > 1 ? <span className="shrink-0 text-caption text-muted-foreground">+{group.cards.length - 1}</span> : null}
        <AskChevron expanded={expanded} />
      </div>
      <div className="col-start-2 flex min-w-0 items-center gap-sm text-caption" data-memory-running={promotion?.state === "running" ? first.number : undefined}>
        {group.pr !== null ? <button type="button" disabled={!group.url} className="pointer-events-auto shrink-0 text-subtle-foreground hover:text-foreground disabled:opacity-50" onClick={() => group.url && actions.openSessionPullRequest({ workspace_id: project.workspace_id, number: group.pr!, url: group.url })} data-memory-open-pr={group.pr}>{t("memory.progress.pr", { number: String(group.pr) })}</button> : null}
        <span className={cn("min-w-0 truncate", status.tone)} title={reason} data-memory-unconfirmed={promotion?.state === "unconfirmed" ? "true" : undefined}>{reason}</span>
        {promotion?.state === "running" && group.pr === null ? <Elapsed since={promotion.started_at_unix_ms} className="shrink-0 text-muted-foreground" data-memory-elapsed={first.number} /> : null}
        <span className="max-w-1/3 shrink-0 truncate font-mono text-muted-foreground" title={group.cards.map((row) => cardName(row.number)).join(" · ")}>{group.cards.map((row) => cardName(row.number)).join(" · ")}</span>
        {(status.agentLink || expanded) && pane ? <button type="button" className="pointer-events-auto ml-auto shrink-0 text-subtle-foreground hover:text-foreground" aria-label={t("memory.progress.openAgent")} onClick={() => actions.openAgent(pane)} data-memory-agent={first.number}>{t("memory.progress.agent")}</button> : null}
      </div>
    </AskRow>
    {expanded ? <div className="grid min-w-0 grid-cols-[auto_1fr] gap-x-sm pt-xs pb-sm" data-memory-progress-detail={first.number}>
      <span aria-hidden="true" className="size-(--size-pr-icon)" />
      <div className="flex min-w-0 flex-col gap-sm">
        {group.cards.map((row) => <div key={row.number} className="min-w-0" data-memory-preview={row.number}>
          {group.cards.length > 1 ? <p className="break-words text-caption text-subtle-foreground"><span className="font-mono text-muted-foreground">{cardName(row.number)}</span> · {row.title}</p> : null}
          {row.body_preview ? <p className="line-clamp-2 whitespace-pre-wrap break-words text-caption text-muted-foreground">{row.body_preview}</p> : null}
        </div>)}
      </div>
    </div> : null}
  </div>;
}

/** A failed question's status stays on the card where the operator can retry. */
function failureKey(failure: MemoryStartFailure | null): MessageKey {
  switch (failure?.code) {
    case undefined: return "memory.failed";
    case "shell_starting": return "memory.startFailure.shellStarting";
    case "agent_not_started": return "memory.startFailure.agentNotStarted";
    case "card_not_processed": return "memory.startFailure.cardNotProcessed";
    case "other": return "memory.startFailure.other";
  }
}

function Question({ question, project, actions, busy, last, selected, selecting, selectionDisabled, onSelect }: { question: MemoryQuestion; project: ProjectMemory; actions: Actions; busy: boolean; last: boolean; selected: boolean; selecting: boolean; selectionDisabled: boolean; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  const id = project.workspace_id;
  const compare = question.kind === "docs_disagree";
  const ask = compare && question.document.authority === "ask";
  const expanded = project.expanded?.number === question.number ? project.expanded : null;
  const failed = question.kind === "failed";
  const unbuilt = question.kind === "unbuilt";
  // What the agent saw stays behind the row; its line names the state.
  const told = question.kind === "unbuilt" ? question.reason : question.kind === "failed" && question.start_failure?.code === "card_not_processed" ? question.start_failure.detail : null;
  const reason = question.kind === "promote" ? [question.human_session_count ? t("memory.promote.human", { count: question.human_session_count }) : null, question.pulled_session_count ? t("memory.promote.sessions", { count: question.pulled_session_count }) : null].filter(Boolean).join(" · ") || t("memory.promote.ask") : question.kind === "docs_disagree" ? t(ask ? "memory.docs.ask" : "memory.docs.newer") : question.kind === "unbuilt" ? t("memory.unbuilt") : question.start_failure?.code === "shell_starting" && question.start_failure.detail ? t("memory.startFailure.shellQuestion", { line: question.start_failure.detail }) : question.start_failure?.code === "card_not_processed" ? `${t("memory.startFailure.cardNotProcessed")} · ${t("memory.failed")}` : question.start_failure?.detail ?? t(failureKey(question.start_failure));
  return (
    <div className={cn("min-w-0", !last && "border-b border-border")} data-memory-question={question.kind} data-memory-card={question.number}>
      <AskRow number={question.number} expanded={!!expanded} onToggle={() => actions.memoryExpand(id, expanded ? null : question.number)}>
      {unbuilt ? <ClockIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) text-muted-foreground" data-memory-unbuilt={question.number} /> : <Checkbox className="pointer-events-auto mt-xxs" checked={selected} disabled={busy || selectionDisabled} onCheckedChange={onSelect} aria-label={t("memory.batch.select", { card: cardName(question.number) })} data-memory-select={question.number} />}
        <div className="flex min-w-0 items-center gap-sm">
          <span className="shrink-0 font-mono text-caption text-muted-foreground">{cardName(question.number)}</span>
          <span className="min-w-0 flex-1 truncate text-body text-foreground" title={question.title}>{question.title}</span>
        </div>
        <div className="col-start-2 flex min-w-0 items-center gap-sm">
          <span className={cn("min-w-0 flex-1 truncate text-caption", failed ? "text-warning" : "text-muted-foreground")} title={reason} data-memory-failure={failed ? "true" : undefined}>{reason}</span>
          {compare ? <button type="button" className="pointer-events-auto shrink-0 text-caption text-subtle-foreground hover:text-foreground" aria-expanded={!!expanded} onClick={() => actions.memoryExpand(id, expanded ? null : question.number)} data-memory-compare={question.number}>{t("memory.docs.compare")}</button> : null}
          <AskChevron expanded={!!expanded} />
        </div>
        {!selecting || ask ? <div className="col-start-2 flex flex-wrap items-center gap-xs">
          {unbuilt ? <Button className="pointer-events-auto" size="sm" variant="outline" disabled={busy} onClick={() => actions.memoryAction(id, "dismiss", question.number)} data-memory-keep={question.number}>{t("memory.unbuilt.keep")}</Button> : <>
            <Button className="pointer-events-auto" size="sm" variant="outline" disabled={busy} onClick={() => actions.memoryAction(id, ask ? "card_right" : "promote", question.number)} data-memory-promote={failed ? undefined : question.number} data-memory-retry={failed ? question.number : undefined}>
              {t(ask ? "memory.docs.cardRight" : compare ? "memory.docs.fix" : failed ? "memory.retry" : "memory.promote.yes")}
            </Button>
            <Button className="pointer-events-auto" size="sm" variant="ghost" disabled={busy} onClick={() => actions.memoryAction(id, compare ? "doc_right" : "dismiss", question.number)} data-memory-dismiss={question.number}>{t(ask ? "memory.docs.docRight" : compare ? "memory.docs.cardWrong" : "memory.promote.no")}</Button>
          </>}
        </div> : null}
      </AskRow>
      {expanded ? <div className="flex min-w-0 flex-col gap-sm pb-sm pl-lg">{told ? <p className="break-words text-caption text-subtle-foreground" data-memory-told={question.number}>{t("memory.agentReason", { reason: told })}</p> : null}{compare && expanded.document_comparison ? <DocumentCompare comparison={expanded.document_comparison} /> : <CardDetail card={expanded} project={project} actions={actions} busy={busy} />}</div> : null}
    </div>
  );
}

function DocumentName({ path }: { path: string }) {
  return <span className="min-w-0 truncate font-mono" title={path} data-memory-document="true">{path.split("/").at(-1) || path}</span>;
}

function DocumentCompare({ comparison }: { comparison: NonNullable<MemoryCard["document_comparison"]> }) {
  const { t, i18n } = useInterfaceTranslation();
  const date = (at: number | null) => at === null ? t("memory.docs.unknownDate") : formatDateTime(requireInterfaceLanguage(i18n.language), at, { year:"numeric", month:"short", day:"numeric" });
  return (
    <div className="flex min-w-0 flex-col gap-sm border-l border-border pl-sm text-caption" data-memory-comparison="true">
      <div><p className="flex min-w-0 items-center gap-xs text-muted-foreground"><DocumentName path={comparison.document} /><span className="shrink-0 whitespace-nowrap">· {date(comparison.document_at_unix_ms)}</span></p><p className="whitespace-pre-wrap break-words text-subtle-foreground">{comparison.document_text}</p></div>
      <div><p className="text-muted-foreground">{t("memory.humanSource")} · {date(comparison.operator_at_unix_ms)}</p><p className="whitespace-pre-wrap break-words text-subtle-foreground">{comparison.operator_text || t("memory.docs.sourceUnavailable")}</p></div>
    </div>
  );
}

function Cards({ project, actions, busy, onDelete }: { project: ProjectMemory; actions: Actions; busy: boolean; onDelete: () => void }) {
  const { t } = useInterfaceTranslation();
  const id = project.workspace_id;
  // The search is the Project's, shared by every window: only a local edit is
  // sent, and otherwise the field follows the accepted query, so two windows
  // never send each other's answer back.
  const shared = project.search?.query ?? "";
  const [query, setQuery] = useState(shared);
  const editing = useRef(false);
  const [all, setAll] = useState(false);
  useEffect(() => {
    // The core trims the words; a field that differs only by that keeps the
    // space the operator just typed.
    if (!editing.current) setQuery((current) => (current.trim() === shared ? current : shared));
  }, [shared]);
  useEffect(() => {
    if (!editing.current) return;
    const timer = window.setTimeout(() => {
      editing.current = false;
      if (query !== shared) actions.memorySearch(id, query);
    }, 220);
    return () => window.clearTimeout(timer);
  }, [actions, id, query, shared]);
  const cards = memoryCards(project);
  // Every card is already asked about above (a failed promotion of the
  // Project's one card): the list would be an empty search (design v7, 5).
  if (project.rows.length > 0 && cards.length === 0 && project.search === null) return null;
  const { shown, folded } = visibleCards(cards, project.expanded?.number ?? null, all || project.search !== null);
  const checking = project.analysis === "checking" || project.analysis === "check_budget_exhausted";
  const waiting = project.rows.some((row) => row.verifying);
  const noUsable = project.usable_cards === 0 && project.analysis !== "ai_not_ready" && (project.rows.length === 0 || waiting);
  return (
    <section aria-label={t("memory.cards")} className="flex flex-col gap-sm" data-memory-cards={project.rows.length}>
      <h3 className="flex items-center gap-sm text-caption">
        <span className="text-subtle-foreground">{t("memory.cards")}</span>
        <span className="text-muted-foreground">{project.rows.length}</span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="icon-sm" variant="ghost" className="ml-auto" aria-label={t("memory.menu")} data-memory-menu="true">
              <MoreHorizontalIcon aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem variant="destructive" disabled={busy || project.rows.length === 0} onSelect={onDelete} data-memory-delete="menu">
              <Trash2Icon aria-hidden="true" />
              {t("memory.deleteData")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </h3>
      {noUsable ? <MemoryEmpty project={project} /> : project.enabled && project.analysis !== "ai_not_ready" && (checking || waiting) ? <MemoryExplanation project={project} /> : null}
      {project.rows.length === 0 ? null : (
        <>
          <Input value={query} onChange={(event) => { editing.current = true; setQuery(event.target.value); }} placeholder={t("memory.search")} aria-label={t("memory.search")} data-memory-search="true" />
          {project.search && cards.length === 0 ? <p className="text-caption text-muted-foreground">{t("memory.noMatch")}</p> : null}
          <ol className="flex flex-col">
            {shown.map((row) => (
              <CardRow key={row.number} row={row} project={project} actions={actions} busy={busy} />
            ))}
          </ol>
          {folded > 0 ? (
            <button type="button" className="flex items-center gap-xs self-start text-caption text-subtle-foreground hover:text-foreground" onClick={() => setAll(true)} data-memory-more={folded}>
              <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />
              {t("memory.more", { count: folded })}
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}

function MemoryEmpty({ project }: { project: ProjectMemory }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex flex-col gap-xs" data-memory-empty="true">
      <span className="flex items-center gap-xs text-body text-foreground">
        <SparklesIcon aria-hidden="true" className="size-(--size-icon-sm) text-muted-foreground" />
        {t(project.rows.length > 0 ? "memory.unusable.title" : "memory.empty.title")}
      </span>
      <MemoryExplanation project={project} />
    </div>
  );
}

/** One shared reason above the list; only a running card gets its own mark. */
function MemoryExplanation({ project }: { project: ProjectMemory }) {
  const { t, i18n } = useInterfaceTranslation();
  const state = project.analysis;
  let message: string;
  if (state === "check_budget_exhausted") {
    if (project.check_resume_at_unix_ms === null) throw new Error("memory_check_reset_missing");
    const time = formatDateTime(requireInterfaceLanguage(i18n.language), project.check_resume_at_unix_ms, { month: "long", day: "numeric", hour: "numeric", minute: "2-digit" });
    message = t("memory.budget.body", { time });
  } else {
    message = t(state === "checking" ? "memory.checking.body" : project.rows.length > 0 ? "memory.pending.body" : "memory.empty.body");
  }
  return <p className="break-keep text-caption text-muted-foreground" data-memory-checks={project.enabled && (state === "checking" || state === "check_budget_exhausted" || project.rows.some((row) => row.verifying)) ? "true" : undefined}>{message}</p>;
}

/** The chevron that shows a card in full or folds it. */
function CardToggle({ number, project, actions }: { number: number; project: ProjectMemory; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const open = project.expanded?.number === number;
  const Icon = open ? ChevronDownIcon : ChevronRightIcon;
  return (
    <button
      type="button"
      aria-expanded={open}
      aria-label={t(open ? "memory.collapse" : "memory.expand", { card: cardName(number) })}
      className="mt-xxs flex size-(--size-icon) items-center justify-center rounded-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
      onClick={() => actions.memoryExpand(project.workspace_id, open ? null : number)}
    >
      <Icon aria-hidden="true" className="size-(--size-icon-sm)" />
    </button>
  );
}

/** Promotion states that already draw their own status line under a card. */
const STATUS_STATES: ReadonlySet<string | undefined> = new Set(["running", "pr_open", "unconfirmed"]);

function CardRow({ row, project, actions, busy }: { row: MemoryRow; project: ProjectMemory; actions: Actions; busy: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const open = project.expanded?.number === row.number;
  const merged = row.promotion?.state === "merged";
  const outdated = row.document_comparison?.authority === "outdated";
  const checking = project.analysis === "checking" && project.checking_card === row.number;
  const waiting = row.verifying && !checking && !STATUS_STATES.has(row.promotion?.state);
  const self = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (open) self.current?.scrollIntoView({ block: "nearest" });
  }, [open]);
  return (
    <li ref={self} className={cn("grid grid-cols-[auto_auto_1fr] gap-x-sm rounded-md px-xs py-sm", open && "bg-card", (merged || waiting || outdated) && !open && "opacity-(--opacity-dimmed)")} data-memory-card={row.number} data-memory-promotion={row.promotion?.state}>
      <CardToggle number={row.number} project={project} actions={actions} />
      <span className="font-mono text-caption text-muted-foreground">{cardName(row.number)}</span>
      <div className="flex min-w-0 flex-col gap-xs">
        <button type="button" className="line-clamp-2 break-words break-keep text-left text-body text-foreground" onClick={() => actions.memoryExpand(project.workspace_id, open ? null : row.number)}>
          {row.title}
        </button>
        <span className="flex flex-wrap items-center gap-x-sm gap-y-xxs text-caption text-muted-foreground">
          <span className="rounded-xs bg-secondary px-xxs">{t(KIND[row.kind])}</span>
          {outdated ? <span className="flex w-full min-w-0 items-center gap-xs" data-memory-outdated={row.number}><span className="shrink-0 whitespace-nowrap">{t("memory.docs.outdated")} ·</span><DocumentName path={row.document_comparison!.document} />{row.document_comparison!.document_at_unix_ms ? <span className="shrink-0 whitespace-nowrap">· {formatDateTime(language, row.document_comparison!.document_at_unix_ms, { year:"numeric", month:"short", day:"numeric" })}</span> : null}</span> : null}
          {merged ? (
            <>
              {row.promotion?.target_path ? (
                <span className="flex min-w-0 rounded-xs bg-secondary px-xxs"><DocumentName path={row.promotion.target_path} /></span>
              ) : row.promotion?.pr_number != null ? (
                // No file was named: the pull request it went in by.
                <PrMark state="merged" number={row.promotion.pr_number} />
              ) : null}
              <span>{t("memory.merged")}</span>
            </>
          ) : (
            <>
              {row.provided_session_count ? <span>{t("memory.provided", { count: row.provided_session_count })}</span> : null}
              {row.pull_count ? <span>{t("memory.pulled", { count: row.pull_count })}</span> : null}
              <span>{t("memory.human", { count: row.human_source_count })}</span>
              <span className="ml-auto shrink-0 whitespace-nowrap font-mono">{memoryTime(row.updated_at_unix_ms, language)}</span>
            </>
          )}
        </span>
        <PromotionStatus row={row} actions={actions} />
        {checking && !STATUS_STATES.has(row.promotion?.state) ? (
          // Quiet and alone: the operator has nothing to do (design 13).
          <span className="text-caption text-muted-foreground" data-memory-verifying={row.number}>
            <span className="whitespace-nowrap">{t("memory.verifying")}</span>
          </span>
        ) : null}
        {row.promotion?.state === "pr_open" && row.promotion.pr_number !== null ? (
          <span className="flex items-center gap-sm text-caption text-subtle-foreground">
            <button type="button" disabled={!row.promotion.pr_url} aria-label={t("memory.openPr", { number: String(row.promotion.pr_number) })} className="rounded-xs outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => row.promotion?.pr_url && actions.openSessionPullRequest({ workspace_id: project.workspace_id, number: row.promotion.pr_number!, url: row.promotion.pr_url })}>
              <PrMark state="pending" number={row.promotion.pr_number} />
            </button>
            {t("memory.prOpen")}
          </span>
        ) : null}
        {open && project.expanded ? <CardDetail card={project.expanded} project={project} actions={actions} busy={busy} /> : null}
      </div>
    </li>
  );
}

function PromotionStatus({ row, actions }: { row: MemoryRow; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return <>
        {row.promotion?.state === "running" ? (
          <span className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground" data-memory-running={row.number}>
            <LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 animate-spin" />
            <span className="shrink-0 whitespace-nowrap">{t("memory.running")}</span>
            {row.promotion.agent ? <span aria-hidden="true">·</span> : null}
            {row.promotion.agent ? (
              row.promotion.agent.pane_id ? (
                <button type="button" className="min-w-0 truncate font-mono underline underline-offset-4 hover:text-foreground" onClick={() => actions.openAgent(row.promotion!.agent!.pane_id!)}>
                  {row.promotion.agent.name}
                </button>
              ) : (
                <span className="min-w-0 truncate font-mono">{row.promotion.agent.name}</span>
              )
            ) : null}
          </span>
        ) : null}
        {row.promotion?.state === "unconfirmed" ? (
          // The agent is gone and GitHub cannot say whether its pull request
          // came; restoring GitHub settles it, so nothing here asks to act.
          <span className="flex min-w-0 items-center gap-xs text-caption text-muted-foreground" data-memory-unconfirmed={row.number}>
            <span className="shrink-0 whitespace-nowrap">{t("memory.unconfirmed")}</span>
            {row.promotion.github_failure && row.promotion.github_failure !== "no_github_remote" ? (
              <>
                <span aria-hidden="true">·</span>
                <span className="min-w-0 truncate">{t(`memory.github.${row.promotion.github_failure}`)}</span>
              </>
            ) : null}
          </span>
        ) : null}
  </>;
}

/** A card in full, in place: what changed, the body, cited files, sources and Forget. */
function CardDetail({ card, project, actions, busy }: { card: MemoryCard; project: ProjectMemory; actions: Actions; busy: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const [previous, setPrevious] = useState(false);
  const humans = card.sources.filter((source) => source.human).length;
  const id = project.workspace_id;
  return (
    <div className="flex flex-col gap-md pt-xs" data-memory-detail={card.number}>
      {card.previous ? (
        <div className="flex flex-col gap-xs">
          <span className="text-caption text-muted-foreground">
            {t("memory.changed", { time: memoryTime(card.previous.changed_at_unix_ms, language) })}
            {" · "}
            <button type="button" aria-expanded={previous} className="underline underline-offset-4 hover:text-foreground" onClick={() => setPrevious(!previous)} data-memory-previous={card.number}>
              {t("memory.previous")}
            </button>
          </span>
          {previous ? (
            <div className="flex flex-col items-start gap-xs border-l border-border pl-sm">
              <p className="whitespace-pre-wrap break-words text-caption text-muted-foreground">{card.previous.body}</p>
              <Button size="sm" variant="outline" disabled={busy} onClick={() => actions.memoryAction(id, "undo", card.number)} data-memory-undo={card.number}>
                <Undo2Icon aria-hidden="true" />
                {t("memory.undo")}
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
      <p className="whitespace-pre-wrap break-words break-keep text-body text-foreground">{card.body}</p>
      {card.document_comparison ? <DocumentCompare comparison={card.document_comparison} /> : null}
      {card.cited_paths.length > 0 ? (
        <div className="flex flex-col gap-xxs">
          <span className="text-caption text-muted-foreground">{t("memory.citedFiles")}</span>
          {card.cited_paths.map((path) => (
            <span key={path} className="flex items-center gap-xs font-mono text-caption text-foreground">
              <FileIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
              <span className="min-w-0 truncate">{path}</span>
            </span>
          ))}
        </div>
      ) : null}
      {card.sources.length > 0 ? (
        <div className="flex flex-col gap-xxs">
          <span className="text-caption text-muted-foreground">{t("memory.sources", { count: card.sources.length, human: humans })}</span>
          {card.sources.map((source, index) => (
            <span key={index} className="grid grid-cols-[auto_1fr_auto_auto] items-center gap-x-sm text-caption">
              <UserIcon aria-hidden="true" className={cn("size-(--size-icon-sm)", source.human ? "text-foreground" : "text-muted-foreground")} />
              <span className="min-w-0 truncate text-subtle-foreground">{source.provider_label}</span>
              <span className="font-mono text-muted-foreground">{source.at_unix_ms ? sourceTime(source.at_unix_ms, language) : ""}</span>
              <span className="text-foreground">{source.human ? t("memory.humanSource") : ""}</span>
            </span>
          ))}
        </div>
      ) : null}
      <Button size="sm" variant="ghost" className="self-end text-destructive hover:text-destructive" disabled={busy} onClick={() => actions.memoryAction(id, "forget", card.number)} data-memory-forget={card.number}>
        <Trash2Icon aria-hidden="true" />
        {t("memory.forget")}
      </Button>
    </div>
  );
}
