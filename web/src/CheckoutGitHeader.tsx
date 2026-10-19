import { ArrowDownIcon, ArrowUpIcon, CheckIcon, CircleHelpIcon, CircleMinusIcon, ClockIcon, FileDiffIcon, GitCommitHorizontalIcon, GitCompareArrowsIcon, LoaderCircleIcon, TriangleAlertIcon, XIcon } from "lucide-react";
import type { Actions } from "./actions";
import { useElapsed, useRemaining } from "./components/elapsed";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { Hint } from "./components/ui/tooltip";
import { useInterfaceTranslation } from "./i18n/client";
import { useShellStore } from "./store";
import type { CheckoutGitHeader as Header, WorkspaceView } from "./workspace";

const iconClass = "size-(--size-icon-sm)";

/** Small details reuse the shell's layer and focus-return contract. */
function Detail({ label, mark, children, className = "", ...attributes }: { label: string; mark: React.ReactNode; children: React.ReactNode; className?: string } & React.HTMLAttributes<HTMLButtonElement>) {
  return <Popover><Hint label={label}><PopoverTrigger asChild><Button variant="ghost" size="sm" className={`h-(--size-control-sm) min-w-0 gap-xs px-xs text-caption font-normal ${className}`} aria-label={label} {...attributes}>{mark}</Button></PopoverTrigger></Hint><PopoverContent align="end" aria-label={label} className="flex flex-col gap-xs break-words text-caption">{children}</PopoverContent></Popover>;
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

export function CheckoutCommit({ header, view, actions, title, time }: { header: Header | null | undefined; view: WorkspaceView; actions: Actions; title: boolean; time: boolean }) {
  const { t, i18n } = useInterfaceTranslation();
  const live = useShellStore((state) => state.connection === "live");
  const facts = header?.facts;
  const elapsed = useElapsed(facts?.commit_unix_seconds == null ? null : facts.commit_unix_seconds * 1000);
  const expired = useRemaining(header?.ci?.checked_at_unix_ms == null ? null : header.ci.checked_at_unix_ms + 90_000) === "past";
  if (!facts?.head_sha) return null;
  const head = facts.head_sha;
  const ci = header?.ci?.sha === head ? header.ci : null;
  const state = !live || header?.state !== "fresh" || expired ? "stale" : ci?.state ?? "unknown";
  const ciLabel = t(`checkoutGit.ci.${state}`);
  const CiIcon = ({ loading: LoaderCircleIcon, passing: CheckIcon, failed: XIcon, pending: LoaderCircleIcon, none: CircleMinusIcon, unknown: CircleHelpIcon, unavailable: TriangleAlertIcon, stale: ClockIcon })[state];
  const commitLabel = t("checkoutGit.commit", { subject: facts.subject });
  return <>
    <Detail key={head} label={commitLabel} className="min-w-(--size-control-sm) shrink" data-checkout-commit={head} mark={<><GitCommitHorizontalIcon className={iconClass} />{title ? <span className="min-w-0 truncate" data-checkout-subject="true">{facts.subject}</span> : null}{time && elapsed ? <span className="shrink-0 text-muted-foreground" data-checkout-age="true">{t("checkoutGit.ago", { time: elapsed })}</span> : null}</>}>
      <p className="font-medium">{facts.subject}</p><p className="font-mono">{head.slice(0, 12)}</p><p>{facts.commit_unix_seconds == null ? t("checkoutGit.notKnown") : new Date(facts.commit_unix_seconds * 1000).toLocaleString(i18n.language)}</p>{header?.state !== "fresh" || !live ? <p role="status">{t("checkoutGit.stale")}</p> : null}
    </Detail>
    <Detail key={`${head}:ci`} label={ciLabel} data-checkout-ci={state} mark={<><CiIcon className={iconClass} /><span>CI</span></>}>
      <p className="font-medium">{ciLabel}</p><p className="font-mono" data-ci-sha="true">{head}</p><p>{t("checkoutGit.checked", { time: ci?.checked_at_unix_ms ? new Date(ci.checked_at_unix_ms).toLocaleString(i18n.language) : t("checkoutGit.notKnown") })}</p><p className="text-muted-foreground">{t("checkoutGit.ciMeaning")}</p>
      {ci?.url && state !== "stale" ? <Button variant="ghost" size="sm" onClick={() => {
        const current = useShellStore.getState().rest?.workspace_view;
        if (current?.device_id === view.device_id && current.path === view.path && current.git?.facts?.head_sha === head && current.git.ci?.url === ci.url) actions.openBrowser(ci.url!, { device_id: view.device_id, path: view.path });
      }}>{t("checkoutGit.openChecks")}</Button> : null}
    </Detail>
  </>;
}
