import { useInterfaceTranslation } from "../i18n/client";
import { ChevronDownIcon, LaptopIcon, LayoutDashboardIcon, PlusIcon, SearchIcon, ServerIcon } from "lucide-react";
import { Kbd } from "./ui/kbd";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { Hint } from "./ui/tooltip";


/** A device the hidden rail's menu lists. */

export type MenuDevice = { id: string; label: string; remote: boolean; connected: boolean };

/** What the top-line name opens while the rail is hidden: the way to every device and back to the rail. */
export type DeviceMenu = {
  devices: readonly MenuDevice[];
  frontId: string;
  onSelect: (deviceId: string) => void;
  onAddDevice: () => void;
  onShowRail: () => void;
};

/**
 * The device in front (`This Mac`, `mini Remote`) as the window's top row
 * draws it over the rail and the sidebar (PRD sidebar-titlebar-refresh B17,
 * B21). While the rail is hidden the name is a menu that lists the devices,
 * Add device and Show rail, since the rail is no longer the way to another
 * device.
 */
export function DeviceTitle({ rail, title, deviceMenu }: { rail: boolean; title: { name: string; note: string | null }; deviceMenu: DeviceMenu }) {
  if (!rail) return <HiddenRailMenu title={title} menu={deviceMenu} />;
  return (
    <span className="flex min-w-0 flex-1 items-baseline gap-sm">
      <span className="min-w-0 truncate text-subhead font-semibold text-foreground" data-sidebar-title-name="true">
        {title.name}
      </span>
      {title.note ? <span className="shrink-0 text-caption text-muted-foreground">{title.note}</span> : null}
    </span>
  );
}

/**
 * The top of the sidebar under the window's top row (PRD
 * sidebar-titlebar-refresh D-08): a visible Search row with the ⌘K chord as
 * bound, then the shared Overview row and, where the host can pick a folder,
 * the Add project row under it (PRD first-run-onboarding B51), with the
 * project tree directly below. Drawn from what the caller reads out of the
 * stores, so a test renders it without them.
 */
export function SidebarHeader({
  overview,
  addProject,
  compact = false,
  searchChord,
  onSearch,
}: {
  compact?: boolean;
  overview?: { selected: boolean; count: number; chord: string | null; onOpen: () => void };
  addProject?: { chord: string | null; onOpen: () => void };
  searchChord: string | null;
  onSearch: () => void;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <div className="shrink-0 px-sm pt-sm pb-xs">
        <button
          type="button"
          data-sidebar-search="true"
          onClick={onSearch}
          className="flex h-(--size-sidebar-search) w-full items-center gap-sm rounded-sm border border-border bg-background px-sm text-left text-body text-muted-foreground outline-none hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring"
        >
          <SearchIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("common.search")}</span>
          {searchChord ? <Kbd data-sidebar-search-chord={searchChord}>{searchChord}</Kbd> : null}
        </button>
      </div>
      {overview ? (
        <Hint label={overview.count > 0 ? t("overview.needsYouHint", { count: overview.count }) : t("overview.title")} shortcut={overview.chord}>
          <button type="button" aria-current={overview.selected ? "page" : undefined} data-sidebar-overview="true" onClick={overview.onOpen}
            className={`flex h-(--size-tab-strip) shrink-0 items-center gap-sm px-md text-body outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${addProject ? "" : "border-b border-border"} ${overview.selected ? "bg-selected text-foreground" : "text-subtle-foreground hover:bg-accent"}`}>
            <LayoutDashboardIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
            <span className="min-w-0 flex-1 truncate text-left">{t("overview.title")}</span>
            {overview.count > 0 ? <span data-overview-count="true" className="text-caption text-warning">{t("overview.askingCount", { count: overview.count })}</span> : null}
            {overview.chord && !compact ? <Kbd className="sidebar-command-keycap">{overview.chord}</Kbd> : null}
          </button>
        </Hint>
      ) : null}
      {addProject ? (
        <button
          type="button"
          data-sidebar-new-workspace="true"
          onClick={addProject.onOpen}
          className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-b border-border px-md text-body text-subtle-foreground outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
        >
          <PlusIcon aria-hidden="true" className="size-(--size-icon) shrink-0" />
          <span className="min-w-0 flex-1 truncate text-left">{t("commands.new_workspace")}</span>
          {addProject.chord && !compact ? <Kbd className="sidebar-command-keycap">{addProject.chord}</Kbd> : null}
        </button>
      ) : null}
    </>
  );
}

/** The device name as a menu while the rail is hidden: every device, Add device, and the way to show the rail again. */
export function HiddenRailMenu({ title, menu }: { title: { name: string; note: string | null }; menu: DeviceMenu }) {
  const { t } = useInterfaceTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("sidebar.switchDevice", { name: `${title.name}${title.note ? ` ${title.note}` : ""}` })}
          data-sidebar-device-menu="true"
          className="flex min-w-0 flex-1 items-baseline gap-sm rounded-sm text-left outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <span className="min-w-0 truncate text-subhead font-semibold text-foreground" data-sidebar-title-name="true">
            {title.name}
          </span>
          {title.note ? <span className="shrink-0 text-caption text-muted-foreground">{title.note}</span> : null}
          <ChevronDownIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 self-center text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" aria-label={t("sidebar.devices")} data-device-menu="true">
        <DropdownMenuRadioGroup value={menu.frontId} onValueChange={menu.onSelect}>
          {menu.devices.map((device) => {
            const Icon = device.remote ? ServerIcon : LaptopIcon;
            return (
              <DropdownMenuRadioItem key={device.id} value={device.id} data-device-menu-item={device.id}>
                <Icon aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">{device.label}</span>
                {device.connected ? null : <span className="shrink-0 text-caption text-muted-foreground">{t("devices.state.unavailable")}</span>}
              </DropdownMenuRadioItem>
            );
          })}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem data-device-menu-add="true" onSelect={menu.onAddDevice}>
          <PlusIcon aria-hidden="true" />
          {t("sidebar.addDevice")}
        </DropdownMenuItem>
        <DropdownMenuItem data-device-menu-show-rail="true" onSelect={menu.onShowRail}>
          {t("sidebar.showRail")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** A tab's hint exists only to show the switch chord, so a tab without one has none. */

