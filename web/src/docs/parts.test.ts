import { describe, expect, it } from "vitest";
import { PART_STATES, PARTS, readPartQuery } from "./parts";
import { RENDERERS } from "./renderers";

// The ids are a contract with the docs pages that frame `?part=<id>`: a part with no renderer would be an
// empty frame, and a renderer no page can name is dead code.
describe("docs parts", () => {
  it("has a renderer for every part id and a part id for every renderer", () => {
    expect(Object.keys(RENDERERS).sort()).toEqual([...PARTS].sort());
  });

  it("lists the ten parts the docs pages name", () => {
    expect([...PARTS].sort()).toEqual(["letters", "memory-cards", "mission-map", "pane-header", "phone", "pr-card", "status-colors", "task-row", "turn-card", "view-slot"]);
  });

  it("reads the part, the theme and the state from the query", () => {
    expect(readPartQuery(new URLSearchParams("part=task-row&state=done&theme=light"))).toEqual({ part: "task-row", theme: "light", state: "done" });
    expect(readPartQuery(new URLSearchParams("part=phone"))).toEqual({ part: "phone", theme: "dark", state: null });
  });

  it("names what is known when the part, the state or the theme is not", () => {
    expect(() => readPartQuery(new URLSearchParams("part=nope"))).toThrow(/Unknown docs part "nope"; known: task-row/);
    expect(() => readPartQuery(new URLSearchParams(""))).toThrow(/Unknown docs part ""/);
    expect(() => readPartQuery(new URLSearchParams("part=task-row&state=asleep"))).toThrow(/Unknown task-row state "asleep"; known: working, question, done/);
    expect(() => readPartQuery(new URLSearchParams("part=phone&state=any"))).toThrow(/Unknown phone state "any"; known: none/);
    expect(() => readPartQuery(new URLSearchParams("part=task-row&theme=sepia"))).toThrow(/Unknown docs theme "sepia"/);
  });

  it("accepts every state a part declares", () => {
    for (const part of PARTS) for (const state of PART_STATES[part]) expect(readPartQuery(new URLSearchParams({ part, state })).state).toBe(state);
  });
});
