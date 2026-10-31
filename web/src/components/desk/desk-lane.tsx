import { ImagesIcon, ListIcon, Maximize2Icon, Minimize2Icon, PlusIcon, XIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState, type ReactNode, type Ref } from "react";
import { tabFit } from "../../AreaTree";
import { tabStripFit, type TabStripFit } from "../../areaLayout";
import { readTabStripSizes } from "../../tabStripSizes";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { LanePinButton } from "../lane-pin";

const RAIL_BUTTON = "relative size-(--size-tool-rail-button) rounded-md [&_svg:not([class*='size-'])]:size-(--size-icon-lg)";

/** ☰'s count of threads asking, in the corner, as the project lane's rail draws its own. */
function RailCount({ count, ...data }: { count: number } & Record<`data-${string}`, string>) {
  if (count === 0) return null;
  return (
    <span aria-hidden="true" className="absolute right-0 top-0 flex h-(--size-icon) min-w-(--size-icon) items-center justify-center rounded-full bg-warning px-xxs text-micro tabular-nums text-background" {...data}>
      {count}
    </span>
  );
}

/** ▣'s dot while an output waits unseen, as the Views chip draws its own (D-64). */
function RailDot({ on, ...data }: { on: boolean } & Record<`data-${string}`, string>) {
  if (!on) return null;
  return <span aria-hidden="true" className="absolute right-xxs top-xxs size-(--size-tab-status-dot) rounded-full bg-foreground" {...data} />;
}

/**
 * Desk's rail at the right edge, always drawn (D-64, D-73, B1, B28, B31): ☰
 * Threads with the count of threads asking, ▣ 결과물 with a dot while an
 * output is unseen. Each
 * is pressed while its column shows and opens or closes it; the pin lives in
 * the columns' heads, not here.
 */
export function DeskRail({ asking, unseen, threads, outputs, onThreads, onOutputs }: { asking: number; unseen: number; threads: boolean; outputs: boolean; onThreads: () => void; onOutputs: () => void }) {
  const { t } = useInterfaceTranslation();
  const item = (name: string, count: string | null, pressed: boolean, onPress: () => void, icon: ReactNode, badge: ReactNode, key: string) => (
    <Hint label={count ? `${name} · ${count}` : name} side="left">
      <Button variant="ghost" size="icon" aria-pressed={pressed} aria-label={name} aria-description={count ?? undefined} className={cn(RAIL_BUTTON, pressed ? "bg-secondary text-foreground" : "text-muted-foreground")} data-desk-rail={key} onClick={onPress}>
        {pressed ? <span aria-hidden="true" className="absolute -left-xxs inset-y-sm w-(--size-tab-indicator) rounded-full bg-primary" /> : null}
        {icon}
        {badge}
      </Button>
    </Hint>
  );
  return (
    <nav className="flex w-(--size-tool-rail) shrink-0 flex-col items-center gap-xxs border-l border-border bg-sidebar py-xs" aria-label={t("desk.lane.rail")} data-desk-rail-nav="true">
      {item(t("desk.threads.title"), asking ? t("desk.lane.threadsCount", { count: asking }) : null, threads, onThreads, <ListIcon aria-hidden="true" />, <RailCount count={asking} data-desk-rail-count="threads" />, "threads")}
      {item(t("desk.outputs.title"), unseen ? t("desk.lane.outputsCount", { count: unseen }) : null, outputs, onOutputs, <ImagesIcon aria-hidden="true" />, <RailDot on={unseen > 0} data-desk-rail-dot="outputs" />, "outputs")}
    </nav>
  );
}

/** Desk's pin, as the lane's pin draws it, with Desk's words. */
export function DeskPin({ pinned, onPin }: { pinned: boolean; onPin: () => void }) {
  const { t } = useInterfaceTranslation();
  return <LanePinButton pinned={pinned} label={t("desk.lane.pinLabel")} hint={pinned ? t("desk.lane.unpin") : t("desk.lane.pin")} state={pinned ? "on" : "off"} onPin={onPin} />;
}

