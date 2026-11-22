import { BOT_SHAPE_SRC, botLook, type BotColor, type BotModeKind } from "../bots";
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

/**
 * `tile` is a device tile's size (the rail and Settings' rows), `row` a menu
 * row's mark, `line` a mark inside a line of text, `badge` the corner of an
 * agent's logo (where Hide AI's owl sits) and `mark` an agent mark's own size,
 * standing in for the logo while logos are hidden.
 */
const SIZE = {
  tile: "size-(--size-icon-button-standard) rounded-md",
  row: "size-(--size-icon-lg) rounded-sm",
  line: "size-(--size-icon) rounded-xs",
  badge: "size-(--size-hide-badge) rounded-xs",
  mark: "size-(--size-agent-badge-compact) rounded-xs",
} as const;

/**
 * A bot's owl on its tile colour, both chosen from its name (`botLook`), so
 * the same bot looks the same wherever it is drawn. A bot in `all` mode, which
 * also reaches the operator's workspaces, wears a lime ring; a bot in its own
 * space none. Decorative: its name is always beside it or in the control's
 * accessible name.
 */
export function BotFace({ name, size, mode, className }: { name: string; size: keyof typeof SIZE; mode?: BotModeKind; className?: string }) {
  const look = botLook(name);
  return (
    <span aria-hidden="true" data-bot-face={`${look.shape}:${look.color}`} data-bot-mode={mode} className={cn("flex shrink-0 overflow-hidden", SIZE[size], TILE_BG[look.color], mode === "all" && "ring-2 ring-primary", className)}>
      <img src={BOT_SHAPE_SRC[look.shape]} alt="" draggable={false} className="size-full select-none" />
    </span>
  );
}
