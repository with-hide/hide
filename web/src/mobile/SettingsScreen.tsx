// The phone's Settings tab: the fourth place of the bottom bar. In this order: the computers this phone is connected to
// (switch, connect another, remove after a confirmation), notifications, language and appearance as one-line values that
// open a sheet to choose from, the privacy policy and support, and the versions. The same screen in a browser, in the
// iOS and Android apps and in the demo; only platform permission and recovery differ, the Android app has no
// notifications yet, and the demo has one sample computer. A choice applies at once and is kept on this phone only (settings.ts).

import { BellIcon } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { Button } from "../components/ui/button";
import { Switch } from "../components/ui/switch";
import { useInterfaceTranslation } from "../i18n/translator";
import { appHost, appPlatform, appPushAvailable } from "./appHost";
import { push } from "./appBridge";
import { appPushRow, refreshAppPush, turnOffAppPush, turnOnAppPush, useAppPush } from "./appPush";
import { DemoHostRows } from "./Demo";
import { HostList } from "./HostList";
import { disableNotifications, enableNotifications, pushSupported, refreshBrowserPush, syncPush, useBrowserPush } from "./push";
import { AboutSection, DisplaySection, ROW_INSET, Section, ValueRow } from "./SettingsParts";
import { usePhone } from "./store";
import { TabBar } from "./TabBar";

/** The browser keeps its own receiving choice, independently of the desktop's pause switch. */
function BrowserNotifications() {
  const { t } = useInterfaceTranslation();
  const state = useBrowserPush();
  useEffect(() => {
    const refresh = () => { refreshBrowserPush(); if (!document.hidden) void syncPush(); };
    refresh();
    document.addEventListener("visibilitychange", refresh);
    window.addEventListener("storage", refresh);
    return () => { document.removeEventListener("visibilitychange", refresh); window.removeEventListener("storage", refresh); };
  }, []);
  const row = !pushSupported() ? "install_first" : state.stopping ? "stopping" : state.asking ? "asking" : state.permission === "denied" ? "denied"
    : state.failure ? "failed" : state.enabled && state.permission === "granted" ? "on" : "off";
  const hint = row === "install_first" ? t("mobile.notifications.installFirst")
    : row === "asking" ? t("mobile.settings.notificationsBrowserAsking")
    : row === "denied" ? t("mobile.notifications.denied")
    : row === "failed" ? t(state.failure === "off" ? "mobile.settings.notificationsOffFailed" : "mobile.settings.notificationsFailed")
    : !state.kept ? t("mobile.settings.notKept")
    : t(row === "on" ? "mobile.settings.notificationsOnHint" : "mobile.settings.notificationsOffHint");
  return <NotificationSwitch row={row} hint={hint} checked={row === "on" || row === "asking" || state.failure === "off"}
    disabled={row === "asking" || row === "stopping" || row === "denied" || row === "install_first"}
    change={(on) => { void (on ? enableNotifications() : disableNotifications()); }} />;
}

/** The same receiving control in the browser and the iOS app, with platform-specific recovery below it. */
function NotificationSwitch({ row, hint, checked, disabled, change, children }: {
  row: string; hint: string; checked: boolean; disabled: boolean; change: (on: boolean) => void; children?: ReactNode;
}) {
  const { t } = useInterfaceTranslation();
  return (
    <div className={`flex flex-col gap-xs py-sm ${ROW_INSET}`} data-phone-notifications={row}>
      <div className="flex min-h-(--size-touch-target) items-center gap-md">
        <BellIcon aria-hidden="true" className="size-(--size-icon) shrink-0 text-muted-foreground" />
        <span id="phone-notifications-label" className="min-w-0 flex-1 text-title text-foreground">{t("mobile.settings.receiveNotifications")}</span>
        <Switch aria-labelledby="phone-notifications-label" checked={checked} disabled={disabled} onCheckedChange={change} />
      </div>
      <p className="break-keep pl-xl text-body leading-relaxed text-muted-foreground wrap-anywhere" role={row === "failed" ? "alert" : "status"}>{hint}</p>
      {children}
    </div>
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
  const android = appPlatform() === "android";
  const hint = row === "asking" ? t(android ? "mobile.settings.notificationsAskingAndroid" : "mobile.settings.notificationsAsking")
    : row === "refused" ? t(android ? "mobile.settings.notificationsRefusedAndroid" : "mobile.settings.notificationsRefused")
    : row === "failed" ? t("mobile.settings.notificationsFailed")
    : row === "on" ? t(state.outdated ? "mobile.settings.notificationsOutdated" : "mobile.settings.notificationsOnHint")
    : t("mobile.settings.notificationsOffHint");
  return (
    <NotificationSwitch row={row} hint={hint} checked={row === "on" || row === "asking"}
      disabled={row === "asking" || row === "refused"}
      change={(on) => { void (on ? turnOnAppPush() : turnOffAppPush()); }}>
      {row === "refused" ? <Button variant="text" className="min-h-(--size-touch-target) self-start px-none pl-xl text-body" data-phone-notifications-settings="true" onClick={() => void push.openSettings().catch(() => undefined)}>{t(android ? "mobile.settings.notificationsOpenSettingsAndroid" : "mobile.settings.notificationsOpenSettings")}</Button> : null}
    </NotificationSwitch>
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
        {inApp ? appPushAvailable() ? <AppNotifications />
          : <ValueRow label={t("mobile.settings.notifications")} value={t("mobile.settings.notificationsNotOnAndroid")} marker="notifications" />
          : demo
          ? <ValueRow label={t("mobile.settings.notifications")} value={t("mobile.demo.sample")} marker="notifications" />
          : <BrowserNotifications />}
      </Section>
      <DisplaySection t={t} openSheet={openSheet} />
      <AboutSection t={t} />
      <div className="min-h-lg flex-1" />
      <TabBar active="settings" />
    </main>
  );
}
