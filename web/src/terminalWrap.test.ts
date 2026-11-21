import { describe, expect, it } from "vitest";
import type { AgentRow } from "./snapshot";
import { useShellStore } from "./store";
import { followWraps, forgetWraps, LINK_END_OSC, paneRowWraps, parseWraps, terminalWraps, WRAP_OSC, wrapsOf, type OscParser } from "./terminalWrap";

describe("parseWraps", () => {
  it("reads which rows soft-wrap from the holder's digits", () => {
    // Rows 0, 1 and 9 of a 4x12 screen, as hide-holder's encoder writes them.
    const wraps = parseWraps("4;12;302")!;
    expect(wraps.cols).toBe(4);
    expect(wraps.rows).toBe(12);
    expect([...wraps.bits].flatMap((bit, row) => (bit ? [row] : []))).toEqual([0, 1, 9]);
    expect([...parseWraps("29;4;")!.bits]).toEqual([0, 0, 0, 0]);
  });

  it("refuses data the holder does not write", () => {
    expect(parseWraps("")).toBeNull();
    expect(parseWraps("29;4")).toBeNull();
    expect(parseWraps("29;4;G")).toBeNull();
    // More digits than the rows hold.
    expect(parseWraps("29;4;11")).toBeNull();
  });
});

describe("wrapsOf", () => {
  it("answers only for the grid the wraps were said for", () => {
    const wraps = parseWraps("29;4;1");
    expect(wrapsOf(wraps, 29, 4, 0, 2)).toEqual([true, false, false]);
    expect(wrapsOf(wraps, 30, 4, 0, 2)).toBeNull();
    expect(wrapsOf(wraps, 29, 5, 0, 2)).toBeNull();
    expect(wrapsOf(null, 29, 4, 0, 2)).toBeNull();
  });
});

/** A 29x4 terminal following its stream; `say` feeds one OSC to it. */
function followed() {
  const handlers = new Map<number, (data: string) => boolean | Promise<boolean>>();
  const parser: OscParser = {
    registerOscHandler(ident, callback) {
      handlers.set(ident, callback);
      return { dispose() {} };
    },
  };
  const term = { parser, cols: 29, rows: 4 };
  followWraps(term);
  return { term, say: (ident: number, data: string) => handlers.get(ident)!(data) };
}

describe("followWraps", () => {
  it("keeps the last row bits the terminal's stream carried until it is reset", () => {
    const { term, say } = followed();
    expect(terminalWraps(term)).toBeNull();
    expect(say(WRAP_OSC, "29;4;1")).toBe(true);
    expect(say(LINK_END_OSC, "29;4;2")).toBe(true);
    expect(terminalWraps(term)?.bits[0]).toBe(1);
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toEqual([true, false, false, false]);
    forgetWraps(term);
    expect(terminalWraps(term)).toBeNull();
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toBeNull();
  });
});

// hide-private issues 312 and 591: whether a row runs on into the next.
describe("paneRowWraps", () => {
  it("reads a shell's line break as an end and an agent's as unknown, and a link end as an end in both", async () => {
    const { term, say } = followed();
    say(WRAP_OSC, "29;4;1");
    say(LINK_END_OSC, "29;4;4");
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toEqual([true, false, false, false]);
    const before = useShellStore.getState().agents;
    useShellStore.setState({ agents: [{ pane_id: "w1:p1" } as AgentRow] });
    try {
      expect(paneRowWraps(term, "w1:p1", 0, 3)).toEqual([true, null, false, null]);
    } finally {
      useShellStore.setState({ agents: before });
    }
  });

  it("knows only the link ends where the holder says no soft wraps, and nothing for another grid", () => {
    const { term, say } = followed();
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toBeNull();
    say(LINK_END_OSC, "29;4;4");
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toEqual([null, null, false, null]);
    say(LINK_END_OSC, "30;4;4");
    expect(paneRowWraps(term, "w1:p1", 0, 3)).toBeNull();
  });
});
