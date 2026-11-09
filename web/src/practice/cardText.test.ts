import { describe, expect, it } from "vitest";
import { cardLines } from "./cardText";

describe("cardLines", () => {
  it("cuts a text into lines, items and a quote, with bold words marked", () => {
    expect(cardLines("**A** then b\n- one **two**\n> say this\n\nlast")).toEqual([
      { kind: "line", parts: [{ text: "A", bold: true }, { text: " then b", bold: false }] },
      { kind: "item", parts: [{ text: "one ", bold: false }, { text: "two", bold: true }] },
      { kind: "quote", parts: [{ text: "say this", bold: false }] },
      { kind: "line", parts: [{ text: "last", bold: false }] },
    ]);
  });
});
