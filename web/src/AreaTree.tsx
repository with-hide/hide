import { ChevronDownIcon, EllipsisIcon, PlusIcon } from "lucide-react";
import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { EntryContextMenu, EntryDropdown, type MenuEntry } from "./components/entry-menu";
import { Button } from "./components/ui/button";
import { Hint } from "./components/ui/tooltip";
import { holdShellDrag } from "./shellDrag";
import { useUiStore } from "./ui";
import { useInterfaceTranslation } from "./i18n/client";
import { IDLE, movePointer, pressTab, relayout, releasePointer, type DragSession } from "./areaDrag";
import { focusFromKeyboard, installFocusModality } from "./areaFocus";
import { RATIO_MAX, RATIO_MIN, RESIZE_STEP, areasOf, dropTarget, findArea, locateDisplay, ratioAtOffset, revealedScroll, sameTarget, singleAreaGeometry, steppedRatio, tabStripFit, areaGeometry,
  type Area, type AreaItem, type AreaLayout, type AreaNode, type AreaSplit, type AreaWords,
  type DividerBox, type DropTarget, type Geometry, type LayoutSizes, type Point, type Rect, type TabFit, type TabSlot, type TabStripSizes } from "./areaLayout";

/**
 * `fit` says how the tab draws its contents in its slot (`tabStripFit`): a
 * tab shrinks from titled through compact to marks as its bar fills, with
 * the selected title kept longest in both Agent and View areas.
 */
export type AreaTabInteraction = { selected: boolean; fit: TabFit; areaActive: boolean; dragging: boolean; press: (event: React.PointerEvent<HTMLElement>) => void; select: () => void };

/**
 * The classes that fit a tab's contents to its slot (`Component / Adaptive
 * Work Tab`): titled, as the bar always drew them; compact, with narrow
 * padding and a truncated title, the close control kept only by the
 * selected tab; as marks, centred with nothing else, or on the selected tab
 * with its close control at the end.
 */
export function tabFit(selected: boolean, fit: TabFit): { tab: string; title: string; close: string } {
  switch (fit) {
    case "titled":
      return { tab: "px-sm", title: "", close: "" };
    case "compact":
      return { tab: "px-xs", title: "", close: selected ? "" : "hidden" };
    case "marks":
      return selected
        ? { tab: "pl-xs", title: "hidden", close: "ml-auto" }
        : { tab: "justify-center px-xs", title: "hidden", close: "hidden" };
  }
}

/**
 * An area's tab slot, at the width `tabStripFit` gave the selected tab or
 * the others, which the bar writes on its tab list. A tab being renamed keeps
 * the preferred width so its field stays usable.
 */
const TAB_SLOT = {
  selected: "flex shrink-0 w-(--tab-selected-width) has-data-renaming:w-(--size-tab-preferred)",
  others: "flex shrink-0 w-(--tab-other-width) has-data-renaming:w-(--size-tab-preferred)",
};

/** The strip's sizes; a token that does not resolve is a broken build, not a strip of empty tabs. */
function readTabStripSizes(): TabStripSizes {
  const read = (name: string) => {
    const value = tokenPx(name);
    if (!(value > 0)) throw new Error(`design token ${name} did not resolve to a pixel size`);
    return value;
  };
  return {
    preferred: read("--size-tab-preferred"),
    titleMin: read("--size-tab-title-min"),
    icon: read("--size-tab-icon-identity"),
    control: read("--size-control-sm"),
  };
}

