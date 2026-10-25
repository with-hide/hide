import {
  ArchiveIcon, ArrowDownIcon, ArrowLeftIcon, ArrowRightIcon, ArrowUpIcon,
  CheckIcon, ClipboardPasteIcon, CopyIcon, ExternalLinkIcon, FileCodeIcon,
  FileTextIcon, FolderOpenIcon, GitForkIcon, HashIcon, ListChecksIcon, MaximizeIcon,
  MinimizeIcon, MoonIcon, NetworkIcon, PanelBottomIcon, PanelLeftIcon,
  PanelRightIcon, PanelTopIcon, PencilIcon, PinIcon, PlayIcon, PlusIcon,
  RefreshCwIcon, SearchIcon, SquareDashedIcon, SquareTerminalIcon, StarIcon, StarOffIcon, Trash2Icon,
  WaypointsIcon, XIcon,
} from "lucide-react";

/** The same action has the same symbol in every menu; labels still name it. */
export const menuSymbols = {
  archive: ArchiveIcon, down: ArrowDownIcon, left: ArrowLeftIcon, right: ArrowRightIcon, up: ArrowUpIcon,
  check: CheckIcon, paste: ClipboardPasteIcon, copy: CopyIcon, external: ExternalLinkIcon,
  source: FileCodeIcon, record: FileTextIcon, folder: FolderOpenIcon, fork: GitForkIcon,
  id: HashIcon, zoom: MaximizeIcon, unzoom: MinimizeIcon, sleep: MoonIcon, relations: NetworkIcon,
  split_down: PanelBottomIcon, split_left: PanelLeftIcon, split_right: PanelRightIcon, split_up: PanelTopIcon,
  rename: PencilIcon, pin: PinIcon, resume: PlayIcon, add: PlusIcon, refresh: RefreshCwIcon,
  find: SearchIcon, select: SquareDashedIcon, star: StarIcon, unstar: StarOffIcon, terminal: SquareTerminalIcon, delete: Trash2Icon, close: XIcon,
  overview: WaypointsIcon, plan: ListChecksIcon,
} as const;

export type MenuSymbol = keyof typeof menuSymbols;
export type MenuGroup = "split" | "move" | "relations" | "fork";
export const menuGroupSymbols: Record<MenuGroup, MenuSymbol> = {
  split: "split_right", move: "right", relations: "relations", fork: "fork",
};

export type MenuPresentation = {
  icon?: MenuSymbol;
  /** Consecutive entries with this group share an action submenu, retaining their command ids. */
  group?: MenuGroup;
  /** A shorter label in the menu only; command labels and availability stay unchanged. */
  menuLabel?: string;
};
