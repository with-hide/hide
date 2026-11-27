import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { appScheme, browserPartition, deviceEmulation, isLocalNetworkHost, localNetworkReach, localPageRequest, isPopup, loadable, parseRemoteSync, placeIn, popupBounds, MAX_AUTHORIZED_SCOPES, MAX_SYNCED_DISPLAYS, offScreenBounds, overCap, parseCommand, parseSync, parseTarget, REMOTE_BOUNDS, remoteFilePartition, remotePagePartition, remoteRequest, remoteRouteAddress, sameFileAddress, toBounds, wireFileAddress } from "./browserSync";

const rect = { x: 0, y: 40, width: 800, height: 600 };
const display = { id: "d1", area_id: "a1", url: "https://a.test/", load: 3, rect, visible: true };
const workspace = "local\u0000/r";
const retained = [{ workspace, id: "d1", area_id: "a1" }];
const sync = { workspace, displays: [display], retained };

describe("what the shell may ask of the browser views (issue 155)", () => {
  it("takes a well-formed sync whole", () => {
    const sync = { workspace, displays: [display, { ...display, id: "d2", area_id: "a2", rect: null, visible: false }], retained: [...retained, { workspace, id: "d2", area_id: "a2" }, { workspace: "ssh\u0000/other", id: "d1", area_id: "a3" }] };
    expect(parseSync(sync)).toEqual(sync);
    expect(parseSync({ workspace: null, displays: [], retained: [] })).toEqual({ workspace: null, displays: [], retained: [] });
  });

  it("drops a sync whole when any part of it is out of shape", () => {
    const bad: unknown[] = [
      null,
      { ...sync, displays: "d1" },
      { ...sync, retained: [] },
      { ...sync, workspace: null },
      { ...sync, workspace: "" },
      { ...sync, displays: [display, display] },
      { ...sync, displays: [{ ...display, load: -1 }] },
      { ...sync, displays: [{ ...display, load: 1.5 }] },
      { ...sync, displays: [{ ...display, url: "x".repeat(8193) }] },
      { ...sync, displays: [{ ...display, rect: { ...rect, width: Number.NaN } }] },
      { ...sync, displays: [{ ...display, rect: { ...rect, height: -1 } }] },
      { ...sync, displays: [{ ...display, rect: { ...rect, x: 1e9 } }] },
      { ...sync, displays: [{ ...display, visible: "yes" }] },
      { ...sync, displays: Array.from({ length: MAX_SYNCED_DISPLAYS + 1 }, (_, i) => ({ ...display, id: `d${i}` })) },
    ];
    for (const value of bad) expect(parseSync(value), JSON.stringify(value)?.slice(0, 80)).toBeNull();
  });

  it("takes an outside file by its grant with no address, and the grants its tabs stand on (PRD outside-path-in-hide)", () => {
    const grant = "0123456789abcdef0123456789abcdef";
    const external = { ...display, url: "", grant };
    const named = { ...sync, displays: [external], external_grants: [grant] };
    expect(parseSync(named)).toEqual(named);
    expect(parseSync({ ...sync, external_grants: [] })).toEqual({ ...sync, external_grants: [] });
    for (const bad of [
      { ...sync, displays: [{ ...external, url: "file:///tmp/a.html" }] },
      { ...sync, displays: [{ ...external, grant: "/tmp/a.html" }] },
      { ...sync, displays: [{ ...display, url: "" }] },
      { ...sync, external_grants: ["/tmp/a.png"] },
      { ...sync, external_grants: grant },
      { ...sync, external_grants: Array.from({ length: 257 }, () => grant) },
    ]) expect(parseSync(bad), JSON.stringify(bad).slice(0, 120)).toBeNull();
  });

  it("carries a retained web page's address and load together, and refuses half of them (PRD lane-desk-pair-view B32)", () => {
    const page = { workspace, id: "d2", area_id: "shelf", url: "https://off.test/", load: 2 };
    expect(parseSync({ ...sync, retained: [...retained, page] })?.retained).toEqual([...retained, page]);
    for (const bad of [{ url: page.url }, { load: 2 }, { url: "", load: 2 }, { url: page.url, load: -1 }, { url: page.url, load: 1.5 }, { url: 7, load: 2 }]) {
      expect(parseSync({ ...sync, retained: [...retained, { workspace, id: "d2", area_id: "shelf", ...bad }] })).toBeNull();
    }
  });

  it("carries whether the retained list is the core's inventory, and refuses any other mark (B32)", () => {
    expect(parseSync({ ...sync, inventoried: true })?.inventoried).toBe(true);
    expect(parseSync(sync)?.inventoried).toBeUndefined();
    for (const inventoried of [false, "true", 1, null]) expect(parseSync({ ...sync, inventoried })).toBeNull();
  });

  it("requires a bounded area identity on placements and retained pages", () => {
    for (const area_id of [undefined, null, "", 1, "a".repeat(8193), "a\u0000b", "a\u0001b"]) {
      expect(parseSync({ ...sync, displays: [{ ...display, area_id }] })).toBeNull();
      expect(parseSync({ ...sync, retained: [{ ...retained[0], area_id }] })).toBeNull();
    }
    const area_id = "a".repeat(8192);
    expect(parseSync({ ...sync, displays: [{ ...display, area_id }], retained: [{ ...retained[0], area_id }] })).not.toBeNull();
  });

  it("preserves an optional bounded exact attachment epoch and rejects malformed epochs", () => {
    expect(parseSync(sync)).toEqual(sync);
    for (const attachment_epoch of ["epoch-1", "A0-z9", "a".repeat(64), "893b5942-eaa5-49ae-95d1-073c35c07ecb"]) {
      expect(parseSync({ ...sync, attachment_epoch })).toEqual({ ...sync, attachment_epoch });
    }
    for (const attachment_epoch of [null, false, 1, {}, "", "a".repeat(65), "a_b", "a b", "é", "a\n", "a\u0000b", "a\u0001b"]) {
      expect(parseSync({ ...sync, attachment_epoch })).toBeNull();
    }
  });

  it("accepts explicit area authority independently of browser inventory and manual placement", () => {
    const authorized_scopes = [
      { workspace, area_id: "empty-area", incarnation: 0 },
      { workspace: "local\u0000/other", area_id: "empty-area", incarnation: 1 },
      { workspace: "ssh\u0000/r", area_id: "empty-area", incarnation: Number.MAX_SAFE_INTEGER },
    ];
    const empty = { workspace: null, displays: [], retained: [], authorized_scopes };
    expect(parseSync(empty)).toEqual(empty);
    expect(parseSync({ ...sync, authorized_scopes })).toEqual({ ...sync, authorized_scopes });
    expect(parseSync({ ...sync, authorized_scopes: [] })).toEqual({ ...sync, authorized_scopes: [] });
    expect(parseSync(sync)).toEqual(sync);
  });

  it("rejects malformed or duplicate positive Workspace area scopes as a whole", () => {
    const scope = { workspace, area_id: "a1", incarnation: 1 };
    const invalid: unknown[] = [null, {}, "a1", [null], [1], [{}], [scope, scope], [scope, { ...scope, incarnation: 2 }]];
    for (const area_id of [undefined, null, "", 1, "a".repeat(8193), "a\u0000b", "a\u0001b"]) invalid.push([{ ...scope, area_id }]);
    for (const owner of [undefined, null, "", "w", "\u0000/r", "local\u0000", "local\u0000/r\u0000other", "local\u0000/r\u0001a1", `local\u0000${"r".repeat(8192)}`]) invalid.push([{ ...scope, workspace: owner }]);
    for (const incarnation of [undefined, null, "1", true, {}, -1, 0.5, Number.NaN, Number.POSITIVE_INFINITY, Number.MAX_SAFE_INTEGER + 1]) invalid.push([{ ...scope, incarnation }]);
    for (const authorized_scopes of invalid) expect(parseSync({ ...sync, authorized_scopes })).toBeNull();
    const authorized_scopes = [{ workspace: `local\u0000${"r".repeat(8186)}`, area_id: "a".repeat(8192), incarnation: 1 }];
    expect(parseSync({ ...sync, authorized_scopes })?.authorized_scopes).toEqual(authorized_scopes);
  });

  it("bounds positive scopes without dropping or inferring authority", () => {
    expect(MAX_AUTHORIZED_SCOPES).toBe(65_792);
    const authorized_scopes = Array.from({ length: MAX_AUTHORIZED_SCOPES }, (_, i) => ({ workspace, area_id: `a${i}`, incarnation: i }));
    expect(parseSync({ ...sync, authorized_scopes })?.authorized_scopes).toEqual(authorized_scopes);
    expect(parseSync({ ...sync, authorized_scopes: [...authorized_scopes, { workspace, area_id: "one-too-many", incarnation: 1 }] })).toBeNull();
  });

  it("matches ownership by the complete Workspace, display and authoritative area", () => {
    expect(parseSync({ ...sync, retained: [{ workspace: "local\u0000/other", id: "d1", area_id: "a1" }] })).toBeNull();
    expect(parseSync({ ...sync, retained: [{ workspace: "ssh\u0000/r", id: "d1", area_id: "a1" }] })).toBeNull();
    expect(parseSync({ ...sync, retained: [{ workspace, id: "d1", area_id: "a2" }] })).toBeNull();
    expect(parseSync({ ...sync, retained: [...retained, ...retained] })).toBeNull();
    for (const owner of ["w", "\u0000/r", "local\u0000", "local\u0000/r\u0000other", "local\u0000/r\u0001d1"]) {
      expect(parseSync({ ...sync, workspace: owner, retained: [{ workspace: owner, id: "d1", area_id: "a1" }] })).toBeNull();
    }
  });

  it("names a page and a toolbar command only in shape", () => {
    expect(parseTarget({ workspace: "w", id: "d1" })).toEqual({ workspace: "w", id: "d1" });
    expect(parseTarget({ workspace: "w" })).toBeNull();
    expect(parseCommand("reload")).toBe("reload");
    for (const zoom of ["zoom_in", "zoom_out", "zoom_reset"]) expect(parseCommand(zoom)).toBe(zoom);
    expect(parseCommand("eval")).toBeNull();
    expect(parseCommand("zoom")).toBeNull();
  });

  it("takes a display's device size, and drops a sync whose size is out of shape (PRD browser-width-zoom D-12)", () => {
    const sized = { ...sync, displays: [{ ...display, device: { width: 390, mobile: true } }] };
    expect(parseSync(sized)).toEqual(sized);
    expect(parseSync({ ...sync, displays: [{ ...display, device: null }] })).toEqual(sync);
    for (const device of [{ width: 0, mobile: true }, { width: 390.5, mobile: true }, { width: 4097, mobile: false }, { width: 390 }, { width: "390", mobile: true }, "mobile"]) {
      expect(parseSync({ ...sync, displays: [{ ...display, device }] }), JSON.stringify(device)).toBeNull();
    }
  });

  it("loads only the web, a local file, or a blank page", () => {
    for (const url of ["https://a.test/", "http://localhost:3000/", "file:///Users/example/a.html", "about:blank"]) expect(loadable(url), url).toBe(true);
    for (const url of ["javascript:alert(1)", "data:text/html,x", "chrome://settings", "about:config", "not a url"]) expect(loadable(url), url).toBe(false);
  });

  it("keeps an opener only for a sized popup, never for a tab or a shift-click", () => {
    expect(isPopup("new-window", "width=420,height=520")).toBe(true);
    expect(isPopup("new-window", "popup")).toBe(true);
    expect(isPopup("new-window", "")).toBe(false);
    for (const disposition of ["foreground-tab", "background-tab", "default", "other"]) expect(isPopup(disposition, "width=420"), disposition).toBe(false);
  });

  it("hands another app only its own links, never a scheme Chromium answers", () => {
    expect(appScheme("slack://channel?team=T1")).toBe("slack:");
    expect(appScheme("zoommtg://zoom.us/join?confno=1")).toBe("zoommtg:");
    expect(appScheme("mailto:a@b.test")).toBe("mailto:");
    for (const url of ["https://a.test/", "file:///a.html", "about:blank", "about:config", "javascript:alert(1)", "data:text/html,x", "blob:https://a.test/1", "chrome://settings", "chrome-error://chromewebdata/", "view-source:https://a.test/", "devtools://devtools/x", "wss://a.test/", "not a url"]) {
      expect(appScheme(url), url).toBeNull();
    }
  });

  it("never offers a link that mounts a share, runs a shell or a script, or opens a remote session", () => {
    for (const url of ["smb://host/share", "afp://host/share", "nfs://host/x", "ftp://host/x", "sftp://host", "ssh://host", "telnet://host", "vnc://host", "news://host/group", "nntp://host/group", "gopher://host", "x-man-page://ls", "applescript://com.apple.scripteditor?action=new", "shortcuts://run-shortcut?name=x", "x-apple.systempreferences:com.apple.preference.security", "disk://x"]) {
      expect(appScheme(url), url).toBeNull();
    }
  });

  it("sizes a popup as asked within the work area, centred over the window", () => {
    const area = { x: 0, y: 25, width: 1440, height: 875 };
    const parent = { x: 100, y: 100, width: 1000, height: 700 };
    expect(popupBounds({ width: 420, height: 520 }, parent, area)).toEqual({ x: 390, y: 190, width: 420, height: 520 });
    expect(popupBounds({ width: 3000, height: 2000 }, parent, area)).toEqual({ x: 0, y: 25, width: 1440, height: 875 });
    expect(popupBounds({ width: 1, height: -5 }, parent, area)).toMatchObject({ width: 320, height: 600 });
    expect(popupBounds({}, { ...parent, x: 1300 }, area)).toEqual({ x: 940, y: 150, width: 500, height: 600 });
  });

  it("shares web login storage across Workspaces while keeping remote localhost and file previews separate", () => {
    const localA = "local\u0000/a";
    const localB = "local\u0000/b";
    const remoteA = "ssh-a\u0000/a";
    const remoteB = "ssh-a\u0000/b";
    const otherDevice = "ssh-b\u0000/a";
    const web = browserPartition(localA, "https://example.com/", "local");
    expect(browserPartition(localB, "https://example.com/", "local")).toBe(web);
    expect(browserPartition(remoteA, "https://example.com/", "local")).toBe(web);
    expect(browserPartition(localA, "http://localhost:3000/", "local")).toBe(web);
    const remoteLoopback = browserPartition(remoteA, "http://localhost:3000/", "local");
    expect(browserPartition(remoteB, "http://127.0.0.1:3000/", "local")).toBe(remoteLoopback);
    expect(browserPartition(remoteB, "http://[::ffff:127.0.0.1]:3000/", "local")).toBe(remoteLoopback);
    expect(browserPartition(otherDevice, "http://localhost:3000/", "local")).not.toBe(remoteLoopback);
    expect(remoteLoopback).not.toBe(web);
    expect(browserPartition(localA, "file:///a/report.html", "local")).not.toBe(web);
    expect(browserPartition(localB, "file:///b/report.html", "local")).not.toBe(browserPartition(localA, "file:///a/report.html", "local"));
  });

  it("keeps absolute remote loopback requests on the View's SSH route", () => {
    const route = { source_url: "http://127.0.0.2:5173/app", url: "http://127.0.0.1:63001/app" };
    expect(remoteRequest(route, "http://localhost:5173/app.js")).toEqual({ redirectURL: "http://127.0.0.1:63001/app.js" });
    expect(remoteRequest(route, "ws://localhost:5173/live")).toEqual({ redirectURL: "ws://127.0.0.1:63001/live" });
    expect(remoteRequest(route, "http://localhost:9000/private")).toEqual({ cancel: true });
    expect(remoteRequest(route, "http://127.0.0.1:63001/app.js")).toEqual({});
    expect(remoteRequest(route, "http://[::ffff:127.0.0.1]:5173/app.js")).toEqual({ redirectURL: "http://127.0.0.1:63001/app.js" });
    expect(remoteRequest(route, "http://[::ffff:127.0.0.1]:9000/private")).toEqual({ cancel: true });
    expect(remoteRequest({ source_url: "http://127.1:5173/", url: "http://127.1:5173/" }, "http://127.1:5173/")).toEqual({ cancel: true });
    expect(remoteRequest({ source_url: "file:///checkout/page.html", url: "http://127.0.0.1:63002/secret/page.html" }, "https://example.com/leak")).toEqual({ cancel: true });
  });
});

