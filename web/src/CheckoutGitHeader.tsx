import { ArrowDownIcon, ArrowUpIcon, CheckIcon, CircleHelpIcon, CircleMinusIcon, ClockIcon, FileDiffIcon, GitCommitHorizontalIcon, GitCompareArrowsIcon, LoaderCircleIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import { useRef } from "react";
import type { Actions } from "./actions";
import { useElapsed, useRemaining } from "./components/elapsed";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { useObservedLine } from "./observed";
import { useShellStore } from "./store";
import type { CheckoutCi, CheckoutCommitRecord, CheckoutGitHeader as Header, WorkspaceView } from "./workspace";

const iconClass = "size-(--size-icon-sm)";

/** Small details reuse the shell's layer and focus-return contract. */
function Detail({ label, mark, children, className = "", ...attributes }: { label: string; mark: React.ReactNode; children: React.ReactNode; className?: string } & React.HTMLAttributes<HTMLButtonElement>) {
  const content = useRef<HTMLDivElement>(null);
  return <Popover><Hint label={label}><PopoverTrigger asChild><Button variant="ghost" size="sm" className={`h-(--size-control-sm) min-w-0 gap-xs px-xs text-caption font-normal ${className}`} aria-label={label} {...attributes}>{mark}</Button></PopoverTrigger></Hint><PopoverContent ref={content} tabIndex={-1} onOpenAutoFocus={(event) => { event.preventDefault(); content.current?.focus({ preventScroll: true }); }} side="bottom" align="end" aria-label={label} className="flex flex-col gap-xs break-words text-caption">{children}</PopoverContent></Popover>;
}

/** A missing/failed read is never the clean state; old counts are labelled stale. */
export function CheckoutFileCount({ header, actions }: { header: Header | null | undefined; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const state = !live ? "stale" : header?.state ?? "loading";
  const facts = header?.facts;
  const abnormal = state !== "fresh";
  const label = abnormal ? t(`checkoutGit.${state}`) : [t("checkoutGit.files", { count: facts?.changed_files ?? 0 }), facts && facts.conflicts > 0 ? t("checkoutGit.conflicts", { count: facts.conflicts }) : null].filter(Boolean).join(" · ");
  if (!abnormal && facts?.changed_files === 0) return null;
  const Icon = state === "loading" ? LoaderCircleIcon : state === "unavailable" ? TriangleAlertIcon : abnormal ? ClockIcon : facts && facts.conflicts > 0 ? TriangleAlertIcon : FileDiffIcon;
  return <Hint label={`${label} · ${t("checkoutGit.countMeaning")}`}><Button variant="ghost" size="sm" className="h-(--size-control-sm) gap-xs px-xs text-caption font-normal" aria-label={`${label} · ${t("checkoutGit.countMeaning")}`} data-checkout-files={state} data-keyboard-stays="true" onMouseDown={(event) => event.preventDefault()} onClick={() => actions.showTool("changes")}><Icon className={iconClass} />{facts ? <span className="tabular-nums">{facts.changed_files}</span> : <span>?</span>}</Button></Hint>;
}

/**
 * When the remote was last seen and whether the background fetch is failing:
 * Hide's own fetch when it fetches this repository, the operator's last
 * `git fetch` when Project Settings turned that off (PRD git-freshness D-03).
 */
function useRemoteCheck(header: Header | null | undefined) {
  const { t, i18n } = useInterfaceTranslation();
  const remote = header?.remote ?? null;
  const auto = remote?.auto ?? false;
  const checkedAt = auto ? remote?.observed?.read_at_unix_ms ?? null : header?.facts?.last_fetch_unix_ms ?? null;
  const elapsed = useElapsed(checkedAt);
  const failing = auto && remote?.observed?.freshness === "failed";
  const checked = auto ? (checkedAt != null && elapsed ? t("checkoutGit.checkedAgo", { time: elapsed }) : t("checkoutGit.notChecked")) : t("checkoutGit.fetched", { time: checkedAt != null ? new Date(checkedAt).toLocaleString(i18n.language) : t("checkoutGit.notKnown") });
  const failure = failing ? (checkedAt != null && elapsed ? t("checkoutGit.fetchFailing", { time: elapsed }) : t("checkoutGit.fetchFailingNever")) : null;
  return { checked, failure, failing };
}

export function CheckoutUpstream({ header }: { header: Header | null | undefined }) {
  const { t } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const remote = useRemoteCheck(header);
  const facts = header?.facts;
  // A Git read that failed or aged keeps its counts, dimmed, with their age (D-09).
  const git = useObservedLine(header?.state === "stale" ? header.observed ?? { freshness: "stale", read_at_unix_ms: null } : null, "git");
  if (!live || !facts || (header?.state !== "fresh" && header?.state !== "stale")) return null;
  const state = facts.upstream_state;
  const tracked = state === "tracked";
  const label = state === "tracked" ? t("checkoutGit.sync", { target: facts.upstream, ahead: facts.ahead, behind: facts.behind }) : t(`checkoutGit.upstream.${state}`);
  // Zero counts stay quiet, but the detail still opens to say when the
  // remote was last seen (B7); a failing fetch dims the counts (B5).
  const quiet = !tracked || (!facts.ahead && !facts.behind);
  return <Detail label={[label, remote.failure, git].filter(Boolean).join(" · ")} className={remote.failing || header.state === "stale" ? "opacity-50" : ""} data-checkout-upstream={facts.upstream_state} data-checkout-remote={remote.failing ? "failing" : "ok"} mark={quiet ? <GitCompareArrowsIcon className={`${iconClass} ${tracked ? "text-muted-foreground" : ""}`} /> : <>{facts.ahead ? <><ArrowUpIcon className={`${iconClass} text-success`} /><span>{facts.ahead}</span></> : null}{facts.behind ? <><ArrowDownIcon className={`${iconClass} text-warning`} /><span>{facts.behind}</span></> : null}</>}>
    <p>{label}</p>{facts.upstream ? <p className="font-mono">{facts.upstream}</p> : null}<p className="text-muted-foreground" data-checkout-remote-checked="true">{remote.checked}</p>{remote.failure ? <p className="text-muted-foreground" role="status">{remote.failure}</p> : null}{git ? <p className="text-muted-foreground" role="status" data-checkout-git-age="true">{git}</p> : null}
  </Detail>;
}

/** The CI look a commit's last answer takes, and whether that answer is current; one that is not stays, dimmed, with its age (D-09, B22). */
function useCiLook(ci: CheckoutCi | null, header: Header | null) {
  const live = useShellStore((state) => state.connection === "live");
  const observed = ci?.observed ?? null;
  const expired = useRemaining(observed?.read_at_unix_ms == null ? null : observed.read_at_unix_ms + 90_000) === "past";
  const current = live && header?.state === "fresh" && !expired && (observed === null || observed.freshness === "fresh");
  const value = ci?.state ?? "unknown";
  const look: keyof typeof ciIcons = !current && value === "loading" ? "stale" : value;
  const line = useObservedLine(current || !observed ? null : { ...observed, freshness: observed.freshness === "fresh" ? "stale" : observed.freshness }, "github");
  return { look, current, line };
}

const ciIcons = { loading: LoaderCircleIcon, passing: CheckIcon, failed: XIcon, pending: LoaderCircleIcon, none: CircleMinusIcon, unknown: CircleHelpIcon, unavailable: TriangleAlertIcon, stale: ClockIcon };
const ciColors = { loading: "text-muted-foreground", passing: "text-success", failed: "text-destructive", pending: "text-warning", none: "text-muted-foreground", unknown: "text-muted-foreground", unavailable: "text-muted-foreground", stale: "text-muted-foreground" };

/** Opens a commit's GitHub page from the header in front, never one a newer frame no longer offers. */
function openCommit(sha: string, url: string, view: WorkspaceView, actions: Actions) {
  const current = useShellStore.getState().rest?.workspace_view;
  if (current?.device_id === view.device_id && current.path === view.path && useShellStore.getState().connection === "live" && current.git?.commit_links?.some((link) => link.sha === sha && link.url === url)) actions.openBrowser(url, { device_id: view.device_id, path: view.path });
}

function CommitHistoryRow({ commit, ci, header, view, actions, incoming = false }: { commit: CheckoutCommitRecord; ci: CheckoutCi | null; header: Header; view: WorkspaceView; actions: Actions; incoming?: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const elapsed = useElapsed(commit.commit_unix_seconds == null ? null : commit.commit_unix_seconds * 1000);
  const { look: state, current, line } = useCiLook(ci, header);
  const Icon = ciIcons[state];
  const label = [t(`checkoutGit.ci.${state}`), line].filter(Boolean).join(" · ");
  const canOpen = Boolean(ci?.url && current);
  const link = header.commit_links?.find((candidate) => candidate.sha === commit.sha)?.url ?? null;
  const unpushed = Boolean(header.commit_pages && !incoming && header.facts?.unpushed_commits?.includes(commit.sha));
  const subject = <p className="line-clamp-2 break-words text-caption font-medium">{commit.subject}</p>;
  return <li className={`min-w-0 py-xs ${incoming ? "opacity-60" : ""}`} data-commit-history-item={commit.sha} data-commit-incoming={incoming ? "true" : undefined}>
    {link ? <button type="button" className="block w-full min-w-0 cursor-pointer rounded-sm text-left hover:underline focus-visible:outline focus-visible:outline-ring" aria-label={`${commit.subject} · ${t("checkoutGit.openCommit")}`} data-commit-link={commit.sha} onClick={() => openCommit(commit.sha, link, view, actions)}>{subject}</button> : subject}
    <div className="mt-xxs flex min-w-0 items-center gap-sm text-micro text-muted-foreground">
      <span className="shrink-0 font-mono">{commit.sha.slice(0, 7)}</span>
      <time className="min-w-0 truncate" dateTime={commit.commit_unix_seconds == null ? undefined : new Date(commit.commit_unix_seconds * 1000).toISOString()} title={commit.commit_unix_seconds == null ? undefined : new Date(commit.commit_unix_seconds * 1000).toLocaleString(i18n.language)}>{elapsed ? t("checkoutGit.ago", { time: elapsed }) : t("checkoutGit.notKnown")}</time>
      {incoming ? <span className="ml-auto shrink-0" data-commit-state="incoming">{t("checkoutGit.incoming")}</span> : unpushed ? <span className="ml-auto shrink-0" data-commit-state="unpushed">{t("checkoutGit.unpushed")}</span> : null}
      {!incoming && ci && ci.state !== "none" ? <Hint label={line ? label : `${label} · ${t("checkoutGit.checked", { time: ci.observed?.read_at_unix_ms ? new Date(ci.observed.read_at_unix_ms).toLocaleString(i18n.language) : t("checkoutGit.notKnown") })}`}><button type="button" disabled={!canOpen} aria-label={`${label}${canOpen ? ` · ${t("checkoutGit.openChecks")}` : ""}`} data-commit-ci={state} data-commit-ci-current={current ? "true" : "false"} className={`ml-auto inline-flex shrink-0 items-center gap-xxs rounded-sm text-micro ${ciColors[state]} ${current ? "" : "opacity-50"} enabled:cursor-pointer enabled:hover:underline focus-visible:outline focus-visible:outline-ring`} onClick={() => {
        const current = useShellStore.getState().rest?.workspace_view;
        const currentCi = current?.git?.recent_ci?.find((row) => row.sha === commit.sha) ?? (current?.git?.ci?.sha === commit.sha ? current.git.ci : null);
        if (current?.device_id === view.device_id && current.path === view.path && current.git?.facts?.head_sha === header.facts?.head_sha && current.git?.state === "fresh" && useShellStore.getState().connection === "live" && currentCi?.observed?.freshness === "fresh" && currentCi.url === ci.url && currentCi.observed.read_at_unix_ms != null && Date.now() - currentCi.observed.read_at_unix_ms <= 90_000) actions.openBrowser(ci.url!, { device_id: view.device_id, path: view.path });
      }}><Icon className={iconClass} />{t(`checkoutGit.short.${state}`)}</button></Hint> : null}
    </div>
  </li>;
}

function CommitHistory({ header, view, actions }: { header: Header; view: WorkspaceView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const remote = useRemoteCheck(header);
  const facts = header.facts;
  const commits = facts?.recent_commits?.length ? facts.recent_commits.slice(0, 5) : facts?.head_sha ? [{ sha: facts.head_sha, subject: facts.subject ?? facts.head_sha.slice(0, 7), commit_unix_seconds: facts.commit_unix_seconds }] : [];
  // The upstream's commits not pulled yet sit above the local ones, dimmed
  // (D-04); while the fetch fails they are the last ones received (B11).
  const incoming = facts?.upstream_state === "tracked" ? (facts.incoming_commits ?? []).slice(0, 5) : [];
  const more = Math.max(0, (facts?.behind ?? 0) - incoming.length);
  return <section data-commit-history="true"><p className="mb-xs text-caption font-medium">{t("checkoutGit.history")}</p>{!live || header.state !== "fresh" ? <p role="status" className="text-micro text-muted-foreground">{t("checkoutGit.stale")}</p> : null}{incoming.length && remote.failure ? <p role="status" className="text-micro text-muted-foreground" data-commit-incoming-age="true">{remote.failure}</p> : null}<ol className="m-0 list-none divide-y divide-border p-0">{incoming.map((commit) => <CommitHistoryRow key={`incoming:${commit.sha}`} incoming commit={commit} header={header} view={view} actions={actions} ci={null} />)}{more && incoming.length ? <li className="py-xs text-micro text-muted-foreground opacity-60" data-commit-incoming-more={more}>{t("checkoutGit.incomingMore", { count: more })}</li> : null}{commits.map((commit) => <CommitHistoryRow key={commit.sha} commit={commit} header={header} view={view} actions={actions} ci={header.recent_ci?.find((row) => row.sha === commit.sha) ?? (header.ci?.sha === commit.sha ? header.ci : null)} />)}</ol></section>;
}

export function CheckoutCommit({ header, view, actions, title, time }: { header: Header | null | undefined; view: WorkspaceView; actions: Actions; title: boolean; time: boolean }) {
  const { t } = useInterfaceTranslation();
  const facts = header?.facts;
  const elapsed = useElapsed(facts?.commit_unix_seconds == null ? null : facts.commit_unix_seconds * 1000);
  const head = facts?.head_sha ?? null;
  const ci = header?.ci?.sha === head ? header?.ci ?? null : null;
  const { look: state, current, line } = useCiLook(ci, header ?? null);
  if (!facts?.head_sha || !header || !head) return null;
  const CiIcon = ciIcons[state];
  const history = <CommitHistory header={header} view={view} actions={actions} />;
  return <>
    <Detail key={head} label={t("checkoutGit.commit", { subject: facts.subject })} className="min-w-(--size-control-sm) shrink" data-checkout-commit={head} mark={<><GitCommitHorizontalIcon className={iconClass} />{title ? <span className="min-w-0 truncate" data-checkout-subject="true">{facts.subject}</span> : null}{time && elapsed ? <span className="shrink-0 text-muted-foreground" data-checkout-age="true">{t("checkoutGit.ago", { time: elapsed })}</span> : null}</>}>{history}</Detail>
    {ci?.state !== "none" ? <Detail key={`${head}:ci`} label={[t(`checkoutGit.ci.${state}`), line].filter(Boolean).join(" · ")} className={`${ciColors[state]} ${current ? "" : "opacity-50"}`} data-checkout-ci={state} data-checkout-ci-current={current ? "true" : "false"} mark={<><CiIcon className={iconClass} /><span>CI</span></>}>{history}</Detail> : null}
  </>;
}
