import { LaptopIcon, PlusIcon, XIcon } from "lucide-react";
import { memo, useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Actions } from "../actions";
import { useInterfaceTranslation } from "../i18n/client";
import { badgeText, deviceConnected, railFrontId, tileCounts, tileHint, tileMonogram, tileName, type TileCounts } from "../devices";
import { cn } from "../lib/utils";
import { computerTiles, numberedComputers, screenState, screenStateKey, type ComputerTile } from "../computers";
import { numberOf } from "../numbering";
import { coreShared, isCoreMachine, machineName, windowDeviceId, wslPlace } from "../screenMachine";
import { deviceConnectOffered, type Peer } from "../pairing";
import { useShellStore } from "../store";
import { toolCells, useFrontTool } from "../toolCells";
import { useUiStore } from "../ui";
import type { AccessConnection } from "../snapshot";
import { BotFace } from "./bot-mark";
import { EntryContextMenu } from "./entry-menu";
import { Button } from "./ui/button";
import { Keycap } from "./ui/keycap";
import { Hint } from "./ui/tooltip";
import { SidebarMenus } from "./sidebar-menus";

/** `core` marks the machine the core runs on, set only while the core works with another machine (Q11). */
/** `place` is where a WSL distribution's device is, which its hint says after the name (PRD wsl-device B5). */
export type Tile = { id: string; label: string; icon: "local" | "remote"; core: boolean; place?: string | null };

const NO_BOTS: AccessConnection[] = [];

/** A paired machine's tile counts nothing until its core sends its counts over the connection (D-50). */
const NO_COUNTS: TileCounts = { needs_you: 0, done: 0 };

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
  const guestPane = useUiStore((s) => s.screen?.kind === "guest");
  const front = useShellStore((s) => railFrontId(s.rest, guestPane));
  const tools = useShellStore((s) => toolCells(s.rest));
  // Only a bot in its own space has a tile: its work gathers in its cell. A
  // 전체 bot works as the operator, so its work is in the operator's lists
  // and it is listed in Settings › 봇 alone.
  const allBots = useShellStore((s) => s.access?.connections ?? NO_BOTS);
  const bots = useMemo(() => allBots.filter((bot) => bot.mode.kind === "own"), [allBots]);
  // A tool's cell in front, read from what the center shows, is the one tile selected in place of a device's (PRD hide-external-control B40).
  const toolFront = useFrontTool()?.connection ?? null;
  // Every computer tile in one group before `+`: this computer, each device,
  // and each paired machine, whose tile shows its own Hide screen (PRD
  // device-connect-iroh D-37, B47).
  // Read when the devices or the paired machines change, which are the only facts the tiles are made of.
  const peers = useShellStore((s) => s.rest?.navigator?.peers);
  const computers = useMemo(() => computerTiles(useShellStore.getState().rest), [devices, peers]);
  const remoteScreen = useUiStore((s) => s.remoteScreen);
  const deviceConnect = useShellStore((s) => deviceConnectOffered(s.daemon));
  const tiles = useMemo(() => {
    const own = windowDeviceId(devices);
    // Every computer runs a core of its own once two Hides connect: no tile names the core's (D-80).
    const shared = !deviceConnect && coreShared(devices);
    return computers.map((computer): { tile: Tile; computer: ComputerTile } => ({
      computer,
      tile: devices?.some((device) => device.id === computer.id)
        ? {
            id: computer.id,
            label: machineName(devices, computer.id, t),
            icon: computer.id === own ? "local" : "remote",
            core: shared && isCoreMachine(devices, computer.id),
            place: wslPlace(devices, computer.id, t),
          }
        : { id: computer.id, label: computer.screen?.name ?? computer.id, icon: "remote", core: false },
    }));
  }, [computers, devices, deviceConnect, t]);
  // A ⌥⌘ hold numbers the tiles ⌥⌘n selects (`computers.ts`).
  const numbered = useUiStore((s) => s.hint === "devices");
  const numbers = useMemo(() => (numbered ? numberedComputers(computers) : null), [numbered, computers]);
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
          {tiles.map(({ tile, computer }) => {
            const number = numbers ? numberOf(numbers, tile.id) : null;
            const selected = remoteScreen !== null ? computer.screen?.device_id === remoteScreen : toolFront === null && tile.id === front;
            return (
              <li key={tile.id} className="flex">
                {computer.screen ? (
                  <ScreenTile tile={tile} peer={computer.screen} selected={selected} number={number} onSelect={() => actions.selectComputer(tile.id)} />
                ) : (
                  <RailTile tile={tile} selected={selected} number={number} actions={actions} />
                )}
              </li>
            );
          })}
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
          {/* Every bot in its own space, under a short line, after the devices and Add device, in the order they were approved (PRD hide-external-control D-50); each opens its cell. */}
          {bots.length > 0 ? <li role="separator" aria-hidden="true" data-rail-tools-separator="true" className="h-(--size-hairline) w-(--size-icon) bg-border" /> : null}
          {bots.map((bot) => {
            const cell = tools.find((one) => one.connection === bot.id);
            return (
              <li key={bot.id} className="flex">
                <BotTileView name={bot.name} turn={cell?.turn ?? 0} connection={bot.id} selected={cell !== undefined && cell.connection === toolFront} onSelect={() => (cell ? actions.focusTool(cell.connection) : actions.openSettings("externalTools"))} />
              </li>
            );
          })}
        </ul>
        {/* View, Help and Settings stand at the rail's foot, as a chat app's rail keeps its own. */}
        <div className="mt-auto flex flex-col items-center gap-xs pt-md" data-rail-menus="true">
          <SidebarMenus actions={actions} place="rail" />
        </div>
      </nav>
    </EntryContextMenu>
  );
}

