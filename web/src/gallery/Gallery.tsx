// The dev-only System gallery (PRD D-04): every state a `System / <Name>`
// sheet draws in Pen, rendered by the real part, in a Light and a Dark column,
// plus a playground row to press the keys Pen cannot show. It has no store and
// no socket; a part that needed one would not be a System part.
//
// A state that opens a layer (menu, dialog, popover, select, toast) renders in
// its own frame document, because those layers are fixed-position and modal:
// in the page they would cover and lock every other cell.

import { ArrowRightIcon, CopyIcon, FolderIcon, Loader2Icon, PlusIcon, SettingsIcon, TrashIcon } from "lucide-react";
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { DESK_SCENES } from "./DeskScene";
import { toast } from "sonner";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "../components/ui/alert-dialog";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Checkbox } from "../components/ui/checkbox";
import { Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandShortcut } from "../components/ui/command";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "../components/ui/context-menu";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuShortcut, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { Input } from "../components/ui/input";
import { RowButton, RowOverlayButton } from "../components/ui/row-button";
import { Textarea } from "../components/ui/textarea";
import { Kbd, KbdGroup } from "../components/ui/kbd";
import { Popover, PopoverContent, PopoverTrigger } from "../components/ui/popover";
import { RadioGroup, RadioGroupItem } from "../components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { Separator } from "../components/ui/separator";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../components/ui/sheet";
import { Slider } from "../components/ui/slider";
import { Toaster } from "../components/ui/sonner";
import { Switch } from "../components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "../components/ui/tabs";
import { ToggleGroup, ToggleGroupItem } from "../components/ui/toggle-group";
import { Hint, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "../components/ui/tooltip";
import { GALLERY, type Section, type StateOf } from "./manifest";

type Theme = "light" | "dark";
type Renderers = { [S in Section]: Record<StateOf<S>, () => ReactNode> };

/** The states that open a layer, rendered in a frame document of their own. */
const FRAMED: { [S in Section]?: readonly StateOf<S>[] } = {
  "Select": ["Open"],
  "Dropdown Menu": ["Open", "Highlighted", "Disabled Item", "Destructive Item"],
  "Context Menu": ["Open", "Highlighted", "Disabled Item"],
  "Dialog": ["Open", "With Close Button"],
  "Alert Dialog": ["Open", "Pending"],
  "Sheet": ["Right", "Left"],
  "Popover": ["Open"],
  "Tooltip": ["Open", "With Shortcut"],
  "Sonner": ["Default", "With Description", "With Action"],
};

function framed(section: Section, state: string) {
  return (FRAMED[section] as readonly string[] | undefined)?.includes(state) ?? false;
}

/** Focus the n-th menu item once the menu is open, so Radix marks it highlighted. */
function HighlightItem({ index }: { index: number }) {
  useEffect(() => {
    const timer = window.setTimeout(() => document.querySelectorAll<HTMLElement>('[role="menuitem"]')[index]?.focus(), 50);
    return () => window.clearTimeout(timer);
  }, [index]);
  return null;
}

/** Open a context menu on the trigger it follows, as a right-click would. */
function OpenContextMenu() {
  useEffect(() => {
    const timer = window.setTimeout(() => {
      const target = document.querySelector<HTMLElement>('[data-slot="context-menu-trigger"]');
      const box = target?.getBoundingClientRect();
      target?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: (box?.left ?? 0) + 24, clientY: (box?.top ?? 0) + 16 }));
    }, 0);
    return () => window.clearTimeout(timer);
  }, []);
  return null;
}

function ShowToast({ kind }: { kind: "plain" | "description" | "action" }) {
  useEffect(() => {
    if (kind === "plain") toast("Diagnostics copied");
    else if (kind === "description") toast("Hook installed", { description: "Claude Code reports to Hide from its next session." });
    else toast("Worktree removed", { action: { label: "Undo", onClick: () => {} } });
  }, [kind]);
  return <Toaster position="top-center" expand visibleToasts={1} duration={Number.POSITIVE_INFINITY} />;
}

const MENU = (
  <>
    <DropdownMenuLabel>Project</DropdownMenuLabel>
    <DropdownMenuItem>
      <FolderIcon />
      Open in Finder
      <DropdownMenuShortcut>⌘O</DropdownMenuShortcut>
    </DropdownMenuItem>
    <DropdownMenuItem>
      <CopyIcon />
      Copy path
    </DropdownMenuItem>
  </>
);

