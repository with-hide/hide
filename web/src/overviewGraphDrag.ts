// The Overview work map's three drag units (PRD overview-main-map D-18, B22,
// B23): a project's band or name moves the project, a ring session or its
// team's outline moves the team, and any other session moves alone. Offsets
// store where something was put, never a step, so repeated drag events of one
// gesture land in the same place; clearing them restores the layout.

type Point = { x: number; y: number };
export type Offsets = ReadonlyMap<string, Point>;
/** A session's place from the layout: its box's top left, whether it stands on the ring, and its team's ring session. */
export type Placed = Point & { ring: boolean; team: string };

/** The React Flow id of a team's outline, and the key its drag is stored under. */
export const teamKey = (team: string) => `team:${team}`;
const read = (offsets: Offsets, key: string) => offsets.get(key) ?? { x: 0, y: 0 };

/** Where a session stands, its project's move left out: its place, moved by its own drag and by its team's. */
export function sessionPlace(id: string, placed: Placed, offsets: Offsets): Placed {
  const own = read(offsets, id);
  const team = read(offsets, teamKey(placed.team));
  return { ...placed, x: placed.x + own.x + team.x, y: placed.y + own.y + team.y };
}

export type Dragged =
  /** A project or team outline: `base` is where it stands before any drag of its own. */
  | { kind: "project" | "team"; id: string; base: Point }
  /** A session: its place from the layout, with `frame` the top left of the project box it is drawn in. */
  | { kind: "session"; id: string; placed: Placed; frame: Point };

/** The offsets after `dragged` is dropped at `position`, in the coordinates React Flow reports for it. */
export function dropAt(offsets: Offsets, dragged: Dragged, position: Point): Map<string, Point> {
  const next = new Map(offsets);
  if (dragged.kind !== "session") {
    next.set(dragged.id, { x: position.x - dragged.base.x, y: position.y - dragged.base.y });
    return next;
  }
  const target = { x: position.x + dragged.frame.x, y: position.y + dragged.frame.y };
  const own = read(offsets, dragged.id);
  const team = read(offsets, teamKey(dragged.placed.team));
  // A ring session carries its team: the team's offset moves, its own stays.
  if (dragged.placed.ring && dragged.placed.team === dragged.id) next.set(teamKey(dragged.id), { x: target.x - dragged.placed.x - own.x, y: target.y - dragged.placed.y - own.y });
  else next.set(dragged.id, { x: target.x - dragged.placed.x - team.x, y: target.y - dragged.placed.y - team.y });
  return next;
}