export type DrawnArea<I extends AreaItem> = { layout: AreaLayout<I>; geometry: Geometry; sizes: LayoutSizes };
export type AreaAdapter<I extends AreaItem> = {
  words: AreaWords;
  /** The page's keyboard owner, independent of each layout's committed active area. */
  keyboardArea?: string | null;
  barAttributes?: Record<string, string>;
  barStart?: (area: Area<I>) => React.ReactNode;
  barEnd?: (area: Area<I>) => React.ReactNode;
  shown?: (area: Area<I>) => I | null;
  splitUnavailable?: string;
  label: (item: I) => string;
  sameContent: (a: I, b: I) => boolean;
  tab: (item: I, interaction: AreaTabInteraction) => React.ReactNode;
  body: (item: I, area: Area<I>) => React.ReactNode;
  empty: (area: Area<I>) => React.ReactNode;
  floating: (item: I) => React.ReactNode;
  menu: (id: string, geometry: Geometry, sizes: LayoutSizes) => MenuEntry<string>[];
  onMenuCloseAutoFocus?: (event: Event) => void;
  runMenu: (command: string, id: string) => void;
  focus: (id: string) => void;
  focusArea: (id: string) => void;
  move: (id: string, areaId: string, index: number) => void;
  split: (id: string, areaId: string, edge: import("./areaLayout").Edge) => void;
  resize: (id: string, ratio: number) => void;
  newTab: (areaId: string) => void;
  newTabLabel: string;
  tabListLabel: string;
  actionsLabel: string;
  newTabShortcut?: string;
  onDraw?: (frame: DrawnArea<I> | null) => void;
  onBody?: (body: HTMLDivElement | null) => void;
};

function tokenPx(name: string): number {
  return Number.parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
}

function readLayoutSizes(): LayoutSizes {
  return {
    areaMinWidth: tokenPx("--size-workspace-area-min"),
    areaMinHeight: tokenPx("--size-view-area-min-height"),
    divider: tokenPx("--size-resize-handle"),
    tabStrip: tokenPx("--size-tab-strip"),
  };
}