describe("a page at a device size (PRD browser-width-zoom B11, B12)", () => {
  it("fits a wider desktop viewport into its rectangle, its width the device's at any page zoom", () => {
    const bounds = { x: 12, y: 60, width: 496, height: 600 };
    const at100 = deviceEmulation({ width: 1280, mobile: false }, bounds, 1);
    expect(at100).toMatchObject({ screenPosition: "desktop", viewSize: { width: 1280, height: Math.round(600 / (496 / 1280)) } });
    expect(at100.scale).toBeCloseTo(496 / 1280);
    // The page's zoom divides the viewport into CSS pixels, so it is laid out zoomed and drawn back at the same size.
    const at110 = deviceEmulation({ width: 1280, mobile: false }, bounds, 1.1);
    expect(at110.viewSize).toEqual({ width: 1408, height: Math.round((600 / (496 / 1280)) * 1.1) });
    expect(at110.scale! * at110.viewSize!.width).toBeCloseTo(496);
  });

  it("lays a narrower mobile viewport out at its own width and scale, which Chromium does not zoom", () => {
    const params = deviceEmulation({ width: 390, mobile: true }, { x: 0, y: 0, width: 390, height: 700 }, 1.25);
    expect(params).toMatchObject({ screenPosition: "mobile", viewSize: { width: 390, height: 700 }, screenSize: { width: 390, height: 700 }, scale: 1 });
  });
});

