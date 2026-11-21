// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { VersionScreen } from "./HostList";
import { renderMarkdown } from "./markdown";
import { useLoaderTranslation } from "./loaderTranslation";
import { appHost, appPlatform, appPushAvailable, serviceWorkerAllowed } from "./appHost";

function inApp(protocol: string) {
  vi.stubGlobal("location", { ...window.location, protocol, pathname: "/" });
}

afterEach(() => { vi.unstubAllGlobals(); Reflect.deleteProperty(navigator, "serviceWorker"); });

describe("the page inside the iOS app", () => {
  it("is the app's own scheme, and then registers no service worker even where the web view offers one", () => {
    Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: {} });
    expect(appHost()).toBe(false);
    expect(serviceWorkerAllowed()).toBe(true);
    inApp("capacitor:");
    expect(appHost()).toBe(true);
    expect(serviceWorkerAllowed()).toBe(false);
  });

  it("tells the user to update the app when the computer is ahead of every bundle in it, and not otherwise", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    function Screen({ appBehind }: { appBehind: boolean }) {
      const { t } = useLoaderTranslation();
      return <VersionScreen t={t} name="Mac" protocol={6} app="3, 4, 5" appBehind={appBehind} />;
    }
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => root.render(<Screen appBehind />));
    expect(host.querySelector("h1")?.textContent).toBe("Update the Hide app");
    await act(async () => root.render(<Screen appBehind={false} />));
    expect(host.querySelector("h1")?.textContent).toBe("Update Hide on this device");
    await act(async () => root.unmount());
  });

  it("draws an http link as a link in a browser and as plain text in the app, which opens only https outside", () => {
    const body = '<a href="http://example.com/a">plain</a> <a href="https://example.com/b">safe</a>';
    const draw = () => renderToStaticMarkup(<>{renderMarkdown(body, { html: true })}</>);
    expect(draw()).toMatch(/<a [^>]*href="http:\/\/example.com\/a"/);
    inApp("capacitor:");
    expect(draw()).not.toMatch(/<a [^>]*href="http:/);
    expect(draw()).toContain('href="https://example.com/b"');
  });
});

describe("the page inside the Android app", () => {
  const at = (origin: string, platform: string | null, plugins: string[] = []) => {
    const url = new URL(origin);
    vi.stubGlobal("location", { ...window.location, protocol: url.protocol, origin: url.origin, pathname: "/" });
    if (platform) vi.stubGlobal("Capacitor", { getPlatform: () => platform, isPluginAvailable: (name: string) => plugins.includes(name) });
  };

  it("is the app's own origin where Capacitor says Android, and then registers no service worker and has no notifications without HidePush", () => {
    Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: {} });
    at("https://localhost", "android");
    expect(appPlatform()).toBe("android");
    expect(appHost()).toBe(true);
    expect(serviceWorkerAllowed()).toBe(false);
    expect(appPushAvailable()).toBe(false);
  });

  it("is a browser page anywhere else, or without Capacitor's word for Android", () => {
    at("https://app.withhide.dev", "android");
    expect(appHost()).toBe(false);
    vi.unstubAllGlobals();
    at("https://localhost", "web");
    expect(appHost()).toBe(false);
    vi.unstubAllGlobals();
    at("https://localhost", null);
    expect(appPlatform()).toBeNull();
  });

  it("has notifications in either app exactly when Capacitor reports a HidePush plugin, never in a browser", () => {
    at("https://localhost", "android", ["HideKeychain", "HidePush"]);
    expect(appPushAvailable()).toBe(true);
    vi.unstubAllGlobals();
    inApp("capacitor:");
    vi.stubGlobal("Capacitor", { isPluginAvailable: (name: string) => name === "HidePush" });
    expect(appPlatform()).toBe("ios");
    expect(appPushAvailable()).toBe(true);
    vi.stubGlobal("Capacitor", { isPluginAvailable: () => false });
    expect(appPushAvailable()).toBe(false);
    vi.unstubAllGlobals();
    at("https://app.withhide.dev", "android", ["HidePush"]);
    expect(appPushAvailable()).toBe(false);
  });
});
