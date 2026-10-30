import { EventEmitter } from "node:events";
import { request } from "node:http";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WebSocket } from "ws";
import { BrowserCdpGateway, CdpActionUncertain, parseBrowserControlResult, type CdpAction, type CdpActionResult, type CdpPage, type CdpRetirement, type CdpScope } from "./browserCdp";

type Json = Record<string, unknown>;
// Electron is the platform boundary. HTTP, WS, scope policy and lifecycle
// remain real; this debugger represents observable native page state.
class ElectronDebugger extends EventEmitter {
  private attached = false;
  cookie = "unchanged";
  file = "unchanged";
  readonly drags: Json[] = [];
  closed = false;
  private sequence = 0;
  isAttached(): boolean { return this.attached; }
  attach(): void { if (this.attached) throw new Error("Already attached"); this.attached = true; }
  detach(): void { this.attached = false; this.emit("detach", {}, "target closed"); }
  async sendCommand(method: string, params: Json = {}, sessionId?: string): Promise<unknown> {
    if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "native-owned-page", url: "https://page.example/" }, childFrames: [{ frame: { id: "real-iframe", parentId: "native-owned-page" } }] } };
    if (method === "Page.createIsolatedWorld") {
      if (params.frameId !== "native-owned-page") throw new Error("Unknown native frame");
      this.emit("message", {}, "Runtime.executionContextCreated", { context: { id: 42, name: params.worldName, auxData: { frameId: params.frameId, isDefault: false } } }, sessionId ?? "");
      return { executionContextId: 42 };
    }
    if (method === "Runtime.evaluate") return { result: { type: "string", value: sessionId ?? "page" } };
    if (method === "Target.getTargetInfo") return { targetInfo: { targetId: "native-owned-page", type: "page" } };
    if (method === "Target.attachToTarget") {
      if (params.targetId !== "native-owned-page") throw new Error("Not this debugger's page");
      const native = `native-alias-${++this.sequence}`;
      this.emit("message", {}, "Target.attachedToTarget", { sessionId: native, targetInfo: { targetId: "native-owned-page", type: "page" } }, "");
      return { sessionId: native };
    }
    if (method === "Target.detachFromTarget") this.emit("message", {}, "Target.detachedFromTarget", { sessionId: params.sessionId }, "");
    if (method === "Network.setCookie" || method === "Storage.setCookies") this.cookie = "changed";
    if (method === "DOM.setFileInputFiles") this.file = "changed";
    if (method === "Input.dispatchDragEvent") this.drags.push(params);
    if (method === "Browser.close") this.closed = true;
    if (method === "Network.getCookies") return { cookies: [{ name: "allowed", domain: new URL((params.urls as string[])[0]!).hostname }] };
    return {};
  }
}
function electronPage(contentsId: number, id: string, area_id = "a1", workspace = "local\0/checkout"): CdpPage & { contents: { debugger: ElectronDebugger } } {
  return { workspace, id, area_id, contents: { id: contentsId, debugger: new ElectronDebugger(), isDestroyed: () => false, getURL: () => `https://page${contentsId}.example/`, getTitle: () => `Page ${contentsId}` } };
}

class CdpClient {
  readonly socket: WebSocket;
  readonly events: Json[] = [];
  private sequence = 0;
  private readonly pending = new Map<number, (reply: Json) => void>();
  constructor(url: string, options?: { headers?: Record<string, string> }) {
    this.socket = new WebSocket(url, options);
    this.socket.on("message", (data) => {
      const reply = JSON.parse(data.toString()) as Json;
      if (typeof reply.id === "number") this.pending.get(reply.id)?.(reply);
      else this.events.push(reply);
    });
  }
  ready(): Promise<void> { return new Promise((resolve, reject) => { this.socket.once("open", resolve); this.socket.once("error", reject); }); }
  call(method: string, params: Json = {}, sessionId?: string, id = ++this.sequence, timeoutMs = 2000): Promise<Json> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error(`No reply: ${method}`)), timeoutMs);
      this.pending.set(id, (reply) => { clearTimeout(timeout); this.pending.delete(id); resolve(reply); });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
}

