import type { Actions } from "../actions";
import { Group, Note, Row } from "../components/settings-rows";
import { Switch } from "../components/ui/switch";
import { hostBridge } from "../host";
import { useInterfaceTranslation } from "../i18n/client";
import { type PushMode } from "../mobileSettings";
import { NotificationRows, useNotificationControl, usePermission } from "../NotificationControls";
import { thisMachineKey } from "../screenMachine";
import { useShellStore } from "../store";

/**
 * Every notification in one page (PRD settings-left-nav B6): this machine's
 * desktop notifications, which only the desktop app has (PRD
 * first-run-onboarding D-14, D-21), and the push to a paired phone.
 */
export function NotificationsTab({ actions }: { actions: Actions }) {
  return (
    <>
      {hostBridge() ? <DesktopGroup actions={actions} /> : null}
      <PhoneGroup actions={actions} />
    </>
  );
}

/** Notify me when an agent needs me, and what the system allows, read while this page is open (B18, B30). */
function DesktopGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const control = useNotificationControl(actions, "settings", usePermission());
  if (!control) return null;
  const sends = control.enabled && control.state !== "no_service";
  return (
    <Group title={t(thisMachineKey(null))} note={sends ? t("firstRun.settings.sentWhen") : undefined} data-settings-group="notifications">
      <NotificationRows control={control} />
    </Group>
  );
}

/** The phone push mode, the daemon's `mobile` frame's (PRD mobile-companion B59). */
function PhoneGroup({ actions }: { actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const mobile = useShellStore((s) => s.mobile);
  return (
    <Group title={t("settings.notifications.phone")} data-settings-group="phone">
      {mobile ? <PhoneRestSwitch mode={mobile.push_mode} actions={actions} /> : <Note tone="pending">{t("mobileSetup.connecting")}</Note>}
    </Group>
  );
}

/** The desktop controls when to pause; each phone controls whether to receive. */
function PhoneRestSwitch({ mode, actions }: { mode: PushMode; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  return (
    <Row label={t("mobileSetup.restSwitch")} detail={<><Note>{t("mobileSetup.restHint")}</Note><Note>{t("mobileSetup.restPhoneHint")}</Note></>}>
      <Switch
        aria-label={t("mobileSetup.restSwitch")}
        data-phone-push-rest="true"
        checked={mode === "app_closed"}
        onCheckedChange={(rest) => actions.setPushMode(rest ? "app_closed" : "always")}
      />
    </Row>
  );
}