describe("keeping pages alive", () => {
  it("closes the hidden pages shown longest ago, never one on screen", () => {
    const views = [
      { key: "a", visible: true, shownAt: 1 },
      { key: "b", visible: false, shownAt: 5 },
      { key: "c", visible: false, shownAt: 2 },
      { key: "d", visible: false, shownAt: 9 },
    ];
    expect(overCap(views, 4)).toEqual([]);
    expect(overCap(views, 2)).toEqual(["c", "b"]);
    expect(overCap(views, 0)).toEqual(["c", "b", "d"]);
  });

  it("places a page on whole window points at the shell's zoom", () => {
    expect(toBounds({ x: 10.4, y: 20.6, width: 100.2, height: 50 }, 1)).toEqual({ x: 10, y: 21, width: 101, height: 50 });
    expect(toBounds({ x: 10, y: 20, width: 100, height: 50 }, 1.25)).toEqual({ x: 13, y: 25, width: 125, height: 63 });
    expect(toBounds(rect, Number.NaN)).toEqual(rect);
  });
});

describe("what a local HTML page may load", () => {
  // Paths are built for the system the test runs on: `file:///checkout/...` is no absolute path on Windows.
  const root = path.resolve("/checkout");
  const inside = (file: string) => file.startsWith(root + path.sep);
  const fileUrl = (...parts: string[]) => pathToFileURL(path.resolve(...parts)).href;
  const decide = (url: string, type = "image") => localPageRequest(url, type, inside);

  it("names this machine and private networks, in every spelling a URL normalizes to", () => {
    const local = ["localhost", "LOCALHOST.", "app.localhost", "printer.local", "router", "127.0.0.1", "127.8.8.8", "0.0.0.0", "10.1.2.3", "172.16.0.1", "172.31.255.1",
      "192.168.1.20", "169.254.169.254", "100.64.0.1", "198.18.0.1", "224.0.0.1", "[::1]", "[::]", "[fe80::1]", "[fd12:3456::1]", "[::ffff:7f00:1]", "[::ffff:a00:1]", "[64:ff9b::7f00:1]", "[ff02::1]"];
    for (const host of local) expect(isLocalNetworkHost(host), host).toBe(true);
    // A URL spells 2130706433 and 0x7f.1 as 127.0.0.1 before this sees it.
    expect(isLocalNetworkHost(new URL("http://2130706433/").hostname)).toBe(true);
    expect(isLocalNetworkHost(new URL("http://0x7f.1/").hostname)).toBe(true);
    for (const host of ["example.com", "cdn.jsdelivr.net", "8.8.8.8", "172.32.0.1", "172.15.0.1", "192.169.0.1", "100.128.0.1", "[2606:4700::1111]", "[::ffff:808:808]"]) {
      expect(isLocalNetworkHost(host), host).toBe(false);
    }
  });

  it("loads the page's own address, and files, styles, scripts, images and frames of its checkout", () => {
    expect(decide(fileUrl("/elsewhere/page.html"), "mainFrame")).toEqual({ cancel: false });
    for (const type of ["image", "stylesheet", "script", "font", "media", "subFrame"]) expect(decide(fileUrl(root, "a", "b.png"), type), type).toEqual({ cancel: false });
  });

  it("refuses a file outside its checkout, and any read of a file by a script", () => {
    for (const type of ["image", "script", "subFrame", "stylesheet"]) expect(decide(fileUrl("/etc/hosts"), type), type).toEqual({ cancel: true, boundary: "file" });
    expect(decide(`${pathToFileURL(root).href}/../etc/hosts`)).toEqual({ cancel: true, boundary: "file" });
    for (const type of ["xhr", "webSocket", "ping", "other"]) expect(decide(fileUrl(root, "a.json"), type), type).toEqual({ cancel: true, boundary: "file" });
    expect(decide("file://server/share/a.png")).toEqual({ cancel: true, boundary: "file" });
  });

  it("refuses this machine and private networks, and holds a public name until it is resolved", () => {
    for (const url of ["http://127.0.0.1:7000/", "http://localhost/", "https://192.168.0.5/x", "ws://[::1]:9/", "http://nas/", "http://10.0.0.1/", "http://2130706433/"]) {
      expect(decide(url, "xhr"), url).toEqual({ cancel: true, boundary: "network" });
    }
    expect(decide("https://example.com/a.js", "script")).toEqual({ resolve: "example.com" });
    expect(decide("https://8.8.8.8/")).toEqual({ cancel: false });
    expect(decide("data:text/plain,hi")).toEqual({ cancel: false });
    expect(decide("blob:null/1")).toEqual({ cancel: false });
    expect(decide("ftp://example.com/a")).toEqual({ cancel: true, boundary: "network" });
    expect(decide("not a url")).toEqual({ cancel: true, boundary: "network" });
  });
});

