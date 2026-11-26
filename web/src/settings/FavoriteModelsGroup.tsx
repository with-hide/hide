import { Group } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { Actions } from "../actions";
import { CandidateAdd, CandidateRows, ConductorError, useConductorEdit } from "./ConductorControls";

// Settings › Agents › Favorite models (operator decision 2026-11-26): the
// agent, model and effort Hide offers first wherever work is handed to an
// agent, each with a line on what it is for. The value is the Conductor's
// candidate list (`conductor_settings.candidates`), now read by missions and
// by Desk's agent menu alike; the rows and their edit events are
// `ConductorControls.tsx`'s, so a row edited here is the row a mission reads.

export function FavoriteModelsGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const { settings, send, error } = useConductorEdit(actions);
  return (
    <>
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
    </>
  );
}
