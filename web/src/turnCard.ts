import type { AskedTurn, LettersSince } from "./snapshot";

/**
 * The questions later turns put to the operator, in time order, drawn under
 * the letters. A question that ended the newest turn is left out while the
 * card's last line already shows it as the ask (`askShown`).
 */
export function askedLines(asked: readonly AskedTurn[] | undefined, askShown: boolean): AskedTurn[] {
  return (asked ?? []).filter((one) => !(one.current && askShown)).sort((a, b) => a.at_unix_ms - b.at_unix_ms);
}

/** How many agents the pane worked with since the request, those past the listed included. */
export function peerCount(letters: LettersSince | undefined): number {
  return letters ? letters.peers.length + letters.more_peers : 0;
}

/** Letters the whole list counts but does not list: older than the newest it carries. */
export function lettersBefore(letters: LettersSince | undefined): number {
  return letters ? Math.max(0, letters.total - letters.recent.length) : 0;
}
