import { DropdownMenu as MenuPrimitive, Slot } from "radix-ui";
import { createContext, useContext, useRef, useState, type ComponentProps, type KeyboardEvent, type MouseEvent, type RefObject } from "react";
import { cn } from "../../lib/utils";
import { useEscapeLayer, useReturnFocus } from "./layer";
import { menuContent, menuItem, menuLabel, menuSeparator, menuShortcut } from "./menu-styles";

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

/**
 * The target: a right-click opens the menu at the pointer, the menu key or ⇧F10 under the target.
 * A `disabled` target offers no menu: the right-click opens nothing, not even the browser's own.
 */
function ContextMenuTrigger({ asChild = false, className, onContextMenu, onKeyDown, onPointerDown, disabled = false, ...props }: ComponentProps<"div"> & { asChild?: boolean; disabled?: boolean }) {
  const { point, openAt, host, returnToInput } = useContextMenu();
  const Comp = asChild ? Slot.Root : "div";
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

function ContextMenuContent({ className, onCloseAutoFocus, ...props }: ComponentProps<typeof MenuPrimitive.Content>) {
  const { point, host, returnToInput } = useContextMenu();
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
        className={cn(menuContent, className)}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (event.defaultPrevented) return;
          returnFocus(event);
        }}
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
