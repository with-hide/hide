// A pull request of a session's list as the core lays it on a row
// (`agent_state::work::family`), for tests that draw one.
import type { PrState, SessionPull } from "../src/snapshot";

export function sessionPull(number: number, state: PrState, extra: Partial<SessionPull> = {}): SessionPull {
  return { url: `https://example.invalid/pull/${number}`, number, title: `PR ${number}`, state, own: true, holders: [], stale: false, last_read_at_unix_ms: null, ...extra };
}
