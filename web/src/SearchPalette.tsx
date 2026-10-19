import { CircleDotIcon, FolderIcon, GlobeIcon, GitBranchIcon, LoaderCircleIcon, SearchIcon, ServerIcon, TriangleAlertIcon, ChevronRightIcon } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import type { Actions } from "./actions";
import { agentProductName } from "./agentAdapters";
import { markTone, markWords } from "./agentRow";
import { StatusMark } from "./components/status-mark";
import { PrMark } from "./components/pr-mark";
import { DeviceChip } from "./components/device-chip";
import { Command, CommandDialog, CommandGroup, CommandInput, CommandItem, CommandList } from "./components/ui/command";
import { Kbd } from "./components/ui/kbd";
import { frontTarget, relationRows, relationsOf, type Relations } from "./relations";
import { filterEntries, githubEntries, groupEntries, openUrlEntry, recentEntries, GITHUB_GROUP, RECENT_GROUP, RELATED_GROUP, SESSIONS_GROUP, sessionEntries, searchEntries, type SearchEntry, type SearchSection, type Tone } from "./search";
import { summariesEnabled } from "./hideAi";
import { workKey, recordTitle } from "./sessionWork";
import { Button } from "./components/ui/button";
import type { LinkSummaries, WorkSession } from "./snapshot";
import { detailOf, type Detail } from "./searchDetail";
import { githubRow, hasGithubProject, ownAnswer, projectRead, projectToRead, startsSearch, type GithubRow, type ProjectRead } from "./searchGithub";
import { browserBridge } from "./host";
import { frontCheckout } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";
import { remoteRequestId } from "./remote";
import { keyboardOwner } from "./viewFocus";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";

// ⌘K (PRD cmdk-navigation): a palette that goes to things. A query finds an
// agent, project, checkout, device, issue or pull request by name or #number;
// an empty one lists what is connected to the thing in front, drawn as a
// Project's Overview groups it; the highlighted row's detail is the right
// half. Nothing here runs a command but `Start an agent…`. GitHub is read only
// by the one project read ⌘K asks for when it opens and by the explicit
// `Search GitHub for "…"` row; typing never calls it.

/** The window is too narrow for a list and a detail below this width; the detail is not drawn then (B26). */
const DETAIL_MIN_WIDTH = 800;

/** Whether a dialog of `width` px has room for the detail beside its list; unmeasured is yes. */
function showsDetail(width: number | null): boolean {
  return width === null || width >= DETAIL_MIN_WIDTH;
}

/** The projects ⌘K has asked to be read since the app started: each is asked for once (B10). */
const askedProjects = new Set<string>();

const GITHUB_ROW_ID = "github-search";

/** The longest query the core searches GitHub for (`SEARCH_QUERY_LIMIT` in herdr-core/src/github.rs). */
const SEARCH_QUERY_LIMIT = 200;

const TONE_CLASS: Record<Tone, string> = {
  working: "text-agent-working",
  attention: "text-warning",
  done: "text-success",
  open: "text-success",
  pending: "text-warning",
  failed: "text-destructive",
  muted: "text-muted-foreground",
};

