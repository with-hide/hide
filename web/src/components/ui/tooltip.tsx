import { Tooltip as TooltipPrimitive } from "radix-ui";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ComponentProps, type FocusEvent, type ReactNode, type RefObject } from "react";
import { visiblePageRects } from "../../browserViews";
import { cn } from "../../lib/utils";
import { shellDragging } from "../../shellDrag";
import { hintSide, type HintSide } from "../../tooltipSide";
import { useUiStore } from "../../ui";

// A hint for a control whose meaning is its icon or its shortcut. The label a
// tooltip shows is also the control's accessible name, so a screen reader and
// a pointer read the same words.

// A hint is read, never used: it holds nothing to click, so it closes when the
// pointer leaves its trigger and lets a click through to whatever it covers.
function TooltipProvider({ delayDuration = 500, disableHoverableContent = true, ...props }: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider data-slot="tooltip-provider" delayDuration={delayDuration} disableHoverableContent={disableHoverableContent} {...props} />;
}

/** The element a hint is about, so its content can place itself beside it. */
const TriggerContext = createContext<RefObject<HTMLElement | null> | null>(null);

function Tooltip(props: ComponentProps<typeof TooltipPrimitive.Root>) {
  const trigger = useRef<HTMLElement | null>(null);
  return (
    <TriggerContext.Provider value={trigger}>
      <TooltipPrimitive.Root data-slot="tooltip" {...props} />
    </TriggerContext.Provider>
  );
}

function TooltipTrigger({ ref, ...props }: ComponentProps<typeof TooltipPrimitive.Trigger>) {
  const trigger = useContext(TriggerContext);
  const attach = useCallback(
    (node: HTMLButtonElement | null) => {
      if (trigger) trigger.current = node;
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
    },
    [trigger, ref],
  );
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" ref={attach} {...props} />;
}

/**
 * A hint is drawn under a browser page it meets, since the page is a native
 * view, and it never freezes one. When pages are shown, the hint measures
 * itself as it mounts, before Radix places it, and takes the first side that
 * fits the window clear of them (`tooltipSide.ts`); with none shown it keeps
 * the side it was given.
 */
function TooltipContent({ ref, className, side = "top", align = "center", sideOffset = 4, children, ...props }: ComponentProps<typeof TooltipPrimitive.Content>) {
  const trigger = useContext(TriggerContext);
  const [clear, setClear] = useState<HintSide | null>(null);
  const measure = useCallback(
    (node: HTMLDivElement | null) => {
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
      const pages = visiblePageRects();
      const anchor = trigger?.current;
      if (!node || !anchor || pages.length === 0) return setClear(null);
      const box = anchor.getBoundingClientRect();
      setClear(
        hintSide({
          side,
          align,
          offset: sideOffset,
          trigger: { x: box.left, y: box.top, width: box.width, height: box.height },
          size: { width: node.offsetWidth, height: node.offsetHeight },
          viewport: { width: document.documentElement.clientWidth, height: document.documentElement.clientHeight },
          pages,
        }),
      );
    },
    [ref, trigger, side, align, sideOffset],
  );
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        ref={measure}
        data-slot="tooltip-content"
        side={clear ?? side}
        align={align}
        sideOffset={sideOffset}
        className={cn("pointer-events-none z-50 max-w-(--size-tooltip-max-width) text-balance rounded-sm border border-border bg-popover px-sm py-xs text-caption text-popover-foreground shadow-lg", className)}
        {...props}
        onMouseDown={(event) => {
          props.onMouseDown?.(event);
          // A hint that can be hovered may hold a button. Pressing it must not
          // move focus: the hint is drawn outside any dialog it opened from,
          // and the dialog's focus trap would take focus back and close the
          // hint before the click lands. The button still gets its click.
          if (event.target instanceof Element && event.target.closest("button, a")) event.preventDefault();
        }}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}

/** What a press on a hinted trigger opens: a menu, a popover, or a dialog. */
const LAYER = '[data-radix-popper-content-wrapper], [role="dialog"], [role="alertdialog"]';

/**
 * The open state of a hint or a card that a trigger shows on hover and focus.
 * A press on the trigger (a click that opens a menu or a dialog, or a
 * right-click that opens a context menu) ends it until the pointer comes back
 * to the trigger or the keyboard leaves it for somewhere outside the layer the
 * press opened. The hover delay that started before the press, and the focus
 * a closing menu hands back to its trigger, would otherwise open it over or
 * after whatever the press opened; Tab to the trigger later still shows it.
 * While open it is registered with the shell as a tooltip, not an Escape
 * layer: Escape closes it through the tooltip's own dismiss and still reaches
 * the terminal or screen it was meant for, and an Escape the shell consumes
 * closes it from `keyboard.ts` instead.
 */
function useHintOpen() {
  const [open, setOpen] = useState(false);
  const pressed = useRef(false);
  useEffect(() => {
    if (!open) return;
    return useUiStore.getState().pushTooltip(() => setOpen(false));
  }, [open]);
  const press = () => {
    pressed.current = true;
    setOpen(false);
  };
  return {
    open,
    // A pane or agent dragged across the trigger is not pointing at it.
    onOpenChange: (next: boolean) => setOpen(next && !pressed.current && !shellDragging()),
    triggerProps: {
      onPointerDown: press,
      onContextMenu: press,
      onPointerEnter: () => {
        pressed.current = false;
      },
      onBlur: (event: FocusEvent<HTMLElement>) => {
        const next = event.relatedTarget;
        if (!(next instanceof Element && next.closest(LAYER))) pressed.current = false;
      },
    },
  };
}

/**
 * The shell's one way to give a control a hint: the trigger keeps its own
 * element (asChild) and gets `label` as its accessible name unless it already
 * has one, and the hint shows `label` and an optional shortcut. `reveals`
 * marks a hint that shows the whole of a clipped name or path instead; that
 * text is already the element's own, so it does not become a second name.
 */
function Hint({
  label,
  shortcut,
  contentClassName,
  side,
  reveals = false,
  active = true,
  children,
}: {
  label: string;
  shortcut?: ReactNode;
  contentClassName?: string;
  side?: "top" | "right" | "bottom" | "left";
  reveals?: boolean;
  /** False while the control says all of this itself (a visible word), so no hint opens. */
  active?: boolean;
  children: ReactNode;
}) {
  const { open, onOpenChange, triggerProps } = useHintOpen();
  return (
    <Tooltip open={active && open} onOpenChange={(next) => (active || !next) && onOpenChange(next)}>
      <TooltipTrigger asChild aria-label={reveals ? undefined : label} {...triggerProps}>
        {children}
      </TooltipTrigger>
      <TooltipContent side={side} className={contentClassName}>
        <span className="inline-flex items-center gap-sm whitespace-pre-line">
          {label}
          {shortcut ? <span className="text-muted-foreground">{shortcut}</span> : null}
        </span>
      </TooltipContent>
    </Tooltip>
  );
}

export { Hint, Tooltip, TooltipContent, TooltipProvider, TooltipTrigger, useHintOpen };
