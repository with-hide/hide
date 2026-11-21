import { describe, expect, it } from "vitest";
import { fetchPath, MAX_QUEUED_BYTES, MAX_SEND_BYTES, parseShow, queueAllows, requestAllowed, socketAddress } from "./remotePage";

// PRD device-connect-iroh B20, security spec 8.3: another machine's screen
// loads its own page and nothing else, and the shell names the machine and
// the place, never an address.
describe("another machine's screen", () => {
  it("loads only its own page and assets, through this computer's daemon", () => {
    expect(fetchPath("hide-remote://host-a/", "host-a")).toBe("/peer-screen/host-a/");
    expect(fetchPath("hide-remote://host-a/index.html", "host-a")).toBe("/peer-screen/host-a/");
    expect(fetchPath("hide-remote://host-a/assets/app-1.js", "host-a")).toBe("/peer-screen/host-a/assets/app-1.js");
    // The host's other routes, another machine's page and a query are refused here.
    expect(fetchPath("hide-remote://host-a/health", "host-a")).toBeNull();
    expect(fetchPath("hide-remote://host-a/ws", "host-a")).toBeNull();
    expect(fetchPath("hide-remote://host-b/", "host-a")).toBeNull();
    // The page's own query stays with the page; an asset's is refused.
    expect(fetchPath("hide-remote://host-a/?probe=1", "host-a")).toBe("/peer-screen/host-a/");
    expect(fetchPath("hide-remote://host-a/assets/app-1.js?x=1", "host-a")).toBeNull();
    expect(fetchPath("http://127.0.0.1:9/", "host-a")).toBeNull();
  });

  it("reaches no network, no loopback and no file", () => {
    expect(requestAllowed("hide-remote://host-a/assets/x.css", "host-a")).toBe(true);
    expect(requestAllowed("data:image/png;base64,AA", "host-a")).toBe(true);
    expect(requestAllowed("http://127.0.0.1:5000/health", "host-a")).toBe(false);
    expect(requestAllowed("https://example.com/", "host-a")).toBe(false);
    expect(requestAllowed("file:///etc/passwd", "host-a")).toBe(false);
    expect(requestAllowed("hide-remote://host-b/", "host-a")).toBe(false);
  });

  it("lets one largest frame wait beside a little more, and no queue past it", () => {
    expect(queueAllows(0, MAX_SEND_BYTES)).toBe(true);
    expect(queueAllows(MAX_QUEUED_BYTES - MAX_SEND_BYTES, MAX_SEND_BYTES)).toBe(true);
    expect(queueAllows(MAX_QUEUED_BYTES - MAX_SEND_BYTES + 1, MAX_SEND_BYTES)).toBe(false);
    // A host that reads nothing: small frames stop once the queue is full.
    expect(queueAllows(MAX_QUEUED_BYTES, 1)).toBe(false);
  });

  it("opens its socket on this computer's daemon", () => {
    expect(socketAddress("http://127.0.0.1:4100", "host-a")).toBe("ws://127.0.0.1:4100/peer-screen/host-a/ws");
  });

  it("takes from the shell a machine id and a place, or none", () => {
    expect(parseShow(null)).toBeNull();
    expect(parseShow({ device: "host-a", rect: { x: 56, y: 0, width: 800, height: 600 } })).toEqual({ device: "host-a", rect: { x: 56, y: 0, width: 800, height: 600 } });
    expect(parseShow({ device: "../etc", rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "example.com:80", rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a", rect: { x: 0, y: 0, width: -1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a" })).toBeUndefined();
  });
});
