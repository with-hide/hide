// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createActions } from "./actions";
import { RunSummary, RunSummaryCard } from "./AutomationRunSummary";
import { TooltipProvider } from "./components/ui/tooltip";
import type { Automation, AutomationRecord, AutomationRunResult } from "./snapshot";
import { useShellStore } from "./store";

// Operator choice 2026-11-24: a run's summary lists what its agent opened (결과); a finished run that opened nothing draws no
// section, a run going keeps the heading and one quiet line, and without a tab on the run's desk the rows are shown, not pressed.

// The shell's modules reach xterm, which asks jsdom for a canvas it lacks.
vi.hoisted(() => {
  HTMLCanvasElement.prototype.getContext = () => null;
});

type Run = Extract<AutomationRecord, { kind: "run" }>;
const RESULTS: AutomationRunResult[] = [
  { key: "k-report", kind: "file", label: "report.md" },
  { key: "k-page", kind: "browser", label: "Issues" },
];
const run = (over: Partial<Run> = {}): Run => ({
  kind: "run", number: 1, trigger: "manual", started_at_unix_ms: Date.now() - 60_000, state: "succeeded", ended_at_unix_ms: Date.now() - 30_000,
  attempt: 1, opened: false, resumable: true, conversation_missing: false, seen: true, answer: "끝났어요", ...over,
});
const automationOf = (record: Run): Automation => ({
  id: "auto-1", name: "보고서", task: "보고서를 열어 줘", schedule: { kind: "daily", time: "09:00" }, reads_as: { kind: "daily", time: "09:00" }, next_run_at_unix_ms: null,
  agent: { kind: "claude" }, folder: { path: "/srv/a", default: true, worktree: false }, running: false, unseen: 0, records: [record],
}) as Automation;

let container: HTMLDivElement;
let root: Root;
const sent = vi.fn(() => true);
const saved = useShellStore.getState();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} unobserve() {} });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  sent.mockClear();
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  useShellStore.setState(saved, true);
  vi.unstubAllGlobals();
});

async function draw(node: React.ReactNode) {
  await act(async () => {
    useShellStore.setState({ connection: "live", rest: {} } as never);
    root.render(<TooltipProvider>{node}</TooltipProvider>);
  });
}

it("draws no results section for a finished run that opened nothing", async () => {
  const record = run();
  await draw(<RunSummary automation={automationOf(record)} run={record} going={false} actions={createActions(sent)} pressable={false} />);
  expect(container.querySelector("[data-automation-run-summary]")).not.toBeNull();
  expect(container.querySelector("[data-automation-run-results]")).toBeNull();
});

it("keeps the results heading and its quiet line for a run that goes and opened nothing yet", async () => {
  const record = run({ state: "running", ended_at_unix_ms: undefined, answer: undefined });
  await draw(<RunSummary automation={automationOf(record)} run={record} going actions={createActions(sent)} pressable={false} />);
  expect(container.querySelector("[data-automation-run-results='0']")).not.toBeNull();
  expect(container.querySelector("[data-automation-run-results-empty]")).not.toBeNull();
});

it("shows the rows of a run with no tab on its desk without a tab strip, and presses none of them", async () => {
  const record = run({ results: RESULTS });
  await draw(<RunSummary automation={automationOf(record)} run={record} going={false} actions={createActions(sent)} pressable={false} />);
  expect([...container.querySelectorAll("[data-automation-run-result]")].map((row) => row.textContent)).toEqual(["report.md", "Issues"]);
  expect(container.querySelector("[data-automation-run-result-open]")).toBeNull();
  expect(container.querySelector('[role="tab"]')).toBeNull();
  await act(async () => container.querySelector<HTMLElement>("[data-automation-run-result]")!.click());
  expect(sent).not.toHaveBeenCalled();
});

it("sends one typed event naming the run and the row when a row of the summary card is pressed", async () => {
  const record = run({ results: RESULTS });
  useShellStore.setState({ automations: { items: [automationOf(record)], unseen: 0 } } as never);
  await draw(<RunSummaryCard automation="auto-1" run={1} actions={createActions(sent)} />);
  await act(async () => container.querySelector<HTMLElement>('[data-automation-run-result-open="k-page"]')!.click());
  expect(sent).toHaveBeenCalledTimes(1);
  expect(sent).toHaveBeenCalledWith({ schema_version: 2, kind: "automation_run_result_open", payload: { id: "auto-1", run: 1, key: "k-page" } });
});
