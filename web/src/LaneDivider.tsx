import { ArrowLeftRightIcon } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { Actions } from "./actions";
import { holdShellDrag } from "./shellDrag";
import { columnLanding, edgeColumn, laneLanding, type LaneFrame, type LaneLanding, type LaneSizes } from "./workspace";

/** One arrow key on a focused divider moves it this many pixels. */
const DIVIDER_STEP = 32;

/**
 * A divider of the lane (PRD right-lane B6, B10; lane-two-columns B17;
 * `Component / Side panel grip`): `lane`, at the lane's left edge, sets the
 * width of the column at that edge, against Agent Views when docked and
 * over them otherwise (`edge`, hung outside the lane's box); `column`,
 * between the viewing column and the tool column, trades width between the
 * two. Nothing at rest, and on hover, keyboard focus and while dragging a
 * hairline with a ⇆ pill at its middle. A drag moves a guide and lands once
 * on release, so no terminal or page resizes during it and every page shows
 * its still; a focused divider moves one step per arrow key, one event per
 * press, each step measured from the last one the core has not yet
 * confirmed.
 */
export function LaneDivider({ kind, label, workspace, acknowledged, frame, origin, sizes, setGuide, actions, edge = false }: { kind: "lane" | "column"; label: string; workspace: string; acknowledged: string | null; frame: LaneFrame; origin: HTMLElement | null; sizes: LaneSizes; setGuide: (x: number | null) => void; actions: Actions; edge?: boolean }) {
  const cancelDrag = useRef<(() => void) | null>(null);
  const [keySession] = useState(() => crypto.randomUUID());
  const keyIntent = useRef<{ frame: LaneFrame; published: LaneFrame; extent: number; request: string; observed: string | null } | null>(null);
  const extent = () => origin?.clientWidth ?? 0;
  // Keep one absolute target until its own acknowledgement, not an older
  // echo from this key session. Other callers and changed geometry rebase it.
  useLayoutEffect(() => {
    const pending = keyIntent.current;
    if (pending && (acknowledged === pending.request ||
      ((acknowledged !== pending.observed || frame.width !== pending.published.width || frame.view?.width !== pending.published.view?.width || frame.tool?.width !== pending.published.tool?.width) && !acknowledged?.startsWith(`${keySession}:`)) ||
      pending.extent !== origin?.clientWidth || pending.frame.mode !== frame.mode || pending.frame.view?.field !== frame.view?.field || pending.frame.tool?.field !== frame.tool?.field)) {
      keyIntent.current = null;
    }
  }, [acknowledged, keySession, origin, frame]);
  useLayoutEffect(() => () => { keyIntent.current = null; }, [workspace, origin]);
  // A captured pointer belongs to the Workspace and geometry it started on.
  // Switching either, or unmounting, retires the guide and its page still.
  useLayoutEffect(() => () => cancelDrag.current?.(), [workspace, origin, frame.mode, frame.width, frame.view?.width, frame.tool?.width]);
  const landingAt = (x: number, basis: LaneFrame): LaneLanding =>
    kind === "lane" ? laneLanding({ x, body: extent(), frame: basis, sizes }) : columnLanding({ x, frame: basis });
  // The frame a landing leaves, so the next arrow key steps from it.
  const landed = (basis: LaneFrame, patch: LaneLanding): LaneFrame => {
    const view = basis.view ? { ...basis.view, width: patch[basis.view.field] ?? basis.view.width } : null;
    const tool = basis.tool ? { ...basis.tool, width: patch[basis.tool.field] ?? basis.tool.width } : null;
    const width = kind === "lane" ? (view?.width ?? 0) + (tool?.width ?? 0) + (view && tool ? sizes.divider : 0) : basis.width;
    return { ...basis, view, tool, width };
  };
  // Where the divider's left edge stands, from its origin's left edge.
  const position = (basis: LaneFrame) => (kind === "lane" ? extent() - basis.width - sizes.divider : basis.view?.width ?? 0);
  const land = (x: number, basis = frame, request?: string) => {
    if (extent() <= 0) return;
    const patch = landingAt(x, basis);
    const changed = [basis.view, basis.tool].some((column) => column && patch[column.field] !== undefined && patch[column.field] !== column.width);
    if (!changed) return;
    if (request) keyIntent.current = { frame: landed(basis, patch), published: frame, extent: extent(), request, observed: acknowledged };
    if (actions.setWorkspaceView({ ...patch, ...(request ? { width_request_id: request } : {}) }) === false) keyIntent.current = null;
  };
  const drag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !origin) return;
    event.preventDefault();
    cancelDrag.current?.();
    keyIntent.current = null;
    const left = origin.getBoundingClientRect().left;
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const at = (next: PointerEvent) => next.clientX - left - sizes.divider / 2;
    // The guide shows where the divider would land, clamped like the landing.
    const move = (next: PointerEvent) => setGuide(position(landed(frame, landingAt(at(next), frame))));
    // The drag marks the root, so a page the guide crosses gives way to its still.
    const release = holdShellDrag("col-resize");
    const end = () => {
      if (cancelDrag.current !== end) return;
      cancelDrag.current = null;
      release();
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", end);
      target.removeEventListener("lostpointercapture", end);
      if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
      setGuide(null);
    };
    const up = (next: PointerEvent) => {
      if (cancelDrag.current !== end) return;
      end();
      land(at(next));
    };
    cancelDrag.current = end;
    // A drag the system cancels lands nothing and leaves no guide behind.
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", end);
    target.addEventListener("lostpointercapture", end);
  };
  const moved = kind === "lane" ? edgeColumn(frame) : frame.view;
  const value = moved?.width ?? 0;
  const min = moved?.min ?? 0;
  // Docked, the lane grows into what Agent Views can spare; over them, to
  // the body's edge; between the columns, into what the tool column can spare.
  const spare = kind === "lane" ? (frame.mode === "overlay" ? frame.agents - frame.width - sizes.divider : frame.agents - sizes.agentMin) : (frame.tool?.width ?? 0) - (frame.tool?.min ?? 0);
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={value + Math.max(0, spare)}
      tabIndex={0}
      data-lane-divider={kind}
      className={`group z-10 flex w-sm shrink-0 cursor-col-resize justify-center outline-none ${edge ? "absolute inset-y-0" : "relative bg-background"}`}
      style={edge ? { left: -sizes.divider } : undefined}
      onPointerDown={drag}
      onBlur={() => { keyIntent.current = null; }}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        // Left widens the lane, or the tool column, as the divider moves left.
        const pending = keyIntent.current;
        const basis = pending && pending.extent === extent() ? pending.frame : frame;
        const request = `${keySession}:${crypto.randomUUID()}`;
        land(position(basis) + (event.key === "ArrowLeft" ? -DIVIDER_STEP : DIVIDER_STEP), basis, request);
      }}
    >
      <DividerGrip className="opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
    </div>
  );
}

/** A drag's guide, drawn where the divider would land, `left` pixels from its origin. */
export function DividerGuide({ left }: { left: number }) {
  return (
    <div className="pointer-events-none absolute inset-y-0 z-30 flex w-sm justify-center" style={{ left }} data-lane-guide="true">
      <DividerGrip />
    </div>
  );
}

/** The divider's grip: a hairline with a ⇆ pill at its middle, drawn on hover, keyboard focus and while dragging. */
function DividerGrip({ className = "" }: { className?: string }) {
  return (
    <span aria-hidden="true" className={`relative h-full w-(--size-hairline) ${className}`} data-lane-grip="true">
      <span className="absolute inset-0 bg-muted-foreground opacity-[var(--opacity-secondary)]" />
      <span className="absolute left-1/2 top-1/2 flex size-(--size-icon-button-toolbar) -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-md border border-border bg-card text-muted-foreground">
        <ArrowLeftRightIcon className="size-(--size-icon-sm)" />
      </span>
    </span>
  );
}
