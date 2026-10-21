// Hide's action menus on the System menu parts (PRD S6 D-13, B18; S5 B11,
// B19). A target's menu acts on the target it was opened on; opening it
// changes nothing - no focus, read state or selection - and Escape closes it
// without choosing. An item the target cannot use is drawn disabled with its
// reason, never hidden behind an action that would fail.

import { DropdownMenu as MenuPrimitive } from "radix-ui";
import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "../lib/utils";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuShortcut, ContextMenuTrigger } from "./ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import { useEscapeLayer, useReturnFocus } from "./ui/layer";
import { menuContent } from "./ui/menu-styles";
import { useInterfaceTranslation } from "../i18n/client";
import { menuGroupSymbols, menuSymbols, type MenuPresentation, type MenuGroup } from "./menu-symbols";
import { Hint } from "./ui/tooltip";

export type MenuEntry<Id extends string = string> = MenuPresentation & {
  id: Id;
  label: string;
  /** Why the action is not offered here; the item is drawn disabled with this as its reason. */
  unavailable: string | null;
  /** Starts a new visual section, drawn after a separator. */
  separated?: boolean;
  /** An action that removes or discards something, drawn in the destructive color. */
  destructive?: boolean;
  /** The chord that does the same, drawn at the item's end; "" or absent draws none. */
  shortcut?: string;
  /** Nested destination actions, rendered with the same menu and keyboard behavior. */
  children?: MenuEntry<Id>[];
  /** One choice out of several: the entry opens a submenu of radio items and picking one selects that choice's id. */
  choices?: { id: Id; label: string; checked: boolean }[];
};

const entryBounds = "max-h-(--radix-dropdown-menu-content-available-height) max-w-[calc(100vw-var(--spacing-md))]";

type Parts = { Item: typeof DropdownMenuItem | typeof ContextMenuItem; Separator: typeof DropdownMenuSeparator | typeof ContextMenuSeparator };

