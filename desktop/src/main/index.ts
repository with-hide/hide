// The desktop host's entry: one instance per profile, one host, one menu.

import path from "node:path";
import { app, Menu } from "electron";
import { loadEnv } from "./env";
import { DesktopHost } from "./host";
import { HostLog } from "./log";
import { SHOW_INACTIVE_SWITCH } from "./launchSwitches";
import { keySystemOf, systemRegistry, type Command } from "../../../web/src/shortcuts";
import { menuBindings, menuTemplate } from "./menu";
import { HostLanguage, languageFile } from "./language";
import { recordUncaught } from "./uncaught";
import { APP_ID } from "./notificationPermission";

const env = loadEnv(process.env);
// Pinned before anything reads it: the single-instance lock, web storage and
// the log live here, apart from the folders the removed native app used.
app.setPath("userData", env.userDataDir ?? path.join(app.getPath("appData"), "hide-desktop"));
app.setAppLogsPath(path.join(app.getPath("userData"), "logs"));

// Windows groups and attributes the app's notifications by this identifier (PRD first-run-onboarding D-13).
if (process.platform === "win32") app.setAppUserModelId(APP_ID);

if (!app.requestSingleInstanceLock()) {
  // B6: the running instance brings its window forward instead.
  app.quit();
} else {
  const log = new HostLog(app.getPath("logs"));
  recordUncaught(log);
  const showInactive = app.commandLine.hasSwitch(SHOW_INACTIVE_SWITCH);
  // The system's language is asked when a word is drawn: the app must be ready first.
  const language = new HostLanguage(
    languageFile(app.getPath("userData")),
    () => env.systemLanguage ?? app.getPreferredSystemLanguages()[0],
    log,
  );
  const host = new DesktopHost(env, log, showInactive, language);
  log.event("host.start", { packaged: app.isPackaged, version: app.getVersion(), electron: process.versions.electron, show_inactive: showInactive });

  app.on("second-instance", () => host.reopen("second-instance"));
  app.on("activate", () => host.reopen("activate"));
  // B7: closing the last window leaves the app running, as macOS apps do.
  app.on("window-all-closed", () => {
    if (process.platform !== "darwin") app.quit();
  });
  app.on("before-quit", () => host.quit());

  // The menu is rebuilt only when the chords it shows or its language change.
  const system = keySystemOf(process.platform);
  let shown: string | null = null;
  let current: readonly Command[] = systemRegistry(system);
  // Desk is off until the page reports otherwise (D-76): its command is then left out.
  let desk = false;
  const showMenu = (registry: readonly Command[]) => {
    current = registry;
    host.setBrowserRegistry(registry);
    const key = JSON.stringify([language.language, desk, registry.map((command) => command.electron)]);
    if (key === shown) return;
    shown = key;
    Menu.setApplicationMenu(
      Menu.buildFromTemplate(menuTemplate({ appName: app.getName(), send: (id) => host.sendCommand(id), developer: !app.isPackaged, system, registry, desk, t: language.t })),
    );
  };
  host.listenDesk((on) => {
    desk = on;
    showMenu(current);
  });
  host.listenLanguage(() => {
    showMenu(current);
    host.languageChanged();
  });
  host.listenBindings((reported) => {
    const resolved = menuBindings(reported, system);
    if ("refused" in resolved) {
      log.event("bindings.refused", { reason: resolved.refused });
      return;
    }
    if (resolved.diagnostic) log.event("bindings.defaults", { detail: resolved.diagnostic });
    showMenu(resolved.registry);
  });

  app.whenReady().then(
    () => {
      showMenu(current);
      host.start();
    },
    (error: unknown) => {
      log.event("host.start_failed", { detail: String(error) });
      app.exit(1);
    },
  );
}
