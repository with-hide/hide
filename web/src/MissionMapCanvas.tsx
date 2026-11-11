import { ArrowDownIcon, ArrowRightIcon, MinusIcon, PlusIcon } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type PointerEvent, type ReactNode, type SyntheticEvent } from "react";
import { flushSync } from "react-dom";
import { Button } from "./components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group";
import { Hint } from "./components/ui/tooltip";
import { holdsCommandKey, keySystem } from "./host";
import { useInterfaceTranslation } from "./i18n/translator";
import { cn } from "./lib/utils";
import { clampMapZoom, mapFit, mapLayout, MAX_MAP_ZOOM, MIN_MAP_ZOOM, stepMapZoom } from "./missionMapZoom";
import type { MissionMapFlow } from "./snapshot";
import { anchoredScroll, wheelZoomFactor, type Point, type Size } from "./viewers/viewerZoom";

// The box a mission's map is drawn in (with-hide/hide-private
// docs/tasks-and-pr-list.md, A mission's map): it opens at 맞춤 and keeps fitting while the operator has not
// zoomed, so a panel opening beside it or a resized slot fits it again; a
// pinch or a wheel with ⌘ (Ctrl off macOS) zooms around the pointer, a
// drag from an empty spot moves the map, and ↓ → · − · % · + · 맞춤 sit at
// its lower right. A plain wheel scrolls as any box does. ↓ and → turn
// every mission's map to run down or across, and the turned map fits again.

/** How far a press moves before it is a drag rather than a press on the map. */
const DRAG_SLOP = 4;

/** A scale the eye cannot tell from another. */
const SCALE_EPSILON = 0.001;

type Measured = {
  /** The map's own size, which a transform does not change. */
  map: Size;
  /** The box's client size. */
  viewport: Size;
  /** The height a box sized by its map may grow to: the page's, so the panel follows under it. */
  cap: number;
  /** A horizontal scroll bar's height, which a box sized by its map adds to its own. */
  bar: number;
  /** How far up from the box's bottom the zoom buttons reach: the strip kept free under the map. */
  below: number;
};

/**
 * `wide` is the slot's two-column layout, where the box takes the height
 * left beside the panel; in a narrow slot the box is as tall as its map,
 * up to `page`'s height, and the panel follows under it.
 */
