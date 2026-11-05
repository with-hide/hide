import { useCallback, useRef, useState, type KeyboardEvent, type MouseEvent, type RefObject } from "react";

// The pick a work page's map keeps, shared by a mission's page and a plan's
// (PRD mission-map-git-criteria B30, B34): one card picked at a time, its
// panel in the column beside the map (under it when the page is narrow, where
// it is brought into view), Escape or a press on the map outside every card
// letting go of it, and a pick whose card a replan took away being no pick.
// The page draws its own cards and panel; this holds only the interaction.

export type MapPick = {
  /** The picked card's key, or null; a key no card has any more reads as null. */
  selected: string | null;
  /** Picks a card (null lets go); a pick brings the panel into view under a narrow map. */
  select: (key: string | null) => void;
  /** Goes on the column the panel stands in, so a narrow page can scroll it into view. */
  panel: RefObject<HTMLDivElement | null>;
  /** Goes on the page's root: Escape lets go of a pick before the lane answers it. */
  onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void;
  /** Goes on the map's box: a press outside every card (`cardSelector`) lets go of the pick. */
  onMapClick: (event: MouseEvent<HTMLDivElement>) => void;
};

/**
 * `keys` are the cards the page draws now; `cardSelector` matches a card and
 * anything else on the map a press must not let go for (a mission's checkers
 * among them).
 */
export function useMapPick(keys: readonly string[], cardSelector: string): MapPick {
  const [picked, setPicked] = useState<string | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const selected = picked !== null && keys.includes(picked) ? picked : null;
  const select = useCallback((key: string | null) => {
    setPicked(key);
    // Under a narrow map the panel may be out of view; it comes into it (B34).
    if (key) requestAnimationFrame(() => panel.current?.scrollIntoView?.({ block: "nearest" }));
  }, []);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || event.defaultPrevented || !selected) return;
    event.preventDefault();
    setPicked(null);
  };
  const onMapClick = (event: MouseEvent<HTMLDivElement>) => {
    // A checker's popover is portalled out of the map, yet its presses bubble here through React.
    if (!event.currentTarget.contains(event.target as Node)) return;
    if (!(event.target as Element).closest(cardSelector)) setPicked(null);
  };
  return { selected, select, panel, onKeyDown, onMapClick };
}
