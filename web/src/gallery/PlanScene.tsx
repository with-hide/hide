// A session's own plan over invented values (PRD hide-plan), one state per
// `state` parameter: running, review, finished. The checklist the pane chip
// opens (PRD mission-plan-visibility B3-B6) sits beside the map the 보기 slot
// draws, both the real components. Nothing leaves this scene.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { PlanChecklist } from "../PlanChecklist";
import { PlanMap, PlanSummaryLine } from "../PlanMap";
import type { PlanStep, PlanView, SnapshotRest } from "../snapshot";
import { useShellStore } from "../store";

const MINUTE = 60_000;
const SESSION = { device_id: "local", agent: "claude", session_id: "gallery-plan" };

function step(key: string, title: string, status: PlanStep["status"], depends_on: string[], extra: Partial<PlanStep> = {}): PlanStep {
  return { key, title, depends_on, status, line: null, output: null, changed: false, change: null, waiting: false, ...extra };
}

/** The eight steps of the mockup's evaluation run (scene 3), as each state leaves them. */
function plan(state: string, now: number): PlanView {
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

export function PlanScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = params.get("state") ?? "running";
  const shown = useMemo(() => plan(state, Date.now()), [state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    useShellStore.setState({ rest: { plans: { items: [shown] } } as unknown as SnapshotRest, agents: [], connection: "live" });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [shown, theme, params]);
  const width = Number(params.get("width") ?? 1080);
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex h-full min-w-0 gap-lg bg-background p-lg text-foreground" data-gallery-scene="plan" data-plan-scene={state}>
        <div className="flex w-(--size-pr-popover) shrink-0 flex-col self-start rounded-md border border-border bg-popover text-caption" data-plan-scene-popover="true">
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
