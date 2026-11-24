// Hide's settings-style rows (a Component, not a System part): a titled group
// of hairline-separated rows, a row's label and control, a reported value, and
// a state or note line. They mirror the native HideSettingsGroup/Row so a web
// sheet reads like the one it replaces; the controls inside are System parts.

import { ChevronRightIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";
import { RowOverlayButton } from "./ui/row-button";

/**
 * A card whose rows are separated by hairlines, like `HideSettingsGroup`; titled unless it stands alone, like the Hide AI switch.
 * A `caption` beside the title says what the group decides; an `action` sits at the title's right edge, for the one button that adds to the list below (Add device).
 * `columns` sits between the title and the card, for the heads of the controls its rows line up at their right edge (Agents' 권한 and 켬).
 * A `note` follows the card: words, or a block of its own (Agents' permission box).
 */
export function Group({ title, caption, note, action, columns, children, ...data }: { title?: string; caption?: ReactNode; note?: ReactNode; action?: ReactNode; columns?: ReactNode; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <section className="mb-lg" {...data}>
      {title || action ? (
        <div className="mb-sm flex items-center justify-between gap-md">
          {title ? (
            <span className="flex min-w-0 items-baseline gap-sm">
              <h3 className="shrink-0 text-body font-semibold text-subtle-foreground">{title}</h3>
              {caption ? <span className="min-w-0 truncate text-caption text-muted-foreground">{caption}</span> : null}
            </span>
          ) : <span />}
          {action}
        </div>
      ) : null}
      {columns}
      <div className="divide-y divide-border rounded-md border border-border bg-card">{children}</div>
      {note ? <div className="mt-sm text-body text-muted-foreground">{note}</div> : null}
    </section>
  );
}

/**
 * What opens under a row when the row is pressed: `label` names the press
 * for a screen reader, and `body` is drawn under the row while `open`.
 * `"none"` keeps the arrow's place empty, for a row that does not open
 * among rows that do, so their names line up.
 */
export type RowFold = { open: boolean; onToggle: () => void; label: string; body: ReactNode } | "none";

/**
 * A row wraps on narrow sheets; a separate value keeps its control at the first line's right edge.
 * `controlsEnd` keeps the controls at the right edge when they wrap under the label, so rows whose controls
 * stand in columns (Agents' 권한 and 켬) keep them lined up at any width.
 * With a `fold` the whole row is one press that opens what is under it, its arrow before the label; the
 * row's own controls and its `detail` stay pressable, and a control inside the label asks for it with `pointer-events-auto`.
 */
export function Row({ label, children, detail, value, className, controlsEnd = false, fold, ...data }: { label: ReactNode; children?: ReactNode; detail?: ReactNode; value?: ReactNode; className?: string; controlsEnd?: boolean; fold?: RowFold } & Record<`data-${string}`, string>) {
  if (fold !== undefined) {
    const opens = fold !== "none";
    return (
      <div className={className} {...data}>
        <div className="relative px-md py-sm">
          {opens ? <RowOverlayButton aria-label={fold.label} aria-expanded={fold.open} onClick={fold.onToggle} data-row-fold={fold.open ? "open" : "closed"} /> : null}
          <div className={cn("flex flex-wrap items-center gap-x-md gap-y-xs", opens && "pointer-events-none relative")}>
            <div className="flex min-w-[min(100%,var(--size-settings-control-w))] flex-1 items-start gap-sm text-subhead text-foreground">
              <span className="flex h-lh w-(--size-icon) shrink-0 items-center">
                {opens ? <ChevronRightIcon aria-hidden="true" className={cn("size-(--size-icon) text-muted-foreground transition-transform", fold.open && "rotate-90")} /> : null}
              </span>
              <div className="min-w-0 flex-1">{label}</div>
            </div>
            {children ? <div className={cn("pointer-events-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-sm", controlsEnd && "ml-auto")}>{children}</div> : null}
          </div>
          {detail ? <div className="relative mt-xs pl-[calc(var(--size-icon)+var(--spacing-sm))]">{detail}</div> : null}
        </div>
        {opens && fold.open ? <div className="pb-md pl-[calc(var(--spacing-md)+var(--size-icon)+var(--spacing-sm))] pr-md">{fold.body}</div> : null}
      </div>
    );
  }
  return (
    <div className={cn("px-md py-sm", className)} {...data}>
      {value !== undefined ? <div className="flex items-start gap-md">
        <div className="flex min-w-0 flex-1 flex-wrap items-baseline justify-between gap-x-md gap-y-xs">
          <div className="min-w-[min(100%,var(--size-settings-control-w))] flex-1 text-subhead text-foreground">{label}</div>
          <div className="min-w-0 max-w-full">{value}</div>
        </div>
        <div className="flex min-h-lh shrink-0 items-center text-subhead">{children}</div>
      </div> : <div className="flex flex-wrap items-center gap-x-md gap-y-xs">
        <div className="min-w-[min(100%,var(--size-settings-control-w))] flex-1 text-subhead text-foreground">{label}</div>
        {children ? <div className={cn("flex min-w-0 max-w-full flex-wrap items-center justify-end gap-sm", controlsEnd && "ml-auto")}>{children}</div> : null}
      </div>}
      {detail ? <div className="mt-xs">{detail}</div> : null}
    </div>
  );
}

/**
 * A folded list inside a group: one summary row that names what is inside and
 * says how much of it matters, with the rows below when opened (design 8, 9).
 * A native `details`, so the keyboard, the expanded state and the screen
 * reader announcement come with it; the rows stay hidden until it is opened.
 */
export function Disclosure({ title, summary, children, ...data }: { title: ReactNode; summary?: ReactNode; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <details className="group/fold" {...data}>
      <summary className="flex cursor-pointer list-none items-center gap-x-sm px-md py-sm text-subhead text-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground transition-transform group-open/fold:rotate-90" />
        <span className="min-w-0 flex-1">{title}</span>
        {summary ? <span className="min-w-0 text-body text-muted-foreground">{summary}</span> : null}
      </summary>
      <div className="divide-y divide-border border-t border-border">{children}</div>
    </details>
  );
}

/** A value a row reports; long paths break anywhere rather than overflow the sheet. */
export function Value({ children, mono = true, ...data }: { children: ReactNode; mono?: boolean } & Record<`data-${string}`, string>) {
  return (
    <span className={`min-w-0 break-all text-right text-body text-subtle-foreground ${mono ? "font-mono" : ""}`} {...data}>
      {children}
    </span>
  );
}

/** `live` is a connection that is up: the success color with a dot, said once beside a row (Settings › Devices). */
export type Tone = "ok" | "live" | "warn" | "error" | "pending" | "local" | "muted";

const TONE_TEXT: Record<Tone, string> = {
  ok: "text-success",
  live: "text-success",
  warn: "text-warning",
  error: "text-destructive",
  pending: "text-subtle-foreground",
  local: "text-primary",
  muted: "text-muted-foreground",
};

const TONE_SYMBOL: Record<Tone, string> = { ok: "✓", live: "●", warn: "!", error: "✕", pending: "…", local: "●", muted: "·" };

/** A state as symbol plus words, never color alone (design 7). */
export function Status({ tone, children, ...data }: { tone: Tone; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <span className={`inline-flex min-w-0 items-baseline gap-xs text-body ${TONE_TEXT[tone]}`} {...data}>
      <span aria-hidden="true" className="font-mono">
        {TONE_SYMBOL[tone]}
      </span>
      <span className="min-w-0 break-words">{children}</span>
    </span>
  );
}

/** One line a row carries about itself: a failure, a warning, or what happens next. */
export function Note({ tone = "muted", children, ...data }: { tone?: Tone; children: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <p role={tone === "error" ? "alert" : undefined} className={`break-words break-keep text-body ${TONE_TEXT[tone]}`} {...data}>
      {children}
    </p>
  );
}
