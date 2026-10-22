import { CrownIcon, LaptopIcon, PlusIcon, XIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { badgeText, deviceConnected, frontDeviceId, railDeviceIds, tileCounts, tileHint, tileMonogram, tileName, type TileCounts } from "../devices";
import { cn } from "../lib/utils";
import { numberOf, numberedDevices } from "../numbering";
import { coreShared, isCoreMachine, machineName, windowDeviceId } from "../screenMachine";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { EntryContextMenu } from "./entry-menu";
import { Keycap } from "./ui/keycap";
import { Hint } from "./ui/tooltip";

/** `core` marks the machine the core runs on, set only while the core works with another machine (Q11). */
export type Tile = { id: string; label: string; icon: "local" | "remote"; core: boolean };

/** The rounded square every tile is, the add tile included; a tile's marks hang off its corners, and the focus outline stands outside the selected ring. */
const TILE = "relative flex size-(--size-icon-button-standard) shrink-0 items-center justify-center rounded-md outline-none focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring";

/** A mark's cut-out: a ring of the rail's own fill, so the mark reads as notched into the tile. */
const CUTOUT = "ring-2 ring-sidebar";

/**
 * The rail on the sidebar's left (PRD home-device-rail D-09, reworked by quick
 * device-rail-badges and quick device-rail-slack): a column the sidebar's full
 * height with the window's own machine first as This Mac (PRD
 * core-host-node-move N1), each other device in the core's order, and `+`
 * directly under the last one, which is Add device. Each tile is a square with
 * no name under it; its hint is the name with its counts. It is shown with one device alone,
 * and a right-click on it offers Hide rail. One tile is selected, ringed.
 * A tile reads only its own facts (its two counts, whether it is connected),
 * so a snapshot that changes another device redraws no tile, and nothing here
 * touches the sidebar's rows.
 */
export function DeviceRail({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  const front = useShellStore((s) => frontDeviceId(s.rest));
  const tiles = useMemo<Tile[]>(() => {
    const own = windowDeviceId(devices);
    const shared = coreShared(devices);
    return railDeviceIds(devices).map((id): Tile => ({
      id,
      label: machineName(devices, id, t),
      icon: id === own ? "local" : "remote",
      core: shared && isCoreMachine(devices, id),
    }));
  }, [devices, t]);
  // A ⌘ hold numbers the tiles ⌘n selects (`numbering.ts`).
  const numbered = useUiStore((s) => s.hint === "devices");
  const numbers = useMemo(() => (numbered ? numberedDevices(devices) : null), [numbered, devices]);
  return (
    <EntryContextMenu
      asChild
      label={t("devices.rail.menu")}
      items={() => [{ id: "hide", label: t("devices.rail.hide"), unavailable: null }]}
      onSelect={() => actions.hideDeviceRail()}
      data-device-rail-menu="true"
    >
      <nav aria-label={t("settings.tabs.devices")} data-device-rail="true" className="flex w-(--size-rail) shrink-0 flex-col overflow-y-auto overflow-x-hidden border-r border-border py-md">
        <ul className="flex flex-col items-center gap-md">
          {tiles.map((tile) => (
            <li key={tile.id} className="flex">
              <RailTile tile={tile} selected={tile.id === front} number={numbers ? numberOf(numbers, tile.id) : null} actions={actions} />
            </li>
          ))}
          <li className="flex">
            <Hint label={t("devices.addTitle")} side="right">
              <button
                type="button"
                data-rail-add="true"
                className={cn(TILE, "border border-dashed border-border text-muted-foreground hover:text-foreground")}
                onClick={() => actions.openAddDevice()}
              >
                <PlusIcon aria-hidden="true" className="size-(--size-icon)" />
              </button>
            </Hint>
          </li>
        </ul>
      </nav>
    </EntryContextMenu>
  );
}

const RailTile = memo(function RailTile({ tile, selected, number, actions }: { tile: Tile; selected: boolean; number: number | null; actions: Actions }) {
  const counts = useShellStore(useShallow((s) => tileCounts(s.rest, s.agents, tile.id)));
  const connected = useShellStore((s) => deviceConnected(s.rest, tile.id));
  return <RailTileView tile={tile} selected={selected} counts={counts} connected={connected} number={number} onSelect={() => actions.focusDevice(tile.id)} />;
});

/**
 * One tile as drawn: a focusable button named by its device, its connection
 * and each count it marks, holding This Mac's laptop or a device's monogram,
 * ringed while selected. One mark at most, notched into the top-right corner,
 * shows the most urgent state: the Needs You count (`9+` from ten), else a dot
 * while it has unseen Done; Working has no mark, and the hint carries every
 * count. A device that is not connected dims its glyph and wears a cross at
 * the bottom-right instead, with no mark, since what it last reported is not
 * current (B3, B8, B41). The core's machine wears a crown notched into the
 * bottom-left corner, the one corner the other marks leave free (PRD
 * core-host-node-move N1). While a ⌘ hold reveals the numbers, the tile's
 * keycap floats at its top left, the corner the marks leave free.
 */
export function RailTileView({ tile, selected, counts, connected, number = null, onSelect }: { tile: Tile; selected: boolean; counts: TileCounts; connected: boolean; number?: number | null; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <Hint label={tileHint(tile.label, connected, counts, t, tile.core)} side="right">
      <button
        type="button"
        aria-label={tileName(tile.label, connected, counts, t, tile.core)}
        aria-pressed={selected}
        data-rail-tile={tile.id}
        data-rail-connected={connected ? "true" : "false"}
        data-rail-core={tile.core ? "true" : undefined}
        className={cn(
          TILE,
          "bg-secondary text-body font-semibold hover:text-foreground",
          selected ? "text-foreground ring-2 ring-foreground ring-offset-2 ring-offset-sidebar" : "text-subtle-foreground",
        )}
        onClick={onSelect}
      >
        <span aria-hidden="true" data-rail-glyph="true" className={cn("flex items-center justify-center", !connected && "opacity-(--opacity-dimmed)")}>
          {tile.icon === "local" ? <LaptopIcon className="size-(--size-icon-lg)" /> : tileMonogram(tile.label)}
        </span>
        {!connected ? null : counts.needs_you > 0 ? (
          <span
            aria-hidden="true"
            data-rail-badge="needs_you"
            data-rail-badge-count={counts.needs_you}
            className={cn(
              "absolute -top-xs -right-xs flex h-(--size-rail-badge) min-w-(--size-rail-badge) items-center justify-center rounded-full bg-warning px-xxs text-(length:--size-rail-badge-text) leading-none font-semibold text-status-foreground tabular-nums",
              CUTOUT,
            )}
          >
            {badgeText(counts.needs_you)}
          </span>
        ) : counts.done > 0 ? (
          <span aria-hidden="true" data-rail-badge="done" className={cn("absolute -top-xxs -right-xxs size-(--size-rail-mark) rounded-full bg-success", CUTOUT)} />
        ) : null}
        {tile.core ? (
          <span aria-hidden="true" data-rail-crown="true" className={cn("absolute -bottom-xs -left-xs flex size-(--size-rail-badge) items-center justify-center rounded-full bg-card text-subtle-foreground", CUTOUT)}>
            <CrownIcon className="size-(--size-rail-crown)" />
          </span>
        ) : null}
        {number !== null ? <Keycap number={number} className="right-auto left-xxs" /> : null}
        {connected ? null : (
          <span aria-hidden="true" data-rail-off="true" className={cn("absolute -right-xs -bottom-xs flex size-(--size-rail-badge) items-center justify-center rounded-full bg-card text-muted-foreground", CUTOUT)}>
            <XIcon className="size-(--size-rail-mark)" />
          </span>
        )}
      </button>
    </Hint>
  );
}