const renderers: Renderers = {
  "Button": {
    "Default": () => <Button>Create worktree</Button>,
    "Default Hover": () => <Button data-gallery-state="hover">Create worktree</Button>,
    "Default Focus": () => <Button data-gallery-state="focus">Create worktree</Button>,
    "Default Disabled": () => <Button disabled>Create worktree</Button>,
    "Pending": () => (
      <Button disabled>
        <Loader2Icon className="animate-spin" />
        Creating…
      </Button>
    ),
    "Secondary": () => <Button variant="secondary">Cancel</Button>,
    "Secondary Hover": () => <Button variant="secondary" data-gallery-state="hover">Cancel</Button>,
    "Soft": () => <Button variant="soft" size="sm">Answer</Button>,
    "Outline": () => <Button variant="outline">Choose folder</Button>,
    "Ghost": () => <Button variant="ghost">Refresh</Button>,
    "Ghost Hover": () => <Button variant="ghost" data-gallery-state="hover">Refresh</Button>,
    "Destructive": () => <Button variant="destructive">Delete worktree</Button>,
    "Destructive Hover": () => <Button variant="destructive" data-gallery-state="hover">Delete worktree</Button>,
    "Link": () => <Button variant="link">Open log</Button>,
    "Text": () => <Button variant="text" size="inline" className="text-caption">Show 3 more</Button>,
    "Text Hover": () => <Button variant="text" size="inline" className="text-caption" data-gallery-state="hover">Show 3 more</Button>,
    "Text Link": () => <Button variant="text-link" size="inline" className="text-caption">Open the session</Button>,
    "Text Link Hover": () => <Button variant="text-link" size="inline" className="text-caption" data-gallery-state="hover">Open the session</Button>,
    "Small": () => <Button size="sm">Retry</Button>,
    "Large": () => <Button size="lg">Register project</Button>,
    "Icon": () => (
      <Hint label="Settings">
        <Button variant="ghost" size="icon">
          <SettingsIcon />
        </Button>
      </Hint>
    ),
    "Icon Small": () => (
      <Hint label="New tab">
        <Button variant="ghost" size="icon-sm">
          <PlusIcon />
        </Button>
      </Hint>
    ),
    "Icon Extra Small": () => (
      <Hint label="More">
        <Button variant="ghost" size="icon-xs">
          <SettingsIcon />
        </Button>
      </Hint>
    ),
    "Row Action": () => <GalleryRowAction />,
    "Row Action Hover": () => <GalleryRowAction state="hover" />,
  },
  "Textarea": {
    "Default": () => <Textarea aria-label="First instruction" rows={3} defaultValue="" className="w-(--size-settings-control-w)" />,
    "Placeholder": () => <Textarea aria-label="First instruction" rows={3} placeholder="What should the agent do first?" className="w-(--size-settings-control-w)" />,
    "Focus": () => <Textarea aria-label="First instruction" rows={3} defaultValue="결제 모듈의 재시도 로직을 정리해 줘" data-gallery-state="focus" className="w-(--size-settings-control-w)" />,
    "Invalid": () => <Textarea aria-label="First instruction" rows={3} defaultValue="" aria-invalid className="w-(--size-settings-control-w)" />,
    "Disabled": () => <Textarea aria-label="First instruction" rows={3} defaultValue="결제 모듈의 재시도 로직을 정리해 줘" disabled className="w-(--size-settings-control-w)" />,
    "Bare": () => (
      <div className="w-(--size-settings-control-w) rounded-sm border border-border">
        <Textarea bare aria-label="First instruction" rows={3} placeholder="What should the agent do first?" className="p-md" />
      </div>
    ),
  },
  "Row Button": {
    "Default": () => <GalleryRowButton />,
    "Hover": () => <GalleryRowButton state="hover" />,
    "Focus": () => <GalleryRowButton state="focus" />,
    "Disabled": () => <GalleryRowButton disabled />,
    "Overlay": () => <GalleryOverlayRow />,
    "Overlay Hover": () => <GalleryOverlayRow state="hover" />,
  },
  "Input": {
    "Default": () => <Input aria-label="Purpose" defaultValue="" className="w-(--size-settings-control-w)" />,
    "Placeholder": () => <Input aria-label="Purpose" placeholder="What this worktree is for" className="w-(--size-settings-control-w)" />,
    "Filled": () => <Input aria-label="Purpose" defaultValue="결제 모듈 리팩터링" className="w-(--size-settings-control-w)" />,
    "Focus": () => <Input aria-label="Purpose" defaultValue="결제 모듈 리팩터링" data-gallery-state="focus" className="w-(--size-settings-control-w)" />,
    "Invalid": () => <Input aria-label="Branch" defaultValue="main" aria-invalid className="w-(--size-settings-control-w)" mono />,
    "Disabled": () => <Input aria-label="Branch" defaultValue="feature/login" disabled className="w-(--size-settings-control-w)" mono />,
    "Mono": () => <Input aria-label="Path" defaultValue="~/projects/hide" className="w-(--size-settings-control-w)" mono />,
  },
  "Select": {
    "Default": () => <GallerySelect value="claude" />,
    "Placeholder": () => <GallerySelect />,
    "Focus": () => <GallerySelect value="claude" state="focus" />,
    "Disabled": () => <GallerySelect value="claude" disabled />,
    "Open": () => <GallerySelect value="claude" open />,
  },
  "Checkbox": {
    "Unchecked": () => <LabeledCheckbox />,
    "Checked": () => <LabeledCheckbox checked />,
    "Indeterminate": () => <LabeledCheckbox checked="indeterminate" />,
    "Focus": () => <LabeledCheckbox state="focus" />,
    "Disabled": () => <LabeledCheckbox disabled />,
    "Checked Disabled": () => <LabeledCheckbox checked disabled />,
  },
  "Switch": {
    "Off": () => <Switch aria-label="Background AI" />,
    "On": () => <Switch aria-label="Background AI" defaultChecked />,
    "Focus": () => <Switch aria-label="Background AI" defaultChecked data-gallery-state="focus" />,
    "Disabled": () => <Switch aria-label="Background AI" disabled />,
  },
  "Radio Group": {
    "Default": () => <GalleryRadio />,
    "Focus": () => <GalleryRadio state="focus" />,
    "Disabled": () => <GalleryRadio disabled />,
  },
  "Toggle Group": {
    "Default": () => <GalleryToggle />,
    "Hover": () => <GalleryToggle state="hover" />,
    "Focus": () => <GalleryToggle state="focus" />,
    "Disabled": () => <GalleryToggle disabled />,
    "Extra Small": () => <GalleryToggle size="xs" />,
  },
  "Slider": {
    "Default": () => <Slider aria-label="Interface font" defaultValue={[13]} min={11} max={17} step={1} className="w-(--size-settings-control-w)" />,
    "Focus": () => <Slider aria-label="Interface font" defaultValue={[13]} min={11} max={17} step={1} className="w-(--size-settings-control-w) [&_[data-slot=slider-thumb]]:ring-1 [&_[data-slot=slider-thumb]]:ring-ring" />,
    "Disabled": () => <Slider aria-label="Interface font" defaultValue={[13]} min={11} max={17} step={1} disabled className="w-(--size-settings-control-w)" />,
  },
  "Tabs": {
    "Default": () => <GalleryTabs />,
    "Hover": () => <GalleryTabs state="hover" />,
    "Focus": () => <GalleryTabs state="focus" />,
    "Disabled": () => <GalleryTabs disabled />,
  },
  "Badge": {
    "Default": () => <Badge variant="default">live</Badge>,
    "Secondary": () => <Badge>3 agents</Badge>,
    "Destructive": () => <Badge variant="destructive">refused</Badge>,
    "Outline": () => <Badge variant="outline">main</Badge>,
  },
  "Kbd": {
    "Default": () => <Kbd>⌘K</Kbd>,
    "Group": () => (
      <KbdGroup>
        <Kbd>⇧</Kbd>
        <Kbd>⌘</Kbd>
        <Kbd>B</Kbd>
      </KbdGroup>
    ),
  },
  "Separator": {
    "Horizontal": () => (
      <div className="flex w-(--size-settings-control-w) flex-col gap-xs text-caption text-subtle-foreground">
        Agents
        <Separator />
        Views
      </div>
    ),
    "Vertical": () => (
      <div className="flex h-(--size-control) items-center gap-sm text-caption text-subtle-foreground">
        Explorer
        <Separator orientation="vertical" />
        Changes
      </div>
    ),
  },
  "Dropdown Menu": {
    "Closed": () => (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="secondary">Project ⋯</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>{MENU}</DropdownMenuContent>
      </DropdownMenu>
    ),
    "Open": () => <GalleryDropdown />,
    "Highlighted": () => <GalleryDropdown highlight={0} />,
    "Disabled Item": () => <GalleryDropdown disabledItem />,
    "Destructive Item": () => <GalleryDropdown destructiveItem />,
  },
  "Context Menu": {
    "Open": () => <GalleryContextMenu />,
    "Highlighted": () => <GalleryContextMenu highlight={0} />,
    "Disabled Item": () => <GalleryContextMenu disabledItem />,
  },
  "Dialog": {
    "Open": () => <GalleryDialog />,
    "With Close Button": () => <GalleryDialog closeButton />,
  },
  "Alert Dialog": {
    "Open": () => <GalleryAlert />,
    "Pending": () => <GalleryAlert pending />,
  },
  "Sheet": {
    "Right": () => <GallerySheet side="right" />,
    "Left": () => <GallerySheet side="left" />,
  },
  "Popover": {
    "Open": () => (
      <Popover open>
        <PopoverTrigger asChild>
          <Button variant="secondary">Pull request 145</Button>
        </PopoverTrigger>
        <PopoverContent align="start">
          <div className="flex flex-col gap-xs">
            <span className="text-subhead font-semibold">Show documents side by side</span>
            <span className="text-caption text-subtle-foreground">CI 5 of 5 green · review requested</span>
          </div>
        </PopoverContent>
      </Popover>
    ),
  },
  "Tooltip": {
    "Open": () => (
      <Tooltip open>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="New agent">
            <PlusIcon />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">New agent</TooltipContent>
      </Tooltip>
    ),
    "With Shortcut": () => (
      <Tooltip open>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon" aria-label="Toggle right lane">
            <ArrowRightIcon />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          <span className="inline-flex items-center gap-sm">
            Toggle right lane <span className="text-muted-foreground">⌘E</span>
          </span>
        </TooltipContent>
      </Tooltip>
    ),
  },
  "Command": {
    "Default": () => <GalleryCommand />,
    "Filtered": () => <GalleryCommand search="set" />,
    "Empty": () => <GalleryCommand search="zzz" />,
  },
  "Sonner": {
    "Default": () => <ShowToast kind="plain" />,
    "With Description": () => <ShowToast kind="description" />,
    "With Action": () => <ShowToast kind="action" />,
  },
};