describe("pages no slot shows (PRD lane-desk-pair-view B32)", () => {
  it("lays one out at the lane's width for a page, or the window's when narrower, at the window's height", () => {
    expect(offScreenBounds({ width: 1400, height: 820 })).toEqual({ x: 0, y: 0, width: 900, height: 820 });
    expect(offScreenBounds({ width: 720.4, height: 600 })).toEqual({ x: 0, y: 0, width: 720, height: 600 });
    expect(offScreenBounds({ width: 0, height: 0 })).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });
});

// docs/BROWSER_DISPLAYS.md, A page in another computer's screen.
describe("another computer's screen's pages", () => {
  const workspace = "mini\u0000/web";
  const display = (id: string, extra: Record<string, unknown> = {}) => ({ id, area_id: "a1", url: `https://${id}.example/`, load: 1, rect: null, visible: false, ...extra });
  const retained = (ids: string[]) => ids.map((id) => ({ workspace, id, area_id: "a1" }));

  it("take a sync that names no grant, scope or epoch, within the screen's bounds", () => {
    const parsed = parseRemoteSync({ workspace, displays: [display("d1")], retained: retained(["d1"]), node: "mini", authorized_scopes: [{ workspace, area_id: "a1", incarnation: 1 }], attachment_epoch: "e1", external_grants: ["0".repeat(32)] });
    expect(parsed).toEqual({ workspace, displays: [display("d1")], retained: retained(["d1"]), node: "mini" });
    expect(parseRemoteSync({ workspace, displays: [display("g1", { url: "", grant: "0".repeat(32) })], retained: retained(["g1"]) })).toBeNull();
    const many = Array.from({ length: REMOTE_BOUNDS.placements + 1 }, (_, n) => `d${n}`);
    expect(parseRemoteSync({ workspace, displays: many.map((id) => display(id)), retained: retained(many) })).toBeNull();
    expect(parseRemoteSync({ workspace: null, displays: [], retained: retained(Array.from({ length: REMOTE_BOUNDS.retained + 1 }, (_, n) => `r${n}`)) })).toBeNull();
  });

  it("keep one login storage per machine, key and pairing, never one of this window's", () => {
    const key = "ab".repeat(32);
    const pairing = "11".repeat(16);
    const partition = remotePagePartition("mini", key, pairing);
    expect(partition).toMatch(/^persist:hide-remote-page-[0-9a-f]{32}$/);
    expect(remotePagePartition("mini", key, pairing)).toBe(partition);
    expect(remotePagePartition("mini", "cd".repeat(32), pairing)).not.toBe(partition);
    expect(remotePagePartition("pc", key, pairing)).not.toBe(partition);
    expect(remotePagePartition("mini", key, "22".repeat(16))).not.toBe(partition);
    for (const url of ["https://mini.example/", "http://localhost:3000/", "file:///Users/me/a.html"]) {
      expect(browserPartition(workspace, url, null)).not.toBe(partition);
      expect(browserPartition(workspace, url, "mini")).not.toBe(partition);
    }
  });

  // TF2: a checkout's HTML of that computer reads none of the screen's
  // logins, nor anything of this window's, and keeps nothing on disk.
  it("keep file pages in memory, apart from the logins of every machine and of this window", () => {
    const key = "ab".repeat(32);
    const pairing = "11".repeat(16);
    const files = remoteFilePartition("mini", key, pairing);
    expect(files).toMatch(/^hide-remote-file-[0-9a-f]{32}$/);
    expect(files.startsWith("persist:")).toBe(false);
    expect(remoteFilePartition("mini", key, "22".repeat(16))).not.toBe(files);
    expect(remoteFilePartition("pc", key, pairing)).not.toBe(files);
    expect(files).not.toBe(remotePagePartition("mini", key, pairing));
    for (const url of ["https://mini.example/", "http://localhost:3000/", "file:///Users/me/a.html"]) expect(browserPartition(workspace, url, "mini")).not.toBe(files);
  });

  // A routed file page's address is that computer's, in its own spelling
  // whatever system this one runs, escaped as a URL path needs.
  it("spell another computer's file address from its wire root by URL alone", () => {
    expect(wireFileAddress("C:/repo", "report.html", "", "#section")).toBe("file:///C:/repo/report.html#section");
    expect(wireFileAddress("C:/repo/", "docs/a.html", "?v=2")).toBe("file:///C:/repo/docs/a.html?v=2");
    expect(wireFileAddress("/Users/me/repo", "a b/100%.html#x?", "", "#top")).toBe("file:///Users/me/repo/a%20b/100%25.html%23x%3F#top");
    expect(wireFileAddress("/srv/문서", "보고서.html")).toBe(`file:///srv/${encodeURIComponent("문서")}/${encodeURIComponent("보고서.html")}`);
    for (const relative of ["../secret.html", "a/../../b.html", "", "a//b.html"]) expect(wireFileAddress("/srv/repo", relative), relative).toBeNull();
    expect(wireFileAddress("", "a.html")).toBeNull();
    // A Windows share keeps its two slashes under an empty host, as that
    // machine spells it (`to_file_url`), never `file://server/`.
    expect(wireFileAddress("//server/share/repo", "report.html", "?v=2", "#section")).toBe("file:////server/share/repo/report.html?v=2#section");
    expect(wireFileAddress("//server/share/repo/", "docs/a b.html")).toBe("file:////server/share/repo/docs/a%20b.html");
    expect(new URL(wireFileAddress("//server/share/repo", "r.html")!).host).toBe("");
    expect(sameFileAddress("file:////server/share/repo/r.html#a", "file:////server/share/repo/r.html?b")).toBe(true);
    expect(sameFileAddress("file:////server/share/repo/r.html", "file:///server/share/repo/r.html")).toBe(false);
  });

  // TF1: a file page of that computer loads only from its own loopback over
  // plain HTTP, which the screen's proxy carries there; a web page only from
  // its own address.
  it("load a file page only from that computer's loopback route, and a web page only from its address", () => {
    const file = "file:///Users/me/repo/report.html";
    for (const url of ["http://127.0.0.1:5000/s/report.html", "http://127.0.0.2:5000/s/report.html", "http://[::1]:5000/s/report.html"]) {
      expect(remoteRouteAddress({ url, source_url: file }), url).toBe(true);
    }
    for (const url of [file, "file:///etc/passwd", "https://127.0.0.1:5000/s/report.html", "http://localhost:5000/s/report.html", "http://127.0.0.1/s/report.html", "http://files.example:5000/s/report.html", "http://10.0.0.2:5000/s/report.html", "http://u:p@127.0.0.1:5000/s/report.html", "not a url"]) {
      expect(remoteRouteAddress({ url, source_url: file }), url).toBe(false);
    }
    expect(remoteRouteAddress({ url: "https://a.example/", source_url: "https://a.example/" })).toBe(true);
    expect(remoteRouteAddress({ url: "http://127.0.0.1:5000/", source_url: "https://a.example/" })).toBe(false);
  });

  it("are cut to the screen's tile, scaled by its zoom, and hidden when nothing of them is inside", () => {
    const tile = { x: 100, y: 50, width: 800, height: 600 };
    expect(placeIn({ x: 10, y: 20, width: 300, height: 200 }, 1, tile)).toEqual({ x: 110, y: 70, width: 300, height: 200 });
    expect(placeIn({ x: 10, y: 20, width: 300, height: 200 }, 2, tile)).toEqual({ x: 120, y: 90, width: 600, height: 400 });
    expect(placeIn({ x: 700, y: -40, width: 400, height: 300 }, 1, tile)).toEqual({ x: 800, y: 50, width: 100, height: 260 });
    expect(placeIn({ x: -500, y: 0, width: 400, height: 300 }, 1, tile)).toBeNull();
    expect(placeIn({ x: 900, y: 0, width: 400, height: 300 }, 1, tile)).toBeNull();
  });

  it("reach a local network only from a document on one, and only a top-level navigation decides its own", () => {
    const rows: [string, string | null, boolean, boolean][] = [
      ["https://cdn.example/app.js", "https://app.example/", false, true],
      ["http://127.0.0.1:5173/api", "https://app.example/", false, false],
      ["ws://localhost:5173/hmr", "https://app.example/", false, false],
      ["http://192.168.0.4/", "https://app.example/", false, false],
      ["http://printer.local/", "https://app.example/", false, false],
      ["http://localhost:5173/app.js", "http://localhost:5173/", false, true],
      ["http://10.0.0.2/api", "http://192.168.0.4/", false, true],
      ["http://127.0.0.1:5173/", null, false, false],
      ["https://cdn.example/a.js", null, false, true],
      ["http://localhost:5173/", "https://app.example/", true, true],
      ["data:text/plain,x", "https://app.example/", false, true],
    ];
    for (const [url, top, mainFrame, allowed] of rows) expect(localNetworkReach(url, top, mainFrame), `${url} from ${top}`).toBe(allowed);
  });
});
