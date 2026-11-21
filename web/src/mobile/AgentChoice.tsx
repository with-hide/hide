// The agent kind and model a phone sheet starts with: what the desktop
// remembered until the operator picks another (PRD home-device-rail D-24, B44).
// The start sheet and the PR hand-over sheet draw the same two selects.

import { useInterfaceTranslation } from "../i18n/translator";
import { Field, SELECT_CLASS } from "./parts";
import type { StartKind } from "./protocol";
import { START_KINDS, type StartChoice, type StartSelection, modelsOf } from "./start";

export function AgentChoice({ selection, choice, disabled = false, onChange, name }: {
  selection: StartSelection;
  choice: StartChoice;
  disabled?: boolean;
  onChange: (choice: StartChoice) => void;
  /** The sheet's name in the `data-phone-<name>-kind` and `-model` attributes the browser tests read. */
  name: string;
}) {
  const { t } = useInterfaceTranslation();
  const models = modelsOf(selection);
  const kindAttr = { [`data-phone-${name}-kind`]: "true" };
  const modelAttr = { [`data-phone-${name}-model`]: "true" };
  return (
    <div className="grid grid-cols-2 gap-sm">
      <Field label={t("mobile.start.kind")}>
        <select
          value={selection.kind}
          disabled={disabled}
          onChange={(event) => onChange({ ...choice, kind: event.target.value as StartKind, model: undefined })}
          className={SELECT_CLASS}
          {...kindAttr}
        >
          {START_KINDS.map((kind) => (
            <option key={kind.id} value={kind.id}>
              {kind.label}
            </option>
          ))}
        </select>
      </Field>
      <Field label={t("mobile.start.model")}>
        <select
          value={selection.model}
          disabled={disabled || models.length === 0}
          onChange={(event) => onChange({ ...choice, model: event.target.value })}
          className={SELECT_CLASS}
          {...modelAttr}
        >
          <option value="">{t("mobile.start.defaultModel")}</option>
          {selection.model && !models.includes(selection.model) ? <option value={selection.model}>{selection.model}</option> : null}
          {models.map((model) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
      </Field>
    </div>
  );
}
