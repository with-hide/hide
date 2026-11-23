import { EllipsisIcon } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import type { Actions } from "./actions";
import { EntryContextMenu, EntryDropdown, type MenuEntry } from "./components/entry-menu";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "./components/ui/alert-dialog";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { RowButton } from "./components/ui/row-button";
import { Hint } from "./components/ui/tooltip";
import { guestPanes, guestTabs, type GuestPane } from "./guestWork";
import { useInterfaceTranslation } from "./i18n/client";
import { cn } from "./lib/utils";
import { catalogWorkspaces } from "./snapshot";
import { useShellStore } from "./store";
import { attachTerminal, focusTerminal } from "./terminals";
import { useUiStore } from "./ui";
import type { TFunction } from "i18next";

type GuestMenuId = "split" | "new_tab" | "rename" | "start_agent" | "close";

/**
 * What a window offers on another machine's pane (PRD device-connect-iroh
 * D-74, B10, B68): splitting, a new tab, renaming and starting an agent are
 * drawn dimmed with the machine to do them on, whether or not that machine
 * is connected, and closing is the one thing left to choose.
 */
export function guestPaneMenu(machine: string, t: TFunction<"translation">): MenuEntry<GuestMenuId>[] {
  const there = t("pairing.work.onThatMachine", { name: machine });
  return [
    { id: "split", label: t("pairing.work.split"), unavailable: there },
    { id: "new_tab", label: t("pairing.work.newTab"), unavailable: there },
    { id: "rename", label: t("pairing.work.rename"), unavailable: there },
    { id: "start_agent", label: t("pairing.work.startAgent"), unavailable: there },
    { id: "close", label: t("pairing.work.close"), unavailable: null, separated: true, destructive: true },
  ];
}

/** Every pane of another machine's work on this computer, placed among this computer's projects and named in this window's words. */
export function useGuestPanes(): GuestPane[] {
  const { t } = useInterfaceTranslation();
  const guests = useShellStore((s) => s.rest?.navigator?.guest_panes);
  const peers = useShellStore((s) => s.rest?.navigator?.peers);
  const projects = useShellStore((s) => s.rest?.navigator?.workspaces);
  // The three inputs and the language are the whole of what the list reads.
  return useMemo(() => {
    const rest = useShellStore.getState().rest;
    return guestPanes(rest, catalogWorkspaces(rest).filter((project) => !project.remote_target_id), (branch) => t("pairing.work.terminal", { branch }));
  }, [guests, peers, projects, t]);
}

/**
 * The center while it shows a pane of another machine's work on this
 * computer, drawn as this computer's own pane is: the tabs of that
 * machine's work in the same project by their names, the pane's line with
 * where it stands, its name, its project, and once the machine that
 * started it, and its terminal. The pane is the other machine's, so this
 * window keeps no state of it: when it is gone from the list, the page says
 * so.
 */
