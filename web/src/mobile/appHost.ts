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

/** The app's own notifications (`HidePush`): the iOS app's alone until the Android app has them, so the Android page never calls the plugin. */
export const appPushAvailable = () => appPlatform() === "ios";

/** Whether this page may register or ask for a service worker: a browser tab can, the app never does. */
export const serviceWorkerAllowed = () => !appHost() && "serviceWorker" in navigator;

/** Marks the page as the app's for the styles that only the app has (no text selection on a control, no callout): one attribute, set once at start. */
export function markAppHost(): void {
  if (appHost()) document.documentElement.dataset.appHost = "true";
}
