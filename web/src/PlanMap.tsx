import type { Actions } from "./actions";
import { formatElapsed, useElapsed } from "./components/elapsed";
import { StatusMark } from "./components/status-mark";
import { Button } from "./components/ui/button";
import { useInterfaceTranslation } from "./i18n/client";
import { requireInterfaceLanguage } from "./i18n/locale";
import { PlanGraph } from "./PlanGraph";
import { planCount, planOfSession, planProposed } from "./plans";
import type { PlanView, WorkSession } from "./snapshot";
import { useShellStore } from "./store";

// A session's plan drawn large in the lane's 보기 slot (PRD hide-plan B11-B13):
// the count and the time since the agent last updated it, one band while the
// plan waits for the operator's review (answered in the conversation, so its
// one button goes there) or once it is finished, and the map.

export function PlanMap({ session, actions }: { session: WorkSession; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const plan = useShellStore((s) => planOfSession(s.rest, session));
  if (!plan) return <p className="px-md py-md text-body text-muted-foreground" data-plan-state="gone">{t("plan.gone")}</p>;
  return <PlanBody plan={plan} actions={actions} />;
}

/**
 * `계획 n/m · 12분 전 갱신`, `계획 8단계 · 2분 전 갱신` while it waits for
 * review (B12), and `계획 끝남 m/m · <걸린 시간>` once finished, from the plan's
 * making to its last update (PRD mission-plan-visibility B6).
 */
export function PlanSummaryLine({ plan }: { plan: PlanView }) {
  const { t, i18n } = useInterfaceTranslation();
  const elapsed = useElapsed(plan.updated_at_unix_ms);
  const finished = plan.progress.finished;
  const count = planProposed(plan) ? t("plan.steps", { count: plan.progress.total }) : planCount(plan);
  const when = finished ? formatElapsed(requireInterfaceLanguage(i18n.language), plan.updated_at_unix_ms - plan.created_at_unix_ms) : elapsed ? t("plan.updated", { elapsed }) : null;
  return (
    <span className="flex min-w-0 items-baseline gap-xs" data-plan-summary={finished ? "finished" : plan.review}>
      <span className="text-muted-foreground">{finished ? t("plan.finished") : t("plan.title")}</span>
      <span className="font-semibold tabular-nums text-foreground">{count}</span>
      {when ? <span className="text-muted-foreground" data-plan-summary-when="true">· {when}</span> : null}
    </span>
  );
}

function PlanBody({ plan, actions }: { plan: PlanView; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const proposed = planProposed(plan);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-plan={plan.pane_id} data-plan-review={plan.review} data-plan-finished={plan.progress.finished || undefined}>
      {proposed ? (
        <div className="mx-md mt-sm flex min-w-0 items-center gap-sm rounded-md bg-muted px-sm py-xs" data-plan-banner="review">
          <StatusMark mark="turn" />
          <span className="min-w-0 flex-1 truncate text-body text-warning" title={plan.ask ?? undefined}>
            {t("plan.reviewBanner")}
          </span>
          <Button size="sm" variant="secondary" onClick={() => actions.openAgent(plan.pane_id)} data-plan-conversation="true">
            {t("plan.toConversation")}
          </Button>
        </div>
      ) : plan.progress.finished ? (
        <div className="mx-md mt-sm flex min-w-0 items-center gap-sm rounded-md bg-muted px-sm py-xs" data-plan-banner="finished">
          <StatusMark mark="done" />
          <span className="min-w-0 flex-1 truncate text-body">{t("plan.finishedBanner")}</span>
        </div>
      ) : null}
      {proposed && plan.ask ? <p className="px-md pt-xs text-caption text-muted-foreground" data-plan-ask="true">{plan.ask}</p> : null}
      <div className="px-md pb-xs pt-sm text-body">
        <PlanSummaryLine plan={plan} />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-md pb-md pt-xs" data-plan-scroll="true">
        <PlanGraph plan={plan} />
      </div>
    </div>
  );
}