export function SearchPalette({ actions }: { actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const rest = useShellStore((s) => s.rest);
  const linkSummaries = useShellStore((s) => s.linkSummaries);
  const workSearch = useShellStore((s) => s.sessionWorkSearch);
  const summaries = summariesEnabled(rest?.status?.background_ai);
  const close = useUiStore((s) => s.closeOverlay);
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState("");
  const [asked, setAsked] = useState<{ requestId: string; query: string } | null>(null);
  const [width, setWidth] = useState<number | null>(null);
  const [box, setBox] = useState<HTMLDivElement | null>(null);
  // What was in front when ⌘K opened, read once: the page's own screen, whether the keyboard was in an agent, and the front checkout.
  const [front] = useState(() => {
    const ui = useUiStore.getState();
    const state = useShellStore.getState();
    const owner = keyboardOwner().kind;
    const checkout = frontCheckout(state.rest);
    return {
      target: frontTarget(state.rest, ui.screen?.kind ?? null, ui.searchOver === "settings", state.focusedPaneId, owner === "pane" || owner === "agent", checkout?.id ?? null),
      checkout: ui.screen?.kind === "workspace" && ui.searchOver !== "settings" ? checkout : null,
    };
  });

  // The one read of the project in front, when nothing has read it (B8, B10).
  useEffect(() => {
    const workspace = projectToRead(useShellStore.getState().rest, front.checkout, askedProjects);
    if (!workspace) return;
    askedProjects.add(workspace.id);
    actions.readProjectTasks(workspace.id);
  }, [actions, front]);

  // The dialog's content mounts after this component does, so the box is found when it appears, not on mount.
  useLayoutEffect(() => {
    if (!box) return undefined;
    setWidth(box.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(([entry]) => setWidth(entry ? entry.contentRect.width : null));
    observer.observe(box);
    return () => observer.disconnect();
  }, [box]);

  const entries = useMemo(() => (rest ? searchEntries(rest, t) : []), [rest, t]);
  const related = useMemo<Relations | null>(() => (front.target ? relationsOf(rest, front.target, t, true) : null), [rest, front, t]);
  const trimmed = query.trim();
  useEffect(() => { actions.searchSessionWork(trimmed); }, [actions, trimmed, summaries]);
  useEffect(() => () => { actions.searchSessionWork(""); }, [actions]);
  const answer = ownAnswer(rest?.issue_work?.search, asked, trimmed);
  const row = githubRow(trimmed, answer, t);
  const showGithub = trimmed.length > 0 && hasGithubProject(rest);

  const sections: SearchSection[] = useMemo(() => {
    if (!trimmed) return [];
    // A page opens in the desktop app's View area, so a browser tab or a screen without a Workspace in front lists the row with the reason.
    const unavailable = !browserBridge() ? t("search.openUrlDesktopOnly") : !front.checkout ? t("search.openUrlNoWorkspace") : null;
    const link = openUrlEntry(trimmed, unavailable, t);
    const found = [...(link ? [{ group: link.group, entries: [link] }] : []), ...groupEntries(filterEntries(entries, trimmed))];
    const records = sessionEntries(workSearch, trimmed);
    const pending = workSearch?.query === trimmed && (workSearch.loading || workSearch.failure);
    const sessionGroups = records.length > 0 || pending ? [{ group: SESSIONS_GROUP, entries: records }] : [];
    const searched = answer?.phase === "ready" ? githubEntries(answer.results, entries, t) : [];
    return [...found, ...sessionGroups, ...(searched.length > 0 ? [{ group: GITHUB_GROUP, entries: searched }] : [])];
  }, [entries, trimmed, answer, workSearch, front, t]);

  const relatedRows = useMemo(() => (trimmed || !related ? [] : relationRows(related)), [trimmed, related]);
  // Recent follows Related on the empty query and leaves out the checkouts already on screen: the one in front and every one Related lists (PRD cmdk-recent B4).
  const recentRows = useMemo(() => {
    if (trimmed) return [];
    const shown = new Set(relatedRows.flatMap((entry) => (entry.kind === "checkout" && entry.checkoutId ? [entry.checkoutId] : [])));
    if (front.checkout) shown.add(front.checkout.id);
    return recentEntries(rest, shown);
  }, [trimmed, rest, relatedRows, front]);
  const listed: SearchEntry[] = trimmed ? sections.flatMap((section) => section.entries) : [...relatedRows, ...recentRows];
  const ids = [...listed.map((entry) => entry.id), ...(showGithub ? [GITHUB_ROW_ID] : [])];
  // A surviving highlight stays; a retired one moves to the first row (B25).
  const selected = ids.includes(highlighted) ? highlighted : (ids[0] ?? "");
  const selectedEntry = listed.find((entry) => entry.id === selected) ?? null;

  const collapsed = !trimmed && listed.length === 0;
  const detailVisible = !collapsed && showsDetail(width);
  const now = Date.now();

  const activate = (entry: SearchEntry) => {
    // The agent in front is where the operator already is; the detail says so (B21).
    if (entry.tag === "here") return;
    // A Recent row of a device that is not connected is dimmed and does nothing, and says nothing (PRD cmdk-recent B9).
    if (entry.dimmed) return;
    close();
    switch (entry.kind) {
      case "session":
        return entry.session ? actions.openSessionRecord(entry.session) : undefined;
      case "command":
        return entry.command === "open_url" && entry.url ? actions.openBrowser(entry.url) : actions.openStartPanel();
      case "device":
        return entry.deviceId ? actions.focusDevice(entry.deviceId) : undefined;
      case "agent":
        return entry.paneId ? actions.openAgent(entry.paneId) : undefined;
      case "project":
        return entry.deviceId && entry.workspaceId ? actions.openProjectById(entry.deviceId, entry.workspaceId) : undefined;
      case "checkout":
        return entry.deviceId && entry.workspaceId && entry.checkoutId ? actions.openWorkspace(entry.deviceId, entry.workspaceId, entry.checkoutId) : undefined;
      case "issue":
      case "pr":
        if (entry.external && entry.url) return actions.openLink(entry.url, true);
        if (!entry.deviceId || !entry.workspaceId) return undefined;
        if (entry.kind === "issue" && entry.taskKey) return actions.openBacklogItem(entry.deviceId, entry.workspaceId, entry.taskKey);
        if (entry.kind === "pr" && entry.number !== undefined) return actions.openPrCard(entry.workspaceId, entry.number);
        return undefined;
    }
  };

  const search = () => {
    // The same query already running is not started again (B19).
    if (!startsSearch(row)) return;
    const requestId = remoteRequestId();
    setAsked({ requestId, query: trimmed });
    actions.searchGithub(requestId, trimmed);
  };

  const onSelect = (id: string) => {
    if (id === GITHUB_ROW_ID) return search();
    const entry = listed.find((candidate) => candidate.id === id);
    if (entry) activate(entry);
  };

  const reads = (entry: SearchEntry): ProjectRead => (entry.kind === "checkout" && entry.workspace ? projectRead(entry.workspace, askedProjects, now, t, language) : null);

  const detail: Detail | null = selectedEntry ? detailOf(rest, selectedEntry, now, t, language, linkSummaries) : null;

  return (
    <CommandDialog
      open
      title={t("commands.search")}
      description={t("search.description")}
      className="w-[calc(var(--size-search-sheet-w)*1.5)]"
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <div ref={setBox} className="flex min-w-0 flex-col" data-cmdk={collapsed ? "collapsed" : "open"}>
        <Command shouldFilter={false} label={t("commands.search")} data-palette="Search" value={selected} onValueChange={setHighlighted}>
          <CommandInput
            value={query}
            placeholder={t("search.placeholder")}
            maxLength={SEARCH_QUERY_LIMIT}
            data-palette-input="true"
            onValueChange={(value) => setQuery(value)}
            wrapperClassName={collapsed ? "border-b-0" : undefined}
            trailing={<Kbd data-palette-esc="true">Esc</Kbd>}
          />
          {collapsed ? null : (
            <div className={`flex min-h-0 ${detailVisible ? "h-(--size-search-sheet-h)" : ""}`}>
              <CommandList className={`${detailVisible ? "max-h-[none] w-[calc(var(--size-search-sheet-w)-var(--spacing-xxs))] shrink-0 border-r border-border" : "min-w-0 flex-1"}`} data-palette-list="true">
                {trimmed ? (
                  <>
                    {sections.length === 0 ? (
                      <div className="px-md py-sm text-caption text-muted-foreground" data-palette-state="no-match">
                        {t("search.noMatch")}
                      </div>
                    ) : null}
                    {sections.map((section) => (
                      <CommandGroup key={section.group.id} heading={t(section.group.label)} data-palette-group={section.group.id}>
                        {section.group.id === "sessions" && workSearch?.query === trimmed && workSearch.loading ? <p className="px-sm py-xs text-caption text-muted-foreground" role="status">{t("common.loading")}</p> : null}
                        {section.group.id === "sessions" && workSearch?.query === trimmed && workSearch.failure ? <p className="px-sm py-xs text-caption text-muted-foreground" role="status">{t("sessionWork.failed")}</p> : null}
                        {section.entries.map((entry) => (
                          <SearchRow key={entry.id} entry={entry} read={reads(entry)} onSelect={() => onSelect(entry.id)} />
                        ))}
                      </CommandGroup>
                    ))}
                    {showGithub ? <GithubSearchRow row={row} onSelect={() => onSelect(GITHUB_ROW_ID)} /> : null}
                  </>
                ) : (
                  <>
                    {relatedRows.length > 0 ? (
                      <CommandGroup heading={t(RELATED_GROUP.label)} data-palette-group={RELATED_GROUP.id}>
                        {relatedRows.map((entry) => (
                          <SearchRow key={entry.id} entry={entry} read={reads(entry)} onSelect={() => onSelect(entry.id)} />
                        ))}
                      </CommandGroup>
                    ) : null}
                    {recentRows.length > 0 ? (
                      <CommandGroup heading={t(RECENT_GROUP.label)} data-palette-group={RECENT_GROUP.id}>
                        {recentRows.map((entry) => (
                          <SearchRow key={entry.id} entry={entry} read={reads(entry)} onSelect={() => onSelect(entry.id)} />
                        ))}
                      </CommandGroup>
                    ) : null}
                  </>
                )}
              </CommandList>
              {detailVisible ? <DetailPane detail={detail} entry={selectedEntry} githubRow={selected === GITHUB_ROW_ID ? row : null} query={trimmed} links={linkSummaries} summaries={summaries} onRecord={(session) => { close(); actions.openSessionRecord(session); }} /> : null}
            </div>
          )}
          {collapsed ? null : (
            <div className="flex items-center gap-md border-t border-border px-md py-xxs text-caption text-muted-foreground" data-palette-footer="true">
              <span className="flex items-center gap-xxs"><Kbd>↑</Kbd><Kbd>↓</Kbd> {t("search.navigate")}</span>
              <span className="flex items-center gap-xxs"><Kbd>↵</Kbd> {t("common.open")}</span>
              <span className="flex items-center gap-xxs"><Kbd>Esc</Kbd> {t("common.close")}</span>
            </div>
          )}
        </Command>
      </div>
    </CommandDialog>
  );
}

function EntryIcon({ entry }: { entry: SearchEntry }) {
  // An agent draws its status mark in the slot, a conversation an empty slot of the same width, so every title starts on one column (PRD agent-mark-quiet D-09).
  if (entry.kind === "agent" || entry.kind === "session") {
    return (
      <span className="flex w-(--size-agent-badge-compact) shrink-0 justify-center" aria-hidden="true">
        {entry.agent ? <StatusMark mark={entry.agent.mark} className={markTone(entry.agent)} /> : null}
      </span>
    );
  }
  if (entry.prState) {
    return (
      <span className="flex w-(--size-agent-badge-compact) shrink-0 justify-center" aria-hidden="true">
        <PrMark state={entry.prState} compact />
      </span>
    );
  }
  const Icon =
    entry.command === "open_url" ? GlobeIcon : entry.kind === "project" ? FolderIcon : entry.kind === "checkout" ? GitBranchIcon : entry.kind === "device" ? ServerIcon : entry.kind === "issue" ? CircleDotIcon : ChevronRightIcon;
  const tone = entry.kind === "issue" ? TONE_CLASS[entry.status?.tone ?? "muted"] : "";
  return (
    <span className={`flex w-(--size-agent-badge-compact) shrink-0 justify-center ${tone}`} aria-hidden="true">
      <Icon />
    </span>
  );
}

function StatusText({ status }: { status: { tone: Tone; label: string } }) {
  return <span className={`shrink-0 whitespace-nowrap text-caption ${TONE_CLASS[status.tone]}`}>{status.label}</span>;
}

/** One result or relation row: its mark, the title and the line under it, and the state at its end. */
function SearchRow({ entry, read, onSelect }: { entry: SearchEntry; read: ProjectRead; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  const depth = entry.depth ?? 0;
  return (
    <CommandItem asChild value={entry.id} onSelect={onSelect}>
      <button type="button" data-palette-row={entry.id} data-palette-depth={depth} data-palette-dim={entry.dimmed ? "true" : undefined} title={[entry.agentKind ? agentProductName(entry.agentKind) : null, entry.agent ? markWords(t, entry.agent) : null].filter(Boolean).join("\n") || undefined} className={`group/palette-row w-full text-left ${entry.dimmed ? "opacity-(--opacity-dimmed)" : ""}`} style={depth > 0 ? { paddingLeft: `calc(var(--spacing-sm) + ${depth} * var(--spacing-lg))` } : undefined}>
        <EntryIcon entry={entry} />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex min-w-0 items-center gap-xs">
            {entry.tag === "parent" ? <span className="shrink-0 text-caption text-muted-foreground">{t("search.parent")}</span> : null}
            <span className="min-w-0 truncate font-medium">{entry.title}</span>
            {entry.agentKind ? <span className="sr-only">, {agentProductName(entry.agentKind)}</span> : null}
            {entry.agent ? <span className="sr-only">, {markWords(t, entry.agent)}</span> : null}
            {entry.chip ? <DeviceChip label={entry.chip.label} local={entry.chip.local} className="max-w-2/5" /> : null}
          </span>
          {entry.subtitle ? <span data-palette-detail="true" className="truncate text-caption text-muted-foreground">{entry.subtitle}</span> : null}
        </span>
        {entry.tag === "here" ? (
          <span className="shrink-0 rounded-full border border-border px-xs text-caption text-subtle-foreground" data-palette-here="true">
            {t("search.here")}
          </span>
        ) : null}
        {read?.state === "reading" ? <LoaderCircleIcon aria-label={t("search.readingGitHub")} className="size-(--size-icon-sm) shrink-0 animate-spin text-muted-foreground" data-palette-read="reading" /> : null}
        {read?.state === "failed" ? (
          <span title={read.tooltip} aria-label={read.tooltip} className="shrink-0 text-warning" data-palette-read="failed">
            <TriangleAlertIcon className="size-(--size-icon-sm)" />
          </span>
        ) : null}
        {entry.status ? <StatusText status={entry.status} /> : null}
        {entry.dimmed ? null : (
          <span aria-hidden="true" data-palette-enter="true" className="invisible shrink-0 text-caption text-muted-foreground group-data-[selected=true]/palette-row:visible">
            ↵
          </span>
        )}
      </button>
    </CommandItem>
  );
}

/** The last row of a search: the explicit GitHub search, never run while typing (B16-B19). */
function GithubSearchRow({ row, onSelect }: { row: GithubRow; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <CommandGroup data-palette-group="github-search">
      <CommandItem asChild value={GITHUB_ROW_ID} onSelect={onSelect}>
        <button type="button" data-palette-row={GITHUB_ROW_ID} data-github-state={row.state} className="group/palette-row w-full text-left">
          <span className="flex w-(--size-agent-badge-compact) shrink-0 justify-center" aria-hidden="true">
            {row.state === "failed" ? <TriangleAlertIcon className="text-warning" /> : <SearchIcon />}
          </span>
          <span className={`min-w-0 flex-1 truncate ${row.state === "none" ? "text-muted-foreground" : "font-medium"}`}>{row.label}</span>
          {row.state === "working" ? <LoaderCircleIcon aria-label={t("search.searchingGitHub")} className="size-(--size-icon-sm) shrink-0 animate-spin text-muted-foreground" data-palette-read="searching" /> : null}
          <span aria-hidden="true" data-palette-enter="true" className="invisible shrink-0 text-caption text-muted-foreground group-data-[selected=true]/palette-row:visible">
            ↵
          </span>
        </button>
      </CommandItem>
    </CommandGroup>
  );
}

function RelationBlock({ relations, on }: { relations: Relations; on: string }) {
  const { t } = useInterfaceTranslation();
  const line = (entry: SearchEntry, depth: number) => (
    <div key={entry.id} data-relation-row={entry.id} data-relation-on={entry.id === on ? "true" : undefined} className={`flex items-center gap-xs py-xxs text-caption ${entry.id === on ? "font-semibold text-foreground" : "text-subtle-foreground"}`} style={depth > 0 ? { paddingLeft: `calc(${depth} * var(--spacing-md))` } : undefined}>
      <span className="flex w-(--size-icon-sm) shrink-0 justify-center [&_svg]:size-(--size-icon-sm)"><EntryIcon entry={entry} /></span>
      <span className="min-w-0 break-words">{entry.tag === "parent" ? `${t("search.parent")} ` : ""}{entry.title}</span>
    </div>
  );
  return (
    <div className="mt-md border-t border-border pt-md" data-detail-relations="true">
      <h4 className="mb-xs text-caption font-semibold text-muted-foreground">{t("search.relations")}</h4>
      {relations.issues.map((entry) => line(entry, 0))}
      {relations.groups.map((group) => (
        <div key={group.head.id}>
          {line(group.head, 0)}
          {group.rows.map((entry) => line(entry, 1 + (entry.depth ?? 0)))}
        </div>
      ))}
    </div>
  );
}

/** The highlighted row, said in full: its state, its facts and the relations it belongs to. */
function DetailPane({ detail, entry, githubRow: row, query, links, summaries, onRecord }: { detail: Detail | null; entry: SearchEntry | null; githubRow: GithubRow | null; query: string; links: LinkSummaries | null; summaries: boolean; onRecord: (session: WorkSession) => void }) {
  const { t } = useInterfaceTranslation();
  if (row) {
    return (
      <aside className="flex min-w-0 flex-1 flex-col overflow-y-auto p-md" data-palette-detail-pane="github">
        <div className="text-caption font-semibold text-muted-foreground">GitHub</div>
        <div className="mt-xs break-words text-subhead font-semibold">{`"${query}"`}</div>
        <p className="mt-sm text-caption text-muted-foreground">{t("search.githubDescription")}</p>
        <ActionLine text={row.state === "failed" ? t("common.retry") : row.state === "none" ? t("search.noResults") : t("search.searchGitHub")} key_={row.state !== "none"} />
      </aside>
    );
  }
  if (!detail || !entry) return <aside className="min-w-0 flex-1" data-palette-detail-pane="empty" />;
  const here = entry.tag === "here";
  return (
    <aside className="flex min-w-0 flex-1 flex-col" data-palette-detail-pane={entry.kind}>
      <div className="min-h-0 flex-1 overflow-y-auto p-md">
        <div className="text-caption font-semibold text-muted-foreground">{detail.kind}</div>
        <div className="mt-xs break-words text-subhead font-semibold">{detail.title}</div>
        {detail.pills.length + detail.tags.length > 0 ? (
          <div className="mt-sm flex flex-wrap gap-xs">
            {detail.pills.map((pill) => (
              <span key={pill.label} className={`rounded-sm bg-secondary px-sm text-caption ${TONE_CLASS[pill.tone]}`}>{pill.label}</span>
            ))}
            {detail.tags.map((tag) => (
              <span key={tag} className="rounded-sm bg-secondary px-sm text-caption text-subtle-foreground">{tag}</span>
            ))}
          </div>
        ) : null}
        {detail.facts.length > 0 ? (
          <dl className="mt-md grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xs border-t border-border pt-md text-caption" data-detail-facts="true">
            {detail.facts.map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-muted-foreground">{label}</dt>
                <dd className="min-w-0 break-words">{value}</dd>
              </div>
            ))}
          </dl>
        ) : null}
        {detail.relations ? <RelationBlock relations={detail.relations} on={entry.id} /> : null}
        {entry.kind === "pr" || entry.kind === "issue" ? <Creators entry={entry} links={links} summaries={summaries} onRecord={onRecord} /> : null}
      </div>
      {detail.action ? <ActionLine text={here ? t("search.currentAgent") : detail.action} key_={!here} /> : null}
    </aside>
  );
}

function Creators({ entry, links, summaries, onRecord }: { entry: SearchEntry; links: LinkSummaries | null; summaries: boolean; onRecord: (session: WorkSession) => void }) {
  const { t } = useInterfaceTranslation();
  const repository = entry.repository ?? entry.workspace?.tasks?.source?.name;
  if (!repository || entry.number === undefined) return null;
  const seen = new Set<string>();
  const creators = Object.values(links?.projects ?? {}).flatMap((project) => (project.creators ?? []).flatMap((creator) => {
    if (creator.kind !== entry.kind || creator.number !== entry.number || creator.repository.toLowerCase() !== repository.toLowerCase() || seen.has(workKey(creator.source))) return [];
    seen.add(workKey(creator.source));
    const work = project.work?.find((work) => workKey(work.session) === workKey(creator.source));
    return [{ session: creator.source, title: work ? recordTitle(work, summaries) : creator.source.session_id }];
  }));
  return creators.length ? <div className="mt-md" data-output-creators="true"><p className="text-caption text-muted-foreground">{t("sessionWork.creator")}</p>{creators.map((creator) => <Button key={workKey(creator.session)} variant="link" size="sm" className="max-w-full" title={agentProductName(creator.session.agent)} onClick={() => onRecord(creator.session)}><span className="truncate">{creator.title}</span><span className="sr-only">, {agentProductName(creator.session.agent)}</span></Button>)}</div> : null;
}

function ActionLine({ text, key_ }: { text: ReactNode; key_: boolean }) {
  return (
    <div className="mt-auto flex items-center gap-xs border-t border-border px-md py-sm text-caption text-muted-foreground" data-palette-action="true">
      {key_ ? <Kbd>↵</Kbd> : null}
      <span>{text}</span>
    </div>
  );
}
