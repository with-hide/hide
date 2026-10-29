import { useState } from "react";
import type { Actions } from "../actions";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Note, Status } from "../components/settings-rows";
import { useInterfaceTranslation } from "../i18n/client";
import type { DeviceSignIn as Wait } from "../snapshot";
import { FixCommand } from "./FixCommand";

/**
 * What a device's connection waits on, on its row (PRD system-ssh-device-link
 * B4, B6, B9, B11, B12): ssh's own question with a masked field, a server
 * to trust or not, a security key to touch, or a page to confirm in the
 * browser. An answer goes to the question
 * it names and is cleared from the field as it is sent; nothing here keeps it.
 * A new question (ssh asking again after a wrong answer, B7) is a new field.
 */
export function DeviceSignIn({ deviceId, wait, actions }: { deviceId: string; wait: Wait; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const cancel = () => actions.answerDeviceSignIn(deviceId, wait.question, { reply: "cancel" });
  switch (wait.kind) {
    case "touch":
      return (
        <div className="space-y-xxs" data-device-sign-in={`${deviceId}:touch`}>
          <Status tone="pending">{t("devices.signIn.touch")}</Status>
          <p className="break-words font-mono text-caption text-muted-foreground">{wait.prompt}</p>
        </div>
      );
    case "browser":
      return (
        <div className="space-y-xxs" data-device-sign-in={`${deviceId}:browser`}>
          <div className="flex flex-wrap items-center gap-x-sm gap-y-xs">
            <Status tone="pending">{t("devices.signIn.browser")}</Status>
            <Button variant="secondary" size="sm" onClick={() => actions.openLink(wait.url, true)} data-device-sign-in-open={deviceId}>
              {t("common.open")}
            </Button>
          </div>
          <p className="break-all font-mono text-caption text-muted-foreground">{wait.url}</p>
          <Note>{t("devices.signIn.browserNote")}</Note>
        </div>
      );
    case "host_key":
      // Neither choice is focused or primary: trusting a server is the
      // operator's reading of the fingerprint, not a default (B12).
      return (
        <div className="space-y-xs" data-device-sign-in={`${deviceId}:host_key`}>
          <Status tone="warn">{t("devices.signIn.hostKey")}</Status>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-sm gap-y-xxs text-caption">
            <dt className="text-muted-foreground">{t("devices.signIn.server")}</dt>
            <dd className="break-all font-mono text-foreground">{wait.host}</dd>
            <dt className="text-muted-foreground">{t("devices.signIn.fingerprint")}</dt>
            <dd className="break-all font-mono text-foreground" data-device-sign-in-fingerprint={deviceId}>{wait.fingerprint}</dd>
          </dl>
          <Note>{t("devices.signIn.hostKeyCheck")}</Note>
          <div className="flex flex-wrap gap-sm">
            <Button variant="secondary" size="sm" onClick={() => actions.answerDeviceSignIn(deviceId, wait.question, { reply: "accept" })} data-device-sign-in-trust={deviceId}>
              {t("devices.signIn.trust")}
            </Button>
            <Button variant="secondary" size="sm" onClick={cancel} data-device-sign-in-cancel={deviceId}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      );
    case "confirm":
      return (
        <div className="space-y-xs" data-device-sign-in={`${deviceId}:confirm`}>
          <p className="break-words font-mono text-caption text-foreground">{wait.prompt}</p>
          <div className="flex flex-wrap gap-sm">
            <Button variant="secondary" size="sm" onClick={() => actions.answerDeviceSignIn(deviceId, wait.question, { reply: "accept" })} data-device-sign-in-accept={deviceId}>
              {t("devices.signIn.confirm")}
            </Button>
            <Button variant="secondary" size="sm" onClick={cancel} data-device-sign-in-cancel={deviceId}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      );
    case "secret":
      return <SecretAnswer key={wait.question} deviceId={deviceId} wait={wait} actions={actions} onCancel={cancel} />;
  }
}

function SecretAnswer({ deviceId, wait, actions, onCancel }: { deviceId: string; wait: Extract<Wait, { kind: "secret" }>; actions: Actions; onCancel: () => void }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState("");
  return (
    <form
      className="space-y-xs"
      data-device-sign-in={`${deviceId}:secret`}
      onSubmit={(event) => {
        event.preventDefault();
        actions.answerDeviceSignIn(deviceId, wait.question, { reply: "text", text });
        setText("");
      }}
    >
      <label className="block break-words font-mono text-caption text-foreground" htmlFor={`device-sign-in-${deviceId}`}>
        {wait.prompt}
      </label>
      <div className="flex flex-wrap items-center gap-sm">
        <Input
          id={`device-sign-in-${deviceId}`}
          type="password"
          autoComplete="off"
          spellCheck={false}
          className="w-auto min-w-0 flex-1"
          aria-label={t("devices.signIn.answer")}
          value={text}
          onChange={(event) => setText(event.target.value)}
          data-device-sign-in-field={deviceId}
        />
        <Button type="submit" size="sm" data-device-sign-in-submit={deviceId}>
          {t("devices.signIn.submit")}
        </Button>
        <Button type="button" variant="secondary" size="sm" onClick={onCancel} data-device-sign-in-cancel={deviceId}>
          {t("common.cancel")}
        </Button>
      </div>
      {wait.key_command ? <KeyCommand deviceId={deviceId} command={wait.key_command} actions={actions} /> : null}
    </form>
  );
}

/** The command that puts the asked key in ssh-agent, so it is typed once (B9). */
function KeyCommand({ deviceId, command, actions }: { deviceId: string; command: string; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <div className="space-y-xxs">
      <Note>{t("devices.signIn.keyCommand")}</Note>
      <FixCommand command={command} what="ssh-add command" actions={actions} data-device-sign-in-key-command={deviceId} />
    </div>
  );
}
