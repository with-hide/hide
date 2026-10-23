// @vitest-environment jsdom
import { afterAll, afterEach, describe, expect, it, vi } from "vitest";
import { hostBridge } from "../host";
import { standInHost } from "./mount";
import { holdPageOutside } from "./outside";
import type { CoreEvent } from "./world";

// Importing the stood-in host loads xterm; answer its canvas probe at the browser boundary.
const browserCanvas = vi.hoisted(() => {
  const original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = () => null;
  return { restore: () => { HTMLCanvasElement.prototype.getContext = original; } };
});
afterAll(() => browserCanvas.restore());

describe("holdPageOutside", () => {
  afterEach(() => {
    delete window.hideHost;
    document.body.innerHTML = "";
  });

  it("opens no window and tells the world the address instead", () => {
    const opened = vi.fn();
    window.open = opened;
    const told: CoreEvent[] = [];
    const release = holdPageOutside((event) => told.push(event));
    expect(window.open("https://example.com/pull/3", "_blank", "noopener")).toBeNull();
    release();
    expect(opened).not.toHaveBeenCalled();
    expect(told).toEqual([{ kind: "practice_outside", payload: { what: "open", detail: "https://example.com/pull/3" } }]);
    expect(window.open).toBe(opened);
  });

  it("keeps a link from leaving the page but lets a link to a place on it through", () => {
    document.body.innerHTML = '<a href="https://example.com/issues" target="_blank"><span>GitHub</span></a><a href="#scenes">Scenes</a>';
    const told: CoreEvent[] = [];
    const release = holdPageOutside((event) => told.push(event));
    const away = new MouseEvent("click", { bubbles: true, cancelable: true });
    document.querySelector("span")!.dispatchEvent(away);
    const here = new MouseEvent("click", { bubbles: true, cancelable: true });
    document.querySelector('a[href="#scenes"]')!.dispatchEvent(here);
    release();
    expect(away.defaultPrevented).toBe(true);
    expect(here.defaultPrevented).toBe(false);
    expect(told.map((event) => event.payload)).toEqual([{ what: "link", detail: "https://example.com/issues" }]);
  });

  it("tells the world every call on the stood-in host, the ones it does not name yet too", async () => {
    window.hideHost = standInHost("darwin");
    const told: CoreEvent[] = [];
    const release = holdPageOutside((event) => told.push(event));
    const bridge = hostBridge()!;
    expect(bridge.platform).toBe("darwin");
    bridge.revealPath("/work/hide-practice/src");
    bridge.openInEditor("/work/hide-practice");
    bridge.browser.command("hide-practice", "b1", { kind: "reload" } as never);
    const unsubscribe = bridge.onWindowChrome(() => undefined);
    unsubscribe();
    expect(await bridge.pickFolder()).toBeNull();
    release();
    expect(told.map((event) => event.payload.detail)).toEqual(["revealPath", "openInEditor", "browser.command", "pickFolder"]);
    expect(hostBridge()).not.toBeNull();
  });
});
