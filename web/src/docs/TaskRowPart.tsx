// The task row the docs teach first: one row that moves from working to a person's turn to done, drawn by the
// sidebar's own `SessionRow`. With `state=` it holds that state; without it, it plays the three in turn, and under
// reduced motion it holds the last one.
import { useEffect, useLayoutEffect, useState } from "react";
import { AgentMark } from "../AgentMark";
import { PrMark } from "../components/pr-mark";
import { SessionRow } from "../components/session-row";
import { StatusMark } from "../components/status-mark";
import { TooltipProvider } from "../components/ui/tooltip";

export const TASK_ROW_STATES = ["working", "question", "done"] as const;
export type TaskRowState = (typeof TASK_ROW_STATES)[number];
const STEP_MS = 2200;

const time = (text: string) => <span className="font-mono text-caption text-muted-foreground">{text}</span>;

export function DocsTaskRow({ params, state }: { params: URLSearchParams; state: TaskRowState | null }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const still = state !== null || window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const [step, setStep] = useState(still && state === null ? TASK_ROW_STATES.length - 1 : state ? TASK_ROW_STATES.indexOf(state) : 0);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme]);
  useEffect(() => {
    if (still) return;
    const timer = window.setInterval(() => setStep((current) => (current + 1) % TASK_ROW_STATES.length), STEP_MS);
    return () => window.clearInterval(timer);
  }, [still]);
  const current = TASK_ROW_STATES[step]!;
  return (
    <TooltipProvider>
      <div className="flex min-w-0 flex-col gap-xs bg-background p-lg text-body text-foreground" data-docs-part="task-row" data-docs-state={current}>
        <ol>
          {current === "working" && <SessionRow mark={<><StatusMark mark="going" /><AgentMark kind="claude" /></>} title="api-refactor" right={time("12m")} line={{ text: "editing 3 files" }} />}
          {current === "question" && <SessionRow mark={<><StatusMark mark="turn" /><AgentMark kind="claude" /></>} title="api-refactor" right={time("14m")} line={{ text: "asks: keep the old endpoint for one release?", className: "text-warning", wrap: true }} />}
          {current === "done" && <SessionRow mark={<><StatusMark mark="done" /><AgentMark kind="claude" /></>} title="api-refactor" chips={<PrMark state="mergeable" number={412} />} right={time("21m")} line={{ text: "tests pass, PR opened" }} />}
        </ol>
      </div>
    </TooltipProvider>
  );
}
