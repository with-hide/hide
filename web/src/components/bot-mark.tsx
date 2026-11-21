import { FolderIcon } from "lucide-react";
import { useCallback } from "react";
import { BOT_SHAPE_SRC, botLook, botWorkspaceName, type BotColor } from "../bots";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import { machineName, windowDeviceId } from "../screenMachine";
import { catalogWorkspaces, type AccessMode } from "../snapshot";
import { useShellStore } from "../store";

const TILE_BG: Record<BotColor, string> = {
  slate: "bg-bot-slate",
  coral: "bg-bot-coral",
  teal: "bg-bot-teal",
  lavender: "bg-bot-lavender",
  mustard: "bg-bot-mustard",
  sage: "bg-bot-sage",
  charcoal: "bg-bot-charcoal",
};

/** `tile` is a device tile's size (the rail and Settings' rows), `row` a menu row's mark, `line` a mark inside a line of text. */
const SIZE = {
  tile: "size-(--size-icon-button-standard) rounded-md",
  row: "size-(--size-icon-lg) rounded-sm",
  line: "size-(--size-icon) rounded-xs",
} as const;

/**
 * A bot's owl on its tile colour, both chosen from its name (`botLook`), so
 * the same bot looks the same wherever it is drawn. Decorative: its name is
 * always beside it or in the control's accessible name.
 */
export function BotFace({ name, size, className }: { name: string; size: keyof typeof SIZE; className?: string }) {
  const look = botLook(name);
  return (
    <span aria-hidden="true" data-bot-face={`${look.shape}:${look.color}`} className={cn("flex shrink-0 overflow-hidden", SIZE[size], TILE_BG[look.color], className)}>
      <img src={BOT_SHAPE_SRC[look.shape]} alt="" draggable={false} className="size-full select-none" />
    </span>
  );
}

/**
 * The folder notched into a tile's bottom-right corner for a bot held inside
 * one workspace; a whole-Hide bot has none. Its parent is positioned and
 * draws the tile.
 */
export function BotWorkspaceBadge({ className }: { className?: string }) {
  return (
    <span aria-hidden="true" data-bot-workspace-badge="true" className={cn("absolute -right-xs -bottom-xs flex size-(--size-rail-badge) items-center justify-center rounded-full bg-card text-subtle-foreground ring-2 ring-sidebar", className)}>
      <FolderIcon className="size-(--size-rail-mark)" />
    </span>
  );
}

/** Names a workspace's machine when it is not this window's, for a workspace a bot is held inside. */
export function useWorkspaceDeviceName(): (deviceId: string) => string | null {
  const { t } = useInterfaceTranslation();
  const devices = useShellStore((s) => s.rest?.navigator?.devices);
  return useCallback((deviceId: string) => (deviceId === windowDeviceId(devices) ? null : machineName(devices, deviceId, t)), [devices, t]);
}

/** The name of the workspace a bot with `mode` is held inside, or null for a whole-Hide bot or none (`botWorkspaceName`). */
export function useBotWorkspaceName(mode: AccessMode | undefined): string | null {
  const { t } = useInterfaceTranslation();
  const deviceName = useWorkspaceDeviceName();
  return useShellStore((s) => (mode ? botWorkspaceName(mode, catalogWorkspaces(s.rest), deviceName, t("externalTools.mode.gone")) : null));
}
