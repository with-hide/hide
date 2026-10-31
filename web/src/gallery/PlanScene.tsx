// A session's own plan over invented values (PRD hide-plan), one state per
// `state` parameter: running, review, finished, and a plan whose steps are
// handed to other agents, at work (`delegated`) and finished
// (`delegated-finished`, PRD hide-plan-delegate B25-B30). The checklist the pane chip
// opens (PRD mission-plan-visibility B3-B6) sits beside the map the 보기 slot
// draws, both the real components. Nothing leaves this scene.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { PlanChecklist } from "../components/plan-checklist";
import { PlanMap, PlanSummaryLine } from "../PlanMap";
import type { PlanDelegation, PlanStep, PlanView, SnapshotRest, WorkSession } from "../snapshot";
import { useShellStore } from "../store";

const MINUTE = 60_000;
const SESSION = { device_id: "local", agent: "claude", session_id: "gallery-plan" };

function step(key: string, title: string, status: PlanStep["status"], depends_on: string[], extra: Partial<PlanStep> = {}): PlanStep {
  return { key, title, depends_on, status, line: null, output: null, changed: false, change: null, waiting: false, delegate: false, evidence: null, delegation: null, ...extra };
}

/** The eight steps of the mockup's evaluation run (scene 3), as each state leaves them. */
export function planFixture(state: string, now: number): PlanView {
  const review = state === "review";
  const finished = state === "finished";
  const done = (key: string, title: string, depends: string[], extra: Partial<PlanStep> = {}) => step(key, title, review ? "planned" : "done", depends, extra);
  const steps = [
    done("prompt", "라벨 프롬프트 v19", []),
    done("bundle", "20세션 평가 묶음", ["prompt"]),
    done("rounds", "1~3차 평가", ["bundle"]),
    done("tests", "core 테스트", ["prompt"]),
    step("round-4", "4차 평가", finished ? "done" : review ? "planned" : "working", ["rounds"], { line: finished || review ? null : "287턴 · 1시간", waiting: review }),
    step("verify", "최종 검증", finished ? "done" : "planned", ["round-4", "tests"], { waiting: !finished }),
    step("pr", "PR 열기", finished ? "done" : "planned", ["verify"], { output: "pr", waiting: !finished, changed: state === "running", change: state === "running" ? "4차 결과를 보고 열기로 바꿈" : null }),
    step("letter", "보고 편지", finished ? "dropped" : "planned", ["pr"], { waiting: !finished }),
  ];
  const counted = steps.filter((row) => row.status !== "dropped");
  return {
    pane_id: "w1:p2",
    session: SESSION,
    steps,
    review: review ? "proposed" : "approved",
    ask: review ? "평가 범위를 20세션으로 늘림" : null,
    progress: {
      current: finished ? null : review ? "prompt" : "round-4",
      next: finished ? null : review ? "bundle" : "verify",
      done: counted.filter((row) => row.status === "done").length,
      total: counted.length,
      finished,
    },
    created_at_unix_ms: now - 90 * MINUTE,
    updated_at_unix_ms: now - (review ? 2 : 12) * MINUTE,
  };
}

const CHILD = { device_id: "local", agent: "codex", session_id: "gallery-plan-child" };

function handed(state: PlanDelegation["state"], name: string, agent: string, session: WorkSession | null): PlanDelegation {
  return { state, attempt: 1, agent_id: `gallery-${name}`, name, agent, pane_id: session ? `w1:${name}` : null, session };
}

/** The mockup's handing plan: the spec done with its file, the core mine, the web screen with web-ui, the review still to hand. */
function delegated(finished: boolean, now: number): PlanView[] {
  const done = (extra: Partial<PlanStep>): Partial<PlanStep> => (finished ? { status: "done", ...extra } : {});
  const steps = [
    step("spec", "스펙 쓰기", "done", [], { output: "file", evidence: { kind: "file", path: "docs/plans/delegate-spec.md" } }),
    step("core", "코어 이벤트", "working", ["spec"], { line: "테스트 쓰는 중", ...done({ line: null }) }),
    step("web", "웹 화면 지도와 팝오버, 긴 이름이 섞인 단계", "working", ["spec"], { output: "pr", delegation: handed(finished ? "reported" : "working", "web-ui", "codex", finished ? null : CHILD), ...done({ evidence: { kind: "pr", repo: "with-hide/hide", number: 1879 } }) }),
    step("review", "리뷰", "planned", ["core", "web"], { waiting: !finished, delegate: true, ...done({ delegation: handed("reported", "reviewer", "claude", null) }) }),
    step("pr", "PR 열기", "planned", ["review"], { waiting: !finished, output: "pr", ...done({ evidence: { kind: "pr", repo: "with-hide/hide", number: 1880 } }) }),
  ];
  const main: PlanView = {
    pane_id: "w1:p2",
    session: SESSION,
    steps,
    review: "approved",
    ask: null,
    progress: { current: finished ? null : "core", next: finished ? null : "review", done: finished ? 5 : 1, total: 5, finished },
    created_at_unix_ms: now - 18 * MINUTE,
    updated_at_unix_ms: finished ? now : now - 2 * MINUTE,
  };
  const child: PlanView = {
    pane_id: "",
    session: CHILD,
    steps: [step("parts", "컴포넌트", "done", []), step("store", "스토어", "done", ["parts"]), step("map", "지도 그리기", "working", ["store"]), step("e2e", "e2e", "planned", ["map"], { waiting: true })],
    review: "approved",
    ask: null,
    progress: { current: "map", next: "e2e", done: 2, total: 4, finished: false },
    created_at_unix_ms: now - 10 * MINUTE,
    updated_at_unix_ms: now - MINUTE,
  };
  return finished ? [main] : [main, child];
}

export function PlanScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "running";
  const plans = useMemo(() => (state.startsWith("delegated") ? delegated(state === "delegated-finished", Date.now()) : [planFixture(state, Date.now())]), [state]);
  const [shown] = plans as [PlanView, ...PlanView[]];
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: { plans: { items: plans } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [plans, theme, params]);
  const width = Number(params.get("width") ?? 1080);
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex h-full min-w-0 gap-lg bg-background p-lg text-foreground" data-gallery-scene="plan" data-plan-scene={state}>
        <div className="flex w-(--size-pr-popover) shrink-0 flex-col self-start rounded-md border border-border bg-popover text-caption [word-break:keep-all]" data-plan-scene-popover="true">
          <div className="px-sm pt-sm"><PlanSummaryLine plan={shown} /></div>
          <div className="px-xs py-sm"><PlanChecklist plan={shown} popover /></div>
        </div>
        <div className="flex min-h-0 min-w-0 flex-1 flex-col rounded-md border border-border">
          <PlanMap session={SESSION} actions={actions} />
        </div>
      </div>
    </TooltipProvider>
  );
}