/** One typed factory per column, sharing the entire area renderer and gesture owner. */
export function createAreaTree<I extends AreaItem>(column: "view" | "agent") {
  const data = (name: string, value: string) => ({ [`data-${column}-${name}`]: value });
  type Tree = DrawnArea<I> & {
    adapter: AreaAdapter<I>;
    draggingId: string | null;
    press: (id: string, event: React.PointerEvent<HTMLElement>) => void;
    takeClick: () => boolean;
    focus: (id: string) => void;
    startResize: (box: DividerBox, event: React.PointerEvent<HTMLElement>) => void;
    menu: (id: string) => MenuEntry<string>[];
  };
  const TreeContext = createContext<Tree | null>(null);
  function useTree(): Tree {
    const tree = useContext(TreeContext);
    if (!tree) throw new Error("an area rendered outside its tree");
    return tree;
  }
function AreaTree({ layout, adapter, children }: { layout: AreaLayout<I>; adapter: AreaAdapter<I>; children?: React.ReactNode }) {
  const { t } = useInterfaceTranslation();
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    if (!body) return undefined;
    const measure = () =>
      setSize((current) => (current.width === body.clientWidth && current.height === body.clientHeight ? current : { width: body.clientWidth, height: body.clientHeight }));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(body);
    return () => observer.disconnect();
  }, [body]);
  const sizes = useMemo(readLayoutSizes, []);
  const measured = size.width > 0 && size.height > 0;
  const rect = useMemo<Rect>(() => ({ x: 0, y: 0, width: size.width, height: size.height }), [size.width, size.height]);
  const whole = useMemo(() => areaGeometry(layout.root, rect, sizes), [layout.root, rect, sizes]);
  const areas = areasOf(layout.root);
  const shownArea = findArea(layout.root, layout.active_area) ?? areas[0] ?? null;
  // A window too small to give every area its minimum shows only the active
  // one, with a switcher to the others; nothing is sent or stored for it, so
  // widening shows the stored tree again (B13, A7).
  const single = measured && !whole.fits && shownArea !== null;
  const geometry = useMemo(() => (single && shownArea ? singleAreaGeometry(shownArea, rect, sizes) : whole), [single, shownArea, rect, sizes, whole]);

  useLayoutEffect(() => {
    adapter.onBody?.(body);
    return () => adapter.onBody?.(null);
  }, [body, adapter.onBody]);
  useLayoutEffect(() => {
    if (!measured) return;
    adapter.onDraw?.({ layout, geometry, sizes });
    return () => adapter.onDraw?.(null);
  }, [measured, layout, geometry, sizes, adapter.onDraw]);
  useEffect(() => installFocusModality(), []);
  // One focus per choice: a second press before the core answered asks again for nothing.
  const claimed = useRef<{ displayId: string; layout: AreaLayout<I> } | null>(null);
  const focus = (displayId: string) => {
    const located = locateDisplay(layout.root, displayId);
    if (!located) return;
    const shownId = adapter.shown ? adapter.shown(located.area)?.id : located.area.active;
    if (layout.active_area === located.area.id && shownId === displayId) return;
    if (claimed.current?.displayId === displayId && claimed.current.layout === layout) return;
    claimed.current = { displayId, layout };
    adapter.focus(displayId);
  };

  // The tab drag (B6-B8). The session lives in a ref, since pointer events
  // outrun renders; the tree redraws only when the previewed target changes,
  // and the floating tab follows the pointer on its own.
  const [session, setSession] = useState<DragSession>(IDLE);
  const sessionRef = useRef<DragSession>(IDLE);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const geometryRef = useRef(geometry);
  geometryRef.current = geometry;
  const swallow = useRef(false);
  const show = (next: DragSession) => {
    sessionRef.current = next;
    setSession(next);
  };
  const swallowClick = () => {
    swallow.current = true;
    window.setTimeout(() => {
      swallow.current = false;
    }, 0);
  };
  const resolve = (displayId: string) => (client: Point): DropTarget => {
    if (!body || !body.contains(document.elementFromPoint(client.x, client.y))) return { kind: "none", reason: t(`panes.area.dropColumn.${column}` as const) };
    const origin = body.getBoundingClientRect();
    const target = dropTarget({
      layout: layoutRef.current,
      geometry: geometryRef.current,
      sizes,
      tabs: measureTabs(body, origin),
      displayId,
      sameContent: adapter.sameContent,
      words: adapter.words,
      point: { x: client.x - origin.left, y: client.y - origin.top },
    });
    return target.kind === "edge" && adapter.splitUnavailable ? { kind: "none", reason: adapter.splitUnavailable } : target;
  };

  const live = session.phase !== "idle";
  useEffect(() => {
    if (!live) return undefined;
    const threshold = tokenPx("--size-tab-drag-activation");
    const move = (event: PointerEvent) => {
      const current = sessionRef.current;
      if (current.phase === "idle") return;
      const next = movePointer(current, event.pointerId, { x: event.clientX, y: event.clientY }, threshold, resolve(current.displayId));
      if (next === current) return;
      sessionRef.current = next;
      if (current.phase !== next.phase || (current.phase === "dragging" && next.phase === "dragging" && !samePreview(current.target, next.target))) setSession(next);
    };
    const up = (event: PointerEvent) => {
      const current = sessionRef.current;
      if (current.phase === "idle" || current.pointerId !== event.pointerId) return;
      const result = releasePointer(current, event.pointerId, { x: event.clientX, y: event.clientY }, resolve(current.displayId));
      show(result.session);
      if (result.dragged) swallowClick();
      if (!result.drop) return;
      if (result.drop.kind === "bar") adapter.move(current.displayId, result.drop.areaId, result.drop.index);
      else adapter.split(current.displayId, result.drop.areaId, result.drop.edge);
    };
    // A drag the system takes away, or the window losing focus, lands nothing.
    const cancel = () => show(IDLE);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", cancel);
    };
    // The listeners read the session, the layout and the geometry through refs.
  }, [live]);

  // A layout that changes under a drag re-resolves its preview in place
  // (B8). A layout effect, so the tab bars are measured as now drawn; the
  // pointer's own moves re-resolve in the listeners.
  useLayoutEffect(() => {
    const current = sessionRef.current;
    if (current.phase !== "dragging") return;
    const next = relayout(current, resolve(current.displayId));
    if (next !== current) show(next);
  }, [layout, geometry]);

  // Escape ends a drag with nothing sent, and the release that follows selects nothing.
  const dragging = session.phase === "dragging";
  useEffect(() => {
    if (!dragging) return undefined;
    return useUiStore.getState().pushEscape(() => {
      show(IDLE);
      swallow.current = true;
      window.addEventListener("pointerup", swallowClick, { once: true, capture: true });
    });
  }, [dragging]);

  // The pointer says whether the place under it would land (B8).
  const cursor = session.phase === "dragging" ? (session.target.kind === "none" && session.target.reason !== null ? "forbidden" : "move") : null;
  useEffect(() => (cursor ? holdShellDrag(cursor, `data-${column}-drag`) : undefined), [cursor]);

  // A divider drag moves a guide line and lands one resize on release (B9);
  // the line is its own component, so a pointer move redraws nothing else.
  const guide = useRef<((rect: Rect | null) => void) | null>(null);
  const startResize = (box: DividerBox, event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 0 || !body) return;
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const origin = body.getBoundingClientRect();
    const row = box.axis === "row";
    const ratioAt = (next: PointerEvent) => ratioAtOffset(box, row ? next.clientX - origin.left - box.span.x : next.clientY - origin.top - box.span.y);
    const move = (next: PointerEvent) => guide.current?.(guideAt(box, ratioAt(next)));
    // The drag marks the root, so a page the guide crosses gives way to its still.
    const release = holdShellDrag(row ? "col-resize" : "row-resize", `data-${column}-drag`);
    const end = () => {
      release();
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", end);
      target.removeEventListener("lostpointercapture", end);
      guide.current?.(null);
    };
    const up = (next: PointerEvent) => {
      end();
      const ratio = ratioAt(next);
      if (Math.abs(ratio - box.ratio) > 0.001) adapter.resize(box.id, ratio);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", end);
    target.addEventListener("lostpointercapture", end);
  };

  const tree: Tree = {
    layout,
    geometry,
    sizes,
    adapter,
    draggingId: session.phase === "dragging" ? session.displayId : null,
    press: (displayId, event) => {
      if (event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      show(pressTab(displayId, event.pointerId, { x: event.clientX, y: event.clientY }));
    },
    takeClick: () => {
      const taken = swallow.current;
      swallow.current = false;
      return taken;
    },
    focus,
    startResize,
    menu: (displayId) => adapter.menu(displayId, geometry, sizes),
  };

  return (
    <TreeContext.Provider value={tree}>
      <div ref={setBody} className="relative flex min-h-0 min-w-0 flex-1" {...data("areas", single ? "single" : "tree")}>
        {children}
        {!measured || !shownArea ? null : single ? (
          <AreaView area={shownArea} index={areas.indexOf(shownArea)} count={areas.length} switcher />
        ) : (
          <NodeView node={layout.root} />
        )}
        <ResizeGuide control={guide} />
        {session.phase === "dragging" ? <DragPreview session={session} /> : null}
      </div>
    </TreeContext.Provider>
  );
}