/**
 * A paired machine's tile (B17, B21): its own Hide screen, or the reason it
 * cannot show now in its hint (not connected, paused, update needed, not
 * allowed), dimmed with the cross while it cannot.
 */
function ScreenTile({ tile, peer, selected, number, onSelect }: { tile: Tile; peer: Peer; selected: boolean; number: number | null; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  const state = screenState(peer);
  const note = state === "ready" ? null : t(screenStateKey(state));
  return <RailTileView tile={tile} selected={selected} counts={NO_COUNTS} connected={state === "ready"} note={note} number={number} onSelect={onSelect} />;
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
 * current (B3, B8, B41). No tile wears a crown: every computer runs a core
 * of its own, so "where the core is" marks nothing (PRD device-connect-iroh
 * D-80). While a ⌘ hold reveals the numbers, the tile's keycap floats at its
 * top left, the corner the marks leave free.
 */
export function RailTileView({ tile, selected, counts, connected, note = null, number = null, onSelect }: { tile: Tile; selected: boolean; counts: TileCounts; connected: boolean; note?: string | null; number?: number | null; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  // The name with where it is, `WSL Ubuntu · This PC`; the glyph keeps the name's letters (`WU`).
  const named = tile.place ? `${tile.label} · ${tile.place}` : tile.label;
  // A paired machine that cannot show says why in its own words (B21).
  const hint = note ? t("pairing.rail.hint", { name: named, state: note }) : tileHint(named, connected, counts, t, tile.core);
  return (
    <Hint label={hint} side="right">
      <button
        type="button"
        aria-label={note ? `${named}, ${note}` : tileName(named, connected, counts, t, tile.core)}
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

/**
 * A bot's tile (PRD hide-external-control D-50, B39, B41): the size of a
 * device's, its owl on the colour its name chooses (`botLook`), so it never
 * reads as a machine. Only a bot in its own space has one, so the tile marks
 * no mode. Its one mark is the grey number of what waits on the bot's answer
 * (`9+` from ten), which no Needs You, badge or notice of the operator's
 * counts; the hint names it a bot in its own space.
 */
export function BotTileView({ name, turn, connection, selected, onSelect }: { name: string; turn: number; connection: string; selected: boolean; onSelect: () => void }) {
  const { t } = useInterfaceTranslation();
  const facts = [name, t("devices.rail.tool"), t("devices.rail.botMode.own"), ...(turn > 0 ? [t("devices.rail.toolTurn", { count: turn })] : [])];
  return (
    <Hint label={facts.join(" · ")} side="right">
      <Button
        variant="ghost"
        size="icon"
        aria-label={facts.join(", ")}
        aria-pressed={selected}
        data-rail-tool={connection}
        className={cn(TILE, "p-0 hover:bg-transparent focus-visible:ring-0", selected && "ring-2 ring-foreground ring-offset-2 ring-offset-sidebar")}
        onClick={onSelect}
      >
        <BotFace name={name} size="tile" />
        {turn > 0 ? (
          <span
            aria-hidden="true"
            data-rail-badge="tool_turn"
            data-rail-badge-count={turn}
            className={cn(
              "absolute -top-xs -right-xs flex h-(--size-rail-badge) min-w-(--size-rail-badge) items-center justify-center rounded-full bg-muted-foreground px-xxs text-(length:--size-rail-badge-text) leading-none font-semibold text-background tabular-nums",
              CUTOUT,
            )}
          >
            {badgeText(turn)}
          </span>
        ) : null}
      </Button>
    </Hint>
  );
}
