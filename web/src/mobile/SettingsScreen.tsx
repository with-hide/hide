// The phone's Settings tab: the fourth place of the bottom bar. In this order: the computers this phone is connected to
// (switch, connect another, remove after a confirmation), notifications, language and appearance as one-line values that
// open a sheet to choose from, the privacy policy and support, and the versions. The same screen in a browser, in the
// iOS app and in the demo; only what a place cannot do differs (the app's notifications are a switch of its own, the demo
// has one sample computer). A choice applies at once and is kept on this phone only (settings.ts).

import { BellIcon, ChevronRightIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/ui/button";
import { RowButton } from "../components/ui/row-button";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/translator";
import { appHost } from "./appHost";
import { push } from "./appBridge";
import { appPushRow, refreshAppPush, turnOffAppPush, turnOnAppPush, useAppPush } from "./appPush";
import { DemoHostRows } from "./Demo";
import { HostList } from "./HostList";
import { notificationRow } from "./protocol";
import { enableNotifications, permission, pushSupported } from "./push";
import { AboutSection, DisplaySection, ROW_INSET, Section, ValueRow } from "./SettingsParts";
import { usePhone } from "./store";
import { TabBar } from "./TabBar";

/** Turn on notifications in a browser, the Home Screen first, or the OS setting that is off (B30). The app has no such control. */
function BrowserNotifications() {
  const { t } = useInterfaceTranslation();
  const pushMode = usePhone((s) => s.pushMode);
  const notifications = usePhone((s) => s.notifications);
  const [asking, setAsking] = useState(false);
  const row = notificationRow({ pushMode, notifications, permission: permission(), supported: pushSupported() });
  if (!row) return <ValueRow label={t("mobile.settings.notifications")} value={t("mobile.settings.on")} marker="notifications" />;
  if (row === "enable") {
    return (
      <RowButton
        disabled={asking}
        data-phone-notifications="enable"
        className={`min-h-(--size-touch-target) gap-md rounded-none py-sm text-title text-foreground ${ROW_INSET}`}
        onClick={() => {
          setAsking(true);
          void enableNotifications().finally(() => setAsking(false));
        }}
      >
        <BellIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1">{t("mobile.notifications.enable")}</span>
        <ChevronRightIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
      </RowButton>
    );
  }
  return (
    <p className={`flex items-start gap-md py-sm text-body leading-relaxed break-keep text-muted-foreground wrap-anywhere ${ROW_INSET}`} data-phone-notifications={row}>
      <BellIcon aria-hidden="true" className="mt-xxs size-(--size-icon) shrink-0" />
      <span>{row === "denied" ? t("mobile.notifications.denied") : t("mobile.notifications.installFirst")}</span>
    </p>
  );
}

/**
 * The app's notification row: one switch, and under it the one sentence that is true now (appPush.ts). A refusal by iOS
 * is told with the way to iOS Settings, because only iOS can turn it back on.
 */
function AppNotifications() {
  const { t } = useInterfaceTranslation();
  const demo = usePhone((s) => s.demo);
  const state = useAppPush();
  const row = appPushRow(state, demo);
  useEffect(() => {
    void refreshAppPush();
    // Back from iOS Settings: the permission may have changed there.
    const again = () => { if (!document.hidden) void refreshAppPush(); };
    document.addEventListener("visibilitychange", again);
    return () => document.removeEventListener("visibilitychange", again);
  }, []);
  const hint = row === "asking" ? t("mobile.settings.notificationsAsking")
    : row === "refused" ? t("mobile.settings.notificationsRefused")
    : row === "failed" ? t("mobile.settings.notificationsFailed")
    : row === "on" ? t(state.outdated ? "mobile.settings.notificationsOutdated" : "mobile.settings.notificationsOnHint")
    : t("mobile.settings.notificationsOffHint");
  return (
    <div className={`flex flex-col gap-xs py-sm ${ROW_INSET}`} data-phone-notifications={row}>
      <div className="flex min-h-(--size-touch-target) items-center gap-md">
        <BellIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
        <span id="app-notifications-label" className="min-w-0 flex-1 text-title text-foreground">{t("mobile.settings.notifications")}</span>
        <Switch
          aria-labelledby="app-notifications-label"
          checked={row === "on" || row === "asking"}
          disabled={row === "asking" || row === "refused"}
          onCheckedChange={(on) => { void (on ? turnOnAppPush() : turnOffAppPush()); }}
        />
      </div>
      <p className="break-keep pl-xl text-body leading-relaxed text-muted-foreground wrap-anywhere" role={row === "failed" ? "alert" : "status"}>{hint}</p>
      {row === "refused" ? <Button variant="text" className="min-h-(--size-touch-target) self-start px-none pl-xl text-body" data-phone-notifications-settings="true" onClick={() => void push.openSettings().catch(() => undefined)}>{t("mobile.settings.notificationsOpenSettings")}</Button> : null}
    </div>
  );
}

export function SettingsScreen({ openSheet = null }: { /** The gallery's: the sheet already open. */ openSheet?: "language" | "theme" | null }) {
  const { t } = useInterfaceTranslation();
  const demo = usePhone((s) => s.demo);
  const devices = usePhone((s) => s.devices);
  const connected = usePhone((s) => s.connected);
  const local = devices.find((device) => device.local);
  const inApp = appHost();
  return (
    <main className="flex min-h-full flex-col" data-phone-settings="true">
      <header className="phone-safe-top px-xl">
        <h1 className="pt-lg text-headline font-semibold text-foreground">{t("mobile.settings.title")}</h1>
      </header>
      <Section title={t("mobile.settings.computers")} data-phone-settings-section="computers" note={demo ? t("mobile.demo.panelNote") : undefined}>
        {demo ? <DemoHostRows /> : <HostList t={t} machines={devices} active={{ online: connected, lastSeen: null, needs: local?.needs_you, working: local?.working }} />}
      </Section>
      <Section title={t("mobile.settings.notifications")} data-phone-settings-section="notifications">
        {inApp ? <AppNotifications /> : <BrowserNotifications />}
      </Section>
      <DisplaySection t={t} openSheet={openSheet} />
      <AboutSection t={t} />
      <div className="min-h-lg flex-1" />
      <TabBar active="settings" />
    </main>
  );
}
