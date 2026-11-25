import { CircleAlertIcon, XIcon } from "lucide-react";
import { useEffect, useRef, useState, type ComponentProps, type CSSProperties, type ReactNode } from "react";
import { cn } from "../../lib/utils";
import { Button } from "./button";
import { Hint } from "./tooltip";

/**
 * How much a notice asks of the operator: `info` tells them something and can
 * leave on its own, `important` waits for them (an action to take, or a state
 * that lasts) and is drawn in the warning colour.
 */
export type ToastLevel = "info" | "important";

/** How long an info notice stays while nothing holds it. */
export const TOAST_INFO_MS = 10_000;

/**
 * The window's corner where notices stand, newest nearest the corner. It takes
 * no press itself, so the screen under its empty space stays usable. `floor`
 * lifts it above the input boxes a screen keeps at the window's foot, in CSS
 * pixels (`noticeFloor.ts`).
 */
export function ToastStack({ floor = 0, children }: { floor?: number; children: ReactNode }) {
  return (
    <div
      data-toast-stack="true"
      style={{ "--toast-floor": `${floor}px` } as CSSProperties}
      className="pointer-events-none absolute bottom-[calc(var(--spacing-lg)+var(--toast-floor))] right-lg z-40 flex w-(--size-toast-w) max-w-[calc(100%-var(--spacing-xxl))] flex-col justify-end gap-sm"
    >
      {children}
    </div>
  );
}

type ToastProps = Omit<ComponentProps<"div">, "role"> & {
  level: ToastLevel;
  /** Words or links that act on the notice, under its text. */
  actions?: ReactNode;
  /** Draws the close button; also what `timeout` calls. */
  onDismiss?: () => void;
  dismissLabel?: string;
  /**
   * Leaves after this many ms, which a bar along its foot counts down; the
   * pointer or the keyboard inside it holds the count, which goes on from
   * where it stopped once they leave.
   */
  timeout?: number;
};

/** One notice in the corner stack. */
export function Toast({ level, actions, onDismiss, dismissLabel, timeout, className, children, ...props }: ToastProps) {
  const [held, setHeld] = useState(false);
  const dismiss = useRef(onDismiss);
  const left = useRef(timeout ?? 0);
  useEffect(() => {
    dismiss.current = onDismiss;
  }, [onDismiss]);
  useEffect(() => {
    if (timeout === undefined || held) return undefined;
    const began = Date.now();
    const timer = window.setTimeout(() => dismiss.current?.(), left.current);
    return () => {
      window.clearTimeout(timer);
      left.current = Math.max(0, left.current - (Date.now() - began));
    };
  }, [timeout, held]);
  return (
    <div
      role={level === "important" ? "alert" : "status"}
      data-toast-level={level}
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setHeld(false);
      }}
      className={cn(
        "pointer-events-auto relative flex items-start gap-sm overflow-hidden rounded-md border bg-popover py-sm pl-md pr-xs text-caption text-popover-foreground shadow-overlay",
        level === "important" ? "border-warning" : "border-border",
        className,
      )}
      {...props}
    >
      {level === "important" ? <CircleAlertIcon aria-hidden="true" className="mt-xxs size-(--size-icon-sm) shrink-0 text-warning" /> : null}
      <div className="flex min-w-0 flex-1 flex-col gap-xs py-xxs">
        <div className="break-keep wrap-anywhere">{children}</div>
        {actions ? <div className="flex flex-wrap items-center gap-md">{actions}</div> : null}
      </div>
      {onDismiss ? (
        <Hint label={dismissLabel ?? ""}>
          <Button variant="ghost" size="icon-xs" aria-label={dismissLabel} onClick={onDismiss}>
            <XIcon />
          </Button>
        </Hint>
      ) : null}
      {timeout !== undefined ? (
        <div
          aria-hidden="true"
          data-toast-countdown="true"
          className="toast-countdown absolute inset-x-0 bottom-0 h-xxs origin-left bg-muted-foreground"
          style={{ animationDuration: `${timeout}ms`, animationPlayState: held ? "paused" : "running" }}
        />
      ) : null}
    </div>
  );
}
