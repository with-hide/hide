// Which rows of a pane's screen soft-wrap, as the pane's holder says them.
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
// it says nothing, and a grid resized here before the holder's next full
// frame no longer matches the size it was said for.

/** The private OSC the holder writes. */
export const WRAP_OSC = 7719;

/** The wraps one frame said, for the grid it was drawn at. */
export type Wraps = { cols: number; rows: number; bits: Uint8Array };

/** The wraps in an OSC's data, or null when it is not one the holder writes. */
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
 * soft-wraps onto the next, or null when the wraps are not known for a grid
 * of `cols` by `rows`.
 */
export function wrapsOf(wraps: Wraps | null, cols: number, rows: number, top: number, bottom: number): boolean[] | null {
  if (!wraps || wraps.cols !== cols || wraps.rows !== rows) return null;
  const answer: boolean[] = [];
  for (let row = top; row <= bottom; row += 1) answer.push(wraps.bits[row] === 1);
  return answer;
}

/** The part of xterm's parser the wraps are read through. */
export type OscParser = { registerOscHandler(ident: number, callback: (data: string) => boolean | Promise<boolean>): { dispose(): void } };

type Followed = { parser: OscParser };

const followed = new WeakMap<Followed, { wraps: Wraps | null }>();

/** Reads the wraps a terminal is sent from now on, as its parser meets them in the stream. */
export function followWraps(term: Followed): void {
  const state: { wraps: Wraps | null } = { wraps: null };
  followed.set(term, state);
  term.parser.registerOscHandler(WRAP_OSC, (data) => {
    state.wraps = parseWraps(data);
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
  if (state) state.wraps = null;
}
