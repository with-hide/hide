import { DropdownMenu as MenuPrimitive, Slot } from "radix-ui";
import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ComponentProps, type KeyboardEvent, type MouseEvent, type RefObject } from "react";
import { cn } from "../../lib/utils";
import { useEscapeLayer, useReturnFocus } from "./layer";
import { menuContent, menuItem, menuLabel, menuSeparator, menuShortcut } from "./menu-styles";
import { useFloatingSurface } from "./floating-surface";

// shadcn's Context Menu API over Radix's menu with a point anchor. Radix's own
// context-menu root has no `open` prop, so it could not join the shell's
// Escape layers (keyboard.ts answers Escape before Radix sees it); a controlled
// menu anchored where the pointer was, or under the target for the menu key
// and ⇧F10, behaves the same and can.

type Point = { x: number; y: number };
type ContextMenuState = {
  point: Point | null;
  openAt: (point: Point) => void;
  host: RefObject<HTMLElement | null>;
  returnToInput: boolean;
};

const ContextMenuContext = createContext<ContextMenuState | null>(null);

function useContextMenu() {
  const state = useContext(ContextMenuContext);
  if (!state) throw new Error("ContextMenu parts must sit inside <ContextMenu>");
  return state;
}

/** `canOpen` is asked on every right-click or menu key; false keeps the menu closed, as a disabled trigger does. */
function ContextMenu({ onOpenChange, canOpen, returnToInput = false, children }: { onOpenChange?: (open: boolean) => void; canOpen?: () => boolean; returnToInput?: boolean; children: React.ReactNode }) {
  const [point, setPoint] = useState<Point | null>(null);
  const host = useRef<HTMLElement | null>(null);
  const change = (next: boolean) => {
    if (!next) setPoint(null);
    onOpenChange?.(next);
  };
  useEscapeLayer(point !== null, () => change(false));
  const openAt = (at: Point) => {
    if (canOpen && !canOpen()) return;
    setPoint(at);
    onOpenChange?.(true);
  };
  return (
    <ContextMenuContext.Provider value={{ point, openAt, host, returnToInput }}>
      <MenuPrimitive.Root open={point !== null} onOpenChange={change}>
        {children}
      </MenuPrimitive.Root>
    </ContextMenuContext.Provider>
  );
}

/** How long a finger rests on a target before its menu opens, and how far it may drift meanwhile (iOS's own long press). */
const LONG_PRESS_MS = 500;
const LONG_PRESS_SLOP_PX = 10;

/**
 * The target: a right-click opens the menu at the pointer, the menu key or ⇧F10 under the target, and a finger
 * held still on it opens the menu where it rests, since iOS sends no `contextmenu` for a long press. The click
 * that ends a long press is not the target's: it is swallowed, so the press that opened the menu does not also
 * open the row. A `disabled` target offers no menu: the right-click opens nothing, not even the browser's own.
 */
function ContextMenuTrigger({ asChild = false, className, onContextMenu, onKeyDown, onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onClickCapture, disabled = false, ...props }: ComponentProps<"div"> & { asChild?: boolean; disabled?: boolean }) {
  const { point, openAt, host, returnToInput } = useContextMenu();
  const Comp = asChild ? Slot.Root : "div";
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number } | null>(null);
  // Set when a long press opened the menu; the next pointer down clears it, so a click that never comes swallows nothing later.
  const held = useRef(false);
  const release = () => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = null;
  };
  useEffect(() => release, []);
  return (
    <>
      <Comp
        data-slot="context-menu-trigger"
        data-state={point ? "open" : "closed"}
        ref={(node: HTMLDivElement | null) => {
          host.current = node;
        }}
        className={className}
        onPointerDown={(event) => {
          onPointerDown?.(event);
          if (returnToInput && event.button === 2) event.preventDefault();
          held.current = false;
          release();
          if (disabled || event.pointerType !== "touch") return;
          const { clientX: x, clientY: y } = event;
          press.current = { x, y, timer: setTimeout(() => { press.current = null; held.current = true; openAt({ x, y }); }, LONG_PRESS_MS) };
        }}
        onPointerMove={(event) => {
          onPointerMove?.(event);
          const at = press.current;
          if (at && Math.hypot(event.clientX - at.x, event.clientY - at.y) > LONG_PRESS_SLOP_PX) release();
        }}
        onPointerUp={(event) => { onPointerUp?.(event); release(); }}
        onPointerCancel={(event) => { onPointerCancel?.(event); release(); }}
        onClickCapture={(event) => {
          onClickCapture?.(event);
          if (!held.current) return;
          held.current = false;
          event.preventDefault();
          event.stopPropagation();
        }}
        onContextMenu={(event: MouseEvent<HTMLDivElement>) => {
          onContextMenu?.(event);
          if (event.defaultPrevented) return;
          event.preventDefault();
          event.stopPropagation();
          if (disabled) return;
          openAt({ x: event.clientX, y: event.clientY });
        }}
        onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
          onKeyDown?.(event);
          if (disabled || event.defaultPrevented) return;
          if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect();
            openAt({ x: box.left, y: box.bottom });
          }
        }}
        {...props}
      />
      <MenuPrimitive.Trigger asChild>
        <span aria-hidden="true" data-context-menu-anchor="" className="pointer-events-none fixed size-(--spacing-none)" style={{ left: point?.x ?? 0, top: point?.y ?? 0 }} />
      </MenuPrimitive.Trigger>
    </>
  );
}

