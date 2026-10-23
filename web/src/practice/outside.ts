// Nothing on a practice screen reaches past the page. The shell leaves a page
// four ways besides the clipboard: `window.open` (a link opened in the
// system's browser, from its actions and from components that call it
// themselves), a link clicked, a navigation of the page, and the desktop
// host's bridge (open in another app, reveal in the file manager, a
// notification, a folder picker), which the practice window only stands in
// for. `holdPageOutside` takes each of them where the page meets the browser,
// so a new outward action the shell grows is held without a list naming it,
// and tells the world of each as `practice_outside`.

import type { HostBridge } from "../host";
import { PRACTICE_OUTSIDE, type CoreEvent } from "./world";

type Report = (what: string, detail: string) => void;

/** `target` with every function on it, and on the objects it holds, reporting its call before it answers. */
function reporting<T extends object>(target: T, path: string, report: Report): T {
  return new Proxy(target, {
    get(object, name, receiver) {
      const value: unknown = Reflect.get(object, name, receiver);
      if (typeof name !== "string") return value;
      if (typeof value === "function") {
        return (...args: unknown[]) => {
          // A subscription (`onStatus`, `onWindowChrome`) listens and sends nothing out.
          if (!name.startsWith("on")) report("host", `${path}${name}`);
          return (value as (...rest: unknown[]) => unknown).apply(object, args);
        };
      }
      return value !== null && typeof value === "object" ? reporting(value, `${path}${name}.`, report) : value;
    },
  });
}

/** The cross-document navigation a `navigate` event starts, which the Navigation API lets the page cancel. */
type NavigateEvent = Event & { cancelable: boolean; destination: { url: string; sameDocument: boolean } };
type Navigation = { addEventListener(type: "navigate", listener: (event: NavigateEvent) => void): void; removeEventListener(type: "navigate", listener: (event: NavigateEvent) => void): void };

/**
 * Holds every way off the page until the function it returns lets them go:
 * `window.open` opens nothing, a link clicked goes nowhere, the page does not
 * navigate away, and a call on the stood-in host is told to the world before
 * the stand-in answers it as it does, with nothing. A browser without the
 * Navigation API (Firefox, older Safari) keeps every other hold; there a page
 * navigation the shell starts by script is not held.
 */
export function holdPageOutside(send: (event: CoreEvent) => void): () => void {
  const report: Report = (what, detail) => send({ kind: PRACTICE_OUTSIDE, payload: { what, detail } });

  const open = window.open;
  window.open = (url?: string | URL) => {
    report("open", String(url ?? ""));
    return null;
  };

  // A link to a place on this page (`#scenes`) stays; every other one would leave it or open a window.
  const onClick = (event: MouseEvent) => {
    const link = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!link || link.getAttribute("href")!.startsWith("#")) return;
    event.preventDefault();
    report("link", (link as HTMLAnchorElement).href);
  };
  document.addEventListener("click", onClick, true);
  document.addEventListener("auxclick", onClick, true);

  const navigation = (window as Window & { navigation?: Navigation }).navigation;
  const onNavigate = (event: NavigateEvent) => {
    if (event.destination.sameDocument || !event.cancelable) return;
    event.preventDefault();
    report("navigate", event.destination.url);
  };
  navigation?.addEventListener("navigate", onNavigate);

  const host = window.hideHost;
  if (host) window.hideHost = reporting<HostBridge>(host, "", report);

  return () => {
    window.open = open;
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("auxclick", onClick, true);
    navigation?.removeEventListener("navigate", onNavigate);
    if (host) window.hideHost = host;
  };
}
