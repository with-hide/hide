import { XIcon } from "lucide-react";
import type { KeyboardEvent, MouseEvent, ReactNode, Ref } from "react";
import { cn } from "../lib/utils";
import { Button } from "./ui/button";
import { Hint } from "./ui/tooltip";

// The frame a map page is drawn in, a mission's (`MissionMap`) and a plan's
// (`PlanMap`) alike (PRD mission-map-git-criteria B30, B34): the page that
// scrolls whole while narrow, the map with the column beside it once the page
// is `mission-wide` (under it while narrow), and the panel of the card the
// operator picked in that column, whose head stays while it scrolls. Each
// page brings its own head, map and panel content; what it names itself by
// goes in `data-*` attributes it passes through.

/** The `data-*` attributes a page names its parts by (`data-mission-map`, `data-plan-panel`). */
export type DataAttributes = { [name: `data-${string}`]: string | boolean | undefined };

/** The page: a `@container/mission` box and the one scroll the narrow page has, which the wide page hands to its map and its column. */
export function MapPage({ pageRef, scrollRef, data, scrollData, onKeyDown, children }: { pageRef?: Ref<HTMLDivElement>; scrollRef?: Ref<HTMLDivElement>; data: DataAttributes; scrollData: DataAttributes; onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void; children: ReactNode }) {
  return (
    <div ref={pageRef} className="@container/mission flex min-h-0 min-w-0 flex-1 flex-col" {...data} onKeyDown={onKeyDown}>
      <div ref={scrollRef} className="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto mission-wide:overflow-hidden" {...scrollData}>
        {children}
      </div>
    </div>
  );
}

/**
 * The map and the column beside it: side by side on a wide page, the column
 * under the map on a narrow one. A press on the map's ground goes to
 * `onMapClick`, and the column is `sideRef`, the box a pick scrolls back to
 * its top (`useMapPick`).
 */
export function MapSplit({ map, side, sideRef, onMapClick, dimmed = false, data, mapData, sideData }: { map: ReactNode; side: ReactNode; sideRef: Ref<HTMLDivElement>; onMapClick: (event: MouseEvent<HTMLDivElement>) => void; dimmed?: boolean; data?: DataAttributes; mapData: DataAttributes; sideData: DataAttributes }) {
  return (
    <div className={cn("flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:flex-1 mission-wide:flex-row", dimmed && "opacity-(--opacity-dimmed)")} {...data}>
      <div className="flex min-w-0 flex-col mission-wide:min-h-0 mission-wide:flex-1" onClick={onMapClick} {...mapData}>
        {map}
      </div>
      <div
        ref={sideRef}
        className="flex min-w-0 flex-col border-t border-border mission-wide:min-h-0 mission-wide:w-(--size-mission-side) mission-wide:shrink-0 mission-wide:overflow-auto mission-wide:border-l mission-wide:border-t-0"
        {...sideData}
      >
        {side}
      </div>
    </div>
  );
}

/** A picked card's panel in the column: its head first, then its rows top down. */
export function MapPanel({ data, children }: { data: DataAttributes; children: ReactNode }) {
  return (
    <div className="@container/mission-panel flex min-w-0 flex-col gap-md px-md pb-sm text-body" {...data}>
      {children}
    </div>
  );
}

/**
 * The panel's head, which stays at the top while the panel scrolls: the
 * card's mark, its whole title, the ways out the page gives it beside ✕
 * (`close` names the ✕), and one line of facts under it when there are any.
 */
export function MapPanelHead({ mark, title, ways, close, onClose, facts }: { mark: ReactNode; title: string; ways?: ReactNode; close: string; onClose: () => void; facts: ReactNode | null }) {
  return (
    <section className="sticky top-0 z-10 -mx-md flex min-w-0 flex-col gap-xxs bg-background px-md pb-xs pt-sm" data-panel-part="head">
      <div className="flex min-w-0 items-start gap-xs">
        <span className="mt-xxs flex shrink-0">{mark}</span>
        <h3 className="min-w-0 flex-1 break-words text-subhead font-semibold [word-break:keep-all]" data-panel-title="true">{title}</h3>
        {ways}
        <Hint label={close}>
          <Button variant="ghost" size="icon-sm" className="-mr-xs shrink-0 text-muted-foreground" aria-label={close} onClick={onClose} data-panel-close="true">
            <XIcon />
          </Button>
        </Hint>
      </div>
      {facts ? (
        <p className="flex min-w-0 flex-wrap items-center gap-x-sm gap-y-xxs text-caption text-muted-foreground" data-panel-meta="true">
          {facts}
        </p>
      ) : null}
    </section>
  );
}
