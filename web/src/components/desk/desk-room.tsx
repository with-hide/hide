import { PanelRightOpenIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ReactNode, type Ref } from "react";
import { DividerGrip } from "../../LaneDivider";
import { holdShellDrag } from "../../shellDrag";
import { noteShellLayer } from "../../browserViews";
import { DESK_WIDTHS, roomWidths } from "../../deskRoom";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import type { ComposerTray } from "../../deskComposer";
import { DeskMark } from "./desk-entry";
import { FileDropZone } from "./file-drop";

/** One arrow key on a focused edge moves it this many pixels, as the lane's divider does. */
const EDGE_STEP = 32;

/**
 * The chat column's head: Desk's own mark and name, and 한눈에's unfold while it is folded. Leaving Desk is the
 * sidebar's Desk row and the recent-tab chord; a new conversation has no entry on screen until one is decided
 * (operator decision 2026-11-27).
 */
export function DeskChatHead({ onGlance = null }: { /** 한눈에 is folded with no session beside Desk: this unfolds it. */ onGlance?: (() => void) | null }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <span className="flex flex-1 items-center gap-xs font-semibold text-foreground"><DeskMark />{t("desk.name")}</span>
      {onGlance ? (
        <Hint label={t("desk.glance.unfold")}>
          <Button variant="ghost" size="icon-sm" className="text-subtle-foreground" aria-label={t("desk.glance.unfold")} data-desk-glance-unfold="true" onClick={onGlance}><PanelRightOpenIcon /></Button>
        </Hint>
      ) : null}
    </>
  );
}

/** Desk's chat column: its head, the conversation, and the composer at the foot, all one place to drop files into the composer. `scrollRef` is the conversation's scroller. */
export function DeskChatColumn({ head, children, composer, tray = null, dragging = false, scrollRef }: { head: ReactNode; children: ReactNode; composer: ReactNode; /** The box's files: a file dropped anywhere on the column attaches there (Doop L). */ tray?: ComposerTray | null; /** A drag of files in progress, for review. */ dragging?: boolean; scrollRef?: Ref<HTMLDivElement> }) {
  return (
    <FileDropZone tray={tray} dragging={dragging} className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-chat-drop="true">
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border px-md text-caption" data-desk-chat-head="true">{head}</div>
      <div ref={scrollRef} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto" data-desk-chat-scroll="true">{children}</div>
      <div className="mx-auto w-full max-w-(--size-editor-measure) shrink-0 px-lg pb-md pt-xs">{composer}</div>
    </FileDropZone>
  );
}

/**
 * One of the room's two drag edges (PRD desk-2-ui D-06, B22): the tab
 * panel's left edge sets the view width, the edge between Desk's desk and
 * the session panel sets the session width. Both widen leftwards. Nothing at
 * rest; on hover, keyboard focus and while dragging the lane divider's grip.
 * A drag moves a guide and lands once on release (`onLand`, one event); a
 * focused edge moves the guide one step per arrow key and lands once on key
 * up, never per key repeat, since each landing resends Desk's section.
 */
function DeskEdge({ label, kind, right, width, min, max, onLand, onGuide }: { label: string; kind: "view" | "session"; right: number; width: number; min: number; max: number; onLand: (width: number) => void; onGuide: (right: number | null) => void }) {
  const clamp = (value: number) => Math.round(Math.max(min, Math.min(max, value)));
  // The width the arrow keys moved to while held, landed on key up.
  const stepped = useRef<number | null>(null);
  const drag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const start = event.clientX;
    const at = (next: PointerEvent) => clamp(width + start - next.clientX);
    const move = (next: PointerEvent) => onGuide(right + at(next));
    const release = holdShellDrag("col-resize");
    const end = () => {
      release();
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", end);
      target.removeEventListener("lostpointercapture", end);
      if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
      onGuide(null);
    };
    const up = (next: PointerEvent) => {
      end();
      const landed = at(next);
      if (landed !== width) onLand(landed);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", end);
    target.addEventListener("lostpointercapture", end);
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      data-desk-edge={kind}
      className="group absolute inset-y-0 z-30 flex w-sm cursor-col-resize justify-center outline-none"
      style={{ right: right + width - EDGE_HALF }}
      onPointerDown={drag}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        const next = clamp((stepped.current ?? width) + (event.key === "ArrowLeft" ? EDGE_STEP : -EDGE_STEP));
        stepped.current = next;
        onGuide(right + next);
      }}
      onKeyUp={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        const next = stepped.current;
        stepped.current = null;
        onGuide(null);
        if (next !== null && next !== width) onLand(next);
      }}
      onBlur={() => {
        if (stepped.current === null) return;
        stepped.current = null;
        onGuide(null);
      }}
    >
      <DividerGrip className="opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
    </div>
  );
}

/** Half an edge's width, so the edge straddles the line it moves. */
const EDGE_HALF = 4;

