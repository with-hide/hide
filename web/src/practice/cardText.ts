// A card's words as short lines instead of one paragraph. The catalog holds plain
// text with three marks: a line is a line, "- " opens a list item, "> " quotes
// an example the learner would say, and **words** are the ones to remember.

export type CardPart = { text: string; bold: boolean };
export type CardLine = { kind: "line" | "item" | "quote"; parts: CardPart[] };

const parts = (text: string): CardPart[] => text.split("**").flatMap((piece, at) => (piece ? [{ text: piece, bold: at % 2 === 1 }] : []));

/** One card text as lines of parts, each line's mark cut off and each `**` pair as bold. */
export function cardLines(text: string): CardLine[] {
  return text.split("\n").filter((line) => line.trim()).map((line) => {
    if (line.startsWith("- ")) return { kind: "item", parts: parts(line.slice(2)) };
    if (line.startsWith("> ")) return { kind: "quote", parts: parts(line.slice(2)) };
    return { kind: "line", parts: parts(line) };
  });
}
