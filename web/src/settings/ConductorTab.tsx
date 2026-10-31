import type { Actions } from "../actions";
import { Group, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { MissionStrength } from "../snapshot";
import { CandidateAdd, CandidateRows, CapControl, CleanupControl, ConductorError, OtherVendorControl, ScopeControl, StrengthControl, useConductorEdit } from "./ConductorControls";

// Settings › Conductor (PRD software-factory-conductor D-31, B48, B49): how
// many agents a mission runs at once, how far a mission goes when it does not
// say, whether Hide removes a done node's merged, clean worktree or asks
// first (PRD mission-node-simple D-14), the candidates a Conductor chooses its workers and checkers from, and
// how strongly a new mission is checked. The controls are the Mission tool's
// defaults' too (`ConductorControls.tsx`, PRD mission-tool D-03); this page
// lays them out as settings rows.

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
        <Row label={<RowLabel title={t("conductor.cleanup")} hint={t("conductor.cleanupHint")} />}>
          <CleanupControl settings={settings} send={send} />
        </Row>
      </Group>
      <ConductorError error={error} />
      <Group title={t("conductor.candidates")} action={<CandidateAdd settings={settings} send={send} />} note={t("conductor.footer")} data-conductor-candidates={String(settings.candidates.length)}>
        {settings.candidates.length > 0 ? (
          // One grid for the head and every row, so the effort column is as wide as its widest row and every row's columns line up.
          <div className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.3fr)_auto_minmax(0,1.4fr)_auto] gap-x-sm">
            <div className="col-span-full grid grid-cols-subgrid items-center px-md py-xs text-caption text-muted-foreground">
              <span>{t("conductor.column.agent")}</span>
              <span>{t("conductor.column.model")}</span>
              <span>{t("conductor.column.effort")}</span>
              <span>{t("conductor.column.purpose")}</span>
              <span />
            </div>
            <CandidateRows settings={settings} send={send} layout="grid" />
          </div>
        ) : null}
      </Group>
      <Group title={t("conductor.verification")} note={t("conductor.verification.footer")} data-conductor-verification={settings.default_strength}>
        <Row label={t("conductor.strength")} detail={<StrengthTerms strength={settings.default_strength} />}>
          <StrengthControl settings={settings} send={send} />
        </Row>
        <Row label={t("conductor.otherVendor")}>
          <OtherVendorControl settings={settings} send={send} />
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
