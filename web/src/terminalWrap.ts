// Which rows of a pane's screen run on into the next, as the pane's holder says them.
//
// The holder places every cell itself with autowrap off, so xterm never
// marks a row `isWrapped`, and a row whose text ran past the last column
// looks the same as one that filled it and ended with a line break. The
// holder says the difference in its own escape sequence (`WRAP_OSC` in
// `hide-holder/src/encode.rs`): `ESC ] 7719 ; <cols> ; <rows> ; <hex> ESC \`,
// where hex digit `k` holds rows `4k` to `4k + 3`, row `4k + j` in bit
// `1 << j`, trailing zero digits left out. Every full frame carries it and a
// delta carries it again only when the wraps changed, so it costs nothing
// per frame that did not move them. A program's own output never reaches
// this stream, since the holder draws frames from what it parsed.
//
// The answer is unknown (null) until a frame said it: a holder built before
// it says nothing, a Windows holder never says it (ConPTY places a wrapped
// line's rows itself and does not reliably leave a soft wrap), and a grid
// resized here before the holder's next full frame no longer matches the
// size it was said for.
//
// The holder also says, on every system and in the same form
// (`LINK_END_OSC`), which rows end with a hyperlink the next row does not
// carry on: the program marked a link that stops at the row's edge. That is
// the fact an agent's pane needs. Agent TUIs break their own long lines with
// line breaks (Claude Code's renderer hard-wraps), so no row of theirs
// soft-wraps, and a URL one broke across rows looks like a URL that filled
// its row followed by an unrelated word; but an agent that marks its links
// (Claude Code once the pane says it shows them, `FORCE_HYPERLINK=1`, and
// Codex always) puts the whole address on every piece, so a broken URL runs
// on and a full one ends (hide-private issue 591).
//
// `paneRowWraps` is the one answer to whether a pane's row runs on into the
// next, for its links (`terminalLinks.ts`) and its copy (`selection.ts`)
// alike.

import { agentsByPane, allAgents } from "./paneAgents";
import { useShellStore } from "./store";

/** The private OSC the holder writes for its soft wraps. */
export const WRAP_OSC = 7719;

/** The private OSC the holder writes for the rows that end with a link. */
export const LINK_END_OSC = 7720;

/** One bit per row one frame said, for the grid it was drawn at. */
export type Wraps = { cols: number; rows: number; bits: Uint8Array };

/** The row bits in an OSC's data, or null when it is not one the holder writes. */
export function parseWraps(data: string): Wraps | null {
  const match = /^(\d{1,4});(\d{1,4});([0-9a-f]*)$/.exec(data);
  if (!match) return null;
  const cols = Number(match[1]);
  const rows = Number(match[2]);
  const hex = match[3]!;
  if (hex.length > Math.ceil(rows / 4)) return null;
  const bits = new Uint8Array(rows);
  for (let digit = 0; digit < hex.length; digit += 1) {
    const value = parseInt(hex[digit]!, 16);
    for (let bit = 0; bit < 4; bit += 1) {
      const row = digit * 4 + bit;
      if (value & (1 << bit) && row < rows) bits[row] = 1;
    }
  }
  return { cols, rows, bits };
}

/**
 * Whether each of the screen rows `top` to `bottom` (zero-based, inclusive)
 * has its bit set (it soft-wraps onto the next, or its link ends there), or
 * null when the bits are not known for a grid of `cols` by `rows`.
 */
export function wrapsOf(wraps: Wraps | null, cols: number, rows: number, top: number, bottom: number): boolean[] | null {
  if (!wraps || wraps.cols !== cols || wraps.rows !== rows) return null;
  const answer: boolean[] = [];
  for (let row = top; row <= bottom; row += 1) answer.push(wraps.bits[row] === 1);
  return answer;
}

/**
 * Whether each screen row `top` to `bottom` (zero-based, inclusive) of the
 * pane runs on into the next: true where the holder said it soft-wraps,
 * false where it said the row's link ends there or, in a pane with no agent,
 * that it ends with a line break, and null where the last-column rule holds
 * instead; null as a whole when the holder said neither for this grid. The
 * pane keeps no scrollback, so a buffer row is the screen row the holder
 * named. An agent's own line break is no end: its TUI breaks a long line
 * itself, so only the link it marked says where its URL stops.
 */
export function paneRowWraps(term: Followed & { cols: number; rows: number }, paneId: string, top: number, bottom: number): (boolean | null)[] | null {
  const state = followed.get(term);
  const soft = wrapsOf(state?.wraps ?? null, term.cols, term.rows, top, bottom);
  const ends = wrapsOf(state?.linkEnds ?? null, term.cols, term.rows, top, bottom);
  if (!soft && !ends) return null;
  const { agents, rest } = useShellStore.getState();
  const agent = agentsByPane(allAgents(agents, rest?.status?.remote)).has(paneId);
  return Array.from({ length: bottom - top + 1 }, (_, row) => {
    if (soft?.[row]) return true;
    if (ends?.[row]) return false;
    return soft && !agent ? false : null;
  });
}

/** The part of xterm's parser the wraps are read through. */
export type OscParser = { registerOscHandler(ident: number, callback: (data: string) => boolean | Promise<boolean>): { dispose(): void } };

type Followed = { parser: OscParser };

type Said = { wraps: Wraps | null; linkEnds: Wraps | null };

const followed = new WeakMap<Followed, Said>();

/** Reads the row bits a terminal is sent from now on, as its parser meets them in the stream. */
export function followWraps(term: Followed): void {
  const state: Said = { wraps: null, linkEnds: null };
  followed.set(term, state);
  term.parser.registerOscHandler(WRAP_OSC, (data) => {
    state.wraps = parseWraps(data);
    return true;
  });
  term.parser.registerOscHandler(LINK_END_OSC, (data) => {
    state.linkEnds = parseWraps(data);
    return true;
  });
}

/** The wraps the terminal was last sent, or null. */
export function terminalWraps(term: Followed): Wraps | null {
  return followed.get(term)?.wraps ?? null;
}

/** Forgets them, as a terminal reset forgets its screen. */
export function forgetWraps(term: Followed): void {
  const state = followed.get(term);
  if (state) {
    state.wraps = null;
    state.linkEnds = null;
  }
}
