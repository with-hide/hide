import { ArrowUpRightIcon } from "lucide-react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Group, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { MissionStrength } from "../snapshot";
import { CapControl, CleanMergedControl, ConductorError, ScopeControl, StrengthControl, useConductorEdit } from "./ConductorControls";

// Settings › Mission (PRD software-factory-conductor D-31, B48, B49): how
// many agents a mission runs at once, how far a mission goes when it does not
// say, whether a merged node's worktree is cleaned up (operator decision
// 2026-11-02, every mission at once), a link to the agent list (Settings › Delegation) a Conductor chooses its workers and checkers by, and
// how strongly a new mission is checked. The controls live in
// `ConductorControls.tsx`; this page lays them out as settings rows, and the
// mission screen's ⚙ opens it (PRD mission-screen B10).

export function ConductorTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { settings, send, error } = useConductorEdit(actions);
  return (
    <div data-settings-conductor="true">
      <Group>
        <Row label={<RowLabel title={t("conductor.cap")} hint={t("conductor.capHint")} />}>
          <CapControl settings={settings} send={send} />
        </Row>
        <Row label={<RowLabel title={t("conductor.scope")} hint={t("conductor.scopeHint")} />}>
          <ScopeControl settings={settings} send={send} />
        </Row>
        <Row label={<RowLabel title={t("conductor.cleanMerged")} hint={t("conductor.cleanMergedHint")} />} data-conductor-cleanup="true">
          <CleanMergedControl settings={settings} send={send} />
        </Row>
      </Group>
      <ConductorError error={error} />
      <Group>
        <Row label={<RowLabel title={t("conductor.models")} hint={t("conductor.modelsHint")} />} data-conductor-models={String(settings.candidates.length)}>
          <Button variant="secondary" size="sm" onClick={() => actions.openSettings("delegation")} data-conductor-models-open="true">
            {t("conductor.modelsOpen", { count: settings.candidates.length })}<ArrowUpRightIcon />
          </Button>
        </Row>
      </Group>
      <Group title={t("conductor.verification")} note={t("conductor.verification.footer")} data-conductor-verification={settings.default_strength}>
        <Row label={t("conductor.strength")} detail={<StrengthTerms strength={settings.default_strength} />}>
          <StrengthControl settings={settings} send={send} />
        </Row>
      </Group>
    </div>
  );
}

function RowLabel({ title, hint }: { title: string; hint: string }) {
  return (
    <span className="flex flex-col">
      <span>{title}</span>
      <span className="text-caption text-muted-foreground">{hint}</span>
    </span>
  );
}

/** What the chosen strength has the Conductor do, how often a node is reviewed again, and the work it suits, as the mission help states it. */
function StrengthTerms({ strength }: { strength: MissionStrength }) {
  const { t } = useInterfaceTranslation();
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-xxs text-caption" data-conductor-strength-terms={strength}>
      {(["does", "rounds", "suits"] as const).map((term) => (
        <div key={term} className="contents">
          <dt className="text-muted-foreground">{t(`conductor.strength.${term}`)}</dt>
          <dd className="break-keep text-subtle-foreground">{t(`conductor.strength.${strength}.${term}`)}</dd>
        </div>
      ))}
    </dl>
  );
}
