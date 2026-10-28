import { useInterfaceTranslation } from "./i18n/client";
import { ChevronDownIcon, LaptopIcon, LayoutDashboardIcon, SearchIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "./actions";
import { deviceScope } from "./agentScope";
import { deviceConnected, frontDeviceId, tileCounts, tileHint, tileMonogram } from "./devices";
import { machineName, windowDeviceId, windowFirst, wslPlace } from "./screenMachine";
import { useShellStore } from "./store";
import { overviewDeviceOf } from "./overviewTasks";
import { useUiStore } from "./ui";
import { MainScreen } from "./MainScreen";
import { Button } from "./components/ui/button";
import { Dialog, DialogContent, DialogTitle } from "./components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { Kbd } from "./components/ui/kbd";
import { Hint } from "./components/ui/tooltip";
import { commandLabel } from "./shortcutLabels";
import { cn } from "./lib/utils";

/** The same selected-device group that the Agents sidebar draws, less the Factory's workers, which 내 차례 counts (PRD software-factory-ui B13). */
export function useOverviewCount() {
  return useShellStore((s) => deviceScope(s.rest, frontDeviceId(s.rest))?.overview_needs_you ?? 0);
}

export function OverviewButton({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const count = useOverviewCount();
  const open = useUiStore((s) => s.overviewOpen);
  const chord = useShellStore((s) => commandLabel("overview", s.rest?.ui_state));
  const description = count > 0 ? t("overview.needsYou", { count }) : undefined;
  return <Hint label={count > 0 ? t("overview.needsYouHint", { count }) : t("overview.title")} shortcut={chord}>
    <Button variant={open ? "secondary" : "ghost"} size="icon-sm" className="relative" aria-label={t("overview.title")} aria-description={description} aria-pressed={open} data-open-overview="true" onClick={actions.toggleOverview}>
      <LayoutDashboardIcon />
      {count > 0 ? <span aria-hidden="true" className="absolute right-xxs top-xxs size-(--size-tab-status-dot) rounded-full bg-warning" data-overview-dot="true" /> : null}
    </Button>
  </Hint>;
}

/**
 * The device the Overview shows (PRD overview-device-tasks D-02): the one its
 * own switch picked, else the one a device Home row named, else the rail's
 * front device. A device removed since it was picked leaves the front one.
 */
export function useOverviewDevice(): string {
  const picked = useUiStore((s) => s.overview.deviceId);
  const named = useUiStore((s) => (s.screen?.kind === "main" ? s.screen.deviceId : undefined));
  return useShellStore((s) => overviewDeviceOf(s.rest, picked, named));
}

/**
 * The Overview of one device (B1-B4): a title, the device switch, and the
 * tabs below. As a layer over work or as the page a device's Home row opens;
 * either way the switch changes only what the Overview shows (B2).
 */
export function OverviewPage({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const deviceId = useOverviewDevice();
  return <div className="flex min-h-0 flex-1 flex-col" data-overview-page="true" data-overview-device={deviceId}>
    <div className="flex shrink-0 flex-wrap items-center gap-md border-b border-border px-lg py-md">
      <h1 className="shrink-0 text-headline font-semibold">{t("overview.title")}</h1>
      <span className="flex-1" />
      <Hint label={t("commands.search")}><Button variant="ghost" size="icon-sm" aria-label={t("commands.search")} onClick={actions.openSearch}><SearchIcon /></Button></Hint>
      <DeviceSwitch selected={deviceId} />
      <Button variant="ghost" size="sm" aria-label={t("overview.close")} onClick={actions.closeOverview}><Kbd>Esc</Kbd></Button>
    </div>
    <MainScreen actions={actions} deviceId={deviceId} />
  </div>;
}

/**
 * A compact device choice names its connection state. Picking another
 * device changes only the Overview and clears its previous project scope.
 */
export function DeviceSwitch({ selected }: { selected: string }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const rows = useMemo(() => {
    const own = windowDeviceId(devices);
    return windowFirst(devices).map((device) => ({ id: device.id, own: device.id === own, label: machineName(devices, device.id, t), place: wslPlace(devices, device.id, t) }));
  }, [devices, t]);
  if (rows.length === 0) return null;
  const current = rows.find((row) => row.id === selected) ?? rows[0]!;
  return <div className="min-w-0 max-w-full" data-overview-devices="true">
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="outline" size="sm" aria-label={t("overviewTasks.devices")} className="max-w-full"><DeviceLabel {...current} /><ChevronDownIcon /></Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end" aria-label={t("overviewTasks.devices")}>
        <DropdownMenuRadioGroup value={selected} onValueChange={(id) => useUiStore.getState().setOverview({ deviceId: id })}>
          {rows.map((row) => <DeviceChoice key={row.id} {...row} />)}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>;
}

/** A device's letters and name, then, for a WSL distribution, where it is, as the sidebar title says it (PRD wsl-device B5). */
function DeviceLabel({ id, own, label, place }: { id: string; own: boolean; label: string; place: string | null }) {
  const { t } = useInterfaceTranslation();
  const connected = useShellStore((s) => deviceConnected(s.rest, id));
  return <span className="flex min-w-0 items-center gap-xs">
    <span aria-hidden="true" className="shrink-0 text-muted-foreground">{own ? <LaptopIcon className="size-(--size-icon)" /> : tileMonogram(label)}</span>
    <span className="truncate font-medium">{label}</span>
    {place ? <span className="shrink-0 text-caption text-muted-foreground" data-device-place="true">{place}</span> : null}
    <span className={cn("shrink-0 text-micro", connected ? "text-success" : "text-muted-foreground")} data-device-choice-offline={!connected || undefined}>{t(connected ? "devices.line.ready" : "devices.rail.notConnected")}</span>
  </span>;
}

const DeviceChoice = memo(function DeviceChoice({ id, own, label, place }: { id: string; own: boolean; label: string; place: string | null }) {
  const { t } = useInterfaceTranslation();
  const counts = useShellStore(useShallow((s) => tileCounts(s.rest, s.agents, id)));
  const connected = useShellStore((s) => deviceConnected(s.rest, id));
  return <DropdownMenuRadioItem
      value={id}
      title={tileHint(place ? `${label} · ${place}` : label, connected, counts, t)}
      data-overview-device-choice={id}
      data-connected={connected ? "true" : "false"}
    >
      <DeviceLabel id={id} own={own} label={label} place={place} />
    </DropdownMenuRadioItem>;
});

export function OverviewModal({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const open = useUiStore((s) => s.overviewOpen);
  return <Dialog baseEscape open={open} onOpenChange={(next) => { if (!next) actions.closeOverview(); }}>
    {open ? <DialogContent initialFocus="container" aria-describedby={undefined} className="h-[calc(100%-2*var(--size-settings-sheet-window-inset))] max-h-(--size-overview-sheet-h) w-[calc(100%-2*var(--size-settings-sheet-window-inset))] max-w-(--size-overview-sheet-w)" data-overview-modal="true" returnFocusTo={() => useUiStore.getState().overviewReturnFocus}>
      <DialogTitle className="sr-only">{t("overview.title")}</DialogTitle>
      <OverviewPage actions={actions} />
    </DialogContent> : null}
  </Dialog>;
}
