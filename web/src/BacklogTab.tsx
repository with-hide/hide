import { ArrowLeftIcon, ChevronDownIcon, ChevronRightIcon, ExternalLinkIcon, FlagIcon, ListFilterIcon, LoaderCircleIcon, PlayIcon, PlusIcon, SearchIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./components/ui/select";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import { readsHere } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { summariesEnabled } from "./hideAi";
import { cn } from "./lib/utils";
import { ageWords } from "./overviewLens";
import { originOf, useBacklogOrigin } from "./backlogOrigin";
import { BACKLOG_FILTERS, backlogLabels, backlogOf, BACKLOG_SHOWN, filterRows, pendingItems, startedKeys, undoRetryable, type BacklogFilters, type BacklogRow } from "./overviewBacklog";
import { isMemoKey, itemToken, projectOrder } from "./overviewTasks";
import { issueDate, type BoardProject } from "./projectBoard";
import { MissedRow } from "./SessionRecord";
import { MISSED_FOLD, openMissedWork, workKey } from "./sessionWork";
import type { BacklogItem, Workspace } from "./snapshot";
import { useShellStore } from "./store";
import { IssueLabelDot, IssueLabelView } from "./issueMarks";
import { useIssueDetail } from "./issueDetails";
import { MarkdownText } from "./MarkdownText";
import { toggledFold, useUiStore, type OverviewPanel } from "./ui";
import { issueSourceName } from "./settings";

const LINK = "inline-flex whitespace-nowrap items-center gap-xxs rounded-xs px-xxs text-caption outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring";

function matchingSources(project: Workspace, source: BacklogFilters["source"]) {
  const sources = project.tasks?.sources ?? (project.tasks?.source ? [project.tasks.source] : []);
  return sources.filter((item) => source === "all" || item.kind === source);
}

/** Unread or failed sources stay actionable even before they have a row. */
export function visibleBacklogProjects(projects: readonly Workspace[], query: string, labels: readonly string[], filters: BacklogFilters, made: readonly BacklogItem[] | undefined): Workspace[] {
  return projects.filter((project) => filterRows(backlogOf(project, true).rows, query, labels, filters).length || pendingItems(made, project).length || matchingSources(project, filters.source).some((source) => source.failure || source.reading));
}

/** Mixed, source-qualified rows. Filters run before each project's 50-row page. */
export function OverviewBacklog({ projects, actions }: { projects: readonly BoardProject[]; deviceId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const only = useUiStore((s) => s.overview.projectId);
  const query = useUiStore((s) => s.overview.backlogQuery);
  const labels = useUiStore((s) => s.overview.backlogLabels);
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const made = useShellStore((s) => s.rest?.issue_work?.backlog);
  const [filters, setFilters] = useState<BacklogFilters>(BACKLOG_FILTERS);
  const scoped = useMemo(() => projects.filter(({ workspace }) => !only || workspace.id === only), [projects, only]);
  const order = useMemo(() => projectOrder(projects, only), [projects, only]);
  const rows = useMemo(() => scoped.flatMap(({ workspace }) => backlogOf(workspace, true).rows), [scoped]);
  const counted = filterRows(rows, query, labels, { ...filters, status: "all" });
  const counts = { open: counted.filter((row) => row.open && !row.started).length, started: counted.filter((row) => row.open && row.started).length, all: counted.length };
  const matching = filterRows(rows, query, labels, filters);
  const visible = only ? order : visibleBacklogProjects(order, query, labels, filters, made);
  return <div className="flex min-w-0 flex-col gap-lg px-xxl py-xl overview-narrow:px-md" data-overview-backlog="true">
    <div className="flex flex-wrap items-center gap-sm"><h2 className="text-headline font-semibold">{t("overviewTasks.backlogTab")}</h2><span className="rounded-sm border border-border px-sm font-mono text-body text-muted-foreground">{matching.length}</span></div>
    <MissedGroup projects={scoped} actions={actions} />
    <div className="min-w-0 rounded-lg border border-border">
    <div className="flex min-w-0 flex-col gap-md rounded-t-lg border-b border-border bg-muted/30 px-lg py-lg" data-backlog-toolbar="true">
      <div className="flex flex-wrap items-center gap-sm">
      <div className="flex flex-wrap items-center gap-xxs" role="group" aria-label={t("backlogView.status")}>
        {(["open", "started", "all"] as const).map((status) => <Button key={status} size="sm" variant={filters.status === status ? "secondary" : "ghost"} aria-pressed={filters.status === status} data-backlog-status={status} onClick={() => setFilters({ ...filters, status })}>{t(`backlogView.${status}`)} <span className="font-mono text-micro text-muted-foreground">{counts[status]}</span></Button>)}
      </div>
      <div className="ml-auto flex items-center gap-xxs rounded-md border border-border bg-background p-xxs" role="group" aria-label={t("backlogView.source")}>
        {(["all", "github", "local"] as const).map((source) => <Button key={source} size="sm" variant={filters.source === source ? "outline" : "ghost"} aria-pressed={filters.source === source} data-backlog-source={source} onClick={() => setFilters({ ...filters, source })}>{t(source === "all" ? "backlogView.allSources" : source === "local" ? "backlogView.memo" : "backlog.sourceGithub")}</Button>)}
      </div>
      </div>
      <div className="flex min-w-0 items-center gap-sm">
      <div className="flex min-w-0 flex-1 items-center gap-xs"><SearchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /><Input value={query} onChange={(event) => useUiStore.getState().setOverview({ backlogQuery: event.target.value })} placeholder={t("backlogView.search")} aria-label={t("backlogView.search")} className="min-w-0" data-backlog-query="true" /></div>
      <Popover>
        <PopoverTrigger asChild><Button size="sm" variant="ghost" data-backlog-filters="true"><ListFilterIcon />{t("backlogView.filters")}{labels.length || filters.author || filters.assignee || filters.status === "closed" ? <span aria-hidden="true">·</span> : null}</Button></PopoverTrigger>
        <PopoverContent align="start" className="flex max-h-(--radix-popover-content-available-height) flex-col gap-md overflow-y-auto">
          <FilterSelect label={t("backlogView.status")} value={filters.status} options={["open", "started", "closed", "all"].map((value) => ({ value, label: t(`backlogView.${value as BacklogFilters["status"]}`) }))} onChange={(status) => setFilters({ ...filters, status: status as BacklogFilters["status"] })} />
          <FilterSelect label={t("backlogView.author")} value={filters.author || "all"} options={[{ value: "all", label: t("backlogView.all") }, ...[...new Set(rows.flatMap((row) => row.author ? [row.author] : []))].sort().map((value) => ({ value, label: value }))]} onChange={(author) => setFilters({ ...filters, author: author === "all" ? "" : author })} />
          <FilterSelect label={t("backlogView.assignee")} value={filters.assignee || "all"} options={[{ value: "all", label: t("backlogView.all") }, ...[...new Set(rows.flatMap((row) => row.assignees))].sort().map((value) => ({ value, label: value }))]} onChange={(assignee) => setFilters({ ...filters, assignee: assignee === "all" ? "" : assignee })} />
          <span className="text-caption text-muted-foreground">{t("backlogView.labels")}</span>
          <LabelChoices projects={scoped} />
          <Button size="sm" variant="ghost" onClick={() => { setFilters(BACKLOG_FILTERS); useUiStore.getState().setOverview({ backlogLabels: [], backlogQuery: "" }); }}>{t("backlogView.reset")}</Button>
        </PopoverContent>
      </Popover>
      <Button size="sm" onClick={() => actions.openQuickAdd()} data-backlog-add="true"><PlusIcon />{t("backlogView.add")}</Button>
      </div>
    </div>
    <div className="overflow-x-auto"><table className="w-full min-w-(--size-editor-min-content) table-fixed text-left text-body" data-backlog-table="true">
      <colgroup><col className="w-[calc(var(--size-control-lg)+var(--spacing-lg))]" /><col /><col className="w-(--size-recent-location-max)" /><col className="w-(--size-recent-location-max)" /><col className="w-(--size-recent-location-max)" /><col className="w-(--size-recent-location-max)" /></colgroup>
      <thead><tr className="border-b border-border bg-muted/30 text-micro text-muted-foreground"><th className="px-sm py-md font-normal">{t("backlogView.id")}</th><th className="px-lg py-md font-normal">{t("backlogView.title")}</th><th className="px-sm py-md font-normal">{t("backlogView.assignee")}</th><th className="px-sm py-md font-normal">{t("backlogView.status")}</th><th className="px-sm py-md font-normal">{t("backlogView.updated")}</th><th aria-label={t("backlog.start")} /></tr></thead>
      {visible.map((project) => readsHere(project, devices) ? <BacklogSection key={project.id} project={project} actions={actions} filters={filters} selected={Boolean(only)} /> : <DialedSection key={project.id} project={project} />)}
    </table></div>
    {!only && !matching.length ? <div className="flex flex-col items-center gap-md px-lg py-xxxl" data-backlog-empty="true"><p className="text-body text-muted-foreground">{t(rows.length ? "backlog.noMatch" : "backlog.empty")}</p><Button size="sm" variant="outline" onClick={() => { setFilters(BACKLOG_FILTERS); useUiStore.getState().setOverview({ backlogLabels: [], backlogQuery: "" }); }}>{t("backlogView.reset")}</Button></div> : null}
    </div>
  </div>;
}

function FilterSelect({ label, value, options, onChange }: { label: string; value: string; options: { value: string; label: string }[]; onChange: (value: string) => void }) {
  return <div className="flex flex-col gap-xs"><span className="text-caption text-muted-foreground">{label}</span><Select value={value} onValueChange={onChange}><SelectTrigger aria-label={label}><SelectValue /></SelectTrigger><SelectContent>{options.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent></Select></div>;
}

function LabelChoices({ projects }: { projects: readonly BoardProject[] }) {
  const { t } = useInterfaceTranslation();
  const picked = useUiStore((s) => s.overview.backlogLabels);
  const labels = useMemo(() => backlogLabels(projects), [projects]);
  return <ToggleGroup type="multiple" value={[...picked]} onValueChange={(names) => useUiStore.getState().setOverview({ backlogLabels: names })} aria-label={t("backlog.labels")} className="w-full flex-wrap">{labels.map((label) => <ToggleGroupItem key={label.name} value={label.name} size="sm" className="min-w-0 max-w-full" data-backlog-label={label.name}><IssueLabelDot color={label.color} /><span className="min-w-0 truncate">{label.name}</span></ToggleGroupItem>)}</ToggleGroup>;
}

function BacklogSection({ project, actions, filters, selected }: { project: Workspace; actions: Actions; filters: BacklogFilters; selected: boolean }) {
  const { t } = useInterfaceTranslation();
  const query = useUiStore((s) => s.overview.backlogQuery);
  const labels = useUiStore((s) => s.overview.backlogLabels);
  const made = useShellStore((s) => s.rest?.issue_work?.backlog);
  const [folded, setFolded] = useState(false);
  const [pages, setPages] = useState(1);
  const backlog = useMemo(() => backlogOf(project, true), [project]);
  const pending = useMemo(() => pendingItems(made, project), [made, project]);
  const rows = useMemo(() => filterRows(backlog.rows, query, labels, filters), [backlog.rows, query, labels, filters]);
  useEffect(() => setPages(1), [query, labels, filters]);
  const shown = rows.slice(0, pages * BACKLOG_SHOWN);
  const sources = matchingSources(project, filters.source);
  return <tbody aria-label={project.label} data-backlog-section={project.id}>
    <tr className="border-b border-border bg-muted/30"><td colSpan={6} className="px-lg py-sm">
      <h3 className="flex min-w-0 flex-wrap items-center gap-sm">
        <button type="button" aria-expanded={!folded} className="flex min-w-0 items-center gap-xs rounded-xs text-caption font-medium outline-none focus-visible:ring-1 focus-visible:ring-ring" onClick={() => setFolded(!folded)} data-backlog-fold={project.id}>{folded ? <ChevronRightIcon className="size-(--size-icon-sm)" /> : <ChevronDownIcon className="size-(--size-icon-sm)" />}<span className="truncate">{project.label}</span></button>
        <span className="font-mono text-micro text-muted-foreground" data-backlog-open={rows.length}>{rows.length}</span>
        <span className="ml-auto text-micro text-muted-foreground">{t("backlogView.counts", { github: rows.filter((row) => !row.local).length, local: rows.filter((row) => row.local).length })}</span>
      </h3>
    </td></tr>
    {sources.map((source) => source.failure || source.reading ? <tr key={source.kind}><td colSpan={6} className="px-lg py-xs">{source.failure ? <p role="status" className="flex flex-wrap items-center gap-xs text-caption text-warning" data-backlog-read-failed={source.kind}><TriangleAlertIcon aria-hidden="true" className="size-(--size-icon-sm)" /><span>{issueSourceName(source, t)} · {t("backlog.readFailed", { reason: source.failure })}</span>{source.kind === "github" ? <button type="button" className={cn(LINK, "underline")} onClick={() => actions.openSettings("github")}>{t("backlog.readFix")}</button> : null}</p> : <p role="status" className="text-caption text-muted-foreground">{issueSourceName(source, t)} · {t("backlogView.reading")}</p>}</td></tr> : null)}
    {!folded ? <>
      {pending.length ? <tr><td colSpan={6} className="px-sm py-xs"><ul>{pending.map((item) => <PendingRow key={item.intent} item={item} project={project} actions={actions} />)}</ul></td></tr> : null}
      {shown.map((row) => <ItemRow key={row.key} row={row} project={project} actions={actions} />)}
      {!shown.length && selected ? <tr><td colSpan={6} className="px-lg py-xl text-body text-muted-foreground">{t(backlog.rows.length ? "backlog.noMatch" : "backlog.empty")}</td></tr> : null}
      {rows.length > BACKLOG_SHOWN ? <tr><td colSpan={6}><div className="flex items-center justify-center gap-sm px-lg py-md"><span className="text-micro text-muted-foreground">{t("backlogView.shown", { shown: shown.length, total: rows.length })}</span>{shown.length < rows.length ? <Button size="sm" variant="ghost" data-backlog-more={project.id} onClick={() => setPages((value) => value + 1)}>{t("backlogView.more", { count: Math.min(BACKLOG_SHOWN, rows.length - shown.length) })}<ChevronDownIcon /></Button> : <Button size="sm" variant="ghost" onClick={() => setPages(1)}>{t("overviewTasks.fold")}</Button>}</div></td></tr> : null}
      {project.tasks?.overflow ? <tr><td colSpan={6}><p role="status" className="px-lg py-sm text-micro text-muted-foreground" data-backlog-overflow="true">{t("backlogView.bounded")}</p></td></tr> : null}
    </> : null}
  </tbody>;
}

function ItemRow({ row, project, actions }: { row: BacklogRow; project: Workspace; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const fromMissed = useShellStore((s) => s.rest?.issue_work?.missed_keys?.some((item) => item.task_key === row.key
    && item.device_id === (row.local ? project.device_id : null)) ?? false);
  const language = requireInterfaceLanguage(i18n.language);
  const checkout = project.checkouts.find((checkout) => checkout.task_key === row.key);
  const open = () => useUiStore.getState().setOverview({ panel: { kind: "item", workspaceId: project.id, taskKey: row.key } });
  return <tr className="group border-b border-border hover:bg-accent" data-backlog-row={row.key}>
    <td className="px-sm py-lg align-top font-mono text-micro text-muted-foreground"><button type="button" className={LINK} onClick={open} aria-label={[row.token, row.title].filter(Boolean).join(" ")}>{row.token}</button></td>
    <td className="min-w-0 px-lg py-lg"><button type="button" className="block w-full truncate rounded-xs text-left text-title font-medium leading-relaxed text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" onClick={open} title={row.title}>{row.title}</button><div className="mt-xs flex min-w-0 flex-wrap items-center gap-xs text-micro text-muted-foreground">{fromMissed ? <FlagIcon aria-label={t("backlog.fromMissed")} className="size-(--size-icon-sm) text-warning" data-backlog-missed="true" /> : null}{row.author ? <span>{row.author}</span> : null}<span>{row.local ? t("backlogView.memo") : "GitHub"}</span>{row.labels.slice(0, 3).map((label) => <IssueLabelView key={label.name} label={label} />)}{row.labels.length > 3 ? <span>+{row.labels.length - 3}</span> : null}</div></td>
    <td className="px-sm py-lg align-top text-micro text-subtle-foreground"><span className="block truncate" title={row.assignees.join(", ")}>{row.assignees.join(", ") || "-"}</span></td>
    <td className="px-sm py-lg align-top"><span className={cn("inline-flex items-center gap-xxs text-micro", !row.open ? "text-muted-foreground" : row.started ? "text-primary" : "text-subtle-foreground")}><span aria-hidden="true" className="size-(--issue-label-dot) rounded-full bg-current" />{t(!row.open ? "backlogView.closed" : row.started ? "backlogView.started" : "backlogView.open")}</span></td>
    <td className="px-sm py-lg align-top font-mono text-micro text-muted-foreground">{row.updated !== null ? <span title={issueDate(row.updated, language)}>{ageWords(language, Date.now() - row.updated, t)}</span> : "-"}</td>
    <td className="px-sm py-lg align-top text-right">{checkout ? <Button size="sm" variant="ghost" onClick={() => { actions.closeOverview(); actions.openWorkspace(project.device_id, project.id, checkout.id); }}>{t("backlogView.openWork")}</Button> : row.open ? <Button size="sm" variant="ghost" onClick={() => useUiStore.getState().setWorkspaceDialog({ kind: "start_issue", workspaceId: project.id, taskKey: row.key })} data-backlog-start={row.key}><PlayIcon />{t("backlog.start")}</Button> : null}</td>
  </tr>;
}

function MissedGroup({ projects, actions }: { projects: readonly BoardProject[]; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  // Kept with the Overview's folds, so the Project tool's 놓친 일 N opens it (PRD lane-two-columns B28).
  const open = useUiStore((s) => s.overview.open.includes(MISSED_FOLD));
  const setOpen = (next: boolean) => {
    const { overview, setOverview } = useUiStore.getState();
    if (next !== overview.open.includes(MISSED_FOLD)) setOverview({ open: toggledFold(overview.open, MISSED_FOLD) });
  };
  const enabled = useShellStore((s) => summariesEnabled(s.rest?.status?.background_ai));
  const links = useShellStore((s) => s.linkSummaries);
  const missed = useMemo(() => (enabled ? openMissedWork(projects.flatMap((project) => links?.projects[project.workspace.id]?.work ?? [])) : []), [enabled, projects, links]);
  if (!missed.length) return null;
  return (
    <section className="col-span-full" aria-label={t("sessionWork.count", { count: missed.length })} data-open-missed="true">
      <button type="button" className="flex items-center gap-xs rounded-xs px-xs py-xs text-caption text-warning outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" aria-expanded={open} onClick={() => setOpen(!open)} data-open-missed-toggle="true">
        <FlagIcon aria-hidden="true" className="size-(--size-icon-sm)" />
        {t("sessionWork.count", { count: missed.length })}
        {open ? <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm)" /> : <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm)" />}
      </button>
      {open ? <ul className="px-xs">{missed.map(({ owner, item }) => <MissedRow key={`${workKey(owner.session)}:${item.id}`} missed={item} record={owner} actions={actions} />)}</ul> : null}
    </section>
  );
}

function DialedSection({ project }: { project: Workspace }) {
  const { t } = useInterfaceTranslation();
  return <tbody aria-label={project.label} data-backlog-section={project.id} data-backlog-dialed="true"><tr><td colSpan={6} className="px-lg py-lg"><h3 className="pb-xs text-subhead font-semibold text-foreground">{project.label}</h3><p className="text-caption text-muted-foreground">{t("backlog.dialed", { project: project.label })}</p></td></tr></tbody>;
}

function PendingRow({ item, project, actions }: { item: BacklogItem; project: Workspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const token = item.task_key ? itemToken(item.task_key, project) : null;
  return (
    <li className={cn("flex min-w-0 items-center gap-sm rounded-sm px-sm py-xs text-caption", item.phase !== "failed" && "opacity-(--opacity-dimmed)")} data-backlog-pending={item.phase} data-backlog-intent={item.intent}>
      <span className="min-w-(--size-control-lg) shrink-0 font-mono text-muted-foreground">{token ?? ""}</span>
      <span className="min-w-0 flex-1 truncate text-foreground">{item.title}</span>
      {item.phase === "failed" ? (
        <>
          <span className="shrink-0 text-destructive" data-backlog-failure={item.failure ?? "refused"}>{t(`backlog.failure.${item.failure ?? "refused"}`)}</span>
          {item.retry ? <Button size="sm" variant="ghost" onClick={() => actions.retryBacklog(item.intent)} data-backlog-retry={item.intent}>{t("common.retry")}</Button> : null}
          <Button size="sm" variant="ghost" onClick={() => actions.dismissBacklog(item.intent)} data-backlog-dismiss={item.intent}>{t("backlog.remove")}</Button>
        </>
      ) : item.phase === "created" ? null : (
        <span className="flex shrink-0 items-center gap-xxs text-muted-foreground"><LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />{t("backlog.creating")}</span>
      )}
    </li>
  );
}

export function BacklogLabelFilter({ projects }: { projects: readonly BoardProject[] }) {
  const { t } = useInterfaceTranslation();
  const picked = useUiStore((s) => s.overview.backlogLabels);
  const labels = useMemo(() => backlogLabels(projects), [projects]);
  if (labels.length === 0) return null;
  return (
    <Popover>
      <Hint label={t("backlog.labels")}>
        <PopoverTrigger asChild>
          <Button variant={picked.length ? "secondary" : "ghost"} size="icon-sm" aria-label={t("backlog.labels")} data-backlog-labels={picked.length ? "active" : "none"}>
            <ListFilterIcon aria-hidden="true" />
          </Button>
        </PopoverTrigger>
      </Hint>
      <PopoverContent align="end" className="flex max-h-(--radix-popover-content-available-height) flex-col gap-sm overflow-y-auto">
        <ToggleGroup type="multiple" value={[...picked]} onValueChange={(names) => useUiStore.getState().setOverview({ backlogLabels: names })} aria-label={t("backlog.labels")} className="w-full flex-wrap">
          {labels.map((label) => (
            <ToggleGroupItem key={label.name} value={label.name} size="sm" className="min-w-0 max-w-full" data-backlog-label={label.name}>
              <IssueLabelDot color={label.color} />
              <span className="min-w-0 truncate">{label.name}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </PopoverContent>
    </Popover>
  );
}

/** Full-width read view. The parent retains the mounted list and its scroll. */
export function BacklogItemPanel({ panel, projects, actions }: { panel: Extract<OverviewPanel, { kind: "item" }>; projects: readonly BoardProject[]; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const project = projects.find(({ workspace }) => workspace.id === panel.workspaceId)?.workspace ?? null;
  const task = project?.tasks?.tasks.find((row) => row.key === panel.taskKey) ?? null;
  const detail = useIssueDetail(actions, project, panel.taskKey);
  const origin = useShellStore((s) => originOf(s.rest, project, panel.taskKey));
  const undo = useShellStore((s) => {
    const result = s.rest?.issue_work?.undo;
    return result?.workspace_id === project?.id && result?.device_id === project?.device_id
      && result?.task_key === panel.taskKey ? result : null;
  });
  const madeByHide = useShellStore((s) => s.rest?.issue_work?.made_keys?.some((key) => key.toLowerCase() === panel.taskKey.toLowerCase()) ?? false);
  const language = requireInterfaceLanguage(i18n.language);
  useBacklogOrigin(actions, project, panel.taskKey);
  useEffect(() => { if (undo?.phase === "done") useUiStore.getState().setOverview({ panel: null }); }, [undo?.phase]);
  const close = () => useUiStore.getState().setOverview({ panel: null });
  const head = <div className="flex shrink-0 items-center border-b border-border px-lg py-sm"><Button variant="ghost" size="sm" onClick={close} data-item-close="true"><ArrowLeftIcon />{t("backlogView.back")}</Button></div>;
  if (!project || !task) return <div className="flex min-h-0 flex-1 flex-col" data-backlog-item="missing">{head}<p role="status" className="px-lg py-md text-caption text-muted-foreground">{t("backlog.panel.missing")}</p></div>;
  const started = startedKeys(project).has(task.key);
  const local = isMemoKey(task.key);
  const undoable = task.open && !started && (local || madeByHide);
  const at = detail?.created_at_unix_ms ?? task.created_at_unix_ms ?? null;
  const labels = detail?.phase === "ready" ? detail.labels : task.labels ?? [];
  const assignees = detail?.phase === "ready" ? detail.assignees : task.assignees ?? [];
  const author = detail?.author ?? task.author;
  const repository = local ? null : task.key.slice("github:".length).split("#")[0];
  const checkout = project.checkouts.find((checkout) => checkout.task_key === task.key);
  return <div className="flex min-h-0 flex-1 flex-col" data-backlog-item={task.key}>
    {head}
    <div className="min-h-0 flex-1 overflow-y-auto px-xxl py-xl text-body [overflow-wrap:anywhere] overview-narrow:px-md">
      <div className="mx-auto max-w-[calc(var(--size-editor-document)+var(--size-tree-ideal)+var(--spacing-xl))]">
      <p className="mb-md flex flex-wrap items-center gap-sm text-micro text-muted-foreground"><span>{project.label}</span><span>{local ? t("backlogView.memo") : "GitHub"}</span><span className="font-mono">{task.id}</span></p>
      <h2 className="whitespace-pre-wrap text-(length:--text-editor-heading-1) font-semibold leading-snug text-foreground">{task.title}</h2>
      <div className="my-xl flex flex-wrap items-center gap-sm"><span className={cn("rounded-sm bg-secondary px-sm py-xs text-caption", task.open ? "text-primary" : "text-muted-foreground")}>{t(!task.open ? "backlogView.closed" : started ? "backlogView.started" : "backlogView.open")}</span>{author ? <span>{author}</span> : null}{at !== null ? <time className="text-micro text-muted-foreground">{issueDate(at, language)}</time> : null}<span className="flex-1" />{task.url && !local ? <Button size="sm" variant="outline" asChild><a href={task.url} target="_blank" rel="noreferrer" data-item-github="true"><ExternalLinkIcon />{t("backlog.openOnGithub")}</a></Button> : null}{checkout ? <Button size="sm" onClick={() => { actions.closeOverview(); actions.openWorkspace(project.device_id, project.id, checkout.id); }}>{t("backlogView.openWork")}</Button> : task.open ? <Button size="sm" onClick={() => useUiStore.getState().setWorkspaceDialog({ kind: "start_issue", workspaceId: project.id, taskKey: task.key })} data-item-start="true"><PlayIcon />{t("backlog.start")}</Button> : null}</div>
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_var(--size-tree-ideal)] gap-xl overview-narrow:grid-cols-1">
        <div className="min-w-0 max-w-(--size-editor-document) [--text-body:var(--text-subhead)] [--text-editor-heading-1:var(--text-editor-heading-3)] [--text-editor-heading-2:var(--text-editor-heading-4)] [--text-editor-heading-3:var(--text-editor-heading-5)]">
          <section className="border-t border-border pt-md" aria-label={t("backlogView.body")}>
            {!detail || detail.phase === "reading" ? <p role="status" className="text-muted-foreground">{t("backlogView.reading")}</p> : detail.phase === "failed" ? <p role="status" className="flex flex-wrap items-center gap-xs text-warning"><span>{t("backlogView.readFailed")}</span><Button size="sm" variant="ghost" onClick={() => actions.requestIssueDetail(project.id, task.key)}>{t("common.retry")}</Button></p> : detail.body ? <MarkdownText text={detail.body} /> : <p className="text-muted-foreground">{t("backlogView.noBody")}</p>}
          </section>
          {!local && detail?.phase === "ready" ? <section className="mt-lg border-t border-border pt-md" aria-label={t("backlogView.activity")} data-item-activity="true"><h3 className="mb-md flex flex-wrap items-center gap-sm font-semibold">{t("backlogView.activity")}<span className="font-normal text-micro text-muted-foreground">{t("backlogView.comments", { shown: detail.comments.length, total: detail.comment_count ?? 0 })}</span></h3>{detail.comments.length ? detail.comments.map((comment, index) => <article key={index} className="mb-xl border-b border-border pb-xl"><p className="mb-sm flex flex-wrap items-center gap-sm text-micro text-muted-foreground"><span className="font-medium text-subtle-foreground">{comment.author ?? t("backlogView.unavailable")}</span>{comment.created_at_unix_ms !== null ? <time>{issueDate(comment.created_at_unix_ms, language)}</time> : null}</p><MarkdownText text={comment.body} /></article>) : <p className="text-muted-foreground">{t("backlogView.noComments")}</p>}{detail.message ? <p role="status" className="my-sm text-warning">{t("backlogView.commentsFailed")}</p> : null}{detail.comments_has_more && detail.comments_cursor ? <Button variant="ghost" size="sm" disabled={detail.comments_loading} data-item-more-comments="true" onClick={() => actions.requestIssueDetail(project.id, task.key, detail.comments_cursor ?? undefined)}>{t(detail.comments_loading ? "backlogView.reading" : "backlogView.moreComments")}</Button> : null}{detail.comments_limited ? <p role="status" className="mt-sm text-micro text-muted-foreground">{t("backlogView.commentLimit")}</p> : null}</section> : null}
        {origin && (origin.phase !== "ready" || origin.session) ? (
          <section className="pt-sm" data-item-origin={origin.phase}>
            <h3 className="mb-xs font-medium text-subtle-foreground">{t("backlog.panel.origin")}</h3>
            {origin.phase === "reading" ? <p className="text-muted-foreground">{t("backlog.panel.originReading")}</p>
              : origin.phase === "failed" ? <p className="text-muted-foreground">{t("backlog.panel.originFailed")}</p>
              : origin.session ? (
                <button type="button" className="flex w-full min-w-0 flex-col items-start gap-xxs rounded-sm px-xs py-xs text-left outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring" onClick={() => { const session = origin.session!; actions.closeOverview(); actions.openSessionRecord(session); }} data-item-origin-session="true">
                  <span className="font-medium text-foreground">{origin.session_title ?? ""}</span>
                  {origin.flow_name ? <span className="text-subtle-foreground">{origin.flow_name}</span> : null}
                  {origin.missed_text ? <span className="flex items-start gap-xxs text-warning"><FlagIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0" />{origin.missed_text}</span> : null}
                </button>
              ) : null}
          </section>
        ) : null}
        </div>
        <aside className="min-w-0 border-l border-border pl-md overview-narrow:border-l-0 overview-narrow:border-t overview-narrow:pl-0 overview-narrow:pt-md"><dl className="flex flex-col gap-lg"><div><dt className="mb-xs text-micro text-muted-foreground">{t("backlogView.assignee")}</dt><dd>{assignees.length ? assignees.join(", ") : "-"}</dd></div><div><dt className="mb-xs text-micro text-muted-foreground">{t("backlogView.labels")}</dt><dd className="flex flex-wrap gap-xs">{labels.length ? labels.map((label) => <IssueLabelView key={label.name} label={label} />) : "-"}</dd></div><div><dt className="mb-xs text-micro text-muted-foreground">{t("backlogView.project")}</dt><dd>{project.label}</dd></div>{repository ? <div><dt className="mb-xs text-micro text-muted-foreground">{t("backlogView.repository")}</dt><dd>{repository}</dd></div> : null}</dl>{undoable ? <Button className="mt-lg" size="sm" variant="outline" disabled={undo?.phase === "working"} onClick={() => actions.undoBacklog(project.id, task.key)} data-item-undo="true">{t(undo?.phase === "working" ? "backlog.panel.undoing" : "backlog.panel.undo")}</Button> : null}{undo?.phase === "failed" ? <p role="status" className="mt-xs text-destructive" data-item-undo-failed={undo.failure ?? "refused"}>{t(undoRetryable(undo.failure) ? "backlog.panel.undoFailed" : "backlog.panel.undoRefused", { reason: undo.failure ? t(`backlog.failure.${undo.failure}`) : "" })}</p> : null}</aside>
      </div>
      </div>
    </div>
  </div>;
}