/**
 * The controls at the right of a lane column's head (D-73, B32, B33): ⤢,
 * the pin beside ✕, and ✕. Only the column at the lane's edge carries the
 * pin and ⤢ (a thread's head has no ⤢); the column beside it closes alone.
 */
export function LaneHeadControls({ edge, expanded = false, pinned, onExpand, onPin, onClose }: { edge: boolean; expanded?: boolean; pinned: boolean; onExpand?: () => void; onPin: () => void; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <>
      {edge && onExpand ? (
        <Hint label={t("desk.lane.expand")}>
          <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("desk.lane.expand")} aria-pressed={expanded} data-desk-lane-expand={expanded ? "on" : "off"} onClick={onExpand}>{expanded ? <Minimize2Icon /> : <Maximize2Icon />}</Button>
        </Hint>
      ) : null}
      {edge ? <DeskPin pinned={pinned} onPin={onPin} /> : null}
      <Hint label={edge ? t("desk.lane.close") : t("desk.lane.closeColumn")}>
        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={edge ? t("desk.lane.close") : t("desk.lane.closeColumn")} data-desk-lane-close="true" onClick={onClose}><XIcon /></Button>
      </Hint>
    </>
  );
}

/**
 * A lane column's head (D-73, B32, B33): what the column shows, then ⤢, the
 * pin beside ✕, and ✕. Filled while pinned, the pin docks the lane beside
 * the chat; unpinned, the lane floats over it.
 */
export function LaneColumnHead({ children, expanded = false, pinned, onExpand, onPin, onClose }: { children: ReactNode; expanded?: boolean; pinned: boolean; onExpand?: () => void; onPin: () => void; onClose: () => void }) {
  return (
    <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-xxs border-b border-border pl-md pr-xs text-caption" data-lane-header="true">
      <span className="flex min-w-0 flex-1 items-center gap-xs">{children}</span>
      <LaneHeadControls edge expanded={expanded} pinned={pinned} onExpand={onExpand} onPin={onPin} onClose={onClose} />
    </div>
  );
}

/** A tab of the 보기 column: a preview (italic) is replaced by the next output pressed; a kept tab stays (B31). */
export type ViewTab = { id: string; label: string; preview: boolean; icon?: ReactNode };

/**
 * The lane's 보기 column for Desk (D-64, B31, B34): its tabs, the preview one
 * in italic, and under them the viewer the active tab holds (an image, a
 * document, a live page, the mission map: Hide's existing viewers).
 * Pressing a tab shows it; pressing a preview again keeps it.
 */
export function DeskViewColumn({ tabs, active, onTab, onKeep, onCloseTab, controls = null, children }: { tabs: readonly ViewTab[]; active: string | null; onTab: (id: string) => void; onKeep: (id: string) => void; onCloseTab: (id: string) => void; controls?: ReactNode; children: ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [zone, fit] = useTabStripFit(tabs.length, tabs.some((tab) => tab.id === active));
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col bg-card" aria-label={t("desk.outputs.title")} data-desk-view-column={tabs.length}>
      <div className="flex h-(--size-tab-strip) shrink-0 items-center border-b border-border pr-xs">
      <div ref={zone} className="flex h-full min-w-0 flex-1">
      <div role="tablist" className="flex h-full min-w-0 items-end overflow-x-auto" data-desk-view-tabs="true">
        {tabs.map((tab) => {
          const on = tab.id === active;
          // The View tab strip's rule (`tabStripFit`): the selected tab keeps its title longest.
          const slot = fit ? (on ? fit.selected : fit.others) : null;
          const shape = tabFit(on, slot?.fit ?? "titled");
          return (
            <Hint key={tab.id} label={tab.preview ? `${tab.label} · ${t("desk.lane.preview")}` : tab.label}>
              <span style={slot ? { width: slot.width } : undefined} className={cn("group/tab flex h-(--size-control-sm) min-w-0 shrink-0 items-center gap-xxs rounded-t-sm", slot ? "" : "max-w-(--size-tab-preferred)", shape.tab, on ? "bg-background text-foreground" : "text-muted-foreground hover:bg-accent")} data-desk-view-tab={tab.id} data-desk-view-tab-preview={tab.preview ? "true" : undefined}>
                <Button
                  variant="text"
                  size="inline"
                  role="tab"
                  aria-selected={on}
                  className={cn("min-w-0 flex-1 justify-start gap-xxs text-caption", on && "text-foreground", tab.preview && "italic")}
                  onClick={() => onTab(tab.id)}
                  onDoubleClick={() => onKeep(tab.id)}
                  onKeyDown={(event) => {
                    // ↵ on the preview keeps it, as a second press does (B31).
                    if (event.key === "Enter" && tab.preview && !event.nativeEvent.isComposing) {
                      event.preventDefault();
                      onKeep(tab.id);
                    }
                  }}
                >
                  {tab.icon}
                  {/* An italic title leans past its box; the end padding keeps its last letter whole. */}
                  <span className={cn("min-w-0 truncate", tab.preview && "pr-xxs", shape.title)}>{tab.label}</span>
                </Button>
                <Button variant="ghost" size="icon-xs" className={cn("shrink-0 text-muted-foreground", !on && "opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100", shape.close)} aria-label={t("desk.lane.closeTab", { name: tab.label })} onClick={() => onCloseTab(tab.id)}><XIcon /></Button>
              </span>
            </Hint>
          );
        })}
      </div>
      </div>
      {controls}
      </div>
      <div role="tabpanel" className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto bg-background">{children}</div>
    </section>
  );
}

