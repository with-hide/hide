// An agent row for a part that draws one product component from the few fields it reads. The cast is the one
// place a fixture says "this is a row": each part names only the facts its component shows, and `pnpm tsc`
// still checks every field it does name against the wire type.
import type { AgentRow } from "../../snapshot";

export const NOW = Date.now();
export const minutesAgo = (minutes: number) => NOW - minutes * 60_000;

export function docsAgent(paneId: string, fields: Partial<AgentRow>): AgentRow {
  return { id: paneId, pane_id: paneId, device_id: "local", device_pane_id: paneId, agent_kind: "claude", ...fields } as AgentRow;
}
