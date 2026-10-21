import { ArrowDownIcon, ArrowUpIcon, CheckIcon, CircleHelpIcon, CircleMinusIcon, ClockIcon, FileDiffIcon, GitCommitHorizontalIcon, GitCompareArrowsIcon, LoaderCircleIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import type { Actions } from "./actions";
import { useElapsed, useRemaining } from "./components/elapsed";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { useShellStore } from "./store";
import type { CheckoutCi, CheckoutCommitRecord, CheckoutGitHeader as Header, WorkspaceView } from "./workspace";

const iconClass = "size-(--size-icon-sm)";

/** Small details reuse the shell's layer and focus-return contract. */
function Detail({ label, mark, children, className = "", ...attributes }: { label: string; mark: React.ReactNode; children: React.ReactNode; className?: string } & React.HTMLAttributes<HTMLButtonElement>) {
  return <Popover><Hint label={label}><PopoverTrigger asChild><Button variant="ghost" size="sm" className={`h-(--size-control-sm) min-w-0 gap-xs px-xs text-caption font-normal ${className}`} aria-label={label} {...attributes}>{mark}</Button></PopoverTrigger></Hint><PopoverContent side="bottom" align="end" aria-label={label} className="flex flex-col gap-xs break-words text-caption">{children}</PopoverContent></Popover>;
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

export function CheckoutUpstream({ header }: { header: Header | null | undefined }) {
  const { t, i18n } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const facts = header?.facts;
  if (!live || !facts || header?.state !== "fresh") return null;
  const label = facts.upstream_state === "tracked" ? t("checkoutGit.sync", { target: facts.upstream, ahead: facts.ahead, behind: facts.behind }) : t(`checkoutGit.upstream.${facts.upstream_state}`);
  if (facts.upstream_state === "tracked" && facts.ahead === 0 && facts.behind === 0) return null;
  return <Detail label={label} data-checkout-upstream={facts.upstream_state} mark={facts.upstream_state === "tracked" ? <>{facts.ahead ? <><ArrowUpIcon className={`${iconClass} text-success`} /><span>{facts.ahead}</span></> : null}{facts.behind ? <><ArrowDownIcon className={`${iconClass} text-warning`} /><span>{facts.behind}</span></> : null}</> : <GitCompareArrowsIcon className={iconClass} />}>
    <p>{label}</p>{facts.upstream ? <p className="font-mono">{facts.upstream}</p> : null}<p className="text-muted-foreground">{t("checkoutGit.fetched", { time: facts.last_fetch_unix_ms ? new Date(facts.last_fetch_unix_ms).toLocaleString(i18n.language) : t("checkoutGit.notKnown") })}</p>
  </Detail>;
}

const ciIcons = { loading: LoaderCircleIcon, passing: CheckIcon, failed: XIcon, pending: LoaderCircleIcon, none: CircleMinusIcon, unknown: CircleHelpIcon, unavailable: TriangleAlertIcon, stale: ClockIcon };
const ciColors = { loading: "text-muted-foreground", passing: "text-success", failed: "text-destructive", pending: "text-warning", none: "text-muted-foreground", unknown: "text-muted-foreground", unavailable: "text-muted-foreground", stale: "text-muted-foreground" };

function CommitHistoryRow({ commit, ci, header, view, actions }: { commit: CheckoutCommitRecord; ci: CheckoutCi | null; header: Header; view: WorkspaceView; actions: Actions }) {
  const { t, i18n } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const elapsed = useElapsed(commit.commit_unix_seconds == null ? null : commit.commit_unix_seconds * 1000);
  const expired = useRemaining(ci?.checked_at_unix_ms == null ? null : ci.checked_at_unix_ms + 90_000) === "past";
  const state = !live || header.state !== "fresh" || expired ? "stale" : ci?.state ?? "unknown";
  const Icon = ciIcons[state];
  const label = t(`checkoutGit.ci.${state}`);
  const canOpen = Boolean(ci?.url && state !== "stale");
  return <li className="min-w-0 py-xs" data-commit-history-item={commit.sha}>
    <p className="line-clamp-2 break-words text-caption font-medium">{commit.subject}</p>
    <div className="mt-xxs flex min-w-0 items-center gap-sm text-micro text-muted-foreground">
      <span className="shrink-0 font-mono">{commit.sha.slice(0, 7)}</span>
      <time className="min-w-0 truncate" dateTime={commit.commit_unix_seconds == null ? undefined : new Date(commit.commit_unix_seconds * 1000).toISOString()} title={commit.commit_unix_seconds == null ? undefined : new Date(commit.commit_unix_seconds * 1000).toLocaleString(i18n.language)}>{elapsed ? t("checkoutGit.ago", { time: elapsed }) : t("checkoutGit.notKnown")}</time>
      {ci && ci.state !== "none" ? <Hint label={`${label} · ${t("checkoutGit.checked", { time: ci.checked_at_unix_ms ? new Date(ci.checked_at_unix_ms).toLocaleString(i18n.language) : t("checkoutGit.notKnown") })}`}><button type="button" disabled={!canOpen} aria-label={`${label}${canOpen ? ` · ${t("checkoutGit.openChecks")}` : ""}`} data-commit-ci={state} className={`ml-auto inline-flex shrink-0 items-center gap-xxs rounded-sm text-micro ${ciColors[state]} enabled:cursor-pointer enabled:hover:underline focus-visible:outline focus-visible:outline-ring`} onClick={() => {
        const current = useShellStore.getState().rest?.workspace_view;
        const currentCi = current?.git?.recent_ci?.find((row) => row.sha === commit.sha) ?? (current?.git?.ci?.sha === commit.sha ? current.git.ci : null);
        if (current?.device_id === view.device_id && current.path === view.path && current.git?.facts?.head_sha === header.facts?.head_sha && current.git?.state === "fresh" && useShellStore.getState().connection === "live" && currentCi?.state !== "stale" && currentCi?.url === ci.url && currentCi.checked_at_unix_ms != null && Date.now() - currentCi.checked_at_unix_ms <= 90_000) actions.openBrowser(ci.url!, { device_id: view.device_id, path: view.path });
      }}><Icon className={iconClass} />{t(`checkoutGit.short.${state}`)}</button></Hint> : null}
    </div>
  </li>;
}

function CommitHistory({ header, view, actions }: { header: Header; view: WorkspaceView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const facts = header.facts;
  const commits = facts?.recent_commits?.length ? facts.recent_commits.slice(0, 5) : facts?.head_sha ? [{ sha: facts.head_sha, subject: facts.subject ?? facts.head_sha.slice(0, 7), commit_unix_seconds: facts.commit_unix_seconds }] : [];
  return <section data-commit-history="true"><p className="mb-xs text-caption font-medium">{t("checkoutGit.history")}</p>{!live || header.state !== "fresh" ? <p role="status" className="text-micro text-muted-foreground">{t("checkoutGit.stale")}</p> : null}<ol className="m-0 list-none divide-y divide-border p-0">{commits.map((commit) => <CommitHistoryRow key={commit.sha} commit={commit} header={header} view={view} actions={actions} ci={header.recent_ci?.find((row) => row.sha === commit.sha) ?? (header.ci?.sha === commit.sha ? header.ci : null)} />)}</ol></section>;
}

export function CheckoutCommit({ header, view, actions, title, time }: { header: Header | null | undefined; view: WorkspaceView; actions: Actions; title: boolean; time: boolean }) {
  const { t } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const facts = header?.facts;
  const elapsed = useElapsed(facts?.commit_unix_seconds == null ? null : facts.commit_unix_seconds * 1000);
  const expired = useRemaining(header?.ci?.checked_at_unix_ms == null ? null : header.ci.checked_at_unix_ms + 90_000) === "past";
  if (!facts?.head_sha || !header) return null;
  const head = facts.head_sha;
  const ci = header.ci?.sha === head ? header.ci : null;
  const state = !live || header.state !== "fresh" || expired ? "stale" : ci?.state ?? "unknown";
  const CiIcon = ciIcons[state];
  const history = <CommitHistory header={header} view={view} actions={actions} />;
  return <>
    <Detail key={head} label={t("checkoutGit.commit", { subject: facts.subject })} className="min-w-(--size-control-sm) shrink" data-checkout-commit={head} mark={<><GitCommitHorizontalIcon className={iconClass} />{title ? <span className="min-w-0 truncate" data-checkout-subject="true">{facts.subject}</span> : null}{time && elapsed ? <span className="shrink-0 text-muted-foreground" data-checkout-age="true">{t("checkoutGit.ago", { time: elapsed })}</span> : null}</>}>{history}</Detail>
    {ci?.state !== "none" ? <Detail key={`${head}:ci`} label={t(`checkoutGit.ci.${state}`)} className={ciColors[state]} data-checkout-ci={state} mark={<><CiIcon className={iconClass} /><span>CI</span></>}>{history}</Detail> : null}
  </>;
}