/** Two previews show the same thing, so the tree need not redraw. */
function samePreview(a: DropTarget, b: DropTarget): boolean {
  if (a.kind === "none" && b.kind === "none") return a.reason === b.reason;
  return sameTarget(a, b);
}

/** Each area's tab rectangles, in the tree's own coordinates. */
function measureTabs(body: HTMLElement, origin: DOMRect): Record<string, TabSlot[]> {
  const slots: Record<string, TabSlot[]> = {};
  for (const bar of body.querySelectorAll<HTMLElement>("[data-area-tab-bar]")) {
    const areaId = bar.dataset.areaTabBar;
    if (!areaId) continue;
    slots[areaId] = [...bar.querySelectorAll<HTMLElement>('[data-area-item]')].map((tab) => {
      const box = tab.getBoundingClientRect();
      return { displayId: tab.dataset.areaItem ?? "", rect: { x: box.left - origin.left, y: box.top - origin.top, width: box.width, height: box.height } };
    });
  }
  return slots;
}

/** The guide line of a divider drag: the only thing a pointer move redraws. */
function ResizeGuide({ control }: { control: React.MutableRefObject<((rect: Rect | null) => void) | null> }) {
  const [rect, setRect] = useState<Rect | null>(null);
  useLayoutEffect(() => {
    control.current = setRect;
    return () => {
      control.current = null;
    };
  }, [control]);
  if (!rect) return null;
  return <div className="pointer-events-none absolute z-20 bg-primary" style={{ left: rect.x, top: rect.y, width: rect.width, height: rect.height }} {...data("resize-guide", "true")} />;
}

