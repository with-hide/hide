// The four marks a task row wears, each with the one word it means. The marks are the product's own `StatusMark`.
import { StatusMark } from "../../components/status-mark";
import type { RowMark } from "../../snapshot";
import { PartFrame } from "../frame";

const MARKS: { mark: RowMark; word: string; means: string }[] = [
  { mark: "turn", word: "Your turn", means: "an agent asks you something" },
  { mark: "done", word: "Done", means: "finished, and you have not looked yet" },
  { mark: "going", word: "Working", means: "an agent is at work" },
  { mark: "quiet", word: "Seen", means: "you looked, nothing is waiting" },
];

export default function StatusColorsPart() {
  return (
    <PartFrame part="status-colors">
      <ul className="grid grid-cols-[auto_auto_1fr] items-center gap-x-md gap-y-sm">
        {MARKS.map(({ mark, word, means }) => (
          <li key={mark} className="contents" data-docs-mark={mark}>
            <StatusMark mark={mark} />
            <span className="text-body text-foreground">{word}</span>
            <span className="min-w-0 text-caption text-muted-foreground">{means}</span>
          </li>
        ))}
      </ul>
    </PartFrame>
  );
}
