import { useState } from "react";
import type { Actions } from "../actions";
import { Switch } from "../components/ui/switch";
import { Group, Note, Row } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { IssueSettings } from "../snapshot";
import { useShellStore } from "../store";
import { coreErrorText } from "../coreError";
import { useErrorSince } from "../errorSince";

/** How work starts from an issue (PRD settings-cleanup B24): the pull request body links the issue. */
export function StartingWorkGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const settings = useShellStore((s) => s.rest?.ui_state?.issue_settings);
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const error = useErrorSince(changedAt, ["issue_settings."]);
  const change = (patch: Partial<IssueSettings>) => {
    setChangedAt(Date.now());
    actions.setIssueSettings(patch);
  };
  return (
    <Group title={t("settings.startingWork")} data-settings-group="issue-start">
      {settings ? (
        <Row label={t("issueSettings.closesInstruction")}>
          <Switch
            checked={settings.closes_instruction}
            onCheckedChange={(checked) => change({ closes_instruction: checked })}
            aria-label={t("issueSettings.closesInstruction")}
            data-issue-closes-instruction={String(settings.closes_instruction)}
          />
        </Row>
      ) : (
        <Row label={<Note>{t("settings.notRead")}</Note>} />
      )}
      {error ? <Row label={<Note tone="error" data-issue-settings-error="true" data-core-error={error.kind}>{coreErrorText(error, t, "coreError.settings.notSaved")}</Note>} /> : null}
    </Group>
  );
}