/** Where a divider dragged to `ratio` would sit. */
function guideAt(box: DividerBox, ratio: number): Rect {
  const row = box.axis === "row";
  const thickness = row ? box.rect.width : box.rect.height;
  const first = Math.round(ratio * ((row ? box.span.width : box.span.height) - thickness));
  return row
    ? { x: box.span.x + first, y: box.span.y, width: thickness, height: box.span.height }
    : { x: box.span.x, y: box.span.y + first, width: box.span.width, height: thickness };
}

function NodeView({ node }: { node: AreaNode<I> }) {
  const tree = useTree();
  if ("area" in node) {
    const areas = areasOf(tree.layout.root);
    return <AreaView area={node.area} index={areas.findIndex((area) => area.id === node.area.id)} count={areas.length} switcher={false} />;
  }
  const { split } = node;
  const box = tree.geometry.dividers.find((divider) => divider.id === split.id);
  const row = split.axis === "row";
  return (
    <div className={`flex min-h-0 min-w-0 flex-1 ${row ? "flex-row" : "flex-col"}`} {...data("split", split.id)}>
      <div className="flex min-h-0 min-w-0 shrink-0" style={box ? (row ? { width: box.first } : { height: box.first }) : undefined}>
        <NodeView node={split.first} />
      </div>
      {box ? <Separator split={split} box={box} /> : null}
      <div className="flex min-h-0 min-w-0 flex-1">
        <NodeView node={split.second} />
      </div>
    </div>
  );
}

/** A divider: dragged with a guide line, or moved a step at a time from the keyboard (B9, B20). */
function Separator({ split, box }: { split: AreaSplit<I>; box: DividerBox }) {
  const tree = useTree();
  const { t } = useInterfaceTranslation();
  const row = split.axis === "row";
  return (
    <div
      role="separator"
      aria-orientation={row ? "vertical" : "horizontal"}
      aria-label={t(`panes.area.resize.${column}.${row ? "side" : "stack"}` as const)}
      aria-valuenow={Math.round(split.ratio * 100)}
      aria-valuemin={Math.round(RATIO_MIN * 100)}
      aria-valuemax={Math.round(RATIO_MAX * 100)}
      tabIndex={0}
      {...data("divider", split.id)}
      className={`relative z-10 shrink-0 bg-border outline-none hover:bg-primary focus-visible:bg-primary ${row ? "w-[var(--size-resize-handle)] cursor-col-resize" : "h-[var(--size-resize-handle)] cursor-row-resize"}`}
      onPointerDown={(event) => tree.startResize(box, event)}
      onKeyDown={(event) => {
        const back = row ? "ArrowLeft" : "ArrowUp";
        const forward = row ? "ArrowRight" : "ArrowDown";
        if (event.key !== back && event.key !== forward) return;
        event.preventDefault();
        const next = steppedRatio(box, event.key === back ? -RESIZE_STEP : RESIZE_STEP);
        if (next !== null) tree.adapter.resize(split.id, next);
      }}
    />
  );
}

