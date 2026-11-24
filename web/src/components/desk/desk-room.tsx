import { PlusIcon, XIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ReactNode, type Ref } from "react";
import { tabFit } from "../../AreaTree";
import { tabStripFit, type TabStripFit } from "../../areaLayout";
import { DividerGrip } from "../../LaneDivider";
import { holdShellDrag } from "../../shellDrag";
import { readTabStripSizes } from "../../tabStripSizes";
import { DESK_WIDTHS, roomWidths } from "../../deskRoom";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";

/** One arrow key on a focused edge moves it this many pixels, as the lane's divider does. */
const EDGE_STEP = 32;

/** The chat column's head: its name and `＋ 새 대화` (B10); ◆ stays in the sidebar (D-38). */
export function DeskChatHead({ onNewConversation }: { onNewConversation: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      <span className="flex-1 font-semibold text-foreground">{t("desk.name")}</span>
      <Button variant="ghost" size="sm" className="h-(--size-control-compact) gap-xxs px-xs text-caption font-normal text-subtle-foreground" data-desk-new-conversation="true" onClick={onNewConversation}><PlusIcon />{t("desk.newConversation")}</Button>
    </>
  );
}

/** Desk's chat column: its head (the name and `＋ 새 대화`), the conversation, and the composer at the foot. `scrollRef` is the conversation's scroller. */
export function DeskChatColumn({ head, children, composer, scrollRef }: { head: ReactNode; children: ReactNode; composer: ReactNode; scrollRef?: Ref<HTMLDivElement> }) {
  return (
    <>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xs border-b border-border px-md text-caption" data-desk-chat-head="true">{head}</div>
      <div ref={scrollRef} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto" data-desk-chat-scroll="true">{children}</div>
      <div className="mx-auto w-full max-w-(--size-editor-measure) shrink-0 px-lg pb-md pt-xs">{composer}</div>
    </>
  );
}

/**
 * One of the room's two drag edges (PRD desk-2-ui D-06, B22): the tab
 * panel's left edge sets the view width, the edge between the tab panel and
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
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => setWidth(Math.floor(node.getBoundingClientRect().width));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

/**
 * Desk's room (PRD desk-2-ui D-01, D-05, D-06, D-12, B1, B2, B17, B22-B24):
 * Desk's chat in the middle, the session open beside it docked at the right
 * (the chat narrows by its width, keeping its own floor, under which the
 * session floats over it), and the tab panel floating at the session's left
 * edge, or at the window's right edge with no session, with its left corners
 * rounded and its shadow; while it shows, the chat and the sidebar dim and
 * a press there closes it. The two edges drag the view and session widths.
 */
