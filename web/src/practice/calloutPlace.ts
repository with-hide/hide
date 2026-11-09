// Where a callout's tag goes: one column beside what it names, each tag level
// with its target and pushed down just far enough not to touch the one above.
// A pure function of heights, so a test can state it.

/** The top of each tag for targets at `anchors` (their vertical middles), in the order given. */
export function placeCallouts(anchors: number[], height: number, gap: number, view: { top: number; bottom: number }): number[] {
  const order = anchors.map((_, at) => at).sort((a, b) => anchors[a]! - anchors[b]!);
  const tops = new Map<number, number>();
  let floor = view.top;
  for (const at of order) {
    const top = Math.max(anchors[at]! - height / 2, floor);
    tops.set(at, top);
    floor = top + height + gap;
  }
  // Pushed past the bottom, the column moves back up as one, keeping its order and gaps.
  const overflow = floor - gap - view.bottom;
  const lift = overflow > 0 ? Math.min(overflow, Math.max(0, (tops.get(order[0]!) ?? view.top) - view.top)) : 0;
  return anchors.map((_, at) => Math.round(tops.get(at)! - lift));
}