function AreaView({ area, index, count, switcher }: { area: Area<I>; index: number; count: number; switcher: boolean }) {
  const tree = useTree();
  const { t } = useInterfaceTranslation();
  const display = tree.adapter.shown ? tree.adapter.shown(area) : area.displays.find((row) => row.id === area.active) ?? null;
  const active = tree.layout.active_area === area.id;
  const keyboard = tree.adapter.keyboardArea === area.id;
  // The operator's pointer, or Tab, into a display makes it the one they
  // work in; a focus the page moved itself asks for nothing (B20).
  const claim = () => {
    if (display && locateDisplay(tree.layout.root, display.id)) tree.focus(display.id);
    else if (!active) tree.adapter.focusArea(area.id);
  };
  return (
    <section
      className="flex min-h-0 min-w-0 flex-1 flex-col"
      aria-label={t(`panes.area.nameOf.${column}` as const, { index: index + 1, total: count })}
      {...data("area-id", area.id)}
      data-active-area={active ? "true" : "false"}
      data-keyboard-area={keyboard ? "true" : "false"}
    >
      <AreaTabBar area={area} active={keyboard} index={index} count={count} switcher={switcher} />
      <div
        className={`relative flex min-h-0 min-w-0 flex-1 flex-col ${column === "view" ? `border-2 ${keyboard ? "border-primary" : "border-transparent"}` : "border border-transparent"}`}
        {...data("body", area.id)}
        onPointerDown={claim}
        onFocus={() => {
          if (focusFromKeyboard()) claim();
        }}
      >
        {display ? tree.adapter.body(display, area) : tree.adapter.empty(area)}
      </div>
    </section>
  );
}

/**
 * An area's own tab bar; only the active area's shown tab carries the accent
 * indicator (B1, B20). Its New tab follows the tabs.
 */