export function DeskRoom({ chat, session = null, panel = null, widths, onDim, onWidth }: {
  chat: ReactNode;
  session?: ReactNode;
  panel?: ReactNode;
  /** The widths the core keeps; null until the operator drags one. */
  widths: { view_width: number | null; session_width: number | null };
  onDim: () => void;
  onWidth: (panel: "view_width" | "session_width", width: number) => void;
}) {
  const { t } = useInterfaceTranslation();
  const [room, available] = useRoomWidth();
  const [guide, setGuide] = useState<number | null>(null);
  const placed = roomWidths(available, widths, session !== null);
  const state = panel !== null ? "panel" : session !== null ? "session" : "closed";
  return (
    <div ref={room} className="relative flex min-h-0 min-w-0 flex-1" data-desk-room={state}>
      <main className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-chat-column="true">{chat}</main>
      {session !== null ? (
        <aside
          aria-label={t("desk.room.session")}
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
          <section
            aria-label={t("desk.room.tabs")}
            className="absolute inset-y-xs z-20 flex min-h-0 min-w-0 overflow-hidden rounded-l-lg border border-r-0 border-border bg-background shadow-overlay"
            style={placed.panelCovers ? { left: 0, right: placed.session } : { right: placed.session, width: placed.view }}
            data-desk-tab-panel={session !== null ? "beside_session" : "window_edge"}
          >
            {panel}
          </section>
          {placed.panelCovers ? null : <DeskEdge kind="view" label={t("desk.room.viewWidth")} right={placed.session} width={placed.view} min={DESK_WIDTHS.viewMin} max={Math.max(DESK_WIDTHS.viewMin, available - placed.session)} onGuide={setGuide} onLand={(width) => onWidth("view_width", width)} />}
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

/** A tab of the tab panel: its name, its dim project when another tab has the same file name (D-25), italic while it is the preview (B36). */
export type PanelTab = { id: string; label: string; project: string | null; preview: boolean; icon?: ReactNode };

/**
 * The tab panel's tab strip and body (PRD desk-2-ui D-08, B18, B21, B36,
 * B40): its tabs drawn as focus mode's View tab strip draws its own
 * (`tabStripFit`, `tabFit`), the front one alone with `✕`, a preview tab in
 * italic that a double press keeps, and `✕` for the whole panel at the
 * strip's end. `tool` is the Explorer's column at the panel's left (B35).
 */
export function DeskTabPanel({ tabs, active, tool = null, onTab, onKeep, onCloseTab, onClose, children }: { tabs: readonly PanelTab[]; active: string | null; tool?: ReactNode; onTab: (id: string) => void; onKeep: (id: string) => void; onCloseTab: (id: string) => void; onClose: () => void; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [zone, fit] = useTabStripFit(tabs.length, tabs.some((tab) => tab.id === active));
  return (
    <>
      {tool !== null ? <div className="flex w-(--size-tree-ideal) min-w-0 shrink-0 flex-col border-r border-border bg-sidebar" data-desk-tool-column="explorer">{tool}</div> : null}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="flex h-(--size-tab-strip) shrink-0 items-center border-b border-border pr-xs">
          <div ref={zone} className="flex h-full min-w-0 flex-1 pl-xs">
            <div role="tablist" className="flex h-full min-w-0 items-end overflow-x-auto" data-desk-tabs={tabs.length}>
              {tabs.map((tab) => {
                const on = tab.id === active;
                // The View tab strip's rule (`tabStripFit`): the selected tab keeps its title longest.
                const slot = fit ? (on ? fit.selected : fit.others) : null;
                const shape = tabFit(on, slot?.fit ?? "titled");
                const name = tab.project ? `${tab.label} · ${tab.project}` : tab.label;
                return (
                  <Hint key={tab.id} label={tab.preview ? `${name} · ${t("desk.lane.preview")}` : name}>
                    <span style={slot ? { width: slot.width } : undefined} className={cn("group/tab flex h-(--size-control-sm) min-w-0 shrink-0 items-center gap-xxs rounded-t-sm", slot ? "" : "max-w-(--size-tab-preferred)", shape.tab, on ? "bg-secondary text-foreground" : "text-muted-foreground hover:bg-accent")} data-desk-tab={tab.id} data-desk-tab-preview={tab.preview ? "true" : undefined} data-desk-tab-active={on ? "true" : undefined}>
                      <Button
                        variant="text"
                        size="inline"
                        role="tab"
                        aria-selected={on}
                        className={cn("min-w-0 flex-1 justify-start gap-xxs text-caption", on && "text-foreground", tab.preview && "italic")}
                        onClick={() => onTab(tab.id)}
                        onDoubleClick={() => onKeep(tab.id)}
                      >
                        {tab.icon}
                        {/* An italic title leans past its box; the end padding keeps its last letter whole. */}
                        <span className={cn("min-w-0 truncate", tab.preview && "pr-xxs", shape.title)}>{tab.label}</span>
                        {tab.project ? <span className="shrink-0 truncate text-muted-foreground" data-desk-tab-project="true">· {tab.project}</span> : null}
                      </Button>
                      {/* Only the tab in front carries its `✕` (B18). */}
                      {on ? <Button variant="ghost" size="icon-xs" className={cn("shrink-0 text-muted-foreground", shape.close)} aria-label={t("desk.lane.closeTab", { name: tab.label })} onClick={() => onCloseTab(tab.id)}><XIcon /></Button> : null}
                    </span>
                  </Hint>
                );
              })}
            </div>
          </div>
          <Hint label={t("desk.room.closePanel")}>
            <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.room.closePanel")} data-desk-panel-close="true" onClick={onClose}><XIcon /></Button>
          </Hint>
        </div>
        <div role="tabpanel" className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto bg-background">{children}</div>
      </div>
    </>
  );
}

/** How many pixels each tab gets, by the View tab strip's rule, measured from the room the strip has. */
function useTabStripFit(count: number, hasSelected: boolean) {
  const zone = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState<TabStripFit | null>(null);
  useLayoutEffect(() => {
    const node = zone.current;
    if (!node) return;
    const sizes = readTabStripSizes();
    const measure = () => {
      const next = tabStripFit(Math.floor(node.getBoundingClientRect().width), count, hasSelected, sizes);
      setFit((drawn) => (drawn && drawn.selected.width === next.selected.width && drawn.others.width === next.others.width && drawn.selected.fit === next.selected.fit && drawn.others.fit === next.others.fit ? drawn : next));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [count, hasSelected]);
  return [zone, fit] as const;
}