export function MissionMapCanvas({ wide, page, flow, onFlow, children, touch = false, focusNodes = [] }: { wide: boolean; page: HTMLElement | null; flow: MissionMapFlow; onFlow: (flow: MissionMapFlow) => void; children: ReactNode; touch?: boolean; focusNodes?: string[] }) {
  const { t } = useInterfaceTranslation();
  const [box, boxRef] = useState<HTMLDivElement | null>(null);
  const content = useRef<HTMLDivElement>(null);
  const controls = useRef<HTMLDivElement>(null);
  const [measured, setMeasured] = useState<Measured | null>(null);
  // Null while the map fits; a scale once the operator zoomed it.
  const interacted = useRef(false);
  const [chosen, setChosen] = useState<number | null>(null);

  useLayoutEffect(() => {
    const inner = content.current;
    const buttons = controls.current;
    if (!box || !inner || !buttons) return undefined;
    const measure = () => {
      const frame = buttons.offsetParent instanceof HTMLElement ? buttons.offsetParent.clientHeight : 0;
      const next: Measured = {
        map: { width: inner.offsetWidth, height: inner.offsetHeight },
        viewport: { width: box.clientWidth, height: box.clientHeight },
        cap: page?.clientHeight ?? box.clientHeight,
        bar: box.offsetHeight - box.clientHeight,
        below: Math.max(0, frame - buttons.offsetTop),
      };
      setMeasured((old) => (old && sameMeasure(old, next) ? old : next));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    observer.observe(inner);
    if (page) observer.observe(page);
    return () => observer.disconnect();
  }, [box, page]);

  const room = measured ? { width: measured.viewport.width, height: wide ? measured.viewport.height : measured.cap } : null;
  const fit = measured && room ? mapFit(measured.map, room, measured.below) : 1;
  const scale = chosen ?? fit;
  const layout = measured ? mapLayout(measured.map, scale, measured.viewport, measured.below) : null;
  const height = !wide && layout && measured ? Math.min(layout.height + measured.bar, measured.cap) : null;
  const drawn = layout?.boxes[0];

  // The latest of each for the listeners, which outlive a render.
  const latest = useRef({ scale, measured, wide });
  useLayoutEffect(() => {
    latest.current = { scale, measured, wide };
  });

  /** Zooms to `target`, keeping the map under `pointer` (the box's middle when null) where it is. */
  const zoomTo = useCallback((target: number, pointer: Point | null) => {
    const { scale: now, measured: size, wide: bounded } = latest.current;
    const next = clampMapZoom(target);
    if (!box || !size || Math.abs(next - now) < SCALE_EPSILON) return;
    const before = mapLayout(size.map, now, size.viewport, size.below);
    const after = mapLayout(size.map, next, size.viewport, size.below);
    const view = { width: size.viewport.width, height: bounded ? size.viewport.height : Math.min(after.height, size.cap) };
    const at = pointer ?? { x: view.width / 2, y: Math.min(size.viewport.height, view.height) / 2 };
    const scroll = anchoredScroll(before, after, { x: box.scrollLeft, y: box.scrollTop }, at, view);
    // The new size and its scroll land in one frame, so the map never shows
    // at the new size from the old scroll position.
    flushSync(() => setChosen(next));
    box.scrollLeft = scroll.x;
    box.scrollTop = scroll.y;
  }, [box]);

  const fitAgain = () => {
    interacted.current = true;
    flushSync(() => setChosen(null));
    if (!box) return;
    box.scrollLeft = 0;
    box.scrollTop = 0;
  };

  const focusKey = JSON.stringify(focusNodes);
  useLayoutEffect(() => {
    if (!touch || !box || !measured || interacted.current) return;
    const keys = new Set<string>(JSON.parse(focusKey));
    const inner = content.current;
    if (!inner) return;
    const origin = inner.getBoundingClientRect();
    const nodes = [...inner.querySelectorAll<HTMLElement>("[data-mission-node]")].filter((node) => keys.has(node.dataset.missionNode ?? ""));
    if (nodes.length === 0) {
      setChosen(null);
      box.scrollLeft = 0;
      box.scrollTop = 0;
      return;
    }
    const rects = nodes.map((node) => node.getBoundingClientRect());
    const now = latest.current.scale;
    const left = (Math.min(...rects.map((r) => r.left)) - origin.left) / now;
    const top = (Math.min(...rects.map((r) => r.top)) - origin.top) / now;
    const width = (Math.max(...rects.map((r) => r.right)) - origin.left) / now - left;
    const height = (Math.max(...rects.map((r) => r.bottom)) - origin.top) / now - top;
    const next = mapFit({ width, height }, measured.viewport, measured.below);
    const after = mapLayout(measured.map, next, measured.viewport, measured.below);
    setChosen(next);
    // Scroll after React lays out the newly scaled surface.
    const frame = requestAnimationFrame(() => {
      box.scrollLeft = after.boxes[0]!.left + (left + width / 2) * next - measured.viewport.width / 2;
      box.scrollTop = after.boxes[0]!.top + (top + height / 2) * next - (measured.viewport.height - measured.below) / 2;
    });
    return () => cancelAnimationFrame(frame);
  }, [touch, box, measured, focusKey]);

  // A map turned the other way is another shape: it fits again from its start.
  const drawnFlow = useRef(flow);
  useLayoutEffect(() => {
    if (drawnFlow.current === flow) return;
    drawnFlow.current = flow;
    interacted.current = false;
    setChosen(null);
    if (!box) return;
    box.scrollLeft = 0;
    box.scrollTop = 0;
  }, [flow, box]);

  const ready = measured !== null;
  useEffect(() => {
    if (!box || !ready) return undefined;
    let factor = 1;
    let client: Point | null = null;
    let frame = 0;
    const onWheel = (event: WheelEvent) => {
      // A pinch arrives as a Ctrl wheel; ⌘ (Ctrl off macOS) with a wheel zooms too, and a plain wheel scrolls.
      if (!event.ctrlKey && !holdsCommandKey(event)) return;
      event.preventDefault();
      factor *= wheelZoomFactor(event.deltaY, event.deltaMode);
      client = { x: event.clientX, y: event.clientY };
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const rect = box.getBoundingClientRect();
        const pointer = client ? { x: client.x - rect.left - box.clientLeft, y: client.y - rect.top - box.clientTop } : null;
        const by = factor;
        factor = 1;
        interacted.current = true;
        zoomTo(latest.current.scale * by, pointer);
      });
    };
    box.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      box.removeEventListener("wheel", onWheel);
      cancelAnimationFrame(frame);
    };
  }, [box, ready, zoomTo]);

  const pannable = !!layout && !!measured && (layout.width > measured.viewport.width + 1 || layout.height > measured.viewport.height + 1);
  const pan = usePan(box, pannable);
  const gestures = useTouchMap(box, touch, (factor, pointer) => {
    interacted.current = true;
    zoomTo(latest.current.scale * factor, pointer);
  }, () => { interacted.current = true; });
  const percent = `${Math.round(scale * 100)}%`;
  return (
    <div className={cn("relative flex min-w-0 flex-col", wide && "min-h-0 flex-1")}>
      <div
        ref={boxRef}
        className={cn("min-h-0 min-w-0 flex-1 overflow-auto [overflow-anchor:none]", touch && "touch-none overscroll-contain", pannable && (pan.dragging ? "cursor-grabbing select-none" : "cursor-grab"))}
        style={height != null ? { height } : undefined}
        data-mission-canvas={chosen === null ? "fit" : "zoomed"}
        data-mission-scale={scale.toFixed(2)}
        {...(touch ? gestures : pan.handlers)}
      >
        <div className="relative" style={layout ? { width: layout.width, height: layout.height } : undefined}>
          <div
            ref={content}
            className={cn("w-max origin-top-left", drawn && "absolute")}
            style={drawn ? { left: drawn.left, top: drawn.top, transform: Math.abs(scale - 1) < SCALE_EPSILON ? undefined : `scale(${scale})` } : undefined}
          >
            {children}
          </div>
        </div>
      </div>
      <div ref={controls} role="group" aria-label={t("mission.zoom")} className={cn("absolute bottom-sm right-sm flex items-center gap-xxs rounded-md border border-border bg-popover p-xxs shadow-lg", touch && "[&_button]:min-h-(--size-touch-target) [&_button]:min-w-(--size-touch-target)")} data-mission-zoom={percent}>
        <ToggleGroup
          type="single"
          value={flow}
          onValueChange={(value) => {
            if ((value === "down" || value === "across") && value !== flow) onFlow(value);
          }}
          aria-label={t("mission.flow")}
          className="bg-transparent p-0"
          data-mission-direction={flow}
        >
          {/* The hint sits inside: as the item's own trigger its `data-state` would hide which way is on. */}
          <ToggleGroupItem value="down" className="px-xs" aria-label={t("mission.flow.down")} data-mission-direction-option="down">
            <Hint label={t("mission.flow.down")}>
              <span className="flex">
                <ArrowDownIcon />
              </span>
            </Hint>
          </ToggleGroupItem>
          {/* The hint sits inside: as the item's own trigger its `data-state` would hide which way is on. */}
          <ToggleGroupItem value="across" className="px-xs" aria-label={t("mission.flow.across")} data-mission-direction-option="across">
            <Hint label={t("mission.flow.across")}>
              <span className="flex">
                <ArrowRightIcon />
              </span>
            </Hint>
          </ToggleGroupItem>
        </ToggleGroup>
        <span aria-hidden="true" className="mx-xxs h-(--size-icon) w-(--size-hairline) bg-border" />
        <Button variant="ghost" size="icon-sm" aria-label={t("documents.zoomOut")} disabled={scale <= MIN_MAP_ZOOM + SCALE_EPSILON} onClick={() => { interacted.current = true; zoomTo(stepMapZoom(scale, "out"), null); }} data-mission-zoom-step="out">
          <MinusIcon />
        </Button>
        <Hint label={t("mission.zoom.hint", { key: keySystem() === "mac" ? "⌘" : "Ctrl" })}>
          <span className="min-w-(--size-control-lg) text-center text-caption font-semibold tabular-nums" aria-live="polite" data-mission-zoom-level="true">
            {percent}
          </span>
        </Hint>
        <Button variant="ghost" size="icon-sm" aria-label={t("documents.zoomIn")} disabled={scale >= MAX_MAP_ZOOM - SCALE_EPSILON} onClick={() => { interacted.current = true; zoomTo(stepMapZoom(scale, "in"), null); }} data-mission-zoom-step="in">
          <PlusIcon />
        </Button>
        <span aria-hidden="true" className="mx-xxs h-(--size-icon) w-(--size-hairline) bg-border" />
        <Button variant="ghost" size="sm" className="whitespace-nowrap" aria-pressed={chosen === null} onClick={fitAgain} data-mission-zoom-step="fit">
          {t("mission.zoom.fit")}
        </Button>
      </div>
    </div>
  );
}