/**
 * Desk's room (D-64, B1, B32): the chat in the middle and, at the right,
 * the lane's columns and the rail. Unpinned, the columns float over the
 * chat and the chat's words move left out from under them; pinned, they
 * dock beside it at the same width and the chat narrows by it. `view` is the 보기 column, `tool` the Threads column; either
 * may be absent.
 */
export function DeskRoom({ chat, view = null, tool = null, pinned, expanded = false, rail }: { chat: ReactNode; view?: ReactNode; tool?: ReactNode; pinned: boolean; expanded?: boolean; rail: ReactNode }) {
  const up = view !== null || tool !== null;
  const over = up && !pinned && !expanded;
  const both = view !== null && tool !== null;
  // The lane is as wide as its columns, pinned or not, and never leaves the chat less than its
  // minimum: the 보기 column gives up its width first. Over the chat, the chat moves left out from
  // under it by the same width, so nothing it says is hidden.
  const width = both ? "w-[min(calc(var(--size-lane-panel-ideal)*2),calc(100%_-_var(--size-lane-min)))]" : "w-[min(var(--size-lane-panel-ideal),calc(100%_-_var(--size-lane-min)))]";
  const covered = both ? "pr-[min(calc(var(--size-lane-panel-ideal)*2),calc(100%_-_var(--size-lane-min)))]" : "pr-[min(var(--size-lane-panel-ideal),calc(100%_-_var(--size-lane-min)))]";
  const placed = expanded ? "absolute inset-0 z-10 bg-background shadow-lg" : pinned ? cn("shrink-0", width) : cn("absolute inset-y-0 right-0 z-10 border-l border-border bg-background shadow-lg", width);
  const columns = up ? (
    <div className={cn("flex min-h-0 min-w-0 overflow-hidden", placed)} data-desk-lane={pinned ? "pinned" : "over"}>
      {view !== null ? <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-lane-column="view">{view}</div> : null}
      {tool !== null ? <aside className={cn("flex min-h-0 min-w-0 flex-col border-l border-border bg-background", both ? "w-(--size-lane-panel-ideal) shrink-0" : "flex-1")} data-desk-lane-column="tool">{tool}</aside> : null}
    </div>
  ) : null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1" data-desk-room={up ? (pinned ? "pinned" : "over") : "closed"}>
      <div className="relative flex min-h-0 min-w-0 flex-1">
        <main className={cn("flex min-h-0 min-w-(--size-lane-min) flex-1 flex-col", pinned && up && !expanded && "border-r border-border", over && covered)} data-desk-chat-column="true">{chat}</main>
        {columns}
      </div>
      {rail}
    </div>
  );
}

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

/** How many pixels each 보기 tab gets, by the View tab strip's rule, measured from the room the strip has. */
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
