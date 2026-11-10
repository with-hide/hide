/**
 * The same page inside the iOS app, which serves this bundle from its own scheme (`capacitor://localhost`,
 * `mobile/`) instead of `app.withhide.dev`. Every difference the app needs hangs off this one answer: it has no
 * service worker (`serviceWorkerAllowed`), no Home Screen to install to, opens only https addresses outside
 * (`webAddress` in markdown.tsx), and names the app when a computer is ahead of it.
 */
export const appHost = () => window.location.protocol === "capacitor:";

/** Whether this page may register or ask for a service worker: a browser tab can, the app never does. */
export const serviceWorkerAllowed = () => !appHost() && "serviceWorker" in navigator;
