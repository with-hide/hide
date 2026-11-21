import type { LettersSince } from "./snapshot";

/** How many agents the pane worked with since the request, those past the listed included. */
export function peerCount(letters: LettersSince | undefined): number {
  return letters ? letters.peers.length + letters.more_peers : 0;
}

/** Letters the whole list counts but does not list: older than the newest it carries. */
export function lettersBefore(letters: LettersSince | undefined): number {
  return letters ? Math.max(0, letters.total - letters.recent.length) : 0;
}