export function GuestPaneScreen({ paneId, actions }: { paneId: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const panes = useGuestPanes();
  const pane = panes.find((row) => row.id === paneId) ?? null;
  if (pane === null) {
    return (
      <div className="flex flex-1 items-center justify-center text-caption text-muted-foreground" role="status" data-guest-pane-gone="true">
        {t("pairing.work.gone")}
      </div>
    );
  }
  const tabs = guestTabs(panes, pane);
  const select = (id: GuestMenuId) => {
    if (id === "close") actions.askCloseGuestPane(pane.id);
  };
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-center-screen="guest" data-guest-pane={pane.id}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-stretch border-b border-border">
        <div role="tablist" aria-label={t("pairing.work.tabs", { name: pane.machine })} className="flex min-w-0 items-stretch overflow-x-auto" data-guest-tabs={pane.deviceId}>
          {tabs.map((tab) => (
            <GuestTab key={tab.id} tab={tab} active={tab.id === pane.id} actions={actions} />
          ))}
        </div>
      </div>
      <EntryContextMenu label={t("pairing.work.menu", { label: pane.label })} items={() => guestPaneMenu(pane.machine, t)} onSelect={select} className="flex min-h-0 min-w-0 flex-1 flex-col" data-guest-pane-menu={pane.id}>
        <header className="flex h-(--size-pane-header) shrink-0 items-center gap-xs bg-card pl-sm pr-xs text-caption text-muted-foreground" data-guest-pane-header={pane.id}>
          {pane.status ? <StatusMark mark={pane.status} data-guest-pane-status={pane.status} /> : null}
          <Hint label={pane.folder ?? pane.label}>
            <span className="min-w-0 truncate font-medium text-foreground" data-guest-pane-name="true">{pane.label}</span>
          </Hint>
          {pane.project ? <span className="min-w-0 shrink-[2] truncate" data-guest-pane-project="true">· {pane.project}</span> : null}
          <span className="min-w-0 flex-1" />
          <GuestMark pane={pane} />
          <span className="min-w-0 shrink-[3] truncate" data-guest-pane-started="true">{t("pairing.work.group", { name: pane.machine })}</span>
          <EntryDropdown
            label={t("pairing.work.menu", { label: pane.label })}
            items={() => guestPaneMenu(pane.machine, t)}
            onSelect={select}
            hint={t("pairing.work.menu", { label: pane.label })}
            trigger={
              <Button variant="ghost" size="icon-sm" data-guest-pane-more={pane.id}>
                <EllipsisIcon aria-hidden="true" />
              </Button>
            }
          />
        </header>
        <GuestTerminal key={pane.id} paneId={pane.id} actions={actions} />
      </EntryContextMenu>
    </div>
  );
}

function GuestTab({ tab, active, actions }: { tab: GuestPane; active: boolean; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <EntryContextMenu
      asChild
      label={t("pairing.work.menu", { label: tab.label })}
      items={() => guestPaneMenu(tab.machine, t)}
      onSelect={(id) => {
        if (id === "close") actions.askCloseGuestPane(tab.id);
      }}
    >
      <RowButton
        role="tab"
        aria-selected={active}
        aria-label={t("pairing.work.rowAria", { label: tab.label, name: tab.machine })}
        data-guest-tab={tab.id}
        className={cn("w-auto max-w-(--size-recent-location-max) shrink-0 rounded-none px-sm py-0 text-caption", active ? "bg-background text-foreground hover:bg-background" : "text-subtle-foreground")}
        onClick={() => actions.openGuestPane(tab.id)}
      >
        <span className="min-w-0 truncate">{tab.label}</span>
      </RowButton>
    </EntryContextMenu>
  );
}

/** The machine's letters beside a row or a tab of its work, with its name as the hint (B10). */
export function GuestMark({ pane }: { pane: Pick<GuestPane, "machine" | "mark"> }) {
  return (
    <Hint label={pane.machine}>
      <span className="shrink-0 rounded-xs bg-device-remote px-xs text-micro font-medium text-primary-foreground" data-guest-mark={pane.machine}>
        {pane.mark}
      </span>
    </Hint>
  );
}

/** The pane's terminal: drawn and typed into by its address here, and sized by this window while it is the one typing. */
function GuestTerminal({ paneId, actions }: { paneId: string; actions: Actions }) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!host.current) return undefined;
    const park = attachTerminal(paneId, host.current, actions.dispatch, actions, 1);
    focusTerminal(paneId);
    return park;
  }, [paneId, actions]);
  // The terminal's element is placed against its host, as in a pane of this computer.
  return <div ref={host} className="relative min-h-0 min-w-0 flex-1 bg-background" data-guest-terminal={paneId} />;
}

/** The confirmation before this window ends another machine's pane: the work in it stops (B10). */
export function ConfirmGuestPaneClose({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const pending = useUiStore((s) => s.guestClose);
  const panes = useGuestPanes();
  const pane = pending ? (panes.find((row) => row.id === pending) ?? null) : null;
  return (
    <AlertDialog open={pane !== null} onOpenChange={(open) => { if (!open) actions.cancelGuestPaneClose(); }}>
      {pane ? (
        <AlertDialogContent data-confirm-guest-close={pane.id}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("pairing.work.closeTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("pairing.work.closeBody", { name: pane.machine })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-guest-close-cancel="true">{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => actions.closeGuestPane()} data-guest-close-go="true">
              {t("pairing.work.closeGo")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}
