import { Select as SelectPrimitive } from "radix-ui";
import { CheckIcon, ChevronDownIcon, ChevronUpIcon } from "lucide-react";
import { createContext, useContext, useRef, type ComponentProps, type ReactNode, type RefObject } from "react";
import { cn } from "../../lib/utils";
import { useLayerOpen, useReturnFocus } from "./layer";
import { menuContent, menuItem, menuLabel, menuSeparator } from "./menu-styles";

/** The open state and trigger of one Select, so its content hands the keyboard back through the shell's guard. */
const SelectLayerContext = createContext<{ open: boolean; trigger: RefObject<HTMLButtonElement | null> } | null>(null);

function Select({ open, defaultOpen, onOpenChange, ...props }: ComponentProps<typeof SelectPrimitive.Root>) {
  const [current, change] = useLayerOpen(open, defaultOpen, onOpenChange);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <SelectLayerContext.Provider value={{ open: current, trigger }}>
      <SelectPrimitive.Root data-slot="select" open={current} onOpenChange={change} {...props} />
    </SelectLayerContext.Provider>
  );
}

function SelectGroup(props: ComponentProps<typeof SelectPrimitive.Group>) {
  return <SelectPrimitive.Group data-slot="select-group" {...props} />;
}

function SelectValue(props: ComponentProps<typeof SelectPrimitive.Value>) {
  return <SelectPrimitive.Value data-slot="select-value" {...props} />;
}

/**
 * `ghost` drops the field's frame for a quiet word that lights on hover, for a
 * menu that sits among words in a composer's row (the start panel's agent and model).
 */
function SelectTrigger({ className, size = "default", variant = "default", children, ...props }: ComponentProps<typeof SelectPrimitive.Trigger> & { size?: "sm" | "default"; variant?: "default" | "ghost" }) {
  const layer = useContext(SelectLayerContext);
  return (
    <SelectPrimitive.Trigger
      ref={layer?.trigger}
      data-slot="select-trigger"
      data-size={size}
      data-variant={variant}
      className={cn(
        "flex w-(--size-settings-control-w) max-w-full items-center justify-between gap-xs whitespace-nowrap rounded-sm border border-input bg-background px-sm text-body text-foreground outline-none data-[variant=ghost]:border-transparent data-[variant=ghost]:bg-transparent data-[variant=ghost]:px-xs data-[variant=ghost]:text-subtle-foreground data-[variant=ghost]:hover:bg-accent data-[variant=ghost]:hover:text-accent-foreground data-[variant=ghost]:data-[state=open]:bg-accent transition-colors focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-(--opacity-disabled) aria-invalid:border-destructive data-[placeholder]:text-muted-foreground data-[size=default]:h-(--size-control) data-[size=sm]:h-(--size-control-sm) *:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-xs [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-(--size-icon) [&_svg:not([class*='text-'])]:text-muted-foreground",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDownIcon />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

function SelectContent({ className, children, position = "popper", onCloseAutoFocus, ...props }: ComponentProps<typeof SelectPrimitive.Content>) {
  const layer = useContext(SelectLayerContext);
  const surface = useRef<HTMLDivElement>(null);
  // Radix hands the keyboard to the trigger from a timer after the close, unconditionally;
  // the shell's guard keeps a control or layer that took the keyboard in the meantime.
  const returnFocus = useReturnFocus(layer?.open ?? false, { target: () => layer?.trigger.current ?? null, within: () => surface.current });
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        ref={surface}
        data-slot="select-content"
        className={cn(menuContent, "relative max-h-(--radix-select-content-available-height)", position === "popper" && "w-full min-w-(--radix-select-trigger-width)", className)}
        position={position}
        sideOffset={position === "popper" ? 4 : undefined}
        onCloseAutoFocus={(event) => {
          onCloseAutoFocus?.(event);
          if (!event.defaultPrevented) returnFocus(event);
        }}
        {...props}
      >
        <SelectPrimitive.ScrollUpButton className="flex cursor-default items-center justify-center py-xxs">
          <ChevronUpIcon className="size-(--size-icon)" />
        </SelectPrimitive.ScrollUpButton>
        <SelectPrimitive.Viewport className={cn(position === "popper" && "h-(--radix-select-trigger-height) w-full min-w-(--radix-select-trigger-width) scroll-my-xxs")}>
          {children}
        </SelectPrimitive.Viewport>
        <SelectPrimitive.ScrollDownButton className="flex cursor-default items-center justify-center py-xxs">
          <ChevronDownIcon className="size-(--size-icon)" />
        </SelectPrimitive.ScrollDownButton>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

function SelectLabel({ className, ...props }: ComponentProps<typeof SelectPrimitive.Label>) {
  return <SelectPrimitive.Label data-slot="select-label" className={cn(menuLabel, className)} {...props} />;
}

/**
 * One option. A `description` is a caption line under the option's name inside the open list only, and `trailing`
 * a quiet figure at the option's right end on its line (a percent left): the trigger shows the name alone, since
 * Radix copies only `ItemText` into the value.
 */
function SelectItem({ className, children, description, trailing, ...props }: ComponentProps<typeof SelectPrimitive.Item> & { description?: ReactNode; trailing?: ReactNode }) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(menuItem, "w-full pr-xl *:data-[slot=select-item-text]:flex *:data-[slot=select-item-text]:min-w-0 *:data-[slot=select-item-text]:items-center *:data-[slot=select-item-text]:gap-sm", className)}
      {...props}
    >
      <span className="absolute right-sm flex size-(--size-icon) items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <CheckIcon className="size-(--size-icon-sm)" />
        </SelectPrimitive.ItemIndicator>
      </span>
      {/* Radix drops ItemText's className, so the row lays its mark beside its name from here, as a menu item does. */}
      {description === undefined ? (
        <>
          <SelectPrimitive.ItemText data-slot="select-item-text">{children}</SelectPrimitive.ItemText>
          {trailing === undefined ? null : (
            <span className="ml-auto shrink-0 pl-md text-caption tabular-nums text-muted-foreground" data-slot="select-item-trailing">
              {trailing}
            </span>
          )}
        </>
      ) : (
        <span className="flex min-w-0 flex-col gap-xxs">
          <span className="flex min-w-0 items-center gap-sm">
            <SelectPrimitive.ItemText data-slot="select-item-text">{children}</SelectPrimitive.ItemText>
          </span>
          <span className="min-w-0 break-all text-caption text-muted-foreground" data-slot="select-item-description">
            {description}
          </span>
        </span>
      )}
    </SelectPrimitive.Item>
  );
}

function SelectSeparator({ className, ...props }: ComponentProps<typeof SelectPrimitive.Separator>) {
  return <SelectPrimitive.Separator data-slot="select-separator" className={cn(menuSeparator, className)} {...props} />;
}

export { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectSeparator, SelectTrigger, SelectValue };