const shutdown: (() => Promise<void>)[] = [];
afterEach(async () => { for (const close of shutdown.splice(0).reverse()) await close(); });
async function fixture(options: { loseFirstOpenResponse?: boolean; alwaysLoseResponse?: boolean; refuseOpen?: boolean; deferCloseRetirement?: boolean; revive?: (scope: CdpScope, pages: CdpPage[]) => Promise<"none" | "loaded" | "failed"> } = {}) {
  const first = electronPage(11, "d1");
  const second = electronPage(12, "d2");
  const foreign = electronPage(13, "d3", "a2");
  const otherCheckout = electronPage(14, "d4", "a1", "local\0/other");
  const pages: CdpPage[] = [first, second, foreign, otherCheckout];
  const listeners = new Set<(retirement?: CdpRetirement) => void>();
  const attached = new Map<string, boolean>();
  const intents = new Map<string, CdpActionResult>();
  const opened: { scope: CdpScope; url: string | undefined }[] = [];
  const actions: CdpAction[] = [];
  const externalOpens: { path: string; page: boolean }[] = [];
  let sequence = 20;
  let incarnation: number | null = 1;
  const changed = (retirement?: CdpRetirement) => { for (const listener of listeners) listener(retirement); };
  let time = Date.now();
  const gateway = new BrowserCdpGateway({
    pages: () => pages,
    incarnation: () => incarnation,
    changed: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    attached: (page, value) => { attached.set(page.id, value); },
    ...(options.revive ? { revive: (scope: CdpScope) => options.revive!(scope, pages) } : {}),
    externalOpen: async (path, page) => { externalOpens.push({ path, page }); return { grant: { id: "a1".repeat(16), name: "r.html", kind: page ? "html" : "text" } }; },
    chromeVersion: "144.0.0.0",
    now: () => time,
    log: () => {},
    // The daemon/core boundary owns creation, membership, close and select.
    action: async (scope, action) => {
      actions.push(action);
      if (options.refuseOpen && action.action === "open") throw new Error("Definitive core refusal");
      if (time - Number(action.request_id.split("-")[0]) > 10 * 60_000) throw new Error("Core intent expired");
      const previous = intents.get(action.request_id);
      if (previous) {
        if (options.alwaysLoseResponse) throw new CdpActionUncertain();
        return previous;
      }
      const view_id = action.display_id ?? `d${++sequence}`;
      if (action.action === "open") {
        opened.push({ scope, url: action.url });
        pages.push(electronPage(sequence + 100, view_id, scope.area_id, scope.workspace));
      } else if (action.action === "close") {
        const index = pages.findIndex((page) => page.id === view_id && page.workspace === scope.workspace && page.area_id === scope.area_id);
        if (index < 0) throw new Error("Not owned by this area");
        const [page] = pages.splice(index, 1);
        if (!options.deferCloseRetirement) changed({ contentsId: page!.contents.id, reason: "closed" });
      }
      const result = { view_id, area_id: scope.area_id, load: 1 };
      intents.set(action.request_id, result);
      if (action.action !== "close" || !options.deferCloseRetirement) changed();
      if (options.alwaysLoseResponse || options.loseFirstOpenResponse && action.action === "open") throw new CdpActionUncertain();
      return result;
    },
  });
  const address = await gateway.start();
  gateway.setAvailable(true);
  shutdown.push(() => gateway.close());
  async function capability(scope: CdpScope = { workspace: first.workspace, area_id: "a1" }) {
    const response = await fetch(`${address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body: JSON.stringify(scope) });
    expect(response.status).toBe(200);
    return await response.json() as { cdp_http_url: string; browser_ws_url: string; display_id?: string };
  }
  async function client(url: string) {
    const client = new CdpClient(url);
    await client.ready();
    shutdown.push(async () => { client.socket.terminate(); });
    return client;
  }
  return { gateway, address, capability, client, first, second, foreign, pages, changed, attached, opened, actions, externalOpens,
    authority: (value: boolean) => { incarnation = value ? 1 : null; changed(); },
    incarnate: (value: number) => { incarnation = value; changed(); }, advance: (milliseconds: number) => { time += milliseconds; } };
}
async function customRequest(url: string, method: string, headers: Record<string, string>, body = ""): Promise<number> {
  return new Promise((resolve, reject) => {
    const req = request(url, { method, headers }, (response) => { response.resume(); resolve(response.statusCode ?? 0); });
    req.on("error", reject);
    req.end(body);
  });
}
async function attach(client: CdpClient, targetId = "page-11", flatten = true): Promise<string> {
  const response = await client.call("Target.attachToTarget", { targetId, flatten });
  expect(response.error).toBeUndefined();
  return (response.result as Json).sessionId as string;
}

describe("scoped desktop CDP public boundary", () => {
  it("settles startup and releases its subscription when the app quits before listening", async () => {
    let subscribed = true;
    const gateway = new BrowserCdpGateway({ pages: () => [], incarnation: () => null, changed: () => () => { subscribed = false; },
      action: async () => { throw new Error("No action expected"); }, attached: () => {}, log: () => {}, chromeVersion: "144.0.0.0" });
    const started = gateway.start().then(() => "listened", () => "closed");
    const closed = gateway.close();
    expect(gateway.close()).toBe(closed);
    await closed;
    expect(await started).toBe("closed");
    expect(subscribed).toBe(false);
    await expect(gateway.start()).rejects.toThrow("closed");
  });
  it("accepts the real daemon result envelope and optional close/select load", () => {
    expect(parseBrowserControlResult({ ok: true, result: { view_id: "d1", area_id: "a1", load: 7 } })).toEqual({ view_id: "d1", area_id: "a1", load: 7 });
    expect(parseBrowserControlResult({ ok: true, result: { view_id: "d1", area_id: "a1" } })).toEqual({ view_id: "d1", area_id: "a1", load: null });
    expect(parseBrowserControlResult({ ok: true, result: { view_id: "d1", area_id: "a1", load: null } })).toEqual({ view_id: "d1", area_id: "a1", load: null });
    expect(parseBrowserControlResult({ view_id: "d1", area_id: "a1", load: 7 })).toBeNull();
    expect(parseBrowserControlResult({ ok: false, result: { view_id: "d1" } })).toBeNull();
  });
  it("requires the private bearer and exposes no raw discovery or raw WS route", async () => {
    const { address } = await fixture();
    const scope = JSON.stringify({ workspace: "local\0/checkout", area_id: "a1" });
    expect(await customRequest(`${address.endpoint}/connect`, "POST", {}, scope)).toBe(401);
    expect(await customRequest(`${address.endpoint}/connect`, "POST", { authorization: `Bearer ${"x".repeat(64)}` }, scope)).toBe(401);
    expect((await fetch(`${address.endpoint}/json/version`)).status).toBe(404);
    const raw = new CdpClient(`${address.endpoint.replace("http:", "ws:")}/devtools/browser`);
    await expect(raw.ready()).rejects.toThrow("403");
  });

  it("rejects rebinding Hosts, cross-origin requests, and WS upgrades", async () => {
    const { address, capability } = await fixture();
    const cap = await capability();
    expect(await customRequest(`${cap.cdp_http_url}/json`, "GET", { host: "evil.example" })).toBe(403);
    expect(await customRequest(`${cap.cdp_http_url}/json`, "GET", { origin: "https://evil.example" })).toBe(403);
    expect(await customRequest(`${cap.cdp_http_url}/json`, "GET", { "x-forwarded-host": "evil.example" })).toBe(403);
    const crossOrigin = new CdpClient(cap.browser_ws_url, { headers: { origin: "https://evil.example" } });
    await expect(crossOrigin.ready()).rejects.toThrow("403");
    expect(address.token).not.toContain(cap.cdp_http_url.split("/").at(-1));
  });

  it("bounds control bodies and rejects invalid workspace or display identity", async () => {
    const { address } = await fixture();
    const headers = { authorization: `Bearer ${address.token}` };
    expect(await customRequest(`${address.endpoint}/connect`, "POST", headers, JSON.stringify({ workspace: "/checkout", area_id: "a1" }))).toBe(400);
    expect(await customRequest(`${address.endpoint}/connect`, "POST", headers, JSON.stringify({ workspace: "local\0/checkout", area_id: "a1", display_id: "shell\0renderer" }))).toBe(400);
    expect(await customRequest(`${address.endpoint}/connect`, "POST", headers, `{"padding":"${"x".repeat(17 * 1024)}"}`)).toBe(413);
  });

  it("lists only eligible native pages in HTTP and synthetic browser Target discovery", async () => {
    const { capability, client } = await fixture();
    const cap = await capability();
    const list = await (await fetch(`${cap.cdp_http_url}/json/list`)).json() as Json[];
    expect(list.map((page) => page.id)).toEqual(["page-11", "page-12"]);
    const browser = await client(cap.browser_ws_url);
    const reply = await browser.call("Target.getTargets");
    const targets = (reply.result as Json).targetInfos as Json[];
    expect(targets.filter((page) => page.type === "page").map((page) => page.targetId)).toEqual(["page-11", "page-12"]);
    expect(typeof targets[0]!.browserContextId).toBe("string");
    expect(targets[0]!.browserContextId).not.toBe("");
    expect(targets.every((page) => page.browserContextId === targets[0]!.browserContextId)).toBe(true);
    expect((await browser.call("Target.getBrowserContexts")).result).toEqual({ browserContextIds: [] });
    const display = await capability({ workspace: "local\0/checkout", area_id: "a1", display_id: "d1" });
    expect(display.display_id).toBe("d1");
    expect((await (await fetch(`${display.cdp_http_url}/json`)).json() as Json[]).map((page) => page.id)).toEqual(["page-11"]);
    const outside = new CdpClient(`${display.browser_ws_url.replace("/browser", "/page/page-12")}`);
    await expect(outside.ready()).rejects.toThrow("403");
  });

  it("denies foreign targets and cross-client sessions through every attach path", async () => {
    const { capability, client } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    expect((await browser.call("Target.attachToTarget", { targetId: "page-13", flatten: true })).error).toBeDefined();
    expect((await browser.call("Target.closeTarget", { targetId: "page-14" })).error).toBeDefined();
    const session = await attach(browser);
    const other = await client(cap.browser_ws_url);
    expect((await other.call("Runtime.evaluate", { expression: "1" }, session)).error).toBeDefined();
    expect((await other.call("Target.sendMessageToTarget", { sessionId: session, message: JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: "1" } }) })).error).toBeDefined();
  });

  it("provides a scoped browser session for independent page sessions without native browser authority", async () => {
    const { first, capability, client } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    const rootPage = await attach(browser);
    const aliasReply = await browser.call("Target.attachToBrowserTarget");
    expect(aliasReply.error).toBeUndefined();
    const alias = (aliasReply.result as Json).sessionId as string;
    const other = await client(cap.browser_ws_url);
    expect((await other.call("Target.getTargets", {}, alias)).error).toBeDefined();
    for (const method of ["Browser.close", "Browser.setDownloadBehavior", "Target.createBrowserContext", "Target.disposeBrowserContext", "Storage.getCookies"]) {
      expect((await browser.call(method, {}, alias)).error, method).toBeDefined();
    }
    expect((await browser.call("Target.attachToTarget", { targetId: "page-13", flatten: true }, alias)).error).toBeDefined();
    const pageReply = await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, alias);
    expect(pageReply.error).toBeUndefined();
    const page = (pageReply.result as Json).sessionId as string;
    expect(page).not.toBe(rootPage);
    expect(browser.events).toContainEqual(expect.objectContaining({ method: "Target.attachedToTarget", sessionId: alias, params: expect.objectContaining({ sessionId: page }) }));
    expect((await browser.call("Runtime.evaluate", { expression: "'page'" }, page)).result).toEqual({ result: { type: "string", value: "native-alias-1" } });
    expect((await browser.call("Page.navigate", { url: "file:///outside/private.txt" }, page)).error).toBeDefined();
    expect((await browser.call("Target.detachFromTarget", { sessionId: rootPage }, alias)).error).toBeDefined();
    expect((await browser.call("Target.detachFromTarget", { sessionId: alias })).error).toBeUndefined();
    expect(browser.events).toContainEqual(expect.objectContaining({ method: "Target.detachedFromTarget", sessionId: alias, params: expect.objectContaining({ sessionId: page }) }));
    expect((await browser.call("Runtime.evaluate", {}, page)).error).toBeDefined();
    expect((await browser.call("Target.getTargets", {}, alias)).error).toBeDefined();
    expect((await browser.call("Runtime.evaluate", {}, rootPage)).error).toBeUndefined();
    expect(first.contents.debugger.isAttached()).toBe(true);
  });

  it("counts virtual browser sessions in the same attachment cap and releases their native children", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const aliases: string[] = [];
    for (let count = 0; count < 64; count++) {
      const reply = await browser.call("Target.attachToBrowserTarget");
      expect(reply.error).toBeUndefined();
      aliases.push((reply.result as Json).sessionId as string);
    }
    expect(new Set(aliases).size).toBe(64);
    expect((await browser.call("Target.attachToBrowserTarget")).error).toBeDefined();
    expect((await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, aliases[0])).error).toBeDefined();
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect((await browser.call("Target.detachFromTarget", { sessionId: aliases.pop() })).error).toBeUndefined();
    const pageReply = await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, aliases[0]);
    expect(pageReply.error).toBeUndefined();
    const page = (pageReply.result as Json).sessionId as string;
    expect((await browser.call("Target.detachFromTarget", { sessionId: aliases[0] })).error).toBeUndefined();
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect((await browser.call("Runtime.evaluate", {}, page)).error).toBeDefined();
  });

  it("detaches only a virtual parent's independent page while another parent keeps the same page", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const alias = async () => ((await browser.call("Target.attachToBrowserTarget")).result as Json).sessionId as string;
    const parentA = await alias(), parentB = await alias();
    const pageA = ((await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, parentA)).result as Json).sessionId as string;
    const pageB = ((await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, parentB)).result as Json).sessionId as string;
    expect(pageA).not.toBe(pageB);
    expect((await browser.call("Target.detachFromTarget", { sessionId: parentA })).error).toBeUndefined();
    expect((await browser.call("Runtime.evaluate", {}, pageA)).error).toBeDefined();
    expect((await browser.call("Runtime.evaluate", {}, pageB)).result).toEqual({ result: { type: "string", value: "native-alias-2" } });
    expect(first.contents.debugger.isAttached()).toBe(true);
    expect((await browser.call("Target.detachFromTarget", { sessionId: parentB })).error).toBeUndefined();
    expect(first.contents.debugger.isAttached()).toBe(false);
  });

  it("revokes a virtual parent before awaiting detach and refuses its in-flight or later child admission", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const parent = ((await browser.call("Target.attachToBrowserTarget")).result as Json).sessionId as string;
    const page = ((await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, parent)).result as Json).sessionId as string;
    first.contents.debugger.emit("message", {}, "Target.attachedToTarget", { sessionId: "native-iframe", targetInfo: { targetId: "owned-iframe", type: "iframe", url: "https://frame.example/" } }, "native-alias-1");
    expect((await browser.call("Browser.getVersion")).error).toBeUndefined();
    const iframe = ((browser.events.find((event) => event.method === "Target.attachedToTarget" && event.sessionId === page)!.params as Json).sessionId) as string;
    expect((await browser.call("Runtime.evaluate", {}, iframe)).error).toBeUndefined();
    const original = first.contents.debugger.sendCommand.bind(first.contents.debugger);
    let releaseAttach!: () => void, releaseDetach!: () => void, observeAttach!: () => void;
    const attachBlocked = new Promise<void>((resolve) => { releaseAttach = resolve; });
    const detachBlocked = new Promise<void>((resolve) => { releaseDetach = resolve; });
    const attachObserved = new Promise<void>((resolve) => { observeAttach = resolve; });
    const boundary = vi.spyOn(first.contents.debugger, "sendCommand").mockImplementation(async (method, params, nativeId) => {
      if (method === "Target.attachToTarget") { observeAttach(); await attachBlocked; }
      if (method === "Target.detachFromTarget" && params?.sessionId === "native-alias-1") await detachBlocked;
      return original(method, params, nativeId);
    });
    try {
      const inFlight = browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, parent);
      await attachObserved;
      const detached = browser.call("Target.detachFromTarget", { sessionId: parent });
      expect((await browser.call("Browser.getVersion")).error).toBeUndefined();
      expect((await browser.call("Runtime.evaluate", {}, page)).error).toBeDefined();
      expect((await browser.call("Runtime.evaluate", {}, iframe)).error).toBeDefined();
      first.contents.debugger.emit("message", {}, "Target.attachedToTarget", { sessionId: "late-native-iframe", targetInfo: { targetId: "late-iframe", type: "iframe", url: "https://frame.example/" } }, "native-alias-1");
      expect((await browser.call("Browser.getVersion")).error).toBeUndefined();
      expect(browser.events.some((event) => event.method === "Target.attachedToTarget" && (event.params as Json).targetInfo && ((event.params as Json).targetInfo as Json).targetId === "late-iframe")).toBe(false);
      expect((await browser.call("Target.attachToTarget", { targetId: "page-12", flatten: true }, parent)).error).toBeDefined();
      releaseDetach();
      expect((await detached).error).toBeUndefined();
      releaseAttach();
      expect((await inFlight).error).toBeDefined();
      expect((await browser.call("Runtime.evaluate", {}, iframe)).error).toBeDefined();
      expect(first.contents.debugger.isAttached()).toBe(false);
      expect(browser.events.filter((event) => event.method === "Target.attachedToTarget" && event.sessionId === parent)).toHaveLength(1);
    } finally { releaseAttach(); releaseDetach(); boundary.mockRestore(); }
  });

  it("includes main-frame preparation in the native command deadline and releases the lease", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const original = first.contents.debugger.sendCommand.bind(first.contents.debugger);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const boundary = vi.spyOn(first.contents.debugger, "sendCommand").mockImplementation(async (method, params, nativeId) => {
      if (method === "Page.getFrameTree" || method === "Runtime.evaluate") await new Promise((resolve) => setTimeout(resolve, 9000));
      return original(method, params, nativeId);
    });
    try {
      const session = await attach(browser);
      const reply = browser.call("Runtime.evaluate", { expression: "'page'" }, session, 500, 25_000);
      // A following protocol reply attests that the real socket delivered
      // the command before advancing only platform timers. Two nine-second operations must
      // still fail at the documented ten-second command deadline.
      expect((await browser.call("Browser.getVersion")).error).toBeUndefined();
      await vi.advanceTimersByTimeAsync(9000);
      expect(boundary.mock.calls.filter(([method]) => method === "Runtime.evaluate")).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1000);
      expect(await reply).toMatchObject({ error: { message: "CDP request timed out" } });
      expect(first.contents.debugger.isAttached()).toBe(false);
      await vi.advanceTimersByTimeAsync(8000);
      expect((await browser.call("Runtime.evaluate", {}, session)).error).toBeDefined();
    } finally { boundary.mockRestore(); vi.useRealTimers(); }
  });

  it("shares one command deadline across every child of a virtual parent and retires them on timeout", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const parent = ((await browser.call("Target.attachToBrowserTarget")).result as Json).sessionId as string;
    const pages: string[] = [];
    for (let count = 0; count < 2; count++) pages.push(((await browser.call("Target.attachToTarget", { targetId: "page-11", flatten: true }, parent)).result as Json).sessionId as string);
    const original = first.contents.debugger.sendCommand.bind(first.contents.debugger);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const boundary = vi.spyOn(first.contents.debugger, "sendCommand").mockImplementation(async (method, params, nativeId) => {
      if (method === "Target.detachFromTarget") await new Promise((resolve) => setTimeout(resolve, 9000));
      return original(method, params, nativeId);
    });
    try {
      const reply = browser.call("Target.detachFromTarget", { sessionId: parent }, undefined, 500, 25_000);
      expect((await browser.call("Browser.getVersion")).error).toBeUndefined();
      await vi.advanceTimersByTimeAsync(9000);
      await vi.advanceTimersByTimeAsync(1000);
      expect(await reply).toMatchObject({ error: { message: "CDP request timed out" } });
      expect(first.contents.debugger.isAttached()).toBe(false);
      for (const page of pages) expect((await browser.call("Runtime.evaluate", {}, page)).error).toBeDefined();
      expect((await browser.call("Target.getTargets", {}, parent)).error).toBeDefined();
      await vi.advanceTimersByTimeAsync(8000);
    } finally { boundary.mockRestore(); vi.useRealTimers(); }
  });

  it("forwards page commands while rejecting browser shutdown, cookie and filesystem mutation", async () => {
    const { first, capability, client, attached } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser);
    expect(attached.get("d1")).toBe(true);
    expect((await browser.call("Runtime.evaluate", { expression: "'page'" }, session)).result).toEqual({ result: { type: "string", value: "page" } });
    for (const method of ["Browser.close", "Browser.setDownloadBehavior", "DOM.setFileInputFiles", "DOM.getFileInfo", "Network.setCookie", "Storage.setCookies", "Network.clearBrowserCookies", "Network.getAllCookies", "Network.loadNetworkResource", "Fetch.takeResponseBodyAsStream"]) {
      expect((await browser.call(method, {}, session)).error, method).toBeDefined();
    }
    expect(first.contents.debugger.closed).toBe(false);
    expect(first.contents.debugger.cookie).toBe("unchanged");
    expect(first.contents.debugger.file).toBe("unchanged");
    expect((await browser.call("Page.navigate", { url: "file:///outside/private.txt" }, session)).error).toBeDefined();
    for (const method of ["Fetch.continueRequest", "Page.getResourceContent"]) expect((await browser.call(method, { url: "file:///outside/private.txt" }, session)).error).toBeDefined();
    expect((await browser.call("Target.createBrowserContext")).error).toBeDefined();
  });

  it("refuses a drag that carries files and forwards one that carries none", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser);
    const drag = { type: "drop", x: 1, y: 2 };
    for (const files of [["/outside/private.txt"], "/outside/private.txt", null]) {
      expect((await browser.call("Input.dispatchDragEvent", { ...drag, data: { items: [], files } }, session)).error, JSON.stringify(files)).toBeDefined();
    }
    expect(first.contents.debugger.drags).toEqual([]);
    for (const data of [{ items: [{ mimeType: "text/plain", data: "hello" }] }, { items: [], files: [] }]) {
      expect((await browser.call("Input.dispatchDragEvent", { ...drag, data }, session)).error).toBeUndefined();
    }
    expect(first.contents.debugger.drags).toHaveLength(2);
  });

  it("refuses multilevel legacy envelopes before any mutation reaches the debugger or core", async () => {
    const { first, capability, client, pages, actions } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser, "page-11", false);
    const dangerous = { id: 91, method: "Page.close" };
    const nested = { id: 90, method: "Target.sendMessageToTarget", params: { sessionId: session, message: JSON.stringify(dangerous) } };
    const response = await browser.call("Target.sendMessageToTarget", { sessionId: session, message: JSON.stringify(nested) });
    expect((response.error as Json).message).toContain("Multilevel legacy");
    expect(first.contents.debugger.closed).toBe(false);
    expect(pages.some((page) => page.id === first.id)).toBe(true);
    expect(actions).toHaveLength(0);
  });

  it("admits OOPIF descendants but never unrelated native targets or unowned sessions", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const root = await attach(browser);
    await browser.call("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }, root);
    first.contents.debugger.emit("message", {}, "Target.attachedToTarget", { sessionId: "native-iframe", targetInfo: { targetId: "real-iframe", parentFrameId: "native-owned-page", type: "iframe", url: "https://iframe.example" } }, "");
    await new Promise((resolve) => setTimeout(resolve, 10));
    const attachment = browser.events.find((event) => event.method === "Target.attachedToTarget" && (event.params as Json).sessionId !== root && event.sessionId === root);
    expect(attachment).toBeDefined();
    const parentInfo = (await browser.call("Target.getTargetInfo", {}, root)).result as Json;
    expect(((attachment!.params as Json).targetInfo as Json).browserContextId).toBe((parentInfo.targetInfo as Json).browserContextId);
    expect((attachment!.params as Json).targetInfo).toMatchObject({ targetId: "real-iframe", parentFrameId: "page-11" });
    const child = (attachment!.params as Json).sessionId as string;
    expect(child).not.toBe("native-iframe");
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, child)).result).toEqual({ result: { type: "string", value: "native-iframe" } });
    first.contents.debugger.emit("message", {}, "Target.attachedToTarget", { sessionId: "native-popup", targetInfo: { targetId: "secret-popup", type: "page", url: "https://secret.example" } }, "");
    first.contents.debugger.emit("message", {}, "Runtime.consoleAPICalled", { secret: "unowned" }, "not-owned");
    await browser.call("Browser.getVersion");
    expect(JSON.stringify(browser.events)).not.toContain("secret-popup");
    expect(JSON.stringify(browser.events)).not.toContain("https://secret.example");
    expect(JSON.stringify(browser.events)).not.toContain("unowned");
    expect((await browser.call("Runtime.evaluate", {}, "native-popup")).error).toBeDefined();
  });

  it("keeps public main-frame identity consistent across frame trees, commands and contexts", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser);
    const tree = (await browser.call("Page.getFrameTree", {}, session)).result as Json;
    expect(tree.frameTree).toMatchObject({ frame: { id: "page-11" }, childFrames: [{ frame: { id: "real-iframe", parentId: "page-11" } }] });
    const world = await browser.call("Page.createIsolatedWorld", { frameId: "page-11", worldName: "client-world" }, session);
    expect(world.error).toBeUndefined();
    expect(world.result).toEqual({ executionContextId: 42 });
    expect(browser.events.find((event) => event.method === "Runtime.executionContextCreated")).toMatchObject({ sessionId: session, params: { context: { id: 42, name: "client-world", auxData: { frameId: "page-11" } } } });
    first.contents.debugger.emit("message", {}, "Page.frameNavigated", { frame: { id: "native-owned-page", url: "https://page.example/" } }, "");
    first.contents.debugger.emit("message", {}, "Page.frameAttached", { frameId: "real-iframe", parentFrameId: "native-owned-page" }, "");
    await browser.call("Browser.getVersion");
    expect(browser.events.find((event) => event.method === "Page.frameNavigated")).toMatchObject({ params: { frame: { id: "page-11" } } });
    expect(browser.events.find((event) => event.method === "Page.frameAttached")).toMatchObject({ params: { frameId: "real-iframe", parentFrameId: "page-11" } });
  });

  it("limits page cookie reads to the current origin", async () => {
    const { capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser);
    expect((await browser.call("Network.getCookies", {}, session)).result).toEqual({ cookies: [{ name: "allowed", domain: "page11.example" }] });
    expect((await browser.call("Network.getCookies", { urls: ["https://other.example"] }, session)).error).toBeDefined();
  });

  it("keeps additional same-page sessions independent and releases only the detached session", async () => {
    const { first, capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const root = await attach(browser);
    const extra = await attach(browser);
    expect(extra).not.toBe(root);
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, extra)).result).toEqual({ result: { type: "string", value: "native-alias-1" } });
    expect((await browser.call("Target.detachFromTarget", { sessionId: extra })).error).toBeUndefined();
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, extra)).error).toBeDefined();
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, root)).error).toBeUndefined();
    expect(first.contents.debugger.isAttached()).toBe(true);
  });

  it("creates through the owning area action, converges retries, and refuses alternate contexts", async () => {
    const { capability, client, opened } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const first = await browser.call("Target.createTarget", { url: "https://checkout.example/index.html" }, undefined, 10);
    const retry = await browser.call("Target.createTarget", { url: "https://checkout.example/index.html" }, undefined, 10);
    expect(first.result).toEqual(retry.result);
    expect((first.result as Json).targetId).toBe("page-121");
    expect(opened).toEqual([{ scope: { workspace: "local\0/checkout", area_id: "a1" }, url: "https://checkout.example/index.html" }]);
    expect((await browser.call("Target.createTarget", { url: "file:///checkout/index.html" })).error).toBeDefined();
    expect(opened).toHaveLength(1);
    expect((await browser.call("Target.createTarget", { url: "about:blank", browserContextId: "other" })).error).toBeDefined();
    const display = await client((await capability({ workspace: "local\0/checkout", area_id: "a1", display_id: "d1" })).browser_ws_url);
    expect((await display.call("Target.createTarget", { url: "about:blank" })).error).toBeDefined();
  });

  it("excludes native files from public inventory and refuses their selected capability", async () => {
    const { first, address, capability, client } = await fixture();
    first.contents.getURL = () => "file:///outside/private.html";
    const cap = await capability();
    const inventory = await (await fetch(`${cap.cdp_http_url}/json/list`)).json() as Json[];
    expect(inventory.map((page) => page.id)).toEqual(["page-12"]);
    const browser = await client(cap.browser_ws_url);
    expect((await browser.call("Target.attachToTarget", { targetId: "page-11" })).error).toBeDefined();
    expect(await customRequest(`${address.endpoint}/connect`, "POST", { authorization: `Bearer ${address.token}` }, JSON.stringify({ workspace: first.workspace, area_id: "a1", display_id: first.id }))).toBe(409);
    expect(first.contents.debugger.isAttached()).toBe(false);
  });

  it("loads an unloaded display's page again before connecting an agent to it, and refuses while its load fails (PRD browser-tab-idle-unload B10, B11)", async () => {
    let outcome: "loaded" | "failed" = "failed";
    const asked: CdpScope[] = [];
    const { address, client } = await fixture({
      revive: async (scope, pages) => {
        asked.push(scope);
        if (scope.display_id !== "unloaded" || pages.some((page) => page.id === "unloaded")) return "none";
        if (outcome === "loaded") pages.push(electronPage(31, scope.display_id, scope.area_id, scope.workspace));
        return outcome;
      },
    });
    const connect = (body: Json) => fetch(`${address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body: JSON.stringify(body) });
    const scope = { workspace: "local\0/checkout", area_id: "a1", display_id: "unloaded" };
    const failed = await connect(scope);
    expect(failed.status).toBe(409);
    expect(await failed.json()).toEqual({ error: "Display page failed to load" });
    // The agent's next connection loads it again.
    outcome = "loaded";
    const connected = await connect(scope);
    expect(connected.status).toBe(200);
    const cap = await connected.json() as { browser_ws_url: string; display_id: string };
    expect(cap.display_id).toBe("unloaded");
    expect((await attach(await client(cap.browser_ws_url), "page-31"))).toEqual(expect.any(String));
    // A live page answers `none` and connects as before; an area connection and Comment never ask.
    expect((await connect({ workspace: "local\0/checkout", area_id: "a1", display_id: "d1" })).status).toBe(200);
    await connect({ workspace: "local\0/checkout", area_id: "a1" });
    await connect({ ...scope, display_id: "other", purpose: "comment" });
    expect(asked.map((row) => row.display_id)).toEqual(["unloaded", "unloaded", "d1"]);
  });

  it("keeps the 409 for a display with nothing to load again, such as one pending since the app started (PRD browser-tab-idle-unload D-16, B15)", async () => {
    const asked: string[] = [];
    const { address } = await fixture({ revive: async (scope) => { asked.push(scope.display_id!); return "none"; } });
    const response = await fetch(`${address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body: JSON.stringify({ workspace: "local\0/checkout", area_id: "a1", display_id: "pending" }) });
    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({ error: "Display has no eligible HTTP(S) browser target; native files are unsupported" });
    expect(asked).toEqual(["pending"]);
  });

  it("carries the area authority in force once an agent's page has loaded again", async () => {
    const during = { change: () => {} };
    const { address, client, incarnate, authority } = await fixture({
      revive: async (scope, pages) => {
        during.change();
        if (!pages.some((page) => page.id === scope.display_id)) pages.push(electronPage(33, scope.display_id!, scope.area_id, scope.workspace));
        return "loaded";
      },
    });
    const connect = () => fetch(`${address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body: JSON.stringify({ workspace: "local\0/checkout", area_id: "a1", display_id: "reloaded" }) });
    // The area moved to a new incarnation during the load: the capability is granted under it and drives the page.
    during.change = () => incarnate(2);
    const moved = await connect();
    expect(moved.status).toBe(200);
    const cap = await moved.json() as { browser_ws_url: string };
    expect(await attach(await client(cap.browser_ws_url), "page-33")).toEqual(expect.any(String));
    // The area lost its authority during the load: nothing is granted.
    during.change = () => authority(false);
    expect((await connect()).status).toBe(409);
  });

  it("grants nothing to a connection whose page loaded across a revoke", async () => {
    const during = { revoke: async (): Promise<unknown> => undefined };
    const { address } = await fixture({
      revive: async (scope, pages) => {
        await during.revoke();
        pages.push(electronPage(32, scope.display_id!, scope.area_id, scope.workspace));
        return "loaded";
      },
    });
    during.revoke = () => fetch(`${address.endpoint}/revoke`, { method: "POST", headers: { authorization: `Bearer ${address.token}` } });
    const response = await fetch(`${address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body: JSON.stringify({ workspace: "local\0/checkout", area_id: "a1", display_id: "late" }) });
    expect(response.status).toBe(503);
  });

  it("revokes an area capability if a bound native generation becomes a file", async () => {
    const { first, capability, client, changed, attached } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    const closed = new Promise<void>((resolve) => browser.socket.once("close", () => resolve()));
    first.contents.getURL = () => "file:///outside/private.html";
    changed();
    await closed;
    expect(attached.get(first.id)).toBe(false);
    expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
  });

  it("returns a close receipt after target removal and rejects ID reuse or expired intents", async () => {
    const { capability, client, advance, opened } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const closed = await browser.call("Target.closeTarget", { targetId: "page-12" }, undefined, 100);
    expect(closed.result).toEqual({ success: true });
    expect((await browser.call("Target.closeTarget", { targetId: "page-12" }, undefined, 100)).result).toEqual({ success: true });
    expect((await browser.call("Target.createTarget", { url: "about:blank" }, undefined, 100)).error).toBeDefined();
    // A long-lived connection still creates a fresh timestamped intent.
    advance(12 * 60_000);
    expect((await browser.call("Target.createTarget", { url: "about:blank" }, undefined, 101)).error).toBeUndefined();
    expect(opened.length).toBe(1);
    expect((await browser.call("Target.closeTarget", { targetId: "page-12" }, undefined, 100)).error).toBeDefined();
  });

  it("recovers a committed open after response loss using the original intent without another page", async () => {
    const { capability, client, opened, actions } = await fixture({ loseFirstOpenResponse: true });
    const browser = await client((await capability()).browser_ws_url);
    const params = { url: "about:blank" };
    expect((await browser.call("Target.createTarget", params, undefined, 500)).error).toBeDefined();
    expect(opened).toHaveLength(1);
    expect((await browser.call("Target.createTarget", params, undefined, 500)).result).toEqual({ targetId: "page-121" });
    expect(actions).toHaveLength(2);
    expect(actions[1]!.request_id).toBe(actions[0]!.request_id);
    expect(opened).toHaveLength(1);
    expect((await browser.call("Target.createTarget", params, undefined, 500)).result).toEqual({ targetId: "page-121" });
    expect(actions).toHaveLength(2);
  });

  it("bounds ambiguous retries, caches definitive refusals and revalidates authority before retry", async () => {
    const lost = await fixture({ alwaysLoseResponse: true });
    const lostCap = await lost.capability();
    const browser = await lost.client(lostCap.browser_ws_url);
    for (let index = 0; index < 4; index++) expect((await browser.call("Target.createTarget", { url: "about:blank" }, undefined, 500)).error).toBeDefined();
    expect(lost.actions).toHaveLength(3);
    expect(new Set(lost.actions.map((action) => action.request_id)).size).toBe(1);
    expect(lost.opened).toHaveLength(1);
    const refused = await fixture({ refuseOpen: true });
    const blocked = await refused.client((await refused.capability()).browser_ws_url);
    for (let index = 0; index < 2; index++) expect((await blocked.call("Target.createTarget", { url: "about:blank" }, undefined, 500)).error).toBeDefined();
    expect(refused.actions).toHaveLength(1);
    expect(refused.opened).toHaveLength(0);
    const closed = new Promise<void>((resolve) => browser.socket.once("close", () => resolve()));
    lost.authority(false);
    await closed;
    const stale = new CdpClient(lostCap.browser_ws_url);
    await expect(stale.ready()).rejects.toThrow("403");
  });

  it("drains its successful selected-display close before permanently revoking the capability", async () => {
    const { first, capability, client, attached } = await fixture();
    const cap = await capability({ workspace: first.workspace, area_id: "a1", display_id: first.id });
    const browser = await client(cap.browser_ws_url);
    const session = await attach(browser);
    const closed = new Promise<number>((resolve) => browser.socket.once("close", (code) => resolve(code)));
    expect((await browser.call("Page.close", {}, session)).result).toEqual({});
    expect(await closed).toBe(1000);
    expect(attached.get(first.id)).toBe(false);
    expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
  });

  it("drains a direct-page close result while keeping the surviving area capability scoped", async () => {
    const { first, capability, client } = await fixture();
    const cap = await capability();
    const pages = await (await fetch(`${cap.cdp_http_url}/json/list`)).json() as Json[];
    const page = await client(pages[0]!.webSocketDebuggerUrl as string);
    const closed = new Promise<number>((resolve) => page.socket.once("close", (code) => resolve(code)));
    expect((await page.call("Page.close")).result).toEqual({});
    expect(await closed).toBe(1000);
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect((await (await fetch(`${cap.cdp_http_url}/json/list`)).json() as Json[]).map((page) => page.id)).toEqual(["page-12"]);
  });

  it("delivers successful selected-display and direct-page closes before a late native retirement", async () => {
    for (const mode of ["browser", "page"] as const) {
      const { first, capability, client, changed } = await fixture({ deferCloseRetirement: true });
      const cap = await capability({ workspace: first.workspace, area_id: "a1", display_id: first.id });
      const browser = await client(mode === "browser" ? cap.browser_ws_url : `${cap.browser_ws_url.replace("/browser", "/page/page-11")}`);
      const closed = new Promise<number>((resolve) => browser.socket.once("close", (code) => resolve(code)));
      const reply = mode === "browser" ? await browser.call("Target.closeTarget", { targetId: "page-11" }) : await browser.call("Page.close");
      expect(reply.error).toBeUndefined();
      expect(reply.result).toEqual(mode === "browser" ? { success: true } : {});
      expect(await closed).toBe(1000);
      expect(first.contents.debugger.isAttached()).toBe(false);
      changed({ contentsId: first.contents.id, reason: "closed" });
      expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
    }
  });

  it("replays a nested close receipt after its session retires without accepting a changed intent", async () => {
    const { capability, client, pages } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser, "page-11", false);
    const params = { sessionId: session, message: JSON.stringify({ id: 70, method: "Page.close" }) };
    expect((await browser.call("Target.sendMessageToTarget", params)).result).toEqual({});
    expect((await browser.call("Target.sendMessageToTarget", params)).result).toEqual({});
    const receipts = browser.events.filter((event) => event.method === "Target.receivedMessageFromTarget")
      .map((event) => JSON.parse((event.params as Json).message as string) as Json).filter((reply) => reply.id === 70);
    expect(receipts).toEqual([{ id: 70, result: {} }, { id: 70, result: {} }]);
    expect(pages.some((page) => page.id === "d1")).toBe(false);
    const changed = { sessionId: session, message: JSON.stringify({ id: 70, method: "Page.bringToFront" }) };
    await browser.call("Target.sendMessageToTarget", changed);
    const last = browser.events.at(-1)!;
    expect((JSON.parse((last.params as Json).message as string) as Json).error).toBeDefined();
  });

  it("revokes authority-loss capabilities permanently, including an empty authorized area", async () => {
    const { first, capability, client, authority, address } = await fixture();
    const area = await capability();
    const empty = await capability({ workspace: first.workspace, area_id: "empty-area" });
    const browser = await client(area.browser_ws_url);
    await attach(browser);
    const closed = new Promise<void>((resolve) => browser.socket.once("close", () => resolve()));
    authority(false);
    await closed;
    expect(first.contents.debugger.isAttached()).toBe(false);
    for (const cap of [area, empty]) expect((await fetch(`${cap.cdp_http_url}/json/version`)).status).toBe(404);
    expect(await customRequest(`${address.endpoint}/connect`, "POST", { authorization: `Bearer ${address.token}` }, JSON.stringify({ workspace: first.workspace, area_id: "a1" }))).toBe(409);
    authority(true);
    const fresh = await capability();
    expect(fresh.cdp_http_url === area.cdp_http_url).toBe(false);
    for (const cap of [area, empty]) expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
  });

  it("revokes a pinned scope incarnation even when revoke/regrant coalesces and the native page is identical", async () => {
    const { first, capability, client, incarnate } = await fixture();
    const cap = await capability();
    const empty = await capability({ workspace: first.workspace, area_id: "empty-area" });
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    const closed = new Promise<void>((resolve) => browser.socket.once("close", () => resolve()));
    // No absent-scope notification or native retirement is delivered.
    incarnate(2);
    await closed;
    expect(first.contents.debugger.isAttached()).toBe(false);
    for (const stale of [cap, empty]) expect((await fetch(`${stale.cdp_http_url}/json/version`)).status).toBe(404);
    const fresh = await capability();
    expect(fresh.cdp_http_url === cap.cdp_http_url).toBe(false);
    const next = await client(fresh.browser_ws_url);
    expect(typeof await attach(next)).toBe("string");
  });

  it("reports scoped discovery and auto-attach for standard flattened clients", async () => {
    const { capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    expect((await browser.call("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true })).error).toBeUndefined();
    await browser.call("Target.setDiscoverTargets", { discover: true });
    expect(browser.events.filter((event) => event.method === "Target.attachedToTarget").length).toBe(2);
    expect(browser.events.filter((event) => event.method === "Target.targetCreated" && ((event.params as Json).targetInfo as Json).type === "page").map((event) => ((event.params as Json).targetInfo as Json).targetId)).toEqual(["page-11", "page-12"]);
    expect((await browser.call("Target.getTargetInfo")).result).toMatchObject({ targetInfo: { type: "browser", targetId: "scoped-browser" } });
  });

  it("provides the scoped tab/page hierarchy Puppeteer requests without a global target", async () => {
    const { capability, client } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    await browser.call("Target.setDiscoverTargets", { discover: true });
    expect((await browser.call("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true, filter: [{ type: "page", exclude: true }, {}] })).error).toBeUndefined();
    const tabs = browser.events.filter((event) => event.method === "Target.attachedToTarget");
    expect(tabs.map((event) => ((event.params as Json).targetInfo as Json).type)).toEqual(["tab", "tab"]);
    const tab = (tabs[0]!.params as Json).sessionId as string;
    await browser.call("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: true, flatten: true }, tab);
    const childEvent = browser.events.find((event) => event.method === "Target.attachedToTarget" && event.sessionId === tab);
    expect(((childEvent!.params as Json).targetInfo as Json).targetId).toBe("page-11");
    const page = (childEvent!.params as Json).sessionId as string;
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, page)).result).toEqual({ result: { type: "string", value: "page" } });
    expect((await browser.call("Runtime.runIfWaitingForDebugger", {}, tab)).error).toBeUndefined();
    expect((await browser.call("Page.enable", {}, tab)).error).toBeDefined();
    expect((await browser.call("Browser.setDownloadBehavior", { behavior: "deny" })).error).toBeDefined();
  });

  it("revokes a selected display capability when its native generation is replaced", async () => {
    const { first, pages, changed, capability, client } = await fixture();
    const cap = await capability({ workspace: first.workspace, area_id: "a1", display_id: "d1" });
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    const closed = new Promise((resolve) => browser.socket.once("close", resolve));
    pages.splice(pages.indexOf(first), 1, electronPage(99, "d1"));
    changed();
    await closed;
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
  });

  it("revokes an area capability at LRU eviction before a display can regenerate", async () => {
    const { first, pages, changed, capability, client } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    const closed = new Promise((resolve) => browser.socket.once("close", resolve));
    pages.splice(pages.indexOf(first), 1);
    changed({ contentsId: first.contents.id, reason: "evicted" });
    await closed;
    pages.push(electronPage(99, "d1"));
    changed();
    expect((await fetch(`${cap.cdp_http_url}/json/list`)).status).toBe(404);
    const fresh = await capability();
    expect(fresh.cdp_http_url === cap.cdp_http_url).toBe(false);
    expect((await (await fetch(`${fresh.cdp_http_url}/json/list`)).json() as Json[]).some((page) => page.id === "page-99")).toBe(true);
  });

  it("revokes sessions as soon as a page moves out of the area and releases the debugger", async () => {
    const { first, capability, client, changed, attached } = await fixture();
    const browser = await client((await capability()).browser_ws_url);
    const session = await attach(browser);
    first.area_id = "a2";
    changed();
    expect((await browser.call("Runtime.evaluate", { expression: "1" }, session)).error).toBeDefined();
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect(attached.get("d1")).toBe(false);
  });

  it("grants an agent's outside file to hided alone, with the control token and a path and kind", async () => {
    const { address, externalOpens } = await fixture();
    const body = JSON.stringify({ path: "/tmp/r.html", page: true });
    expect(await customRequest(`${address.endpoint}/external-open`, "POST", { "content-type": "application/json" }, body)).toBe(401);
    expect(await customRequest(`${address.endpoint}/external-open`, "POST", { authorization: `Bearer ${address.token}` }, JSON.stringify({ path: "/tmp/r.html" }))).toBe(400);
    expect(externalOpens).toEqual([]);
    const response = await fetch(`${address.endpoint}/external-open`, { method: "POST", headers: { authorization: `Bearer ${address.token}`, "content-type": "application/json" }, body });
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ grant: { id: "a1".repeat(16), name: "r.html", kind: "html" } });
    expect(externalOpens).toEqual([{ path: "/tmp/r.html", page: true }]);
  });

  it("revokes everything a node's ended link authorized and stays available for the next", async () => {
    const { first, address, capability, client } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    expect(await customRequest(`${address.endpoint}/revoke`, "POST", {})).toBe(401);
    const lost = new Promise((resolve) => browser.socket.once("close", resolve));
    expect(await customRequest(`${address.endpoint}/revoke`, "POST", { authorization: `Bearer ${address.token}` })).toBe(200);
    await lost;
    expect(first.contents.debugger.isAttached()).toBe(false);
    expect((await fetch(`${cap.cdp_http_url}/json/version`)).status).toBe(404);
    const next = await capability();
    expect(next.cdp_http_url).not.toBe(cap.cdp_http_url);
    expect((await fetch(`${next.cdp_http_url}/json/version`)).status).toBe(200);
  });

  it("releases leases on disconnect, native detach, daemon loss and app shutdown", async () => {
    const { first, capability, client, gateway, attached } = await fixture();
    const cap = await capability();
    const browser = await client(cap.browser_ws_url);
    await attach(browser);
    first.contents.debugger.detach();
    expect(attached.get("d1")).toBe(false);
    await attach(browser);
    const closed = new Promise((resolve) => browser.socket.once("close", resolve));
    browser.socket.terminate();
    await closed;
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(first.contents.debugger.isAttached()).toBe(false);
    const next = await client(cap.browser_ws_url);
    await attach(next);
    const lost = new Promise((resolve) => next.socket.once("close", resolve));
    gateway.setAvailable(false);
    await lost;
    expect(first.contents.debugger.isAttached()).toBe(false);
    gateway.setAvailable(true);
    expect((await fetch(`${cap.cdp_http_url}/json/version`)).status).toBe(404);
    await gateway.close();
    await expect(fetch(cap.cdp_http_url)).rejects.toThrow();
  });

  it("rejects the first connection and capability beyond their caps", async () => {
    const { capability, client, address } = await fixture();
    const cap = await capability();
    for (let index = 0; index < 8; index++) await client(cap.browser_ws_url);
    const overflow = new CdpClient(cap.browser_ws_url);
    await expect(overflow.ready()).rejects.toThrow("429");
    for (let index = 2; index <= 64; index++) await capability({ workspace: "local\0/checkout", area_id: `a${index}` });
    expect(await customRequest(`${address.endpoint}/connect`, "POST", { authorization: `Bearer ${address.token}` }, JSON.stringify({ workspace: "local\0/checkout", area_id: "a65" }))).toBe(429);
  });

  it("closes oversized and excessive requests with an actionable protocol status", async () => {
    const { capability, client } = await fixture();
    const cap = await capability();
    const oversized = await client(cap.browser_ws_url);
    const tooLarge = new Promise<number>((resolve) => oversized.socket.once("close", (code) => resolve(code)));
    oversized.socket.send("x".repeat(4 * 1024 * 1024 + 1));
    expect(await tooLarge).toBe(1009);
    const limited = await client(cap.browser_ws_url);
    for (let index = 0; index < 600; index++) await limited.call("Browser.getVersion");
    const exhausted = new Promise<number>((resolve) => limited.socket.once("close", (code) => resolve(code)));
    limited.socket.send(JSON.stringify({ id: 601, method: "Browser.getVersion" }));
    expect(await exhausted).toBe(1013);
  });
});

describe("comment capability on a local file page", () => {
  /** The fixture's page area plus one page of the file partition, as `Browser.cdpPages` lists it. */
  async function withLocalPage() {
    const set = await fixture();
    const local: CdpPage & { contents: { debugger: ElectronDebugger } } = { ...electronPage(21, "d9"), local: true };
    local.contents.getURL = () => "file:///checkout/index.html";
    set.pages.push(local);
    const connect = async (body: Json) => {
      const response = await fetch(`${set.address.endpoint}/connect`, { method: "POST", headers: { authorization: `Bearer ${set.address.token}`, "content-type": "application/json" }, body: JSON.stringify(body) });
      return { status: response.status, body: await response.json() as { browser_ws_url?: string; cdp_http_url?: string } };
    };
    const scope = { workspace: local.workspace, area_id: "a1", display_id: "d9" };
    return { ...set, local, connect, scope };
  }
  const reached = (debug: ElectronDebugger) => vi.spyOn(debug, "sendCommand");

  it("serves the comment session's own commands on a file page and nothing past them", async () => {
    const { local, connect, scope, client } = await withLocalPage();
    const answer = await connect({ ...scope, purpose: "comment" });
    expect(answer.status).toBe(200);
    const sent = reached(local.contents.debugger);
    const comment = await client(answer.body.browser_ws_url!);
    const targets = await comment.call("Target.getTargets");
    expect((targets.result as { targetInfos: Json[] }).targetInfos.map((info) => info.targetId)).toContain("page-21");
    const session = await attach(comment, "page-21");
    const allowed: [string, Json][] = [
      ["Page.enable", {}], ["Runtime.enable", {}], ["Runtime.addBinding", { name: "__hideCommentSend", executionContextName: "hide-comment" }],
      ["Page.getFrameTree", {}], ["Page.createIsolatedWorld", { frameId: "page-21", worldName: "hide-comment" }],
      ["Runtime.evaluate", { expression: "1", contextId: 42 }], ["Page.captureScreenshot", { format: "png" }],
    ];
    for (const [method, params] of allowed) expect((await comment.call(method, params, session)).error, method).toBeUndefined();
    expect(sent.mock.calls.map(([method]) => method)).toEqual(expect.arrayContaining(["Page.enable", "Runtime.evaluate", "Page.captureScreenshot"]));
  });

  it("refuses every other command for the comment purpose before it reaches the page", async () => {
    const { local, connect, scope, client } = await withLocalPage();
    const comment = await client((await connect({ ...scope, purpose: "comment" })).body.browser_ws_url!);
    const session = await attach(comment, "page-21");
    const sent = reached(local.contents.debugger);
    const refused: [string, Json][] = [
      ["Page.navigate", { url: "file:///outside/private.txt" }], ["Page.navigate", { url: "https://example.test/" }], ["Page.reload", {}],
      ["Fetch.enable", {}], ["Network.enable", {}], ["Network.loadNetworkResource", { url: "file:///outside/private.txt" }],
      ["Input.dispatchMouseEvent", { type: "mousePressed", x: 1, y: 1 }], ["Input.dispatchKeyEvent", { type: "keyDown" }],
      ["DOM.getDocument", {}], ["DOM.setFileInputFiles", { files: ["/etc/hosts"] }], ["Page.setBypassCSP", { enabled: true }],
      ["Page.setDownloadBehavior", { behavior: "allow" }], ["Page.close", {}], ["Page.bringToFront", {}],
      ["Emulation.setScriptExecutionDisabled", { value: true }], ["Debugger.enable", {}],
      ["Target.createTarget", { url: "about:blank" }], ["Target.closeTarget", { targetId: "page-21" }], ["Target.setAutoAttach", { autoAttach: true, flatten: true }],
      ["Target.sendMessageToTarget", { sessionId: session, message: JSON.stringify({ id: 1, method: "Page.navigate", params: { url: "file:///x" } }) }],
      ["Browser.getVersion", {}],
      // The page's own world is never the comment's: an evaluation names its isolated one.
      ["Runtime.evaluate", { expression: "document.title" }],
      ["Runtime.evaluate", { expression: "document.title", contextId: 1 }],
      // Nor may a command of the allowlist carry more than the session sends.
      ["Page.createIsolatedWorld", { frameId: "page-21", worldName: "x", grantUniveralAccess: true }],
      ["Runtime.evaluate", { expression: "1", contextId: 42, userGesture: true }], ["Runtime.evaluate", { expression: "1", contextId: 42, includeCommandLineAPI: true }],
      ["Runtime.addBinding", { name: "__hideCommentSend" }],
    ];
    for (const [method, params] of refused) {
      const reply = await comment.call(method, params, method.startsWith("Target.") || method.startsWith("Browser.") ? undefined : session);
      expect(reply.error, `${method} ${JSON.stringify(params)}`).toBeDefined();
    }
    expect(sent).not.toHaveBeenCalled();
  });

  it("passes the page events the comment session reads and holds back the rest", async () => {
    const { local, connect, scope, client } = await withLocalPage();
    const comment = await client((await connect({ ...scope, purpose: "comment" })).body.browser_ws_url!);
    const session = await attach(comment, "page-21");
    for (const method of ["Runtime.consoleAPICalled", "Network.requestWillBeSent", "Page.frameNavigated", "Runtime.bindingCalled", "Page.navigatedWithinDocument", "Inspector.detached"]) {
      local.contents.debugger.emit("message", {}, method, { frame: { id: "x" } }, "");
    }
    await vi.waitFor(() => expect(comment.events.map((event) => event.method)).toContain("Inspector.detached"));
    // The gateway's own attach notice is not a page event.
    const page = comment.events.filter((event) => event.method !== "Target.attachedToTarget");
    expect(page.map((event) => event.method)).toEqual(["Page.frameNavigated", "Runtime.bindingCalled", "Page.navigatedWithinDocument", "Inspector.detached"]);
    expect(page.every((event) => event.sessionId === session)).toBe(true);
  });

  it("keeps a file page away from an agent: no capability, no listing, no attach", async () => {
    const { local, address, connect, scope, capability, client } = await withLocalPage();
    // An agent's connect on the display keeps today's refusal, with or without a purpose it does not hold.
    expect((await connect(scope)).status).toBe(409);
    const cap = await capability();
    const inventory = await (await fetch(`${cap.cdp_http_url}/json/list`)).json() as Json[];
    expect(inventory.map((page) => page.id)).not.toContain("page-21");
    const agent = await client(cap.browser_ws_url);
    const targets = await agent.call("Target.getTargets");
    expect(JSON.stringify(targets.result)).not.toContain("page-21");
    expect((await agent.call("Target.attachToTarget", { targetId: "page-21" })).error).toBeDefined();
    expect(local.contents.debugger.isAttached()).toBe(false);
    expect(address.token).toBeTruthy();
  });

  it("grants the comment purpose only for one display and only by the name comment", async () => {
    const { connect, scope, capability } = await withLocalPage();
    expect((await connect({ workspace: scope.workspace, area_id: "a1", purpose: "comment" })).status).toBe(400);
    expect((await connect({ ...scope, purpose: "agent" })).status).toBe(400);
    expect((await connect({ ...scope, purpose: ["comment"] })).status).toBe(400);
    // A comment capability is its own: an agent's capability for the same display on a web page is another path.
    const web = { workspace: scope.workspace, area_id: "a1", display_id: "d1" };
    const comment = await connect({ ...web, purpose: "comment" });
    const agent = await capability(web);
    expect(comment.status).toBe(200);
    expect(comment.body.browser_ws_url).not.toBe(agent.browser_ws_url);
  });

  it("has no discovery, and a web page held for a comment is bounded the same way", async () => {
    const { connect, scope, client, first } = await withLocalPage();
    const answer = await connect({ ...scope, purpose: "comment" });
    expect((await fetch(`${answer.body.cdp_http_url}/json/list`)).status).toBe(404);
    expect((await fetch(`${answer.body.cdp_http_url}/json/version`)).status).toBe(404);
    const web = await connect({ workspace: scope.workspace, area_id: "a1", display_id: first.id, purpose: "comment" });
    const comment = await client(web.body.browser_ws_url!);
    const session = await attach(comment, "page-11");
    expect((await comment.call("Page.navigate", { url: "https://example.test/" }, session)).error).toBeDefined();
    expect((await comment.call("Page.enable", {}, session)).error).toBeUndefined();
  });

  it("ends a comment lease when the page stops being a file page, and when its authority goes", async () => {
    const { local, connect, scope, client, changed, attached, authority } = await withLocalPage();
    const comment = await client((await connect({ ...scope, purpose: "comment" })).body.browser_ws_url!);
    await attach(comment, "page-21");
    const closed = new Promise<void>((resolve) => comment.socket.once("close", () => resolve()));
    local.contents.getURL = () => "https://elsewhere.example/";
    changed();
    await closed;
    expect(attached.get("d9")).toBe(false);
    authority(false);
    expect((await connect({ ...scope, purpose: "comment" })).status).toBe(409);
  });
});
