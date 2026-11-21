// For the demo's tests: every way a page reaches the network is made to fail and to be counted, because the sample host may use none of them.

export type NetworkGuard = { calls: string[]; release: () => void };

/** Replaces WebSocket, fetch, XMLHttpRequest, EventSource and sendBeacon with ones that record the attempt and refuse it. */
export function blockNetwork(stub: (name: string, value: unknown) => void, unstub: () => void): NetworkGuard {
  const calls: string[] = [];
  const refuse = (kind: string) => (target: unknown) => {
    calls.push(`${kind} ${String(target)}`);
    throw new Error(`no ${kind} in a demo`);
  };
  stub("WebSocket", Object.assign(function Refused(url: string) { refuse("WebSocket")(url); }, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 }));
  stub("fetch", (url: string) => { calls.push(`fetch ${String(url)}`); return Promise.reject(new Error("no fetch in a demo")); });
  stub("XMLHttpRequest", function Refused() { refuse("XMLHttpRequest")(""); });
  stub("EventSource", function Refused(url: string) { refuse("EventSource")(url); });
  const beacon = Object.getOwnPropertyDescriptor(navigator, "sendBeacon");
  Object.defineProperty(navigator, "sendBeacon", { configurable: true, value: (url: string) => { calls.push(`sendBeacon ${url}`); return false; } });
  return {
    calls,
    release: () => {
      unstub();
      if (beacon) Object.defineProperty(navigator, "sendBeacon", beacon);
      else delete (navigator as { sendBeacon?: unknown }).sendBeacon;
    },
  };
}