function sameMeasure(a: Measured, b: Measured): boolean {
  return a.map.width === b.map.width && a.map.height === b.map.height && a.viewport.width === b.viewport.width && a.viewport.height === b.viewport.height && a.cap === b.cap && a.bar === b.bar && a.below === b.below;
}

/**
 * Dragging from an empty spot moves the map under the pointer, as a hand tool
 * does. A press that moves less than `DRAG_SLOP` stays a press, so a press on
 * an empty spot still lets go of the pick; one that moved is no press at all.
 * A card, a button or the box's own scroll bar keeps its press.
 */
function usePan(box: HTMLDivElement | null, pannable: boolean) {
  const [dragging, setDragging] = useState(false);
  const from = useRef<{ pointer: number; x: number; y: number; left: number; top: number; moved: boolean } | null>(null);
  const swallow = useRef(false);
  const end = (event: PointerEvent<HTMLElement>) => {
    const start = from.current;
    if (start?.pointer !== event.pointerId) return;
    from.current = null;
    if (!start.moved) return;
    swallow.current = true;
    setDragging(false);
  };
  return {
    dragging,
    handlers: {
      onPointerDown: (event: PointerEvent<HTMLElement>) => {
        // Every press starts anew: a drag that ended outside the box left no click to swallow.
        swallow.current = false;
        if (!pannable || !box || event.button !== 0 || event.pointerType === "touch") return;
        const target = event.target as Element;
        if (target.closest("button, a, input, [data-mission-node]")) return;
        // A press past the box's client area is on its scroll bar.
        const rect = box.getBoundingClientRect();
        if (event.clientX - rect.left - box.clientLeft >= box.clientWidth || event.clientY - rect.top - box.clientTop >= box.clientHeight) return;
        from.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY, left: box.scrollLeft, top: box.scrollTop, moved: false };
      },
      onPointerMove: (event: PointerEvent<HTMLElement>) => {
        const start = from.current;
        if (!start || start.pointer !== event.pointerId || !box) return;
        const dx = event.clientX - start.x;
        const dy = event.clientY - start.y;
        if (!start.moved) {
          if (Math.hypot(dx, dy) < DRAG_SLOP) return;
          start.moved = true;
          event.currentTarget.setPointerCapture(event.pointerId);
          setDragging(true);
        }
        box.scrollLeft = start.left - dx;
        box.scrollTop = start.top - dy;
      },
      onPointerUp: end,
      onPointerCancel: end,
      // The click a drag ends in is no press on the map.
      onClickCapture: (event: SyntheticEvent) => {
        if (!swallow.current) return;
        swallow.current = false;
        event.stopPropagation();
      },
    },
  };
}

