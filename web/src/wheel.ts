// The wheel policy behind pane scrolling (PRD S2 B19): whole rows per
// event with the pixel remainder
// carried, and the crossterm modifier bitset.

/** The crossterm modifier bitset the engine's terminal.scroll takes, and the core reads for a click. */
export function pointerModifiers(event: { shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean }): number {
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

/**
 * Whole rows one wheel event moves the pane, with the pixel remainder kept
 * for the next event. A trackpad reports pixels (`deltaMode` 0) that add up
 * across events; a wheel notch reports lines and always moves at least one
 * row rather than rounding away.
 */
export function wheelRows(
  delta: number,
  deltaMode: number,
  rowHeight: number,
  remainder: number,
): { rows: number; remainder: number } {
  if (deltaMode !== 0) {
    const rounded = Math.round(delta);
    const rows = rounded !== 0 ? rounded : delta > 0 ? 1 : delta < 0 ? -1 : 0;
    return { rows, remainder: 0 };
  }
  if (rowHeight <= 0) return { rows: 0, remainder };
  const total = remainder + delta;
  const rows = Math.trunc(total / rowHeight);
  return { rows, remainder: total - rows * rowHeight };
}