function GallerySelect({ value, open, disabled, state }: { value?: string; open?: boolean; disabled?: boolean; state?: "focus" }) {
  return (
    <Select defaultValue={value} open={open} disabled={disabled}>
      <SelectTrigger aria-label="Agent" data-gallery-state={state}>
        <SelectValue placeholder="Choose an agent" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="claude">Claude Code</SelectItem>
        <SelectItem value="codex">Codex</SelectItem>
        <SelectItem value="none" disabled>
          None installed
        </SelectItem>
      </SelectContent>
    </Select>
  );
}

function LabeledCheckbox({ checked, disabled, state }: { checked?: boolean | "indeterminate"; disabled?: boolean; state?: "focus" }) {
  return (
    <label className="flex items-center gap-sm text-body">
      <Checkbox defaultChecked={checked} disabled={disabled} data-gallery-state={state} />
      Also delete the branch
    </label>
  );
}

function GalleryRadio({ disabled, state }: { disabled?: boolean; state?: "focus" }) {
  return (
    <RadioGroup defaultValue="claude" disabled={disabled} aria-label="Agent">
      {["claude", "codex"].map((id) => (
        <label key={id} className="flex items-center gap-sm text-body">
          <RadioGroupItem value={id} data-gallery-state={id === "claude" ? state : undefined} />
          {id === "claude" ? "Claude Code" : "Codex"}
        </label>
      ))}
    </RadioGroup>
  );
}

