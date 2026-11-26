import { Group, Note } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { Actions } from "../actions";
import { CandidateAdd, CandidateRows, ConductorError, useConductorEdit } from "./ConductorControls";

// Settings › Models by purpose (operator decision 2026-11-26): the agents and models
// the operator wants work handed to, each with a line on what it is for. Desk,
// a mission's Conductor and any agent that starts another read it as `hide
// agent catalog`'s `candidates`; it limits none of them. The value is
// `conductor_settings.candidates`, and the rows and their edit events are
// `ConductorControls.tsx`'s.

export function DelegationTab({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { settings, send, error } = useConductorEdit(actions);
  return (
    <div data-settings-delegation="true">
      <div className="mb-lg">
        <Note>{t("delegation.intro")}</Note>
      </div>
      <Group title={t("conductor.candidates")} action={<CandidateAdd settings={settings} send={send} />} note={t("conductor.footer")} data-conductor-candidates={String(settings.candidates.length)}>
        {settings.candidates.length > 0 ? (
          // One grid for the head and every row, so the effort column is as wide as its widest row and every row's columns line up.
          // The agent column is the widest: an item's name carries its profile (`Claude Code · opengateway`).
          <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,1fr)_auto_minmax(0,1.4fr)_auto] gap-x-sm">
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
      <ConductorError error={error} />
    </div>
  );
}