/** The room's width, measured as the window draws it. */
function useRoomWidth() {
  const ref = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ width: 0, reach: 0 });
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => {
      const rect = node.getBoundingClientRect();
      const next = { width: Math.floor(rect.width), reach: Math.max(0, Math.floor(rect.left)) };
      setBox((was) => (was.width === next.width && was.reach === next.reach ? was : next));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    // The sidebar folding moves the room's left edge as well as its width.
    if (node.parentElement) observer.observe(node.parentElement);
    return () => observer.disconnect();
  }, []);
  return [ref, box.width, box.reach] as const;
}

/**
 * Desk's room (PRD desk-2-ui D-01, D-05, D-06, D-12, B1, B2, B17, B22-B24;
 * PRD desk-own-desk D-05, B16): Desk's chat in the middle, the session open
 * beside it, or 한눈에 while none is, docked at the right (the chat narrows by its width, keeping its
 * own floor, under which the session floats over it), and Desk's own desk
 * floating at the session's left edge, or at the window's right edge with no
 * session, with its left corners rounded and its shadow; while it shows, the
 * chat and the sidebar dim and a press there hides it. The two edges drag
 * the desk's and the session's widths.
 */
export function DeskRoom({ chat, session = null, glance = false, panel = null, expanded = false, widths, onDim, onWidth }: {
  chat: ReactNode;
  /** What stands at the right: the session beside Desk, or 한눈에 while none is open (`glance`), at the session's width. */
  session?: ReactNode;
  glance?: boolean;
  panel?: ReactNode;
  /** The desk covers Desk's chat whole (`⤢`, PRD desk-own-desk B40). */
  expanded?: boolean;
  /** The widths the core keeps; null until the operator drags one. */
  widths: { view_width: number | null; session_width: number | null };
  onDim: () => void;
  onWidth: (panel: "view_width" | "session_width", width: number) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [room, available, reach] = useRoomWidth();
  const [guide, setGuide] = useState<number | null>(null);
  const placed = roomWidths(available, reach, widths, session !== null);
  // The desk moves without changing size when the session beside it opens,
  // closes or changes width, which no resize of a page's slot reports: each
  // render of the room places the desk's pages again, at most once a frame,
  // and the host is sent nothing when a page's rectangle did not move.
  useLayoutEffect(() => noteShellLayer());
  const state = panel !== null ? "panel" : session !== null ? (glance ? "glance" : "session") : "closed";
  return (
    <div ref={room} className="relative flex min-h-0 min-w-0 flex-1" data-desk-room={state}>
      <main className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-chat-column="true">{chat}</main>
      {session !== null ? (
        <aside
          aria-label={glance ? t("desk.glance.title") : t("desk.room.session")}
          className={cn("z-20 flex min-h-0 flex-col border-l border-border bg-background", placed.sessionFloats ? "absolute inset-y-0 right-0 shadow-overlay" : "relative shrink-0")}
          style={{ width: placed.session }}
          data-desk-session-panel={placed.sessionFloats ? "over" : "docked"}
        >
          {session}
        </aside>
      ) : null}
      {panel !== null ? (
        <>
          {/* The dim covers Desk's chat and the sidebar; the session panel stands above it, bright and usable (B17). */}
          <div aria-hidden="true" className="fixed inset-0 z-10 bg-scrim" data-desk-dim="true" onPointerDown={(event) => { event.preventDefault(); onDim(); }} />
          {/* Flush with the room's top and foot, so its tab strip lines up with Desk's head and the session's `‹ 한눈에` line. */}
          <section
            aria-label={t("desk.room.tabs")}
            className="absolute inset-y-0 z-20 flex min-h-0 min-w-0 overflow-hidden border-l border-border bg-background shadow-overlay"
            style={placed.panelCovers || expanded ? { left: 0, right: placed.session } : { right: placed.session, width: placed.view }}
            data-desk-tab-panel={session !== null ? "beside_session" : "window_edge"}
            data-desk-expanded={expanded ? "true" : undefined}
          >
            {panel}
          </section>
          {placed.panelCovers || expanded ? null : <DeskEdge kind="view" label={t("desk.room.viewWidth")} right={placed.session} width={placed.view} min={DESK_WIDTHS.viewMin} max={Math.max(DESK_WIDTHS.viewMin, placed.viewMax)} onGuide={setGuide} onLand={(width) => onWidth("view_width", width)} />}
        </>
      ) : null}
      {session !== null ? <DeskEdge kind="session" label={t("desk.room.sessionWidth")} right={0} width={placed.session} min={DESK_WIDTHS.sessionMin} max={Math.max(DESK_WIDTHS.sessionMin, available - (panel !== null ? DESK_WIDTHS.viewMin : DESK_WIDTHS.chatMin))} onGuide={setGuide} onLand={(width) => onWidth("session_width", width)} /> : null}
      {guide !== null ? (
        <div className="pointer-events-none absolute inset-y-0 z-40 flex w-sm justify-center" style={{ right: guide - EDGE_HALF }} data-desk-guide="true">
          <DividerGrip />
        </div>
      ) : null}
    </div>
  );
}
