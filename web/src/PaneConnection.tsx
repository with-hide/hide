import { TriangleAlertIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
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
import { badgeVariants } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import { useInterfaceTranslation } from "./i18n/client";
import { connectionCopy, notConnected, offersReopen, offersSharedServerOff, reopenFailureKey, reopenPending, sharedServerOutcome } from "./paneConnectionRules";
import { remoteContext } from "./remote";
import { machineName, windowDeviceId } from "./screenMachine";
import type { PaneRow } from "./snapshot";
import { useShellStore } from "./store";

// The "Not connected" chip in a pane header and the popover that fixes it
// (PRD settings-cleanup D-11, D-12, B26 to B31). The core judges the
// connection (`PaneChildren.connection`) and owns what Reopen is doing, so
// the popover only shows the snapshot: the pending and failed states, and the
// result of turning off Codex's shared server, are read back, never assumed.
// A pane that is connected, or that has nothing to judge, draws nothing.
// Turning the shared server off also stops the one running, which disconnects
// every Codex attached to it, so the link asks first in the device-removal
// pattern with no button focused (PRD codex-daemon-apply B1-B3, D-06).

export function PaneConnectionChip({ pane, actions, local }: { pane: PaneRow; actions: Actions; local: boolean }) {
  const connection = notConnected(pane.children?.connection);
  if (!connection) return null;
  return <ConnectionPopover paneId={pane.id} connection={connection} actions={actions} local={local} />;
}

function ConnectionPopover({
  paneId,
  connection,
  actions,
  local,
}: {
  paneId: string;
  connection: NonNullable<ReturnType<typeof notConnected>>;
  actions: Actions;
  local: boolean;
}) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const [askedOff, setAskedOff] = useState(false);
  // The answer on screen when the operator confirmed: it belongs to an
  // earlier request, so it stays hidden until the core's state moves on.
  const [askedOver, setAskedOver] = useState<string | null>(null);
  const [confirmingOff, setConfirmingOff] = useState(false);
  // One press is one event: a second press before the next snapshot would
  // otherwise send again, because the core's pending state has not arrived.
  const sentAgainst = useRef<unknown>(null);
  const rest = useShellStore((s) => s.rest);
  const device = local ? rest?.navigator?.devices?.find((row) => row.kind !== "remote") : remoteContext(rest)?.device;
  const machineId = device?.id ?? null;
  // The copy names the machine as this window does: its own is this Mac, wherever the core runs.
  const here = machineId !== null && machineId === windowDeviceId(rest?.navigator?.devices);
  const machine = machineId ? machineName(rest?.navigator?.devices, machineId, t) : "";
  const copy = connectionCopy(connection.reason);
  const pending = reopenPending(connection);
  const failure = reopenFailureKey(connection);
  const offState = JSON.stringify(device?.kit?.codex_daemon_off ?? null);
  useEffect(() => {
    if (askedOver !== null && offState !== askedOver) setAskedOver(null);
  }, [askedOver, offState]);
  const outcome = askedOver === offState ? null : sharedServerOutcome(device?.kit?.codex_daemon_off, askedOff);
  const offersOff = offersSharedServerOff(connection) && machineId !== null && outcome?.phase !== "pending";
  // A confirmation whose link went away (the server is off, the pane
  // reconnected, a turn-off already runs) closes rather than send a request
  // that no longer matches what it named.
  useEffect(() => {
    if (confirmingOff && !offersOff) setConfirmingOff(false);
  }, [confirmingOff, offersOff]);
  const once = (send: () => void) => {
    if (sentAgainst.current === rest) return;
    sentAgainst.current = rest;
    send();
  };
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        // The confirmation takes the focus; the popover stays to show the answer.
        if (!next && confirmingOff) return;
        setOpen(next);
        if (!next) setAskedOff(false);
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          className={`${badgeVariants({ variant: "outline" })} cursor-pointer gap-xxs text-warning outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring`}
          data-pane-connection={connection.reason}
          data-pane-connection-pane={paneId}
        >
          <TriangleAlertIcon aria-hidden="true" />
          {t("panes.connection.chip")}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        role="dialog"
        aria-label={t(copy.title)}
        className="flex flex-col gap-sm"
        data-pane-connection-popover={connection.reason}
      >
        <strong className="text-subhead text-foreground">{t(copy.title)}</strong>
        <p className="text-body text-muted-foreground">{t(copy.reason)}</p>
        <div className="flex items-center gap-sm">
          {offersReopen(connection) && copy.reopen ? (
            <Button size="sm" aria-busy={pending} disabled={pending} data-pane-reopen={paneId} onClick={() => once(() => actions.reopenPane(paneId))}>
              {pending ? t("panes.connection.reopening") : t(copy.reopen)}
            </Button>
          ) : null}
          <Button size="sm" variant="ghost" data-pane-connection-dismiss={paneId} onClick={() => setOpen(false)}>
            {t("panes.connection.notNow")}
          </Button>
        </div>
        {failure ? (
          <p className="text-caption text-destructive" role="alert" data-pane-reopen-failed={connection.reopen?.state === "failed" ? connection.reopen.reason : undefined}>
            {t(failure)}
          </p>
        ) : null}
        {offersSharedServerOff(connection) && machineId ? (
          <div className="flex flex-col gap-xxs border-t border-border pt-sm">
            <Button
              variant="link"
              size="sm"
              className="h-auto justify-start p-none text-caption"
              disabled={outcome?.phase === "pending"}
              data-codex-shared-server-off={machineId}
              onClick={() => setConfirmingOff(true)}
            >
              {t("panes.connection.sharedServer.link")}
            </Button>
            <span className="text-caption text-muted-foreground">
              {here ? t("panes.connection.sharedServer.scopeLocal") : t("panes.connection.sharedServer.scopeDevice", { device: machine })}
            </span>
          </div>
        ) : null}
        {/* The answer outlives the reason it fixed: once the server is off, this pane reads "started before Hide was set up". */}
        {outcome ? (
          <span
            className={`text-caption ${outcome.phase === "failed" ? "text-destructive" : "text-muted-foreground"}`}
            role={outcome.phase === "failed" ? "alert" : "status"}
            data-codex-shared-server-outcome={outcome.phase}
          >
            {outcome.phase === "pending" ? t("panes.connection.sharedServer.pending") : outcome.phase === "done" ? t("panes.connection.sharedServer.done") : t(outcome.key)}
          </span>
        ) : null}
      </PopoverContent>
      {confirmingOff && offersOff && machineId ? (
        <AlertDialog open onOpenChange={(next) => { if (!next) setConfirmingOff(false); }}>
          <AlertDialogContent data-codex-shared-server-confirm={machineId}>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {here ? t("panes.connection.sharedServer.confirmTitleLocal") : t("panes.connection.sharedServer.confirmTitleDevice", { device: machine })}
              </AlertDialogTitle>
              <AlertDialogDescription>{t("panes.connection.sharedServer.confirmEffect")}</AlertDialogDescription>
            </AlertDialogHeader>
            <p className="text-body text-subtle-foreground" data-codex-shared-server-effect="disconnect">
              {t("panes.connection.sharedServer.confirmDisconnect")}
            </p>
            <p className="text-body text-subtle-foreground" data-codex-shared-server-effect="keeps">
              {t("panes.connection.sharedServer.confirmKeeps")}
            </p>
            <AlertDialogFooter>
              <AlertDialogCancel data-codex-shared-server-keep={machineId}>{t("panes.connection.sharedServer.confirmCancel")}</AlertDialogCancel>
              <AlertDialogAction
                data-codex-shared-server-go={machineId}
                onClick={() =>
                  once(() => {
                    setAskedOff(true);
                    setAskedOver(offState);
                    actions.turnOffCodexSharedServer(machineId);
                  })
                }
              >
                {t("panes.connection.sharedServer.confirmGo")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </Popover>
  );
}