function GalleryToggle({ disabled, state, size }: { disabled?: boolean; state?: "hover" | "focus"; size?: "xs" }) {
  return (
    <ToggleGroup type="single" defaultValue="agents" aria-label="Layout" disabled={disabled}>
      <ToggleGroupItem value="agents" size={size}>Agents</ToggleGroupItem>
      <ToggleGroupItem value="together" size={size} data-gallery-state={state}>
        Together
      </ToggleGroupItem>
      <ToggleGroupItem value="views" size={size}>Views</ToggleGroupItem>
    </ToggleGroup>
  );
}

function GalleryRowAction({ state }: { state?: "hover" }) {
  return (
    <div className="flex w-(--size-settings-control-w) items-center gap-xs rounded-xs bg-accent px-xs py-xs">
      <span className="min-w-0 flex-1 truncate text-body text-foreground">payments-retry</span>
      <Hint label="More">
        <Button variant="row-action" size="icon-xs" data-gallery-state={state}>
          <SettingsIcon />
        </Button>
      </Hint>
    </div>
  );
}

function GalleryRowButton({ disabled, state }: { disabled?: boolean; state?: "hover" | "focus" }) {
  return (
    <div className="w-(--size-settings-control-w)">
      <RowButton disabled={disabled} data-gallery-state={state}>
        <FolderIcon className="size-(--size-icon-sm) text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-body text-foreground">payments-retry</span>
        <span className="text-caption text-muted-foreground">3분 전</span>
      </RowButton>
    </div>
  );
}

