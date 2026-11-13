/**
 * The same page inside the iOS or the Android app, which serves this bundle from its own origin instead of
 * `app.withhide.dev`: the iOS app from its own scheme (`capacitor://localhost`), the Android app from
 * `https://localhost`, where Capacitor's bridge says it is Android (`mobile/`). Every difference the apps need hangs off
 * this one answer: no service worker (`serviceWorkerAllowed`), no Home Screen to install to, only https addresses opened
 * outside (`webAddress` in markdown.tsx), and the app named when a computer is ahead of it.
 */
export type AppPlatform = "ios" | "android";

/** Which app the page is inside, or null in a browser. */
export function appPlatform(): AppPlatform | null {
  if (window.location.protocol === "capacitor:") return "ios";
  const bridge = (window as unknown as { Capacitor?: { getPlatform?: () => unknown } }).Capacitor;
  if (window.location.origin === "https://localhost" && typeof bridge?.getPlatform === "function" && bridge.getPlatform() === "android") return "android";
  return null;
}

export const appHost = () => appPlatform() !== null;

/**
 * The app's own notifications: only an app whose Capacitor reports a `HidePush` plugin, which the iOS app has and the
 * Android app gains with its notifications. The page never calls a plugin the app does not have: the Android app's gate
 * drops such a call unanswered (`BridgeGate`).
 */
export const appPushAvailable = () => {
  if (!appHost()) return false;
  const bridge = (window as unknown as { Capacitor?: { isPluginAvailable?: (name: string) => boolean } }).Capacitor;
  return bridge?.isPluginAvailable?.("HidePush") === true;
};

/** Whether this page may register or ask for a service worker: a browser tab can, the app never does. */
export const serviceWorkerAllowed = () => !appHost() && "serviceWorker" in navigator;

/** Marks the page as the app's for the styles that only the app has (no text selection on a control, no callout): one attribute, set once at start. */
export function markAppHost(): void {
  if (appHost()) document.documentElement.dataset.appHost = "true";
}
