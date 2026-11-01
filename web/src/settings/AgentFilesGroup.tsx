import { useEffect } from "react";
import type { Actions } from "../actions";
import { Group, Note, Row, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import { useShellStore } from "../store";

/**
 * The operator's own agent files under `~/.hide/agents` that Hide left out
 * (PRD engine-bundle-3-core-owns D-17, B24): one row per file, its name with
 * 불러오지 않음 and the reason dimmed under it, which the operator fixes in the
 * file. The core's machine only, and only with the development switch on: the
 * core then publishes `status.agent_definitions`, read again as the page
 * opens; off, or with every file read, there is no group and nothing is asked.
 */
export function AgentFilesGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const definitions = useShellStore((s) => s.rest?.status?.agent_definitions);
  // The core publishes the field only with the switch on, from its start;
  // then the page asks for a fresh read once as it opens, and off it asks for
  // nothing.
  const enabled = definitions != null;
  useEffect(() => {
    if (enabled) actions.readAgentDefinitions();
  }, [actions, enabled]);
  if (!definitions || (definitions.rejected.length === 0 && definitions.unreadable === null)) return null;
  return (
    <Group title={t("agents.definitions.title")} data-agents-definitions="true">
      {definitions.unreadable !== null ? (
        <Row label={<Note data-agents-definitions-unreadable="true">{t("agents.definitions.unreadable", { reason: definitions.unreadable })}</Note>} />
      ) : null}
      {definitions.rejected.map((rejected) => (
        <Row
          key={rejected.file}
          label={<span className="font-mono">{rejected.file}</span>}
          value={<Status tone="muted">{t("agents.definitions.leftOut")}</Status>}
          detail={<Note>{rejected.reason}</Note>}
          data-agents-definition-left-out={rejected.file}
        />
      ))}
    </Group>
  );
}
