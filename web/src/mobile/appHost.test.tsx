// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { VersionScreen } from "./HostList";
import { renderMarkdown } from "./markdown";
import { useLoaderTranslation } from "./loaderTranslation";
import { appHost, serviceWorkerAllowed } from "./appHost";

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