function AreaTabBar({ area, active, index, count, switcher }: { area: Area<I>; active: boolean; index: number; count: number; switcher: boolean }) {
  const tree = useTree();
  const { t } = useInterfaceTranslation();
  const shown = area.displays.find((row) => row.id === area.active) ?? null;
  const selectedId = tree.adapter.shown ? tree.adapter.shown(area)?.id : area.active;
  // Both columns share the room left of the bar's own controls in stages,
  // the selected tab keeping its title longest (`tabStripFit`). The room is
  // the zone's, which the bar sizes, so the tabs' own widths never feed back.
  const zone = useRef<HTMLDivElement>(null);
  const newTab = useRef<HTMLButtonElement>(null);
  const strip = useRef<HTMLDivElement>(null);
  const [fits, setFits] = useState<{ selected: TabFit; others: TabFit }>({ selected: "titled", others: "titled" });
  const tabCount = area.displays.length;
  const hasSelected = area.displays.some((display) => display.id === selectedId);
  useLayoutEffect(() => {
    const node = zone.current;
    const list = strip.current;
    if (!node || !list) return;
    const sizes = readTabStripSizes();
    // Fractional bounds, rounded down: the tabs fill the bar to its end and never overflow it by a sliver.
    const measure = () => tabStripFit(Math.floor(node.getBoundingClientRect().width - (newTab.current?.getBoundingClientRect().width ?? 0)), tabCount, hasSelected, sizes);
    // The widths go straight onto the tab list; only a change of fit renders
    // the bar again, so a resize costs a style write, not a render per pixel.
    // Both land before paint, so the frame never shows tabs overflowing.
    const apply = (next: ReturnType<typeof measure>, render: (update: () => void) => void) => {
      const widths = { "--tab-selected-width": `${next.selected.width}px`, "--tab-other-width": `${next.others.width}px` };
      for (const [name, value] of Object.entries(widths)) list.style.setProperty(name, value);
      render(() => setFits((drawn) => (drawn.selected === next.selected.fit && drawn.others === next.others.fit ? drawn : { selected: next.selected.fit, others: next.others.fit })));
    };
    apply(measure(), (update) => update());
    const observer = new ResizeObserver(() => apply(measure(), flushSync));
    observer.observe(node);
    return () => observer.disconnect();
  }, [tabCount, hasSelected]);
  // The shown view's tab stays in sight however many tabs the area holds
  // (B20, B21): the strip scrolls itself, and nothing around it, whenever
  // the shown view changes, the tabs change density or the strip is resized.
  useLayoutEffect(() => {
    const list = strip.current;
    if (!list) return;
    const reveal = () => {
      const tab = list.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
      if (!tab) return;
      const offset = tab.getBoundingClientRect().left - list.getBoundingClientRect().left + list.scrollLeft;
      list.scrollLeft = revealedScroll(list.scrollLeft, list.clientWidth, offset, tab.offsetWidth);
    };
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(list);
    return () => observer.disconnect();
  }, [area.active, area.displays.length, fits]);
  return (
    <div className={`flex h-[var(--size-tab-strip)] shrink-0 items-stretch border-b border-border ${active ? "bg-background" : "bg-card"}`} data-area-tab-bar={area.id} {...data("tab-bar", area.id)} {...tree.adapter.barAttributes}>
      {tree.adapter.barStart?.(area)}
      <div ref={zone} className="flex min-w-0 flex-1 items-stretch">
        <div ref={strip} role="tablist" aria-label={t("panes.area.tabListOf", { label: tree.adapter.tabListLabel, index: index + 1, total: count })} className="flex min-w-0 items-stretch overflow-x-auto">
          {area.displays.map((display) => (
            <EntryContextMenu
              key={display.id}
              returnToInput={column === "view"}
              title={tree.adapter.label(display)}
              label={t(`panes.area.itemActions.${column}` as const, { label: tree.adapter.label(display) })}
              items={() => tree.menu(display.id)}
              onSelect={(id) => tree.adapter.runMenu(id, display.id)}
              onCloseAutoFocus={tree.adapter.onMenuCloseAutoFocus}
              className={TAB_SLOT[display.id === selectedId ? "selected" : "others"]}
              data-tab-menu={display.id}
              data-area-item={display.id}
            >
              {tree.adapter.tab(display, { selected: display.id === selectedId, fit: fits[display.id === selectedId ? "selected" : "others"], areaActive: active, dragging: tree.draggingId === display.id, press: (event) => tree.press(display.id, event), select: () => { if (!tree.takeClick()) tree.focus(display.id); } })}
            </EntryContextMenu>
          ))}
        </div>
        <Hint label={tree.adapter.newTabLabel} shortcut={tree.adapter.newTabShortcut}>
          <button
            ref={newTab}
            type="button"
            aria-label={tree.adapter.newTabLabel}
            {...data("new-tab", area.id)} {...(column === "agent" ? { "data-new-agent-tab": "true" } : {})}
            className="flex min-w-[var(--size-tab-overflow-control)] shrink-0 items-center justify-center text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:bg-accent"
            onClick={() => tree.adapter.newTab(area.id)}
          >
            <PlusIcon className="size-(--size-icon)" />
          </button>
        </Hint>
      </div>
      {switcher ? <AreaSwitcher current={area.id} /> : null}
      {tree.adapter.barEnd?.(area)}
      {shown ? (
        <EntryDropdown
          title={tree.adapter.label(shown)}
          label={t("panes.area.actionsFor", { actions: tree.adapter.actionsLabel, label: tree.adapter.label(shown) })}
          hint={t("panes.area.actionsFor", { actions: tree.adapter.actionsLabel, label: tree.adapter.label(shown) })}
          items={tree.menu(shown.id)}
          onSelect={(id) => tree.adapter.runMenu(id, shown.id)}
          trigger={
            <button
              type="button"
              {...data("overflow", shown.id)}
              className="flex min-w-[var(--size-tab-overflow-control)] shrink-0 items-center justify-center text-subtle-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:bg-accent"
            >
              <EllipsisIcon className="size-(--size-icon)" />
            </button>
          }
        />
      ) : null}
    </div>
  );
}

