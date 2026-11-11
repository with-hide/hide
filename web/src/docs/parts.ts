// The ten parts the user docs frame, and the `state=` each one accepts. The ids are a contract with the docs pages
// (`/embed/docs-embed.html?part=<id>`); `renderers.ts` must carry a renderer for every one, and `parts.test.ts` fails
// when the two lists drift apart.

export const PART_STATES = {
  "task-row": ["working", "question", "done"],
  "status-colors": [],
  "turn-card": ["question", "answer"],
  "pane-header": ["working", "asking"],
  "pr-card": [],
  "view-slot": [],
  "memory-cards": [],
  "mission-map": [],
  letters: ["sent", "received"],
  phone: [],
} as const satisfies Record<string, readonly string[]>;

export type PartId = keyof typeof PART_STATES;
export const PARTS = Object.keys(PART_STATES) as PartId[];

/** What one part reads from the query string: the theme, the language, and an optional state to hold. */
export type PartQuery = { part: PartId; theme: "dark" | "light"; state: string | null };

export function isPartId(value: string): value is PartId {
  return Object.hasOwn(PART_STATES, value);
}

/** Read `?part=&theme=&state=`; an unknown part, theme or state is an error that names what is known. */
export function readPartQuery(params: URLSearchParams): PartQuery {
  const part = params.get("part") ?? "";
  if (!isPartId(part)) throw new Error(`Unknown docs part "${part}"; known: ${PARTS.join(", ")}`);
  const theme = params.get("theme") ?? "dark";
  if (theme !== "dark" && theme !== "light") throw new Error(`Unknown docs theme "${theme}"; known: dark, light`);
  const state = params.get("state");
  const known: readonly string[] = PART_STATES[part];
  if (state !== null && !known.includes(state)) {
    throw new Error(`Unknown ${part} state "${state}"; known: ${known.length > 0 ? known.join(", ") : "none"}`);
  }
  return { part, theme, state };
}
