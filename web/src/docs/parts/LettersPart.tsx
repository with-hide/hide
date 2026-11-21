// A letter between two agents: the three marks the product wears for letters, then one letter line that is sent
// and then answered. Without `state=` the two lines arrive in turn; under reduced motion, or with `state=`, the
// picture holds still (`sent` shows the first line, `received` both).
import { useEffect, useState } from "react";
import { LetterTag, PaneLetterButton, RowLetterMark } from "../../components/letter-mark";
import { MentionChip } from "../../components/mention-chip";
import { useReducedMotion } from "../../lib/useReducedMotion";
import { PartFrame } from "../frame";
import type { PartProps } from "../renderers";

const STEP_MS = 1800;
const HOLD_MS = 2600;

// The docs' own sample words, English only; the product's strings go through the translator, these are invented values.
const WHERE = { row: "On a task row: a letter is waiting there.", header: "In a pane header: press it to hand the letters over." };

const LINES = [
  { key: "sent", glyph: "↗", tone: "text-agent-working", from: "api-refactor", to: "reviewer", text: "Please review the new orders endpoint." },
  { key: "received", glyph: "↙", tone: "text-success", from: "reviewer", to: "api-refactor", text: "Looks good. One question about paging." },
] as const;

export default function LettersPart({ state }: PartProps) {
  const reduced = useReducedMotion();
  const held = state === "sent" ? 1 : state === "received" || reduced ? 2 : null;
  const [shown, setShown] = useState(held ?? 1);
  useEffect(() => {
    if (held !== null) return;
    const timer = window.setTimeout(() => setShown((count) => (count === 1 ? 2 : 1)), shown === 1 ? STEP_MS : HOLD_MS);
    return () => window.clearTimeout(timer);
  }, [held, shown]);
  const count = held ?? shown;
  return (
    <PartFrame part="letters">
      <div className="flex flex-col gap-lg">
        <ul className="flex flex-col gap-sm text-caption text-muted-foreground">
          <li className="flex items-center gap-sm"><RowLetterMark /><span>{WHERE.row}</span></li>
          <li className="flex items-center gap-sm"><PaneLetterButton paneId="docs-letters" letters={2} onDeliver={() => undefined} /><span>{WHERE.header}</span></li>
        </ul>
        <ol className="flex flex-col gap-xs rounded-md border border-border bg-popover p-sm text-caption text-popover-foreground" data-docs-letters={count}>
          {LINES.slice(0, count).map(({ key, glyph, tone, from, to, text }) => (
            <li key={key} className="flex min-w-0 items-baseline gap-xs" data-docs-letter={key}>
              <span aria-hidden="true" className={`w-(--size-icon-sm) shrink-0 text-center ${tone}`}>{glyph}</span>
              <LetterTag label={`${from} to ${to}`} who={<MentionChip kind="session" name={to} />} className="text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate">{text}</span>
            </li>
          ))}
        </ol>
      </div>
    </PartFrame>
  );
}
