// Where a pane's screen stands in its history, as the pane's holder says it.
//
// The pane keeps no scrollback: the holder redraws its viewport in place, so
// the screen alone cannot say which history row a screen row is, or whether
// the program takes the pointer. The holder says both in its own escape
// sequence (`VIEW_OSC` in `hide-holder/src/encode.rs`):
// `ESC ] 7721 ; <top> ; <bottom> ; <flags> ESC \`, the history row the
// screen's top row is and the most it can be, counted from the oldest row the
// holder keeps, and flags bit 0 when the program takes pointer events and
// bit 1 while the alternate screen shows. Every full frame carries it and a
// delta carries it again only when it moved. A program's own output never
// reaches this stream, since the holder draws frames from what it parsed.
//
// The answer is unknown (null) until a frame said it: a holder built before
// it says nothing.

/** The private OSC the holder writes for where its screen stands. */
export const VIEW_OSC = 7721;

/** Where the screen stands and what the program asked of the pointer. */
export type View = { top: number; bottom: number; mouse: boolean; alternate: boolean };

/** The view in an OSC's data, or null when it is not one the holder writes. */
export function parseView(data: string): View | null {
  const match = /^(\d{1,20});(\d{1,20});([0-3])$/.exec(data);
  if (!match) return null;
  const top = Number(match[1]);
  const bottom = Number(match[2]);
  if (!Number.isSafeInteger(top) || !Number.isSafeInteger(bottom) || top > bottom) return null;
  const flags = Number(match[3]);
  return { top, bottom, mouse: (flags & 1) !== 0, alternate: (flags & 2) !== 0 };
}

/** The part of xterm's parser the view is read through. */
type Followed = { parser: { registerOscHandler(ident: number, callback: (data: string) => boolean | Promise<boolean>): { dispose(): void } } };

const followed = new WeakMap<Followed, { view: View | null }>();

/** Reads the view a terminal is sent from now on, as its parser meets it in the stream. */
export function followView(term: Followed): void {
  const state = { view: null as View | null };
  followed.set(term, state);
  term.parser.registerOscHandler(VIEW_OSC, (data) => {
    state.view = parseView(data);
    return true;
  });
}

/** The view the terminal was last sent, or null. */
export function terminalView(term: Followed): View | null {
  return followed.get(term)?.view ?? null;
}

/** Forgets it, as a terminal reset forgets its screen. */
export function forgetView(term: Followed): void {
  const state = followed.get(term);
  if (state) state.view = null;
}