/**
 * A menu anchored at the pointer opens under the press that opened it: macOS sends `contextmenu` on the button's
 * press, so the button is released over the menu. Radix chooses an item on a pointer up that had no pointer down on
 * it, so that release would choose whatever item opened under the pointer and close the menu. The content marks that
 * release handled, which Radix's item reads and leaves alone, while the release still reaches every other listener;
 * the next press inside the menu ends the guard, so every later click chooses. A press dragged onto an item and
 * released there therefore opens the menu without choosing.
 */
export function useOpeningPressGuard(at: Point | null): Pick<ComponentProps<"div">, "onPointerDownCapture" | "onPointerUpCapture"> {
  const opening = useRef(false);
  // Set in the commit that opens the menu, or moves it to a new point, before the browser can deliver the release.
  const open = at !== null;
  useLayoutEffect(() => { opening.current = open; }, [open, at?.x, at?.y]);
  return {
    onPointerDownCapture: () => { opening.current = false; },
    onPointerUpCapture: (event) => {
      if (!opening.current) return;
      opening.current = false;
      event.preventDefault();
    },
  };
}

function ContextMenuContent({ className, onCloseAutoFocus, ...props }: ComponentProps<typeof MenuPrimitive.Content>) {
  const { point, host, returnToInput } = useContextMenu();
  const guard = useOpeningPressGuard(point);
  // Return to the real context target rather than the invisible anchor, unless
  // a newer control or layer already took the keyboard before Radix's timer.
  const returnFocus = useReturnFocus(point !== null, returnToInput ? undefined : {
    target: () => host.current?.matches("[tabindex], button, a[href]") ? host.current : host.current?.querySelector<HTMLElement>("[tabindex], button, a[href]") ?? null,
  });
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        data-slot="context-menu-content"
        align="start"
        side="bottom"
        sideOffset={0}
        className={cn(menuContent, useFloatingSurface(), className)}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (event.defaultPrevented) return;
          returnFocus(event);
        }}
        {...guard}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

function ContextMenuGroup(props: ComponentProps<typeof MenuPrimitive.Group>) {
  return <MenuPrimitive.Group data-slot="context-menu-group" {...props} />;
}

function ContextMenuItem({ className, inset, variant = "default", ...props }: ComponentProps<typeof MenuPrimitive.Item> & { inset?: boolean; variant?: "default" | "destructive" }) {
  return <MenuPrimitive.Item data-slot="context-menu-item" data-inset={inset} data-variant={variant} className={cn(menuItem, className)} {...props} />;
}

function ContextMenuLabel({ className, inset, ...props }: ComponentProps<typeof MenuPrimitive.Label> & { inset?: boolean }) {
  return <MenuPrimitive.Label data-slot="context-menu-label" data-inset={inset} className={cn(menuLabel, className)} {...props} />;
}

function ContextMenuSeparator({ className, ...props }: ComponentProps<typeof MenuPrimitive.Separator>) {
  return <MenuPrimitive.Separator data-slot="context-menu-separator" className={cn(menuSeparator, className)} {...props} />;
}

function ContextMenuShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span data-slot="context-menu-shortcut" className={cn(menuShortcut, className)} {...props} />;
}

export { ContextMenu, ContextMenuTrigger, ContextMenuContent, ContextMenuGroup, ContextMenuItem, ContextMenuLabel, ContextMenuSeparator, ContextMenuShortcut };