function FlatEntryItems<Id extends string>({ items, onSelect, parts, withIcons = items.some((item) => item.icon) }: { items: MenuEntry<Id>[]; onSelect: (id: Id) => void; parts: Parts; withIcons?: boolean }) {
  const { Item, Separator } = parts;
  return (
    <>
      {items.map((item) => (
        <Fragment key={item.id}>
          {item.separated ? <Separator /> : null}
          {item.children ? (
            <NestedEntryMenu item={item} onSelect={onSelect} parts={parts} withIcons={withIcons} />
          ) : item.choices ? (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger disabled={item.unavailable !== null} data-menu-item={item.id}>
                <MenuSymbolSlot name={item.icon} reserved={withIcons} destructive={item.destructive} />
                <span data-menu-label="">{item.menuLabel ?? item.label}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent aria-label={item.label} data-menu-choices={item.id}>
                <DropdownMenuRadioGroup value={item.choices.find((choice) => choice.checked)?.id ?? ""} onValueChange={(value) => onSelect(value as Id)}>
                  {item.choices.map((choice) => (
                    <DropdownMenuRadioItem key={choice.id} value={choice.id} data-menu-item={choice.id}>
                      <span data-menu-label="">{choice.label}</span>
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
          ) : (
            <Item disabled={item.unavailable !== null} variant={item.destructive ? "destructive" : "default"} data-menu-item={item.id} textValue={item.menuLabel ?? item.label} className="flex-col items-stretch gap-none" onSelect={() => onSelect(item.id)}>
              <span className="flex items-center gap-sm">
                <MenuSymbolSlot name={item.icon} reserved={withIcons} destructive={item.destructive} />
                <span data-menu-label="">{item.menuLabel ?? item.label}</span>
                {item.shortcut ? <ContextMenuShortcut data-menu-shortcut={item.shortcut}>{item.shortcut}</ContextMenuShortcut> : null}
              </span>
              {item.unavailable ? <span data-menu-reason="" className={cn("text-caption text-muted-foreground", withIcons && "pl-[calc(var(--size-icon)+var(--spacing-sm))]")}>{item.unavailable}</span> : null}
            </Item>
          )}
        </Fragment>
      ))}
    </>
  );
}

/** Each destination level owns Escape and returns focus to its own trigger. */
function NestedEntryMenu<Id extends string>({ item, onSelect, parts, withIcons }: { item: MenuEntry<Id>; onSelect: (id: Id) => void; parts: Parts; withIcons: boolean }) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const close = () => { setOpen(false); trigger.current?.focus(); };
  useLayoutEffect(() => {
    if (open && item.unavailable !== null) { setOpen(false); trigger.current?.focus(); }
  }, [open, item.unavailable]);
  useEscapeLayer(open && item.unavailable === null, close);
  return <DropdownMenuSub open={open && item.unavailable === null} onOpenChange={setOpen}>
    <DropdownMenuSubTrigger ref={trigger} disabled={item.unavailable !== null} data-menu-item={item.id} textValue={item.label} onPointerLeave={event => { if (event.relatedTarget instanceof Node && content.current?.contains(event.relatedTarget)) event.preventDefault(); }}>
      <MenuSymbolSlot name={item.icon} reserved={withIcons} />
      <span className="flex min-w-0 flex-1 flex-col"><span className="truncate" data-menu-label="">{item.menuLabel ?? item.label}</span>{item.unavailable ? <span className="text-caption text-muted-foreground" data-menu-reason="">{item.unavailable}</span> : null}</span>
    </DropdownMenuSubTrigger>
    <MenuPrimitive.Portal><DropdownMenuSubContent ref={content} className={entryBounds} aria-label={item.label} onEscapeKeyDown={event => { event.preventDefault(); close(); }}><FlatEntryItems items={item.children ?? []} onSelect={onSelect} parts={parts} /></DropdownMenuSubContent></MenuPrimitive.Portal>
  </DropdownMenuSub>;
}

function MenuSymbolSlot({ name, reserved, destructive }: { name?: MenuPresentation["icon"]; reserved: boolean; destructive?: boolean }) {
  if (!name) return reserved ? <span aria-hidden="true" className="size-(--size-icon) shrink-0" /> : null;
  const Icon = menuSymbols[name];
  return <Icon aria-hidden="true" data-menu-icon={name} className={cn("size-(--size-icon) shrink-0", destructive && "text-destructive")} />;
}

/** Only the visual grouping changes: callers keep one flat, checked command list. */
function EntryItems<Id extends string>({ items, onSelect, parts }: { items: MenuEntry<Id>[]; onSelect: (id: Id) => void; parts: Parts }) {
  const runs: MenuEntry<Id>[][] = [];
  for (const item of items) {
    const last = runs.at(-1);
    if (last && last[0]?.group === item.group) last.push(item);
    else runs.push([item]);
  }
  return runs.map((entries) => {
    const first = entries[0]!;
    return first.group
      ? <ActionSubmenu key={first.id} group={first.group} items={entries} onSelect={onSelect} parts={parts} />
      : <FlatEntryItems key={first.id} withIcons={items.some((item) => item.icon || item.group)} items={entries} onSelect={onSelect} parts={parts} />;
  });
}

function ActionSubmenu<Id extends string>({ group, items, onSelect, parts }: { group: MenuGroup; items: MenuEntry<Id>[]; onSelect: (id: Id) => void; parts: Parts }) {
  const { t } = useInterfaceTranslation();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const reason = items[0]!.unavailable;
  // Collapse an identical refusal once; different reasons remain beside each choice.
  const unavailable = reason !== null && items.every((item) => item.unavailable === reason) ? reason : null;
  useLayoutEffect(() => {
    if (open && unavailable !== null) {
      setOpen(false);
      trigger.current?.focus();
    }
  }, [open, unavailable]);
  useEscapeLayer(open && unavailable === null, () => { setOpen(false); trigger.current?.focus(); });
  const label = t(`menus.group.${group}`);
  const { Separator } = parts;
  return (
    <>
      {items[0]!.separated ? <Separator /> : null}
      <DropdownMenuSub open={open && unavailable === null} onOpenChange={setOpen}>
        <DropdownMenuSubTrigger ref={trigger} disabled={unavailable !== null} textValue={label} data-menu-group={group}
          onPointerLeave={(event) => {
            // A submenu can flip left at the window edge. When the pointer
            // already reached it, keep focus there instead of relying on the
            // previous horizontal direction used by Radix's grace polygon.
            if (event.relatedTarget instanceof Node && content.current?.contains(event.relatedTarget)) event.preventDefault();
          }}>
          <MenuSymbolSlot name={menuGroupSymbols[group]} reserved />
          <span className="flex min-w-0 flex-1 flex-col">
            <span data-menu-label="">{label}</span>
            {unavailable ? <span data-menu-reason="" className="text-caption text-muted-foreground">{unavailable}</span> : null}
          </span>
        </DropdownMenuSubTrigger>
        <MenuPrimitive.Portal>
          <DropdownMenuSubContent ref={content} className={entryBounds} aria-label={label} data-menu-submenu={group} onEscapeKeyDown={(event) => { event.preventDefault(); setOpen(false); trigger.current?.focus(); }}>
            <FlatEntryItems items={items.map((item, index) => index === 0 ? { ...item, separated: false } : item)} onSelect={onSelect} parts={parts} />
          </DropdownMenuSubContent>
        </MenuPrimitive.Portal>
      </DropdownMenuSub>
    </>
  );
}

function MenuTitle({ title }: { title?: string }) {
  return title ? <MenuPrimitive.Label title={title} className="max-w-(--size-relationship-list-max) truncate border-b border-border px-sm py-xs text-caption text-muted-foreground" data-menu-title="">{title}</MenuPrimitive.Label> : null;
}

/**
 * Wraps a target so a right-click opens its menu at the pointer, and the menu
 * key or ⇧F10 under the target. `items` is read while the menu is open, so
 * confirmed changes keep its choices current without reopening it.
 * The `data-*` hooks name both the target and its menu, which opens in a
 * portal outside the target. `asChild` makes the one child element the
 * target itself (a list row) instead of wrapping it. A `disabled` target, or
 * one whose `items` are empty when asked, offers no menu at all rather than
 * opening one with nothing in it.
 */
export function EntryContextMenu<Id extends string>({
  label,
  title,
  items,
  onSelect,
  children,
  className,
  onCloseAutoFocus,
  asChild = false,
  disabled = false,
  returnToInput = false,
  ...data
}: {
  label: string;
  title?: string;
  items: () => MenuEntry<Id>[];
  onSelect: (id: Id) => void;
  children: ReactNode;
  className?: string;
  onCloseAutoFocus?: (event: Event) => void;
  asChild?: boolean;
  disabled?: boolean;
  returnToInput?: boolean;
} & Record<`data-${string}`, string>) {
  const [open, setOpen] = useState(false);
  const entries = open ? items() : [];
  return (
    <ContextMenu
      returnToInput={returnToInput}
      canOpen={() => items().length > 0}
      onOpenChange={setOpen}
    >
      <ContextMenuTrigger asChild={asChild} disabled={disabled} className={cn("relative", className)} {...data}>
        {children}
      </ContextMenuTrigger>
      {entries.length ? (
        <ContextMenuContent className={entryBounds} aria-label={label} onCloseAutoFocus={onCloseAutoFocus} {...data}>
          <MenuTitle title={title} />
          <EntryItems items={entries} onSelect={onSelect} parts={{ Item: ContextMenuItem, Separator: ContextMenuSeparator }} />
        </ContextMenuContent>
      ) : null}
    </ContextMenu>
  );
}

/**
 * A menu a control opens, anchored under it. `trigger` is the control itself;
 * `hint` names an icon-only trigger in a tooltip and as its accessible name.
 */
export function EntryDropdown<Id extends string>({
  label,
  title,
  items,
  onSelect,
  trigger,
  hint,
  align = "end",
  onOpenChange,
  ...data
}: {
  label: string;
  title?: string;
  /** The items, or a reader called while open so confirmed changes keep its choices current. */
  items: MenuEntry<Id>[] | (() => MenuEntry<Id>[]);
  onSelect: (id: Id) => void;
  trigger: ReactNode;
  hint?: string;
  align?: "start" | "center" | "end";
  onOpenChange?: (open: boolean) => void;
} & Record<`data-${string}`, string>) {
  const [open, setOpen] = useState(false);
  const returnFocus = useReturnFocus(open);
  const entries = typeof items === "function" ? (open ? items() : []) : items;
  const control = <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>;
  return (
    <DropdownMenu
      onOpenChange={(open) => {
        setOpen(open);
        onOpenChange?.(open);
      }}
    >
      {hint ? <Hint label={hint}>{control}</Hint> : control}
      <DropdownMenuContent className={entryBounds} aria-label={label} align={align} onCloseAutoFocus={returnFocus} {...data}>
        <MenuTitle title={title} />
        <EntryItems items={entries} onSelect={onSelect} parts={{ Item: DropdownMenuItem, Separator: DropdownMenuSeparator }} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * A menu opened at a point the caller chose (a pane header's click, a View
 * tab's overflow), closed by `onClose`. Closing gives the keyboard back to
 * whatever held it, as a context menu does.
 */
export function EntryPointMenu<Id extends string>({
  label,
  title,
  items,
  onSelect,
  at,
  onClose,
  ...data
}: {
  label: string;
  title?: string;
  items: MenuEntry<Id>[];
  onSelect: (id: Id) => void;
  /** The viewport point the menu opens at. */
  at: { x: number; y: number } | null;
  onClose: () => void;
} & Record<`data-${string}`, string>) {
  useEscapeLayer(at !== null, onClose);
  const returnFocus = useReturnFocus(at !== null);
  return (
    <MenuPrimitive.Root open={at !== null} onOpenChange={(open) => (open ? undefined : onClose())}>
      <MenuPrimitive.Trigger asChild>
        <span aria-hidden="true" className="pointer-events-none fixed size-(--spacing-none)" style={{ left: at?.x ?? 0, top: at?.y ?? 0 }} />
      </MenuPrimitive.Trigger>
      <MenuPrimitive.Portal>
        <MenuPrimitive.Content aria-label={label} align="start" sideOffset={0} className={cn(menuContent, entryBounds)} onCloseAutoFocus={returnFocus} {...data}>
          <MenuTitle title={title} />
          <EntryItems items={items} onSelect={onSelect} parts={{ Item: DropdownMenuItem, Separator: DropdownMenuSeparator }} />
        </MenuPrimitive.Content>
      </MenuPrimitive.Portal>
    </MenuPrimitive.Root>
  );
}