/** A narrow window shows one area; this names it and switches to another (B13). */
function AreaSwitcher({ current }: { current: string }) {
  const tree = useTree();
  const { t } = useInterfaceTranslation();
  const areas = areasOf(tree.layout.root);
  const index = areas.findIndex((area) => area.id === current);
  const items: MenuEntry<string>[] = areas.map((area, at) => {
    const shown = area.displays.find((row) => row.id === area.active);
    return { id: area.id, label: `${area.id === current ? "✓ " : ""}${t("panes.area.areaN", { index: at + 1 })}${shown ? ` · ${tree.adapter.label(shown)}` : ""}`, unavailable: null };
  });
  return (
    <EntryDropdown
      label={t(`panes.area.switch.${column}` as const, { index: index + 1, total: areas.length })}
      items={items}
      onSelect={(id) => {
        if (id !== current) tree.adapter.focusArea(id);
      }}
      trigger={
        <Button
          variant="ghost"
          className="min-w-[var(--size-tab-overflow-control)] shrink-0 rounded-none text-subtle-foreground hover:text-foreground focus-visible:bg-accent"
          aria-label={t(`panes.area.switch.${column}` as const, { index: index + 1, total: areas.length })}
          {...data("area-switch", current)}
        >
          {index + 1}/{areas.length}
          <ChevronDownIcon />
        </Button>
      }
    />
  );
}

/** The drag's one preview: an insertion line, or a split's destination and its label (B6-B8, D-14). */
function DragPreview({ session }: { session: Extract<DragSession, { phase: "dragging" }> }) {
  const tree = useTree();
  const display = locateDisplay(tree.layout.root, session.displayId)?.display ?? null;
  const target = session.target;
  return (
    <>
      {target.kind === "bar" ? (
        <div
          className="pointer-events-none absolute z-30 w-[var(--size-tab-indicator)] -translate-x-1/2 bg-primary"
          style={{ left: target.line.x, top: target.line.y, height: target.line.height }}
          data-area-drop="bar" {...data("drop", "bar")}
        />
      ) : null}
      {target.kind === "edge" ? (
        <div
          className="pointer-events-none absolute z-30"
          style={{ left: target.preview.x, top: target.preview.y, width: target.preview.width, height: target.preview.height }}
          data-area-drop={target.edge} {...data("drop", target.edge)}
        >
          <div className="absolute inset-0 bg-primary opacity-[var(--opacity-selected-fill)]" />
          <div className="absolute inset-0 border border-primary" />
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="rounded-sm bg-popover px-sm py-xxs text-caption text-foreground shadow-lg">{target.label}</span>
          </div>
        </div>
      ) : null}
      {display ? <FloatingTab display={display} start={session.point} reason={target.kind === "none" ? target.reason : null} /> : null}
    </>
  );
}

/** The dragged tab under the pointer; it follows every move without redrawing the tree. */
function FloatingTab({ display, start, reason }: { display: I; start: Point; reason: string | null }) {
  const tree = useTree();
  const [point, setPoint] = useState(start);
  useEffect(() => {
    const move = (event: PointerEvent) => setPoint({ x: event.clientX, y: event.clientY });
    window.addEventListener("pointermove", move);
    return () => window.removeEventListener("pointermove", move);
  }, []);
  return (
    <div
      className="pointer-events-none fixed z-50 flex max-w-[var(--size-tab-preferred)] translate-x-sm translate-y-sm flex-col gap-xxs rounded-sm border border-border bg-secondary px-sm py-xxs text-caption text-foreground shadow-lg"
      style={{ left: point.x, top: point.y }}
      data-area-drag-tab={display.id} {...data("drag-tab", display.id)}
    >
      <span className="flex min-w-0 items-center gap-xs">
        {tree.adapter.floating(display)}
      </span>
      {reason ? <span className="text-muted-foreground">{reason}</span> : null}
    </div>
  );
}

return AreaTree;
}
