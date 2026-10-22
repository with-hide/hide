import { ArrowLeftIcon, ArrowRightIcon, ChevronRightIcon, InfoIcon, LoaderCircleIcon, SearchIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef } from "react";
import type { Actions } from "./actions";
import { AGENT_ADAPTERS, agentProductName } from "./agentAdapters";
import { AgentMark } from "./AgentMark";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Hint } from "./components/ui/tooltip";
import { deviceConnected } from "./devices";
import { useInterfaceTranslation } from "./i18n/client";
import { formatDateTime } from "./i18n/format";
import { requireInterfaceLanguage } from "./i18n/locale";
import { cn } from "./lib/utils";
import { historyAnswer, historyInitial, historyPage, HISTORY_PAGE_LIMIT, useHistoryStore, type HistoryState } from "./overviewHistoryState";
import type { BoardProject } from "./projectBoard";
import { HighlightedRequest } from "./RequestHighlight";
import { workKey } from "./sessionWork";
import type { SessionSearchHit, SessionSearchMatch, SessionSearchStatus } from "./snapshot";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

const SELECT = "h-(--size-control-sm) max-w-full rounded-sm border border-input bg-background px-xs text-caption text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring";
const STATUSES = ["all", "current", "working", "done", "needs_you", "archived", "retained"] as const;

