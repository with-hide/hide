// The two confirmations of 세션 보관 (PRD session-archive B6, B28): stopping
// running work to archive it, and deleting an archived session. Neither
// button holds the keyboard when a sheet opens, so an Enter meant for
// something else presses neither.

import type { Actions } from "./actions";
import { archiveWorking } from "./archive";
import { markTone, rowAccessibleName } from "./agentRow";
import { agentProductName } from "./agentAdapters";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { AgentRecordTitle } from "./components/agent-record-title";
import { RowMark } from "./components/subtree-list";
import { useInterfaceTranslation } from "./i18n/client";
import { agentsByPane, allAgents } from "./paneAgents";
import { useShellStore } from "./store";
import { useUiStore } from "./ui";

/** 작업을 멈추고 보관할까요?: what is working now in the session, then 작업 멈추고 보관 / 취소 (B6). */
export function ConfirmArchive({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pending = useUiStore((s) => s.pendingArchive);
  // Re-derived from the snapshot only while it is open; closed, this always-mounted gate ignores it.
  const agents = useShellStore((s) => (pending ? s.agents : null));
  const remote = useShellStore((s) => (pending ? s.rest?.status?.remote : undefined));
  const byPane = agents ? agentsByPane(allAgents(agents, remote)) : null;
  const root = pending && byPane ? byPane.get(pending.paneId) : undefined;
  const working = root && byPane ? archiveWorking(root, byPane) : [];
  return (
    <AlertDialog open={pending != null && root != null} onOpenChange={(open) => { if (!open) actions.cancelArchive(); }}>
      {root ? (
        <AlertDialogContent data-confirm-archive={root.pane_id} initialFocus="container">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("archive.confirm.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("archive.confirm.working")}</AlertDialogDescription>
          </AlertDialogHeader>
          <ul className="flex min-w-0 flex-col gap-xxs text-caption" data-archive-working="true">
            {working.map((agent) => (
              <li key={agent.pane_id} aria-label={rowAccessibleName(t, agent, null)} data-archive-working-row={agent.pane_id} className="flex min-w-0 items-start gap-xs">
                <span className="flex w-(--size-agent-mark) shrink-0 justify-center"><RowMark mark={agent.mark} tone={markTone(agent)} status={null} /></span>
                <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-xs">
                  <AgentRecordTitle agent={agent} className="text-foreground" />
                  <span className="shrink-0 text-subtle-foreground">{agentProductName(agent.agent_kind)}</span>
                </span>
              </li>
            ))}
          </ul>
          <AlertDialogFooter>
            <AlertDialogCancel data-archive-cancel="true">{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => actions.confirmArchive()} data-archive-confirm="true">{t("archive.confirm.action")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/** 보관함에서 지울까요?: names the session and what stays, then 보관함에서 지우기 / 남겨두기 (B28). */
export function ConfirmArchiveDelete({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pending = useUiStore((s) => s.pendingArchiveDelete);
  const row = useShellStore((s) => (pending ? s.rest?.navigator?.archived_sessions?.find((archived) => archived.id === pending.id) : undefined));
  const name = row?.members[0]?.title ?? "";
  return (
    <AlertDialog open={pending != null && row != null} onOpenChange={(open) => { if (!open) actions.keepArchived(); }}>
      {row ? (
        <AlertDialogContent data-confirm-archive-delete={row.id} initialFocus="container">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("archive.delete.title", { name })}</AlertDialogTitle>
            <AlertDialogDescription>{t("archive.delete.body")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-archive-keep="true">{t("archive.delete.keep")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => actions.confirmArchiveDelete()} data-archive-delete="true">{t("archive.delete.action")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}
