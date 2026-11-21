// The task row the docs teach first: one row that moves from working to a person's turn to done, drawn by the
// sidebar's own `SessionRow`. With `state=` it holds that state; without it, it plays the three in turn, and under
// reduced motion it holds the last one.
import { useEffect, useState } from "react";
import { AgentMark } from "../../AgentMark";
import { PrMark } from "../../components/pr-mark";
import { SessionRow } from "../../components/session-row";
import { StatusMark } from "../../components/status-mark";
import { useReducedMotion } from "../../lib/useReducedMotion";
import { PartFrame } from "../frame";
import { PART_STATES } from "../parts";
import type { PartProps } from "../renderers";

const STATES = PART_STATES["task-row"];
type TaskRowState = (typeof STATES)[number];
const STEP_MS = 2200;

const time = (text: string) => <span className="font-mono text-caption text-muted-foreground">{text}</span>;

export default function TaskRowPart({ state }: PartProps) {
  const held = state === null ? null : STATES.indexOf(state as TaskRowState);
  const reduced = useReducedMotion();
  const still = held !== null || reduced;
  const [step, setStep] = useState(held ?? (reduced ? STATES.length - 1 : 0));
  useEffect(() => {
    if (still) return;
    const timer = window.setInterval(() => setStep((current) => (current + 1) % STATES.length), STEP_MS);
    return () => window.clearInterval(timer);
  }, [still]);
  const current = still ? held ?? STATES.length - 1 : step;
  const shown = STATES[current]!;
  return (
    <PartFrame part="task-row">
      <ol data-docs-state={shown}>
        {shown === "working" && <SessionRow mark={<><StatusMark mark="going" /><AgentMark kind="claude" /></>} title="api-refactor" right={time("12m")} line={{ text: "editing 3 files" }} />}
        {shown === "question" && <SessionRow mark={<><StatusMark mark="turn" /><AgentMark kind="claude" /></>} title="api-refactor" right={time("14m")} line={{ text: "asks: keep the old endpoint for one release?", className: "text-warning", wrap: true }} />}
        {shown === "done" && <SessionRow mark={<><StatusMark mark="done" /><AgentMark kind="claude" /></>} title="api-refactor" chips={<PrMark state="mergeable" number={412} />} right={time("21m")} line={{ text: "tests pass, PR opened" }} />}
      </ol>
    </PartFrame>
  );
}