/** Two retained pointers at most; one coalesced frame, cancelled with the canvas. A tap stays a node press. */
function useTouchMap(box: HTMLDivElement | null, enabled: boolean, zoom: (factor: number, at: Point) => void, moved: () => void) {
  const pointers = useRef(new Map<number, Point>());
  const swallow = useRef(false);
  const start = useRef<Point | null>(null);
  const pending = useRef<{ factor: number; at: Point; dx: number; dy: number } | null>(null);
  const frame = useRef(0);
  const latest = useRef({ zoom, moved });
  useLayoutEffect(() => { latest.current = { zoom, moved }; });
  useEffect(() => () => { cancelAnimationFrame(frame.current); }, []);
  const end = (event: PointerEvent<HTMLElement>) => {
    pointers.current.delete(event.pointerId);
    start.current = pointers.current.values().next().value ?? null;
  };
  return {
    onPointerDown: (event: PointerEvent<HTMLElement>) => {
      if (!enabled || !box || event.button !== 0 || pointers.current.size >= 2) return;
      if (pointers.current.size === 0) {
        swallow.current = false;
        start.current = { x: event.clientX, y: event.clientY };
      }
      pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
    },
    onPointerMove: (event: PointerEvent<HTMLElement>) => {
      if (!box || !pointers.current.has(event.pointerId)) return;
      const before = [...pointers.current.values()];
      pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
      const after = [...pointers.current.values()];
      const midpoint = (points: Point[]): Point => ({ x: points.reduce((sum, p) => sum + p.x, 0) / points.length, y: points.reduce((sum, p) => sum + p.y, 0) / points.length });
      const from = midpoint(before);
      const to = midpoint(after);
      if (!swallow.current && after.length === 1 && start.current && Math.hypot(to.x - start.current.x, to.y - start.current.y) < DRAG_SLOP) return;
      const firstMove = !swallow.current;
      swallow.current = true;
      event.currentTarget.setPointerCapture(event.pointerId);
      const distance = (points: Point[]) => Math.hypot(points[1]!.x - points[0]!.x, points[1]!.y - points[0]!.y);
      const factor = after.length === 2 && distance(before) > 0 ? distance(after) / distance(before) : 1;
      const rect = box.getBoundingClientRect();
      const at = { x: from.x - rect.left, y: from.y - rect.top };
      const delta = firstMove && after.length === 1 && start.current ? { x: to.x - start.current.x, y: to.y - start.current.y } : { x: to.x - from.x, y: to.y - from.y };
      const old = pending.current;
      pending.current = { factor: (old?.factor ?? 1) * factor, at: old?.at ?? at, dx: (old?.dx ?? 0) + delta.x, dy: (old?.dy ?? 0) + delta.y };
      if (frame.current) return;
      frame.current = requestAnimationFrame(() => {
        frame.current = 0;
        const change = pending.current;
        pending.current = null;
        if (!change) return;
        latest.current.moved();
        latest.current.zoom(change.factor, change.at);
        box.scrollLeft -= change.dx;
        box.scrollTop -= change.dy;
      });
    },
    onPointerUp: end,
    onPointerCancel: end,
    onLostPointerCapture: (event: PointerEvent<HTMLElement>) => {
      // Moving implicit touch capture from a card to the canvas releases the
      // card first. Its bubbling event does not end the canvas's gesture.
      if (event.target === event.currentTarget) end(event);
    },
    onClickCapture: (event: SyntheticEvent) => {
      if (!swallow.current) return;
      event.preventDefault();
      event.stopPropagation();
      swallow.current = false;
    },
  };
}