function GalleryOverlayRow({ state }: { state?: "hover" }) {
  return (
    <div className="relative w-(--size-settings-control-w)">
      <RowOverlayButton aria-label="Expand card 12" data-gallery-state={state} />
      <div className="pointer-events-none relative flex min-w-0 items-center gap-xs px-xs py-sm">
        <span className="min-w-0 flex-1 truncate text-body text-foreground">답변은 한국어로</span>
        <Button variant="text" size="inline" className="pointer-events-auto text-caption">Forget</Button>
      </div>
    </div>
  );
}

function GalleryTabs({ disabled, state }: { disabled?: boolean; state?: "hover" | "focus" }) {
  return (
    <Tabs defaultValue="general">
      <TabsList aria-label="Settings section">
        <TabsTrigger value="general">General</TabsTrigger>
        <TabsTrigger value="appearance" data-gallery-state={state}>
          Appearance
        </TabsTrigger>
        <TabsTrigger value="devices" disabled={disabled}>
          Devices
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );
}

function GalleryDropdown({ highlight, disabledItem, destructiveItem }: { highlight?: number; disabledItem?: boolean; destructiveItem?: boolean }) {
  return (
    <DropdownMenu open modal={false}>
      <DropdownMenuTrigger asChild>
        <Button variant="secondary">Project ⋯</Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {MENU}
        {disabledItem ? (
          <DropdownMenuItem disabled className="flex-col items-start gap-none">
            <span>New worktree</span>
            <span className="text-caption">Not a Git repository</span>
          </DropdownMenuItem>
        ) : null}
        {destructiveItem ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive">
              <TrashIcon />
              Remove project
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
      {highlight !== undefined ? <HighlightItem index={highlight} /> : null}
    </DropdownMenu>
  );
}

function GalleryContextMenu({ highlight, disabledItem }: { highlight?: number; disabledItem?: boolean }) {
  return (
    <ContextMenu>
      <ContextMenuTrigger className="flex h-(--size-control-lg) w-(--size-settings-control-w) items-center rounded-sm border border-dashed border-border px-sm text-caption text-muted-foreground">
        README.md
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem>Open to the Side</ContextMenuItem>
        <ContextMenuItem>Rename…</ContextMenuItem>
        {disabledItem ? <ContextMenuItem disabled>Open in Browser</ContextMenuItem> : null}
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive">Move to Trash</ContextMenuItem>
      </ContextMenuContent>
      <OpenContextMenu />
      {highlight !== undefined ? <HighlightItem index={highlight} /> : null}
    </ContextMenu>
  );
}

function GalleryDialog({ closeButton }: { closeButton?: boolean }) {
  return (
    <Dialog open modal={false}>
      <DialogContent showCloseButton={closeButton} onInteractOutside={(event) => event.preventDefault()}>
        <DialogHeader>
          <DialogTitle>New worktree</DialogTitle>
          <DialogDescription>A branch checked out in its own folder beside hide.</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-sm">
          <Input aria-label="Branch" defaultValue="feature/결제-리팩터" mono />
          <LabeledCheckbox checked />
        </DialogBody>
        <DialogFooter>
          <Button variant="secondary">Cancel</Button>
          <Button>Create worktree</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function GalleryAlert({ pending }: { pending?: boolean }) {
  return (
    <AlertDialog open>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete worktree feature/login?</AlertDialogTitle>
          <AlertDialogDescription>The folder and its uncommitted changes are removed. The branch stays.</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>Keep worktree</AlertDialogCancel>
          <AlertDialogAction disabled={pending}>
            {pending ? <Loader2Icon className="animate-spin" /> : null}
            {pending ? "Deleting…" : "Delete worktree"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function GallerySheet({ side }: { side: "right" | "left" }) {
  return (
    <Sheet open modal={false}>
      <SheetContent side={side} onInteractOutside={(event) => event.preventDefault()}>
        <SheetHeader>
          <SheetTitle>Tools</SheetTitle>
          <SheetDescription>Explorer and Changes, over the narrow window.</SheetDescription>
        </SheetHeader>
      </SheetContent>
    </Sheet>
  );
}

function GalleryCommand({ search }: { search?: string }) {
  return (
    <Command className="w-(--size-search-sheet-w) border border-border" defaultValue="settings">
      <CommandInput placeholder="Run a command" value={search} onValueChange={() => {}} />
      <CommandList>
        <CommandEmpty>No command matches.</CommandEmpty>
        <CommandGroup heading="Workspace">
          <CommandItem value="settings">
            <SettingsIcon />
            Open Settings
            <CommandShortcut>⌘,</CommandShortcut>
          </CommandItem>
          <CommandItem value="split">
            Split right
            <CommandShortcut>⌘D</CommandShortcut>
          </CommandItem>
          <CommandItem value="reset" disabled>
            Reset text size
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </Command>
  );
}

/** One state in one theme; a layer state is a frame document of its own. */
function stateRenderer(section: Section, state: string): () => ReactNode {
  const render = (renderers[section] as Record<string, (() => ReactNode) | undefined>)[state];
  if (!render) throw new Error(`No gallery renderer for ${section}/${state}`);
  return render;
}

function Cell({ section, state, theme }: { section: Section; state: string; theme: Theme }) {
  const render = stateRenderer(section, state);
  if (framed(section, state)) {
    const src = `/gallery?frame=${encodeURIComponent(`${section}/${state}`)}&theme=${theme}`;
    return <iframe loading="lazy" title={`${section} ${state} ${theme}`} src={src} className="h-(--size-search-sheet-h) w-full border-0 bg-background" />;
  }
  return <div className="flex min-h-(--size-control-lg) items-center p-md">{render()}</div>;
}

function Playground({ section }: { section: Section }) {
  const [open, setOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  switch (section) {
    case "Dialog":
      return (
        <Dialog open={open} onOpenChange={setOpen}>
          <Button onClick={() => setOpen(true)}>Open dialog</Button>
          <DialogContent showCloseButton>
            <DialogHeader>
              <DialogTitle>Rename purpose</DialogTitle>
              <DialogDescription>Tab stays inside; Escape closes and returns focus.</DialogDescription>
            </DialogHeader>
            <DialogBody>
              <Input aria-label="Purpose" defaultValue="결제 모듈" />
            </DialogBody>
            <DialogFooter>
              <Button variant="secondary" onClick={() => setOpen(false)}>
                Cancel
              </Button>
              <Button onClick={() => setOpen(false)}>Save</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      );
    case "Command":
      return (
        <>
          <Button variant="secondary" onClick={() => setPaletteOpen(true)}>
            Open palette
          </Button>
          <CommandDialog open={paletteOpen} onOpenChange={setPaletteOpen}>
            <Command>
              <CommandInput placeholder="Run a command" />
              <CommandList>
                <CommandEmpty>No command matches.</CommandEmpty>
                <CommandGroup heading="Workspace">
                  {["Open Settings", "Split right", "Split down", "Close pane", "Toggle zoom"].map((label) => (
                    <CommandItem key={label} onSelect={() => setPaletteOpen(false)}>
                      {label}
                    </CommandItem>
                  ))}
                </CommandGroup>
              </CommandList>
            </Command>
          </CommandDialog>
        </>
      );
    case "Sonner":
      return (
        <Button variant="secondary" onClick={() => toast("Diagnostics copied")}>
          Show toast
        </Button>
      );
    case "Context Menu":
      return (
        <ContextMenu>
          <ContextMenuTrigger tabIndex={0} className="flex h-(--size-control-lg) w-(--size-settings-control-w) items-center rounded-sm border border-dashed border-border px-sm text-caption text-muted-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring">
            Right-click or ⇧F10
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem>Open to the Side</ContextMenuItem>
            <ContextMenuItem>Rename…</ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem variant="destructive">Move to Trash</ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
      );
    case "Dropdown Menu":
      return (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="secondary">Project ⋯</Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>{MENU}</DropdownMenuContent>
        </DropdownMenu>
      );
    case "Select":
      return <GallerySelect value="claude" />;
    case "Tooltip":
      return (
        <Hint label="New agent" shortcut="⌘T">
          <Button variant="ghost" size="icon">
            <PlusIcon />
          </Button>
        </Hint>
      );
    case "Popover":
      return (
        <Popover>
          <PopoverTrigger asChild>
            <Button variant="secondary">Pull request 145</Button>
          </PopoverTrigger>
          <PopoverContent align="start">Escape or a click outside closes it.</PopoverContent>
        </Popover>
      );
    case "Alert Dialog":
      return (
        <AlertDialog open={open} onOpenChange={setOpen}>
          <Button variant="destructive" onClick={() => setOpen(true)}>
            Delete worktree…
          </Button>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Delete worktree feature/login?</AlertDialogTitle>
              <AlertDialogDescription>Nothing has focus until you choose.</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Keep worktree</AlertDialogCancel>
              <AlertDialogAction>Delete worktree</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      );
    case "Sheet":
      return (
        <Sheet open={open} onOpenChange={setOpen}>
          <Button variant="secondary" onClick={() => setOpen(true)}>
            Open tools
          </Button>
          <SheetContent>
            <SheetHeader>
              <SheetTitle>Tools</SheetTitle>
              <SheetDescription>Escape closes it.</SheetDescription>
            </SheetHeader>
          </SheetContent>
        </Sheet>
      );
    default: {
      return <>{stateRenderer(section, GALLERY[section][0])()}</>;
    }
  }
}

/** The shell's Escape owner, reduced to the gallery: the innermost layer closes. */
function useGalleryEscape() {
  useEffect(() => {
    const onKeyDown = async (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      const { useUiStore } = await import("../ui");
      const innermost = useUiStore.getState().escapeLayers.at(-1);
      if (!innermost) return;
      event.preventDefault();
      event.stopPropagation();
      innermost();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);
}

export function Gallery() {
  useGalleryEscape();
  const sections = Object.keys(GALLERY) as Section[];
  return (
    <TooltipProvider>
      <div className="flex h-full overflow-hidden bg-background text-foreground">
        <nav className="w-(--size-sidebar-min) shrink-0 overflow-auto border-r border-border bg-sidebar p-sm text-body">
          <h1 className="px-sm py-xs text-title font-semibold">System</h1>
          <a href="#scenes" className="block rounded-xs px-sm py-xxs text-subtle-foreground hover:bg-accent hover:text-foreground">
            Scenes
          </a>
          <a href="#shared-parts" className="block rounded-xs px-sm py-xxs text-subtle-foreground hover:bg-accent hover:text-foreground">
            Shared parts
          </a>
          <a href="#desk" className="block rounded-xs px-sm py-xxs text-subtle-foreground hover:bg-accent hover:text-foreground">
            Desk
          </a>
          {sections.map((section) => (
            <a key={section} href={`#${encodeURIComponent(section)}`} className="block rounded-xs px-sm py-xxs text-subtle-foreground hover:bg-accent hover:text-foreground">
              {section}
            </a>
          ))}
        </nav>
        <main className="min-w-0 flex-1 overflow-auto p-xl">
          <Scenes />
          <SharedParts />
          <Desk />
          {sections.map((section) => (
            <section key={section} id={encodeURIComponent(section)} data-gallery-section={section} className="mb-xxxl">
              <h2 className="mb-sm text-headline font-semibold">{section}</h2>
              <div className="grid grid-cols-[var(--size-sidebar-min)_minmax(0,1fr)_minmax(0,1fr)] overflow-hidden rounded-md border border-border">
                <div className="bg-card px-md py-xs text-caption text-muted-foreground">State</div>
                <div className="bg-card px-md py-xs text-caption text-muted-foreground">Light</div>
                <div className="bg-card px-md py-xs text-caption text-muted-foreground">Dark</div>
                {(GALLERY[section] as readonly string[]).map((state) => (
                  <GalleryRow key={state} section={section} state={state} />
                ))}
                <div className="border-t border-border px-md py-sm text-caption text-muted-foreground">Playground</div>
                {(["light", "dark"] as const).map((theme) => (
                  <div key={theme} className={`${theme} border-t border-border bg-background p-md text-foreground`}>
                    <Playground section={section} />
                  </div>
                ))}
              </div>
            </section>
          ))}
        </main>
      </div>
    </TooltipProvider>
  );
}

/**
 * The production screens the gallery renders on synthetic scenes, one
 * document each (sceneData.ts): the Projects sidebar at the shipped width, a
 * narrow width, and a larger interface font, with the Pen frame's titles and
 * with long Korean ones, in both themes. Hover, focus and the folds are live.
 */
const SCENE_CASES = [
  { label: "Reference · shipped width", query: "content=reference" },
  { label: "Long titles · shipped width", query: "content=long" },
  { label: "Long titles · narrow (240)", query: "content=long&width=240" },
  { label: "Reference · larger text", query: "content=reference&scale=1.25" },
  { label: "Devices · This Mac in front", query: "content=reference&devices=two" },
  { label: "Devices · mini in front", query: "content=reference&devices=two&front=mini" },
  { label: "Devices · one device", query: "content=reference" },
  { label: "Devices · busy mini (9+)", query: "content=reference&devices=busy&front=mini" },
  { label: "Devices · rail hidden", query: "content=reference&devices=two&rail=hidden" },
  { label: "Devices · not connected", query: "content=reference&devices=two&front=offline" },
  { label: "Devices · long names · narrow (240)", query: "content=long&devices=two&width=240" },
] as const;

function Scenes() {
  return (
    <section id="scenes" data-gallery-section="Scenes" className="mb-xxxl">
      <h2 className="mb-sm text-headline font-semibold">Projects sidebar scene</h2>
      <div className="grid grid-cols-[var(--size-sidebar-min)_minmax(0,1fr)_minmax(0,1fr)] overflow-hidden rounded-md border border-border">
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Case</div>
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Light</div>
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Dark</div>
        {SCENE_CASES.map(({ label, query }) => (
          <SceneRow key={label} label={label} query={query} />
        ))}
      </div>
    </section>
  );
}

/**
 * The parts the sidebar, the session record and Desk draw alike (PRD
 * session-docs-and-assistant D-72), in their states, in both themes.
 */
function SharedParts() {
  return (
    <section id="shared-parts" data-gallery-section="Shared parts" className="mb-xxxl">
      <h2 className="mb-sm text-headline font-semibold">Shared parts</h2>
      <div className="grid grid-cols-2 overflow-hidden rounded-md border border-border">
        {(["light", "dark"] as const).map((theme) => (
          <iframe key={theme} loading="lazy" title={`Shared parts ${theme}`} src={`/gallery?scene=shared-parts&theme=${theme}`} className="h-(--size-gallery-scene-h) w-full border-0" />
        ))}
      </div>
    </section>
  );
}

/**
 * Desk 1차 for review before it is wired (PRD session-docs-and-assistant):
 * every scene of the approved mockup and the states it leaves out, from the
 * real Desk parts over invented values, in both themes.
 */
function Desk() {
  return (
    <section id="desk" data-gallery-section="Desk" className="mb-xxxl">
      <h2 className="mb-sm text-headline font-semibold">Desk</h2>
      <div className="grid grid-cols-[var(--size-sidebar-min)_minmax(0,1fr)_minmax(0,1fr)] overflow-hidden rounded-md border border-border">
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Scene</div>
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Light</div>
        <div className="bg-card px-md py-xs text-caption text-muted-foreground">Dark</div>
        {DESK_SCENES.map(({ state, label }) => (
          <Fragment key={state}>
            <div className="border-t border-border px-md py-sm text-caption text-subtle-foreground">{label}</div>
            {(["light", "dark"] as const).map((theme) => (
              <div key={theme} className="border-t border-border">
                <iframe loading="lazy" title={`${label} ${theme}`} src={`/gallery?scene=desk&state=${state}&theme=${theme}`} className="h-(--size-gallery-scene-h) w-full border-0" />
              </div>
            ))}
          </Fragment>
        ))}
      </div>
    </section>
  );
}

function SceneRow({ label, query }: { label: string; query: string }) {
  return (
    <>
      <div className="border-t border-border px-md py-sm text-caption text-subtle-foreground">{label}</div>
      {(["light", "dark"] as const).map((theme) => (
        <div key={theme} className="border-t border-border">
          <iframe loading="lazy" title={`${label} ${theme}`} src={`/gallery?scene=projects-sidebar&theme=${theme}&${query}`} className="h-(--size-gallery-scene-h) w-full border-0" />
        </div>
      ))}
    </>
  );
}

function GalleryRow({ section, state }: { section: Section; state: string }) {
  return (
    <>
      <div className="border-t border-border px-md py-sm text-caption text-subtle-foreground">{state}</div>
      {(["light", "dark"] as const).map((theme) => (
        <div key={theme} data-gallery-state-name={state} data-gallery-theme={theme} className={`${theme} border-t border-border bg-background text-foreground`}>
          <Cell section={section} state={state} theme={theme} />
        </div>
      ))}
    </>
  );
}

/** One layer state alone in its frame document, in one theme. */
export function GalleryFrame({ section, state, theme }: { section: Section; state: string; theme: Theme }) {
  useGalleryEscape();
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme]);
  const render = stateRenderer(section, state);
  return (
    <TooltipProvider>
      <div ref={root} data-gallery-frame={`${section}/${state}`} className="flex h-full items-start bg-background p-lg text-foreground">
        {render()}
      </div>
    </TooltipProvider>
  );
}
