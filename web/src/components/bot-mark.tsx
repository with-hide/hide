import { FolderIcon } from "lucide-react";
import { BOT_SHAPE_SRC, botLook, type BotColor } from "../bots";
import { cn } from "../lib/utils";

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