/** A retained session search. Filters and cursors are applied by the existing ledger worker. */
export function OverviewHistory({ projects, deviceId, actions }: { projects: BoardProject[]; deviceId: string; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const language = requireInterfaceLanguage(i18n.language);
  const projectId = useUiStore((s) => s.overview.projectId);
  const hasRecord = useUiStore((s) => s.overview.panel?.kind === "record");
  const paletteOpen = useUiStore((s) => s.overlay === "search");
  const project = projects.find((p) => p.workspace.id === projectId && p.workspace.device_id === deviceId)?.workspace ?? null;
  const scope = JSON.stringify([deviceId, projectId, project?.path ?? null]);
  const state = useHistoryStore((s) => s.entry);
  const update = useHistoryStore((s) => s.update);
  const answer = useShellStore((s) => s.sessionWorkSearch);
  const connection = useShellStore((s) => s.connection);
  const generation = useShellStore((s) => s.viewGeneration);
  const rest = useShellStore((s) => s.rest);
  const online = deviceConnected(rest, deviceId);
  const transport = connection === "live";
  const scroll = useRef<HTMLDivElement>(null);
  const rows = useRef(new Map<string, HTMLButtonElement>());
  const current = state.scope === scope;

  useLayoutEffect(() => {
    if (!current) update(() => historyInitial(scope));
  }, [scope, current, update]);

  useEffect(() => {
    if (!current || !transport || paletteOpen || (projectId && !project)) return;
    const requestId = `history-${crypto.randomUUID()}`;
    update((held) => held.scope === scope ? { ...held, requestId, loading: true, failure: null } : held);
    // One cancellable input debounce, owned by this screen. No background search timer.
    const timer = window.setTimeout(() => {
      actions.searchSessionWork(state.query, {
        request_id: requestId, device_id: deviceId, project_root: project?.path,
        agent: state.agent || undefined, status: state.status === "all" ? undefined : state.status,
        since_unix_ms: state.since, include_titles: state.extra, include_flows: state.extra,
        cursor: state.cursors[state.page],
      });
    }, 180);
    return () => window.clearTimeout(timer);
  }, [actions, scope, current, transport, paletteOpen, generation, deviceId, projectId, project?.path, state.revision, state.query, state.agent, state.status, state.since, state.extra, state.page, update]);

  useEffect(() => { if (current) update((held) => historyAnswer(held, answer)); }, [answer, current, update]);
  useLayoutEffect(() => {
    if (current && scroll.current) scroll.current.scrollTop = state.scroll;
  }, [current, state.page, state.results]);
  useEffect(() => {
    if (!hasRecord && state.selected) rows.current.get(state.selected)?.focus({ preventScroll: true });
  }, [hasRecord, state.selected]);

  const criteria = (patch: Partial<HistoryState>) => update((held) => ({
    ...held, ...patch, cursors: [null], page: 0, scroll: 0, selected: null,
    requestId: null, results: [], next: null, failure: null, loading: true, revision: held.revision + 1,
  }));
  const open = (hit: SessionSearchHit, match?: SessionSearchMatch) => {
    update((held) => ({ ...held, selected: workKey(hit.session) }));
    useUiStore.getState().setOverview({ panel: {
      kind: "record", session: hit.session, returnTo: "history",
      ...(match?.kind === "request" && match.request_offset !== null ? { locator: { request_offset: match.request_offset, query: state.query.trim() } } : {}),
    } });
  };
  const failed = state.failure !== null;
  const unavailableProject = projectId !== null && !project;
  const loading = !current || state.loading;
  const results = current ? state.results : [];
  const date = (at: number) => formatDateTime(language, at, { year: "numeric", month: "short", day: "numeric" });
  const time = (at: number) => formatDateTime(language, at, { hour: "2-digit", minute: "2-digit", hour12: false });

  return <section className="flex h-full min-h-0 min-w-0 flex-1 flex-col" data-overview-history="true">
    <div className="shrink-0 space-y-md px-xl pt-xl pb-md">
      <h1 className="text-title font-semibold">{t("overviewHistory.title")}</h1>
      <div className="flex min-w-0 flex-wrap items-center gap-sm">
        <div className="flex min-w-0 flex-1 basis-1/2 items-center gap-sm">
          <SearchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
          <Input type="search" maxLength={200} value={state.query} placeholder={t("overviewHistory.search")} aria-label={t("overviewHistory.search")} onChange={(event) => criteria({ query: event.target.value })} data-history-query="true" />
        </div>
        <select className={SELECT} aria-label={t("overviewHistory.scope")} value={state.extra ? "extra" : "requests"} onChange={(event) => criteria({ extra: event.target.value === "extra" })} data-history-scope="true">
          <option value="requests">{t("overviewHistory.requests")}</option><option value="extra">{t("overviewHistory.extra")}</option>
        </select>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-sm">
        <select className={SELECT} aria-label={t("overviewHistory.status")} value={state.status} onChange={(event) => criteria({ status: event.target.value as SessionSearchStatus | "all" })} data-history-status="true">
          {STATUSES.map((status) => <option key={status} value={status}>{t(`overviewHistory.${status}`)}</option>)}
        </select>
        <select className={SELECT} aria-label={t("overviewHistory.agent")} value={state.agent} onChange={(event) => criteria({ agent: event.target.value })} data-history-agent="true">
          <option value="">{t("overviewHistory.allAgents")}</option>{AGENT_ADAPTERS.map((agent) => <option key={agent.id} value={agent.herdr_kind}>{agent.label}</option>)}
        </select>
        <select className={SELECT} aria-label={t("overviewHistory.period")} value={state.days} onChange={(event) => { const days = Number(event.target.value); criteria({ days, since: days ? Date.now() - days * 86_400_000 : null }); }} data-history-period="true">
          <option value={0}>{t("overviewHistory.allTime")}</option>{[7, 30, 90].map((days) => <option key={days} value={days}>{t("overviewHistory.days", { count: days })}</option>)}
        </select>
        <Hint label={t("overviewHistory.limit")}>
          <button type="button" className="ml-auto inline-flex items-center gap-xs rounded-xs text-micro text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring" data-history-retention="true"><InfoIcon aria-hidden="true" className="size-(--size-icon-sm)" />{t("overviewHistory.limitLabel")}</button>
        </Hint>
      </div>
      {!online ? <p role="status" className="text-micro text-muted-foreground" data-history-offline="true">{t("overviewHistory.offline")}</p> : null}
    </div>
    <div ref={scroll} className="min-h-0 flex-1 overflow-y-auto px-xl" onScroll={(event) => { const top = event.currentTarget.scrollTop; update((held) => ({ ...held, scroll: top })); }} aria-busy={loading && transport} data-history-results="true">
      {!transport ? <p role="status" className="py-lg text-caption text-muted-foreground" data-history-disconnected="true">{t("overviewHistory.disconnected")}</p>
        : unavailableProject || failed ? <div role="status" className="py-lg text-caption" data-history-failure={state.failure ?? "project_unavailable"}><p>{t(state.failure?.includes("capacity") ? "overviewHistory.capacity" : "overviewHistory.failed")}</p><Button variant="ghost" size="sm" onClick={() => update((held) => ({ ...held, revision: held.revision + 1 }))}>{t("common.retry")}</Button></div>
        : loading ? <p role="status" className="flex items-center gap-xs py-lg text-caption text-muted-foreground" data-history-loading="true"><LoaderCircleIcon aria-hidden="true" className="size-(--size-icon-sm) animate-spin" />{t("overviewHistory.loading")}</p>
        : results.length === 0 ? <p role="status" className="py-lg text-caption text-muted-foreground" data-history-empty="true">{t(state.query.trim() ? "overviewHistory.noResults" : "overviewHistory.empty")}</p>
        : <>
          <p className="py-sm text-micro text-muted-foreground" aria-live="polite">{t("overviewHistory.resultCount", { count: results.length })}</p>
          <ol className="min-w-0" data-history-session-count={results.length}>
            {results.map((hit, index) => {
              const key = workKey(hit.session);
              const matches = (hit.matches ?? []).slice(0, 2);
              const preferred = matches.find((match) => match.kind === "request");
              const original = projects.find((p) => p.workspace.device_id === hit.session.device_id && p.workspace.path === hit.project_root)?.workspace;
              const day = date(hit.at_unix_ms);
              const startsDate = index === 0 || day !== date(results[index - 1]!.at_unix_ms);
              return <li key={key} className="min-w-0" data-history-session={key}>
                {startsDate ? <h2 className="flex items-center gap-md pt-lg pb-sm text-micro font-medium text-muted-foreground" data-history-date="true"><span>{day}</span><span aria-hidden="true" className="min-w-0 flex-1 border-t border-border" /></h2> : null}
                <div className="flex min-w-0 items-start gap-lg border-b border-border py-lg">
                  <time dateTime={new Date(hit.at_unix_ms).toISOString()} className="shrink-0 pt-xs text-micro text-muted-foreground tabular-nums">{time(hit.at_unix_ms)}</time>
                  <div className="min-w-0 flex-1">
                  <button type="button" ref={(node) => { if (node) rows.current.set(key, node); else rows.current.delete(key); }} className="min-w-0 flex-1 rounded-xs text-left outline-none focus-visible:ring-1 focus-visible:ring-ring" onClick={() => open(hit, preferred)}>
                    <span className="block whitespace-normal text-body font-medium text-foreground [overflow-wrap:anywhere]">{hit.title}</span>
                  </button>
                {matches.map((match, index) => <button key={`${match.kind}:${match.request_offset}:${index}`} type="button" className="mt-sm block w-full min-w-0 rounded-xs text-left text-body outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring" onClick={() => open(hit, match)} data-history-match={match.kind} data-history-request-offset={match.request_offset ?? undefined}>
                  <span className="mr-sm text-micro text-muted-foreground">{t(`overviewHistory.kind.${match.kind}`)}</span><span className="whitespace-pre-wrap text-subtle-foreground [overflow-wrap:anywhere]"><HighlightedRequest text={match.text} query={state.query} /></span>
                </button>)}
                {!matches.length ? <p className="mt-xs text-caption text-muted-foreground">{t("overviewHistory.noRequest")}</p> : null}
                {(hit.match_count ?? 0) > matches.length ? <p className="mt-xs text-micro text-muted-foreground">{t("overviewHistory.moreMatches", { count: hit.match_count! - matches.length })}</p> : null}
                <p className="mt-sm flex min-w-0 flex-wrap items-center gap-xs text-micro text-muted-foreground"><span className="truncate" title={hit.project_root ?? undefined}>{original?.label ?? hit.project_root ?? t("overviewHistory.originalUnknown")}</span><span>·</span><AgentMark kind={hit.session.agent} /><span>{agentProductName(hit.session.agent)}</span>{hit.request_count !== undefined ? <><span>·</span><span>{t("overviewHistory.requestCount", { count: hit.request_count })}</span></> : null}</p>
                  </div>
                  <span className={cn("shrink-0 pt-xs text-micro", hit.status === "working" ? "text-agent-working" : "text-muted-foreground")} title={hit.status === "retained" || !hit.status ? t("overviewHistory.retainedHint") : undefined}>{t(`overviewHistory.${hit.status ?? "retained"}`)}</span>
                  <ChevronRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" />
                </div>
              </li>;
            })}
          </ol>
        </>}
    </div>
    <div className="flex shrink-0 flex-wrap items-center gap-sm border-t border-border px-xl py-sm">
      <Button variant="ghost" size="sm" disabled={loading || state.page === 0 || !transport} onClick={() => update((held) => historyPage(held, -1))} data-history-previous="true"><ArrowLeftIcon />{t("overviewHistory.previous")}</Button>
      <span className="text-micro text-muted-foreground">{t("overviewHistory.page", { page: state.page + 1 })}</span>
      <Button variant="ghost" size="sm" disabled={loading || failed || !state.next || state.page + 1 >= HISTORY_PAGE_LIMIT || !transport} onClick={() => update((held) => historyPage(held, 1))} data-history-next="true">{t("overviewHistory.next")}<ArrowRightIcon /></Button>
      {state.page + 1 >= HISTORY_PAGE_LIMIT && state.next ? <p role="status" className="text-micro text-muted-foreground">{t("overviewHistory.capacity")}</p> : null}
    </div>
  </section>;
}
