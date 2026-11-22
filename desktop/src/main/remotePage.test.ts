import { describe, expect, it } from "vitest";
import { fetchPath, MAX_PROMPT, MAX_QUEUED_BYTES, MAX_SEND_BYTES, parseGo, parseShow, parseWorkResult, queueAllows, requestAllowed, socketAddress } from "./remotePage";

const KEY = "a".repeat(64);
const OTHER = "b".repeat(64);

// PRD device-connect-iroh B20, security spec 8.3: another machine's screen
// loads its own page and nothing else, and the shell names the machine and
// the place, never an address.
describe("another machine's screen", () => {
  it("loads only its own page and assets, through this computer's daemon", () => {
    expect(fetchPath("hide-remote://host-a/", "host-a", KEY)).toBe(`/peer-screen/host-a/?key=${KEY}`);
    expect(fetchPath("hide-remote://host-a/index.html", "host-a", KEY)).toBe(`/peer-screen/host-a/?key=${KEY}`);
    expect(fetchPath("hide-remote://host-a/assets/app-1.js", "host-a", KEY)).toBe(`/peer-screen/host-a/assets/app-1.js?key=${KEY}`);
    // The host's other routes, another machine's page and a query are refused here.
    expect(fetchPath("hide-remote://host-a/health", "host-a", KEY)).toBeNull();
    expect(fetchPath("hide-remote://host-a/ws", "host-a", KEY)).toBeNull();
    expect(fetchPath("hide-remote://host-b/", "host-a", KEY)).toBeNull();
    // The page's own query stays with the page; an asset's is refused.
    expect(fetchPath("hide-remote://host-a/?probe=1", "host-a", KEY)).toBe(`/peer-screen/host-a/?key=${KEY}`);
    expect(fetchPath("hide-remote://host-a/assets/app-1.js?x=1", "host-a", KEY)).toBeNull();
    expect(fetchPath("http://127.0.0.1:9/", "host-a", KEY)).toBeNull();
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

  it("opens its socket on this computer's daemon, for the key its page was opened for", () => {
    expect(socketAddress("http://127.0.0.1:4100", "host-a", KEY)).toBe(`ws://127.0.0.1:4100/peer-screen/host-a/ws?key=${KEY}`);
  });

  it("takes from the shell a machine id, its key and a place, or none", () => {
    expect(parseShow(null)).toBeNull();
    expect(parseShow({ device: "host-a", key: KEY, rect: { x: 56, y: 0, width: 800, height: 600 } })).toEqual({ device: "host-a", key: KEY, rect: { x: 56, y: 0, width: 800, height: 600 }, name: null });
    // The name the window calls the machine by goes with it when it is one short line.
    const named = (name: unknown) => parseShow({ device: "host-a", key: KEY, rect: { x: 0, y: 0, width: 1, height: 1 }, name })?.name;
    expect(named("build-box")).toBe("build-box");
    expect([named("a\nb"), named("  "), named("x".repeat(129)), named(7)]).toEqual([null, null, null, null]);
    // S1: a request names the key; one without it, or with a key Hide never spells, is no request.
    expect(parseShow({ device: "host-a", rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a", key: OTHER.toUpperCase(), rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a", key: "../x", rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "../etc", key: KEY, rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "example.com:80", key: KEY, rect: { x: 0, y: 0, width: 1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a", key: KEY, rect: { x: 0, y: 0, width: -1, height: 1 } })).toBeUndefined();
    expect(parseShow({ device: "host-a" })).toBeUndefined();
  });
});

// PRD device-connect-iroh D-73, D-78: the shell's ask of another computer's
// screen is a work to show or a start to make, for one device and key, and
// nothing else reaches the page.
describe("an ask of another machine's screen", () => {
  const start = { op: "start", request: "r-1", place: { checkout: "/home/me/app" }, provider: "codex", model: null, prompt: "fix it", profile: null };

  it("takes a work to show and a start in a checkout or a new worktree", () => {
    expect(parseGo({ device: "host-a", key: KEY, work: { op: "show", pane: "w1:p2" } })).toEqual({ device: "host-a", key: KEY, work: { op: "show", pane: "w1:p2" } });
    expect(parseGo({ device: "host-a", key: KEY, work: start })?.work).toEqual(start);
    const worktree = { ...start, place: { repository: "/home/me/app", branch: "fix", base: null }, model: "o4", profile: "work" };
    expect(parseGo({ device: "host-a", key: KEY, work: worktree })?.work).toEqual(worktree);
  });

  it("refuses an ask that names no machine, no key, no place or another operation", () => {
    expect(parseGo({ device: "http://x", key: KEY, work: start })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: "abcd", work: start })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { ...start, place: null } })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { ...start, place: { repository: "/r" } } })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { ...start, provider: "" } })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { ...start, prompt: "x".repeat(MAX_PROMPT + 1) } })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { op: "close", pane: "w1:p2" } })).toBeUndefined();
    expect(parseGo({ device: "host-a", key: KEY, work: { op: "show", pane: "" } })).toBeUndefined();
    expect(parseGo(null)).toBeUndefined();
  });
});

// How a page says a start it was handed ended: each outcome with what it
// carries, and nothing else.
describe("a page's answer for a start", () => {
  it("takes each outcome with what it carries", () => {
    expect(parseWorkResult({ request: "r-1", outcome: "started" })).toEqual({ request: "r-1", outcome: "started" });
    expect(parseWorkResult({ request: "r-1", outcome: "starting" })).toEqual({ request: "r-1", outcome: "starting" });
    expect(parseWorkResult({ request: "r-1", outcome: "late", kind: "x" })).toEqual({ request: "r-1", outcome: "late" });
    expect(parseWorkResult({ request: "r-1", outcome: "not_added", kind: "workspace.create_refused" })).toEqual({ request: "r-1", outcome: "not_added", kind: "workspace.create_refused" });
    expect(parseWorkResult({ request: "r-1", outcome: "failed", code: null })).toEqual({ request: "r-1", outcome: "failed", code: null });
    expect(parseWorkResult({ request: "r-1", outcome: "agent_failed", reason: "exited", line: "boom" })).toEqual({ request: "r-1", outcome: "agent_failed", reason: "exited", line: "boom" });
  });

  it("refuses an answer with no request, an unknown outcome or a missing reason", () => {
    expect(parseWorkResult({ outcome: "started" })).toBeUndefined();
    expect(parseWorkResult({ request: "r-1", outcome: "done" })).toBeUndefined();
    expect(parseWorkResult({ request: "r-1", outcome: "refused" })).toBeUndefined();
    expect(parseWorkResult({ request: "r-1", outcome: "failed", code: 3 })).toBeUndefined();
    expect(parseWorkResult({ request: "x".repeat(129), outcome: "started" })).toBeUndefined();
    expect(parseWorkResult(null)).toBeUndefined();
  });
});
