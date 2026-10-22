// Runs the production host and its scoped endpoint against a private daemon
// and Herdr server. The operator's app, browser and profile are never used.
import { chromium, expect, type Browser, type ElectronApplication, type Page } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { createServer, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { WebSocket } from "ws";
import type { BrowserSync } from "../../web/src/host";
import { startHerdr, type HerdrFixture } from "../../web/e2e/herdr-fixture";
import { chooseExpand, enterWorkspace, showDocuments } from "../../web/e2e/wire";
import { BROWSER_SYNC_CHANNEL } from "../src/channel";
import { fitWindow, HIDE_CLI, hostLog, isolate, launch, REPO, test, type Isolated } from "./fixture";

test.describe.configure({ timeout: 180_000 });
let herdr: HerdrFixture;
let run: Isolated;
let app: ElectronApplication | null = null;
let browser: Browser | null = null;
let server: Server;
let origin: string;
let sequence = 0;
let fileCanary: string;
let fileUrl: string;
let pendingPreload: ServerResponse | null = null;
let fileRedirectRequested = false;
let fileRedirect: { status: number; location: string } | null = null;
/** Two native attempts during the lease and two after it. */
const MAX_DOWNLOAD_REQUESTS = 4;
let downloadRequests = 0;

type Reply = { id?: number; sessionId?: string; result?: Record<string, unknown>; error?: { code: number; message: string } };
type Capability = { cdp_http_url: string; browser_ws_url: string };
type Target = { id: string; type: string; url: string; webSocketDebuggerUrl: string };
type CoreView = { view_id: string; area_id: string; kind: string; target: string; active_area: boolean; selected: boolean };
type AuthoritySync = BrowserSync & { authorized_scopes: { workspace: string; area_id: string; incarnation: number }[] };
type Send = (method: string, params?: Record<string, unknown>, sessionId?: string) => Promise<Reply>;
type TargetEvent = { method: string; parentSessionId: string | null; sessionId: string; targetId: string; type: string | null; url: string | null };
type FrameTree = { frame: { id: string; name?: string; url: string } };

function quote(value: string): string { return `'${value.replaceAll("'", "'\\''")}'`; }
/** Each raw connection and request has a deadline and closes on every path. */
async function withCdp<T>(url: string, use: (send: Send, socket: WebSocket) => Promise<T>): Promise<T> {
  const socket = new WebSocket(url, { maxPayload: 4 * 1024 * 1024 });
  let sequence = 0;
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => finish(new Error("Scoped browser connection timed out")), 10_000);
      const finish = (error?: Error) => {
        clearTimeout(timer); socket.removeListener("open", opened); socket.removeListener("error", failed);
        if (error) reject(error); else resolve();
      };
      const opened = () => finish();
      const failed = () => finish(new Error("Scoped browser command could not connect"));
      socket.once("open", opened); socket.once("error", failed);
    });
    // Errors are delivered to the active request below; no transport error
    // may escape as an unhandled emitter error between sequential requests.
    socket.on("error", () => {});
    const send: Send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => finish(undefined, new Error("Scoped browser command timed out")), 10_000);
      const finish = (reply?: Reply, error?: Error) => {
        clearTimeout(timer); socket.removeListener("message", received); socket.removeListener("close", closed); socket.removeListener("error", failed);
        if (error) reject(error); else resolve(reply!);
      };
      const received = (data: Buffer) => {
        try {
          const reply = JSON.parse(data.toString()) as Reply;
          if (reply.id === id) finish(reply);
        } catch { finish(undefined, new Error("Invalid scoped browser reply")); }
      };
      const closed = () => finish(undefined, new Error("Scoped connection closed before its reply"));
      const failed = () => finish(undefined, new Error("Scoped browser command failed"));
      socket.on("message", received); socket.once("close", closed); socket.once("error", failed);
      if (socket.readyState !== WebSocket.OPEN) closed();
      else socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
    return await use(send, socket);
  } finally { socket.terminate(); }
}
function waitForClose(socket: WebSocket): Promise<number> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.removeListener("close", closed); reject(new Error("Scoped socket did not close")); }, 10_000);
    const closed = (code: number) => { clearTimeout(timer); resolve(code); };
    socket.once("close", closed);
  });
}
async function refusedWebSocket(url: string): Promise<number> {
  const socket = new WebSocket(url);
  let timer: NodeJS.Timeout | undefined;
  try {
    return await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error("Stale capability refusal timed out")), 10_000);
      socket.on("error", () => reject(new Error("Expected an HTTP capability refusal")));
      socket.once("open", () => reject(new Error("A stale capability accepted a connection")));
      socket.once("unexpected-response", (_request, response) => {
        response.resume(); resolve(response.statusCode ?? 0);
      });
    });
  } finally { clearTimeout(timer); socket.terminate(); }
}
async function browserCommand(url: string, method: string, params: Record<string, unknown>): Promise<Reply> {
  return withCdp(url, (send) => send(method, params));
}
/** The core's own diagnostic lines, which say why a page report was not applied. */
function coreLog(env: Record<string, string>): { kind?: unknown; scheme?: unknown }[] {
  const file = path.join(env.HIDE_STATE_DIR!, "Logs", "core.jsonl");
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8").split("\n").filter(Boolean).flatMap((line) => { try { return [JSON.parse(line) as { kind?: unknown }]; } catch { return []; } });
}
async function fromPane(args: string[], succeeds = true): Promise<Record<string, unknown>> {
  const stem = path.join(herdr.root, `cdp-command-${++sequence}`);
  const output = `${stem}.json`, status = `${stem}.status`;
  const command = `HIDE_STATE_DIR=${quote(run.env.HIDE_STATE_DIR!)} ${[HIDE_CLI, ...args].map(quote).join(" ")} > ${quote(output)}; printf '%s' "$?" > ${quote(status)}\n`;
  const sent = spawnSync(herdr.bin, ["pane", "send-text", herdr.panes[0]!, command], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
  expect(sent.status).toBe(0);
  await expect.poll(() => fs.existsSync(status), { timeout: 30_000 }).toBe(true);
  expect(fs.statSync(output).size, "bounded isolated Workspace reply").toBeLessThanOrEqual(64 * 1024);
  let parsed: unknown;
  try { parsed = JSON.parse(fs.readFileSync(output, "utf8").trim().split("\n").at(-1) ?? "null"); }
  catch { throw new Error("Invalid isolated Workspace command reply"); }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error("Invalid isolated Workspace command reply");
  const reply = parsed as Record<string, unknown>;
  // Report only the finite reason identifier, never capability-bearing output.
  const reason = typeof reply.reason === "string" && /^[a-z_]{1,64}$/.test(reply.reason) ? reply.reason : "unclassified";
  if (succeeds && Number(fs.readFileSync(status, "utf8")) !== 0 && app) {
    const native = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()
      .filter((window) => window.getParentWindow() === null)
      .flatMap((window) => window.contentView.children.slice(0, 12).map((view) => {
        const contents = (view as { webContents?: Electron.WebContents }).webContents;
        if (!contents || contents.isDestroyed()) return { destroyed: true };
        const url = contents.getURL();
        return { protocol: url.split(":", 1)[0], loading: contents.isLoading(), manual: contents.getTitle() === "Manual local file" };
      }))).catch(() => [{ unavailable: true }]);
    const page = reply.page as { state?: string; load?: number } | undefined;
    console.log("Workspace page failure", JSON.stringify({ reason, state: page?.state, load: page?.load, native,
      events: hostLog(run.env).filter((row) => row.event.startsWith("browser.")).slice(-12).map((row) => ({ event: row.event, reason: row.reason, protocol: row.protocol, method: row.method, code: row.code })),
      core: coreLog(run.env).filter((row) => String(row.kind ?? "").startsWith("browser.")).slice(-12).map((row) => ({ kind: row.kind, scheme: row.scheme })) }));
  }
  expect(Number(fs.readFileSync(status, "utf8")) === 0, `isolated Workspace command result (${reason})`).toBe(succeeds);
  return reply;
}
async function coreViews(): Promise<CoreView[]> {
  return ((await fromPane(["view", "list"])).result as { views: CoreView[] }).views;
}
async function targets(capability: Capability): Promise<Target[]> {
  const response = await fetch(`${capability.cdp_http_url}/json/list`, { signal: AbortSignal.timeout(10_000) });
  expect(response.status).toBe(200);
  return await response.json() as Target[];
}
/** The documents expanded over the whole body, so their areas have room to split (PRD right-lane B8; lane-two-columns B20). */
async function expandViews(shell: Page): Promise<void> {
  await fitWindow(app!, { width: 1024, height: 640 });
  await showDocuments(shell);
  await chooseExpand(shell);
  await expect(shell.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "expanded");
}
/** Attest the real Chromium IDs through each candidate WebContents itself. */
async function excludedRendererIds(foreignUrl: string): Promise<string[]> {
  return app!.evaluate(async ({ BrowserWindow }, url) => {
    const window = BrowserWindow.getAllWindows().find((window) => window.getParentWindow() === null)!;
    const foreign = window.contentView.children.flatMap((view) => {
      const contents = (view as { webContents?: Electron.WebContents }).webContents;
      return contents?.getURL() === url ? [contents] : [];
    });
    if (foreign.length !== 1) throw new Error("Expected one foreign candidate display");
    const ids: string[] = [];
    for (const contents of [window.webContents, foreign[0]!]) {
      if (contents.debugger.isAttached()) throw new Error("Excluded renderer already has a debugger owner");
      contents.debugger.attach("1.3");
      try {
        const result = await contents.debugger.sendCommand("Target.getTargetInfo") as { targetInfo: { targetId: string } };
        if (typeof result.targetInfo.targetId !== "string") throw new Error("Native target attestation failed");
        ids.push(result.targetInfo.targetId, `page-${contents.id}`);
      } finally { contents.debugger.detach(); }
    }
    return ids;
  }, foreignUrl);
}

async function ownedFrameProvenance(parentUrl: string, childUrl: string) {
  return app!.evaluate(async ({ BrowserWindow }, { parentUrl, childUrl }) => {
    const windows = BrowserWindow.getAllWindows().filter((window) => window.getParentWindow() === null);
    if (windows.length !== 1) throw new Error("Expected exactly one candidate main window");
    const contents = windows[0]!.contentView.children.flatMap((view) => {
      const page = (view as { webContents?: Electron.WebContents }).webContents;
      return page?.getURL() === parentUrl ? [page] : [];
    });
    if (contents.length !== 1) throw new Error("Expected exactly one owned fixture WebContents");
    const parent = contents[0]!.mainFrame;
    const children = parent.framesInSubtree.filter((frame) => !frame.isDestroyed() && frame.url === childUrl);
    if (children.length !== 1 || !children[0]!.parent) throw new Error("Expected exactly one owned cross-site child frame");
    // Page.getFrameTree contains local frames only. The parent's actual DOM
    // owner supplies the remote child's ID independently of the gateway.
    const debugger_ = contents[0]!.debugger;
    if (!debugger_.isAttached()) throw new Error("The owned page must have an active debugger lease");
    const document = await debugger_.sendCommand("DOM.getDocument", { depth: 0 }) as { root: { nodeId: number } };
    const selected = await debugger_.sendCommand("DOM.querySelector", { nodeId: document.root.nodeId, selector: "#cross-site" }) as { nodeId: number };
    if (!Number.isInteger(selected.nodeId) || selected.nodeId <= 0) throw new Error("The owned iframe DOM node is absent");
    const described = await debugger_.sendCommand("DOM.describeNode", { nodeId: selected.nodeId, depth: 0 }) as { node: { nodeName: string; frameId?: string; attributes?: string[] } };
    const owner = described.node;
    if (owner.nodeName !== "IFRAME" || typeof owner.frameId !== "string" || owner.frameId.length === 0
      || !owner.attributes || owner.attributes.length > 64 || owner.attributes.length % 2 !== 0) throw new Error("The owned iframe has no native frame identity");
    const attributes = Object.fromEntries(Array.from({ length: owner.attributes.length / 2 }, (_, index) => [owner.attributes![index * 2]!, owner.attributes![index * 2 + 1]!]));
    if (attributes.id !== "cross-site" || attributes.name !== children[0]!.name || attributes.src !== childUrl) throw new Error("The native DOM owner does not identify the expected child");
    const identity = (frame: Electron.WebFrameMain) => ({ processId: frame.processId, routingId: frame.routingId, osProcessId: frame.osProcessId, name: frame.name });
    return { contentsId: contents[0]!.id, frameOwner: { frameId: owner.frameId, attributes }, parent: identity(parent), child: identity(children[0]!), childParent: identity(children[0]!.parent!) };
  }, { parentUrl, childUrl });
}

test.beforeAll(async () => {
  herdr = await startHerdr({ agents: false });
  fileCanary = path.join(herdr.root, "outside-cdp-canary.html");
  fs.writeFileSync(fileCanary, '<title>Forbidden file canary</title><h1>FORBIDDEN_CDP_FILE_CONTENT</h1>');
  fileUrl = pathToFileURL(fileCanary).href;
  server = createServer((request, response) => {
    if (request.url === "/owned-download") {
      if (downloadRequests === MAX_DOWNLOAD_REQUESTS) { response.writeHead(429); response.end("Fixture download request limit exceeded"); return; }
      downloadRequests++;
      response.writeHead(200, { "content-type": "application/octet-stream", "content-disposition": 'attachment; filename="owned-download.txt"' });
      response.end("OWNED_DOWNLOAD_FIXTURE");
      return;
    }
    if (request.url === "/file-redirect") {
      if (fileRedirectRequested) { response.writeHead(429); response.end("Fixture redirect request limit exceeded"); return; }
      fileRedirectRequested = true;
      response.setHeader("location", fileUrl);
      response.writeHead(302);
      response.once("finish", () => { fileRedirect = { status: response.statusCode, location: String(response.getHeader("location")) }; });
      response.end();
      return;
    }
    if (request.url === "/pending-preload") {
      if (pendingPreload) { response.writeHead(503); response.end(); return; }
      pendingPreload = response;
      response.once("close", () => { if (pendingPreload === response) pendingPreload = null; });
      // One fixture-owned pending first response. The test deliberately
      // attempts a file load before this native generation commits HTTP.
      return;
    }
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end('<!doctype html><meta charset="utf-8"><title>Scoped browser fixture</title><h1>Scoped browser fixture</h1><input aria-label="Name"><button onclick="document.querySelector(\'output\').textContent=document.querySelector(\'input\').value">Apply</button><output aria-label="Result"></output>');
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
test.afterAll(async () => {
  herdr?.stop();
  await new Promise<void>((resolve) => server?.close(() => resolve()));
});
test.beforeEach(() => {
  run = isolate(herdr, "browser-cdp"); downloadRequests = 0;
  fileRedirectRequested = false; fileRedirect = null;
});
test.afterEach(async () => {
  pendingPreload?.end();
  pendingPreload = null;
  await browser?.close().catch(() => undefined);
  browser = null;
  await app?.close().catch(() => undefined);
  app = null;
  run.cleanup();
});

test("browser CDP: standard Playwright controls the scoped native page and disconnect preserves it", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await fitWindow(app, { width: 1024, height: 640 });
  await enterWorkspace(launched.page, "fixture");
  const opened = await fromPane(["browser", "open", `${origin}/fixture`, "--wait"]);
  const displayId = (opened.result as { view_id: string }).view_id;
  expect(typeof displayId).toBe("string");
  await expect.poll(() => fromPane(["browser", "connect", "--display", displayId]).then((answer) => answer.ok), { timeout: 20_000 }).toBe(true);
  const connected = await fromPane(["browser", "connect", "--display", displayId]);
  const result = connected.result as { cdp_http_url: string; browser_ws_url: string };
  expect(typeof result.cdp_http_url).toBe("string");
  expect((await fetch(`${new URL(result.cdp_http_url).origin}/json/version`)).status).toBe(404);
  const inventory = await (await fetch(`${result.cdp_http_url}/json/list`)).json() as { type: string; title: string; id: string }[];
  expect(inventory.length).toBe(1);
  expect(inventory[0]?.type).toBe("page");
  // The default client requests a global download directory. This gateway
  // deliberately supports the existing context with no global defaults.
  try { browser = await chromium.connectOverCDP(result.browser_ws_url, { noDefaults: true, timeout: 20_000 }); }
  catch { throw new Error("The scoped Playwright connection could not initialize"); }
  const context = browser.contexts()[0]!;
  expect(context.pages().length).toBe(1);
  const page = context.pages()[0]!;
  await page.getByRole("textbox", { name: "Name" }).fill("한글 scoped input");
  await page.getByRole("button", { name: "Apply" }).click();
  await expect(page.locator("output")).toHaveText("한글 scoped input");
  const cdp = await context.newCDPSession(page);
  const childUrl = `${origin.replace("127.0.0.1", "localhost")}/cross-site`;
  const childName = "scoped-oopif-child";
  let oopifEvidence: string | undefined;
  try {
    await page.evaluate(({ url, name }) => {
      const frame = document.createElement("iframe");
      frame.id = "cross-site";
      frame.name = name;
      frame.src = url;
      document.body.append(frame);
    }, { url: childUrl, name: childName });
    const frame = page.frameLocator("#cross-site");
    await frame.getByRole("textbox", { name: "Name" }).fill("descendant input");
    await frame.getByRole("button", { name: "Apply" }).click();
    await expect(frame.locator("output")).toHaveText("descendant input");
    // Cross-site interaction alone can succeed in one renderer. Attest the
    // actual frame tree of this exact candidate WebContents before claiming
    // an OOPIF, without adding site-isolation flags or touching another app.
    const provenance = await ownedFrameProvenance(`${origin}/fixture`, childUrl);
    const evidenceDir = process.env.HIDE_E2E_SCREENSHOT_DIR ?? path.join(REPO, "agents", "runs", "browser-control");
    fs.mkdirSync(evidenceDir, { recursive: true });
    const evidence = path.join(fs.mkdtempSync(path.join(evidenceDir, "oopif-")), "renderer-provenance.json");
    fs.writeFileSync(evidence, JSON.stringify({ candidatePid: app.process().pid, ...provenance }, null, 2));
    oopifEvidence = evidence;
    expect(provenance.childParent).toEqual(provenance.parent);
    expect(provenance.child.name).toBe(childName);
    expect(provenance.contentsId).toBe(Number(inventory[0]!.id.slice("page-".length)));
    expect(provenance.child.processId, "Electron default used one renderer; this run cannot prove OOPIF isolation").not.toBe(provenance.parent.processId);
    expect(provenance.child.osProcessId).not.toBe(provenance.parent.osProcessId);
    await expect(cdp.send("Browser.close")).rejects.toThrow();
    await expect(browser.newContext()).rejects.toThrow();
  } finally {
    await cdp.detach();
  }
  const candidate = await app.evaluate(({ BrowserWindow }) => {
    const windows = BrowserWindow.getAllWindows().filter((window) => window.getParentWindow() === null);
    if (windows.length !== 1) throw new Error("The candidate must have one main window");
    return { pid: process.pid, windowId: windows[0]!.getMediaSourceId().split(":")[1]! };
  });
  expect(candidate.pid).toBe(app.process().pid);
  const evidence = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (evidence) {
    fs.mkdirSync(evidence, { recursive: true });
    const capture = spawnSync("/usr/sbin/screencapture", ["-x", "-o", "-l", candidate.windowId, path.join(evidence, "browser-cdp-native.png")], { encoding: "utf8" });
    expect(capture.status, "candidate window capture failed").toBe(0);
  }
  await browser.close();
  browser = null;
  await expect.poll(() => app!.evaluate(({ BrowserWindow }, url) => {
    const views = BrowserWindow.getAllWindows()[0]!.contentView.children;
    return views.filter((view) => (view as { webContents?: Electron.WebContents }).webContents?.getURL() === url).length;
  }, `${origin}/fixture`)).toBe(1);
  // A standard flattened raw connection now owns the debugger. Record exact
  // parent/child protocol IDs, execute in that child session, and retire the
  // frame through its parent. No private Playwright transport or nested
  // protocol workaround is needed to address an attached descendant.
  await withCdp(result.browser_ws_url, async (send, socket) => {
    const events: TargetEvent[] = [];
    const proof: Record<string, unknown> = {};
    let overflow = false, malformed = false;
    const observed = (data: Buffer) => {
      try {
        const event = JSON.parse(data.toString()) as { method?: string; sessionId?: string; params?: { sessionId?: string; targetId?: string; targetInfo?: { targetId: string; type: string; url: string } } };
        if (event.method !== "Target.attachedToTarget" && event.method !== "Target.detachedFromTarget") return;
        if (events.length === 8) { overflow = true; return; }
        const params = event.params;
        const targetId = params?.targetInfo?.targetId ?? params?.targetId;
        if (typeof params?.sessionId !== "string" || typeof targetId !== "string") { malformed = true; return; }
        events.push({ method: event.method, parentSessionId: event.sessionId ?? null, sessionId: params.sessionId, targetId, type: params.targetInfo?.type ?? null, url: params.targetInfo?.url ?? null });
      } catch { malformed = true; }
    };
    if (!oopifEvidence) throw new Error("Native OOPIF provenance was not recorded");
    const evidence = oopifEvidence;
    const native = JSON.parse(fs.readFileSync(evidence, "utf8")) as Record<string, unknown>;
    const record = () => fs.writeFileSync(evidence, JSON.stringify({ ...native, scoped: { events, overflow, malformed, ...proof } }, null, 2));
    socket.on("message", observed);
    try {
      const attached = await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true });
      expect(attached.error).toBeUndefined();
      const parentSessionId = attached.result!.sessionId as string;
      expect(typeof parentSessionId).toBe("string");
      const parentAttachment = events.find((event) => event.method === "Target.attachedToTarget" && event.sessionId === parentSessionId);
      expect(parentAttachment).toMatchObject({ parentSessionId: null, targetId: inventory[0]!.id, type: "page", url: `${origin}/fixture` });
      proof.parentAttachment = parentAttachment;
      expect((await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true }, parentSessionId)).error).toBeUndefined();
      const childAttachment = () => events.find((event) => event.method === "Target.attachedToTarget" && event.parentSessionId === parentSessionId && event.type === "iframe");
      await expect.poll(() => childAttachment() !== undefined, { timeout: 10_000 }).toBe(true);
      const child = childAttachment()!;
      expect(child.sessionId).not.toBe(parentSessionId);
      proof.childAttachment = child;
      const expression = "({href:location.href,name:window.name,isTop:window===window.top,heading:document.querySelector('h1')?.textContent})";
      const identity = { href: childUrl, name: childName, isTop: false, heading: "Scoped browser fixture" };
      let childRuntime: Reply | undefined;
      await expect.poll(async () => {
        childRuntime = await send("Runtime.evaluate", { expression, returnByValue: true }, child.sessionId);
        expect(childRuntime.error).toBeUndefined();
        expect(childRuntime.sessionId).toBe(child.sessionId);
        return childRuntime.result;
      }, { timeout: 10_000 }).toMatchObject({ result: { value: identity } });
      proof.childRuntime = childRuntime;
      const nativeDuringChildSession = await ownedFrameProvenance(`${origin}/fixture`, childUrl);
      proof.nativeDuringChildSession = nativeDuringChildSession;
      expect(nativeDuringChildSession.contentsId).toBe(Number(inventory[0]!.id.slice("page-".length)));
      expect(nativeDuringChildSession.childParent).toEqual(nativeDuringChildSession.parent);
      expect(nativeDuringChildSession.child.name).toBe(childName);
      expect(nativeDuringChildSession.child.processId, "Electron default used one renderer during the child session; OOPIF proof is unavailable").not.toBe(nativeDuringChildSession.parent.processId);
      expect(nativeDuringChildSession.child.osProcessId).not.toBe(nativeDuringChildSession.parent.osProcessId);
      const childInfo = await send("Target.getTargetInfo", {}, child.sessionId);
      expect(childInfo.error).toBeUndefined();
      expect(childInfo.result).toMatchObject({ targetInfo: { targetId: child.targetId, type: "iframe" } });
      const parentTree = await send("Page.getFrameTree", {}, parentSessionId);
      const childTree = await send("Page.getFrameTree", {}, child.sessionId);
      expect(parentTree.error).toBeUndefined();
      expect(childTree.error).toBeUndefined();
      expect((parentTree.result as { frameTree: FrameTree }).frameTree.frame).toMatchObject({ id: inventory[0]!.id, url: `${origin}/fixture` });
      const childFrame = (childTree.result as { frameTree: FrameTree }).frameTree.frame;
      expect(childFrame).toMatchObject({ id: nativeDuringChildSession.frameOwner.frameId, name: childName, url: childUrl });
      expect(child.targetId).toBe(nativeDuringChildSession.frameOwner.frameId);
      proof.frameMapping = { parentFrameId: (parentTree.result as { frameTree: FrameTree }).frameTree.frame.id, nativeChildFrameId: nativeDuringChildSession.frameOwner.frameId, childFrameId: childFrame.id, childTargetId: child.targetId, childSessionId: child.sessionId, parentSessionId };
      record();
      const removed = await send("Runtime.evaluate", { expression: "document.querySelector('#cross-site').remove(); 'removed'", returnByValue: true }, parentSessionId);
      expect(removed.result).toMatchObject({ result: { value: "removed" } });
      const detached = () => events.find((event) => event.method === "Target.detachedFromTarget" && event.parentSessionId === parentSessionId && event.sessionId === child.sessionId && event.targetId === child.targetId);
      await expect.poll(() => detached() !== undefined, { timeout: 10_000 }).toBe(true);
      proof.detached = detached();
      const staleRuntime = await send("Runtime.evaluate", { expression, returnByValue: true }, child.sessionId);
      expect(staleRuntime.error).toBeDefined();
      expect(staleRuntime.result).toBeUndefined();
      expect(staleRuntime.sessionId).toBe(child.sessionId);
      const staleDetach = await send("Target.detachFromTarget", { sessionId: child.sessionId }, parentSessionId);
      expect(staleDetach.error).toBeDefined();
      expect(staleDetach.result).toBeUndefined();
      proof.retirementRefusals = { runtime: staleRuntime, detach: staleDetach };
      const parentRuntime = await send("Runtime.evaluate", { expression, returnByValue: true }, parentSessionId);
      expect(parentRuntime.result).toMatchObject({ result: { value: { href: `${origin}/fixture`, isTop: true, heading: "Scoped browser fixture" } } });
      proof.parentRuntime = parentRuntime;
      expect(overflow, "Scoped target event witness exceeded its eight-event cap").toBe(false);
      expect(malformed, "Scoped target event witness received an invalid event").toBe(false);
    } finally {
      socket.off("message", observed);
      record();
    }
  });
  expect((await browserCommand(result.browser_ws_url, "Browser.getVersion", {})).error).toBeUndefined();
});

test("browser CDP: creation stays in its originating area while another area is active and foreign renderer IDs are refused", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const first = await fromPane(["browser", "open", `${origin}/first`, "--wait"]);
  await expandViews(launched.page);
  const firstId = (first.result as { view_id: string }).view_id;
  const originalArea = (await coreViews()).find((view) => view.view_id === firstId)!.area_id;
  const connected = await fromPane(["browser", "connect"]);
  const result = connected.result as Capability;
  const foreign = await fromPane(["browser", "open", `${origin}/foreign`, "--wait"]);
  const foreignId = (foreign.result as { view_id: string }).view_id;
  const split = await fromPane(["view", "split", foreignId, "--area", originalArea, "--edge", "right"]);
  const foreignArea = (split.result as { area_id: string }).area_id;
  expect(foreignArea).not.toBe(originalArea);
  await fromPane(["view", "select", foreignId]);
  await expect.poll(() => coreViews().then((views) => views.find((view) => view.view_id === foreignId)?.active_area)).toBe(true);
  const excludedIds = await excludedRendererIds(`${origin}/foreign`);
  expect(excludedIds).toHaveLength(4);
  const inventory = await targets(result);
  expect(inventory).toHaveLength(1);
  expect(inventory[0]!.url).toBe(`${origin}/first`);
  await withCdp(result.browser_ws_url, async (send) => {
    for (const targetId of excludedIds) {
      expect((await send("Target.getTargetInfo", { targetId })).error).toBeDefined();
      expect((await send("Target.attachToTarget", { targetId, flatten: true })).error).toBeDefined();
    }
  });
  try { browser = await chromium.connectOverCDP(result.browser_ws_url, { noDefaults: true, timeout: 20_000 }); }
  catch { throw new Error("The scoped Playwright connection could not initialize"); }
  const context = browser.contexts()[0]!;
  await expect.poll(() => coreViews().then((views) => views.find((view) => view.view_id === foreignId)?.active_area)).toBe(true);
  const page = await context.newPage();
  await page.goto(`${origin}/created`);
  await expect(page.getByRole("heading")).toHaveText("Scoped browser fixture");
  const views = await coreViews();
  const created = views.filter((view) => view.kind === "browser" && view.view_id !== firstId && view.view_id !== foreignId);
  expect(created).toHaveLength(1);
  expect(created[0]!.area_id).toBe(originalArea);
  expect(views.find((view) => view.view_id === foreignId)!.area_id).toBe(foreignArea);
  expect(context.pages()).toHaveLength(2);
  await page.close();
  await expect.poll(() => coreViews().then((rows) => rows.some((view) => view.view_id === created[0]!.view_id))).toBe(false);
  expect((await coreViews()).filter((view) => view.kind === "browser")).toHaveLength(2);
});

test("browser CDP: selected-display and direct-page close replies arrive before capability shutdown", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  for (const mode of ["browser", "page"] as const) {
    const opened = await fromPane(["browser", "open", `${origin}/close-${mode}`, "--wait"]);
    const displayId = (opened.result as { view_id: string }).view_id;
    const capability = (await fromPane(["browser", "connect", "--display", displayId])).result as Capability;
    const inventory = await targets(capability);
    expect(inventory).toHaveLength(1);
    await withCdp(mode === "browser" ? capability.browser_ws_url : inventory[0]!.webSocketDebuggerUrl, async (send, socket) => {
      if (mode === "browser") expect((await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true })).error).toBeUndefined();
      const closed = waitForClose(socket);
      const [reply, code] = await Promise.all([
        mode === "browser" ? send("Target.closeTarget", { targetId: inventory[0]!.id }) : send("Page.close"), closed,
      ]);
      expect(reply.error).toBeUndefined();
      expect(reply.result).toEqual(mode === "browser" ? { success: true } : {});
      expect(code).toBe(1000);
    });
    expect((await fetch(`${capability.cdp_http_url}/json/version`)).status).toBe(404);
    expect(await refusedWebSocket(capability.browser_ws_url)).toBe(403);
    await expect.poll(() => coreViews().then((rows) => rows.some((view) => view.view_id === displayId))).toBe(false);
  }
});

test("browser CDP: injected incarnation-only sync revokes old URLs without replacing the native page", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const url = `${origin}/incarnation`;
  const opened = await fromPane(["browser", "open", url, "--wait"]);
  const displayId = (opened.result as { view_id: string }).view_id;
  const areaId = (await coreViews()).find((view) => view.view_id === displayId)!.area_id;
  const capability = (await fromPane(["browser", "connect", "--display", displayId])).result as Capability;
  const identity = () => app!.evaluate(({ BrowserWindow }, address) => {
    const contents = BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
      const page = (view as { webContents?: Electron.WebContents }).webContents;
      return page?.getURL() === address ? [page.id] : [];
    });
    if (contents.length !== 1) throw new Error("Expected one retained native generation");
    return contents[0]!;
  }, url);
  const contentsId = await identity();
  // Observe a real shell->native sync, then change only its incarnation at
  // the trusted IPC boundary. This proves native consumption/revocation;
  // it does not prove the core generates a new incarnation on unregister.
  await app.evaluate(({ ipcMain, BrowserWindow }, channel) => {
    const sender = BrowserWindow.getAllWindows()[0]!.webContents.id;
    const probe: { value?: unknown; listener: (event: Electron.IpcMainEvent, value: unknown) => void } = {
      listener: (event, value) => { if (event.sender.id === sender) probe.value = value; },
    };
    (globalThis as { cdpSyncProbe?: typeof probe }).cdpSyncProbe = probe;
    ipcMain.on(channel, probe.listener);
  }, BROWSER_SYNC_CHANNEL);
  try {
    await fitWindow(app, { width: 1024, height: 630 });
    const observed = () => app!.evaluate(() => (globalThis as { cdpSyncProbe?: { value?: unknown } }).cdpSyncProbe?.value);
    await expect.poll(async () => (await observed() as AuthoritySync | undefined)?.authorized_scopes?.some((scope) => scope.area_id === areaId)).toBe(true);
    const sync = await observed() as AuthoritySync;
    const scope = sync.authorized_scopes.find((scope) => scope.workspace === sync.workspace && scope.area_id === areaId)!;
    expect(Number.isSafeInteger(scope.incarnation)).toBe(true);
    expect(scope.incarnation).toBeLessThan(Number.MAX_SAFE_INTEGER);
    const changed: AuthoritySync = { ...sync, authorized_scopes: sync.authorized_scopes.map((row) => row === scope ? { ...row, incarnation: row.incarnation + 1 } : row) };
    expect(changed.displays).toEqual(sync.displays);
    expect(changed.retained).toEqual(sync.retained);
    await withCdp(capability.browser_ws_url, async (send, socket) => {
      const inventory = await targets(capability);
      expect((await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true })).error).toBeUndefined();
      const closed = waitForClose(socket);
      await launched.page.evaluate((state) => {
        if (!window.hideHost) throw new Error("Expected the trusted native bridge");
        window.hideHost.browser.sync(state);
      }, changed);
      await closed;
    });
    expect(await identity()).toBe(contentsId);
    expect((await fetch(`${capability.cdp_http_url}/json/version`)).status).toBe(404);
    expect(await refusedWebSocket(capability.browser_ws_url)).toBe(403);
    const fresh = (await fromPane(["browser", "connect", "--display", displayId])).result as Capability;
    expect(fresh.browser_ws_url === capability.browser_ws_url).toBe(false);
    await withCdp(fresh.browser_ws_url, async (send) => {
      const inventory = await targets(fresh);
      const attached = await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true });
      expect(attached.error).toBeUndefined();
      const reply = await send("Runtime.evaluate", { expression: "document.querySelector('h1').textContent", returnByValue: true }, attached.result!.sessionId as string);
      expect(reply.result).toMatchObject({ result: { value: "Scoped browser fixture" } });
    });
    expect(await identity()).toBe(contentsId);
  } finally {
    await app.evaluate(({ ipcMain }, channel) => {
      const root = globalThis as { cdpSyncProbe?: { listener: (event: Electron.IpcMainEvent, value: unknown) => void } };
      if (root.cdpSyncProbe) ipcMain.removeListener(channel, root.cdpSyncProbe.listener);
      delete root.cdpSyncProbe;
    }, BROWSER_SYNC_CHANNEL);
  }
});

test("browser CDP: the native guard cancels a pending preattach file load and remains restricted after disconnect", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const opened = await fromPane(["browser", "open", `${origin}/pending-preload`]);
  const displayId = (opened.result as { view_id: string }).view_id;
  const refusals = (id: string) => hostLog(run.env).filter((line) => line.event === "browser.request_refused" && line.reason === "cdp_file_boundary" && line.display_id === id);
  await expect.poll(() => pendingPreload !== null).toBe(true);
  const contentsId = await app.evaluate(({ BrowserWindow }) => {
    const pages = BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
      const contents = (view as { webContents?: Electron.WebContents }).webContents;
      return contents ? [contents] : [];
    });
    if (pages.length !== 1) throw new Error("Expected one pending candidate page");
    if (pages[0]!.debugger.isAttached()) throw new Error("Preload proof must precede debugger attachment");
    if (!pages[0]!.isLoadingMainFrame()) throw new Error("The first HTTP load must still be pending");
    return pages[0]!.id;
  });
  expect(refusals(displayId)).toHaveLength(0);
  // App-owned loadURL normally admits a file. Before any debugger lease,
  // then after a lease has ended, the candidate's structured request refusal
  // attributes cancellation to its native guard rather than HTTP->file CORS.
  const refusedLoads = async (contentsId: number, displayId: string) => {
    for (let attempt = 0; attempt < 2; attempt++) {
      const result = await app!.evaluate(async ({ webContents }, { contentsId, canary }) => {
        const contents = webContents.fromId(contentsId);
        if (!contents || contents.isDestroyed()) throw new Error("The owned native generation ended");
        let loaded = true;
        try { await contents.loadURL(canary); } catch { loaded = false; }
        const body = await contents.executeJavaScript("document.body?.innerText ?? ''") as string;
        return { loaded, body };
      }, { contentsId, canary: fileUrl });
      expect(result.loaded).toBe(false);
      expect(result.body).not.toContain("FORBIDDEN_CDP_FILE_CONTENT");
      await expect.poll(() => refusals(displayId).length).toBe(1);
    }
    expect(refusals(displayId)[0]).toMatchObject({ display_id: displayId, resource_type: "mainFrame", reason: "cdp_file_boundary" });
    for (const key of ["url", "source_url", "token", "contents"]) expect(refusals(displayId)[0]).not.toHaveProperty(key);
  };
  await refusedLoads(contentsId, displayId);
  // A blocked error-page commit may conservatively quarantine the attacked
  // generation. Test normal HTTP and post-lease behavior on a fresh page.
  await fromPane(["view", "close", displayId]);
  const later = await fromPane(["browser", "open", `${origin}/post-lease-guard`, "--wait"]);
  const laterId = (later.result as { view_id: string }).view_id;
  const capability = (await fromPane(["browser", "connect", "--display", laterId])).result as Capability;
  const inventory = await targets(capability);
  expect(inventory).toHaveLength(1);
  expect(inventory[0]!.id).toMatch(/^page-\d+$/);
  const laterContents = Number(inventory[0]!.id.slice("page-".length));
  expect(laterContents).not.toBe(contentsId);
  await withCdp(capability.browser_ws_url, async (send) => {
    const attached = await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true });
    expect(attached.error).toBeUndefined();
    expect((await send("Runtime.evaluate", { expression: "document.querySelector('h1').textContent", returnByValue: true }, attached.result!.sessionId as string)).result).toMatchObject({ result: { value: "Scoped browser fixture" } });
  });
  await expect.poll(() => app!.evaluate(({ webContents }, id) => webContents.fromId(id)?.debugger.isAttached(), laterContents)).toBe(false);
  expect(refusals(laterId)).toHaveLength(0);
  await refusedLoads(laterContents, laterId);
});

test("browser CDP: native files cannot be read through runtime, frames, redirects or creation", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const opened = await fromPane(["browser", "open", `${origin}/protected`, "--wait"]);
  const displayId = (opened.result as { view_id: string }).view_id;
  const connected = await fromPane(["browser", "connect"]);
  const capability = connected.result as { browser_ws_url: string; cdp_http_url: string };
  try { browser = await chromium.connectOverCDP(capability.browser_ws_url, { noDefaults: true, timeout: 20_000 }); }
  catch { throw new Error("The scoped Playwright connection could not initialize"); }
  const context = browser.contexts()[0]!;
  const page = context.pages()[0]!;
  const cdp = await context.newCDPSession(page);
  await expect(cdp.send("Page.navigate", { url: fileUrl })).rejects.toThrow();
  expect((await browserCommand(capability.browser_ws_url, "Target.createTarget", { url: fileUrl })).error).toBeDefined();
  await expect(cdp.send("Network.loadNetworkResource", { url: fileUrl, options: { disableCache: true, includeCredentials: false } })).rejects.toThrow();
  // The address is assembled at runtime. The native policy receives the
  // resolved request/navigation; the gateway never parses JavaScript.
  const encoded = [...fileUrl].map((letter) => letter.charCodeAt(0));
  const navigationRefusals = () => hostLog(run.env).filter((line) => line.event === "browser.navigation_refused" && line.reason === "cdp_file_boundary" && line.display_id === displayId);
  const navigation = await cdp.send("Runtime.evaluate", { expression: `location.assign(String.fromCharCode(...${JSON.stringify(encoded)})); 'attempted'`, returnByValue: true });
  expect(navigation.exceptionDetails).toBeUndefined();
  expect(navigation.result.value).toBe("attempted");
  await expect(page.getByRole("heading")).toHaveText("Scoped browser fixture");
  expect(page.url()).toBe(`${origin}/protected`);
  const repeatedNavigation = await cdp.send("Runtime.evaluate", { expression: `for(let attempt=0;attempt<20;attempt++) location.assign(String.fromCharCode(...${JSON.stringify(encoded)})); 'attempted'`, returnByValue: true });
  expect(repeatedNavigation.exceptionDetails).toBeUndefined();
  expect(repeatedNavigation.result.value).toBe("attempted");
  // Chromium can preempt an HTTP-to-file attempt before the app sees it.
  // Separate app-owned main-frame and iframe cases require guard causality.
  expect(navigationRefusals().length).toBeLessThanOrEqual(1);
  const popups = await cdp.send("Runtime.evaluate", { expression: `for(let attempt=0;attempt<20;attempt++) window.open(String.fromCharCode(...${JSON.stringify(encoded)})); 'attempted'`, returnByValue: true });
  expect(popups.exceptionDetails).toBeUndefined();
  expect(popups.result.value).toBe("attempted");
  const popupRefusals = hostLog(run.env).filter((line) => line.event === "browser.window_open_refused" && line.reason === "cdp_file_boundary" && line.display_id === displayId);
  // Chromium may reject a popup before Electron receives it. When the
  // native handler does run, repeated attempts still emit at most once.
  expect(popupRefusals.length).toBeLessThanOrEqual(1);
  await expect(page.getByRole("heading")).toHaveText("Scoped browser fixture");
  const resource = await cdp.send("Runtime.evaluate", { expression: `fetch(String.fromCharCode(...${JSON.stringify(encoded)})).then(r => r.text()).then(() => 'read', () => 'blocked')`, awaitPromise: true, returnByValue: true });
  expect(resource.result.value).toBe("blocked");
  await cdp.send("Runtime.evaluate", { expression: `const frame=document.createElement('iframe'); frame.srcdoc=${JSON.stringify(`<iframe src="${fileUrl}"></iframe>`)}; document.body.append(frame); const image=document.createElement('img'); image.src=${JSON.stringify(fileUrl)}; document.body.append(image);` });
  const nativeState = () => app!.evaluate(({ BrowserWindow }, url) => {
    const contents = BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
      const page = (view as { webContents?: Electron.WebContents }).webContents;
      return page && page.getURL() === url ? [page] : [];
    });
    if (contents.length !== 1) throw new Error("Expected exactly one protected candidate page");
    return contents[0]!.mainFrame.framesInSubtree.map((frame) => frame.url);
  }, `${origin}/protected`);
  await expect.poll(async () => (await nativeState()).length).toBeGreaterThanOrEqual(2);
  expect((await nativeState()).some((url) => url.startsWith("file:"))).toBe(false);
  expect(await page.locator("body").innerText()).not.toContain("FORBIDDEN_CDP_FILE_CONTENT");
  // A manual file display retains ordinary native behavior but is excluded
  // even when its source is within this checkout and its renderer is live.
  const manual = path.join(herdr.root, "fixture", "manual-cdp-file.html");
  fs.writeFileSync(manual, "<title>Manual local file</title><h1>Manual local file</h1>");
  const manualOpened = await fromPane(["browser", "open", pathToFileURL(manual).href, "--wait"]);
  const manualId = (manualOpened.result as { view_id: string }).view_id;
  expect((await fromPane(["browser", "connect", "--display", manualId], false)).ok).toBe(false);
  const inventory = await (await fetch(`${capability.cdp_http_url}/json/list`)).json() as { id: string; url: string }[];
  expect(inventory).toHaveLength(1);
  expect(inventory[0]!.url).toBe(`${origin}/protected`);
  // A script installed before disconnect must not gain file access after it.
  await cdp.send("Runtime.evaluate", { expression: `addEventListener('cdp-file-attempt', () => location.assign(${JSON.stringify(fileUrl)}));` });
  await cdp.detach();
  await browser.close();
  browser = null;
  await app.evaluate(async ({ BrowserWindow }, url) => {
    for (const view of BrowserWindow.getAllWindows()[0]!.contentView.children) {
      const contents = (view as { webContents?: Electron.WebContents }).webContents;
      if (contents?.getURL() === url) await contents.executeJavaScript("dispatchEvent(new Event('cdp-file-attempt'))");
    }
  }, `${origin}/protected`);
  expect((await nativeState()).some((url) => url.startsWith("file:"))).toBe(false);
  const again = await fromPane(["browser", "connect", "--display", displayId]);
  try { browser = await chromium.connectOverCDP((again.result as { browser_ws_url: string }).browser_ws_url, { noDefaults: true, timeout: 20_000 }); }
  catch { throw new Error("The scoped Playwright reconnection could not initialize"); }
  await expect(browser.contexts()[0]!.pages()[0]!.getByRole("heading")).toHaveText("Scoped browser fixture");
  await browser.close();
  browser = null;
  // A fresh native generation records redirect denial without sharing the
  // earlier attempts' finite diagnostic categories.
  const redirectOpened = await fromPane(["browser", "open", `${origin}/redirect-guard`, "--wait"]);
  const redirectId = (redirectOpened.result as { view_id: string }).view_id;
  const redirectCapability = (await fromPane(["browser", "connect", "--display", redirectId])).result as Capability;
  const redirectInventory = await targets(redirectCapability);
  expect(redirectInventory).toHaveLength(1);
  expect(redirectInventory[0]!.id).toMatch(/^page-\d+$/);
  const redirectContents = Number(redirectInventory[0]!.id.slice("page-".length));
  try { browser = await chromium.connectOverCDP(redirectCapability.browser_ws_url, { noDefaults: true, timeout: 20_000 }); }
  catch { throw new Error("The scoped redirect proof connection could not initialize"); }
  const nativeRefusals = () => hostLog(run.env).filter((line) => ["browser.navigation_refused", "browser.request_refused"].includes(line.event as string) && line.reason === "cdp_file_boundary" && line.display_id === redirectId).length;
  expect(nativeRefusals()).toBe(0);
  const redirectPage = browser.contexts()[0]!.pages()[0]!;
  expect(fileRedirectRequested).toBe(false);
  expect(fileRedirect).toBeNull();
  await expect(redirectPage.goto(`${origin}/file-redirect`)).rejects.toThrow();
  expect(fileRedirectRequested).toBe(true);
  await expect.poll(() => fileRedirect).toEqual({ status: 302, location: fileUrl });
  expect(nativeRefusals()).toBeLessThanOrEqual(2);
  for (const event of ["browser.navigation_refused", "browser.request_refused"]) {
    expect(hostLog(run.env).filter((line) => line.event === event && line.reason === "cdp_file_boundary" && line.display_id === redirectId).length).toBeLessThanOrEqual(1);
  }
  const redirectNative = await app.evaluate(async ({ webContents }, id) => {
    const contents = webContents.fromId(id);
    if (!contents || contents.isDestroyed()) throw new Error("The redirect page's native generation ended");
    return { urls: contents.mainFrame.framesInSubtree.map((frame) => frame.url), body: await contents.executeJavaScript("document.body?.innerText ?? ''") as string };
  }, redirectContents);
  expect(redirectNative.urls.some((url) => url.startsWith("file:"))).toBe(false);
  expect(redirectNative.body).not.toContain("FORBIDDEN_CDP_FILE_CONTENT");
  const urls = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
    const contents = (view as { webContents?: Electron.WebContents }).webContents;
    return contents ? contents.mainFrame.framesInSubtree.map((frame) => frame.url) : [];
  }));
  expect(urls).not.toContain(fileUrl);
  await redirectPage.goto(`${origin}/redirect-guard`);
  await expect(redirectPage.getByRole("heading")).toHaveText("Scoped browser fixture");
  expect(redirectPage.url()).toBe(`${origin}/redirect-guard`);
  expect(await redirectPage.locator("body").innerText()).not.toContain("FORBIDDEN_CDP_FILE_CONTENT");
});

test("browser CDP: the owned native iframe request is causally canceled before file content", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const url = `${origin}/frame-guard`;
  const opened = await fromPane(["browser", "open", url, "--wait"]);
  const displayId = (opened.result as { view_id: string }).view_id;
  const capability = (await fromPane(["browser", "connect", "--display", displayId])).result as Capability;
  const refusals = () => hostLog(run.env).filter((line) => line.event === "browser.request_refused" && line.reason === "cdp_file_boundary" && line.display_id === displayId && line.resource_type === "subFrame");
  await withCdp(capability.browser_ws_url, async (send) => {
    const inventory = await targets(capability);
    const attached = await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true });
    expect(attached.error).toBeUndefined();
    const sessionId = attached.result!.sessionId as string;
    expect((await send("Runtime.evaluate", { expression: "const frame=document.createElement('iframe'); frame.srcdoc='<h1>Owned frame</h1>'; document.body.append(frame); 'created'", returnByValue: true }, sessionId)).error).toBeUndefined();
    await expect.poll(() => app!.evaluate(({ BrowserWindow }, address) => {
      const contents = BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
        const page = (view as { webContents?: Electron.WebContents }).webContents;
        return page?.getURL() === address ? [page] : [];
      })[0];
      return contents?.mainFrame.framesInSubtree.some((frame) => frame.url === "about:srcdoc");
    }, url)).toBe(true);
    expect(refusals()).toHaveLength(0);
    // Public CDP refuses file Page.navigate. This deliberate app-owned
    // native command bypasses that protocol refusal only inside the private
    // candidate, to exercise the independent subframe request guard.
    const result = await app!.evaluate(async ({ BrowserWindow }, { address, canary }) => {
      const contents = BrowserWindow.getAllWindows()[0]!.contentView.children.flatMap((view) => {
        const page = (view as { webContents?: Electron.WebContents }).webContents;
        return page?.getURL() === address ? [page] : [];
      })[0];
      if (!contents || !contents.debugger.isAttached()) throw new Error("Expected the owned native debugger lease");
      const command = async (method: string, params: Record<string, unknown> = {}) => {
        let timer: NodeJS.Timeout | undefined;
        try {
          return await Promise.race([contents.debugger.sendCommand(method, params), new Promise<never>((_resolve, reject) => {
            timer = setTimeout(() => reject(new Error("Owned native frame command timed out")), 10_000);
          })]);
        } finally { clearTimeout(timer); }
      };
      const tree = await command("Page.getFrameTree") as { frameTree: { childFrames?: { frame: { id: string; url: string } }[] } };
      const frame = tree.frameTree.childFrames?.find((child) => child.frame.url === "about:srcdoc");
      if (!frame) throw new Error("Expected an attested child of the owned page");
      try { await command("Page.navigate", { frameId: frame.frame.id, url: canary }); }
      catch { /* Cancellation may reject the native command before its reply. */ }
      const bodies: string[] = [];
      for (const child of contents.mainFrame.framesInSubtree) {
        if (child.isDestroyed()) continue;
        bodies.push(await child.executeJavaScript("document.body?.innerText ?? ''") as string);
      }
      return { bodies };
    }, { address: url, canary: fileUrl });
    await expect.poll(() => refusals().length).toBe(1);
    expect(result.bodies.join("\n")).not.toContain("FORBIDDEN_CDP_FILE_CONTENT");
    expect(refusals()[0]).not.toHaveProperty("url");
  });
});

test("browser CDP: controlled downloads are always canceled and log once after disconnect", async () => {
  const launched = await launch(run.env);
  app = launched.app;
  await enterWorkspace(launched.page, "fixture");
  const opened = await fromPane(["browser", "open", `${origin}/download-guard`, "--wait"]);
  const displayId = (opened.result as { view_id: string }).view_id;
  const capability = (await fromPane(["browser", "connect", "--display", displayId])).result as Capability;
  const inventory = await targets(capability);
  expect(inventory).toHaveLength(1);
  const contentsId = Number(inventory[0]!.id.slice("page-".length));
  const downloadDir = path.join(run.root, "downloads");
  fs.mkdirSync(downloadDir);
  const refusals = () => hostLog(run.env).filter((line) => line.event === "browser.download_refused" && line.reason === "cdp_filesystem_boundary" && line.display_id === displayId);
  const configuredPath = await app.evaluate(({ app: candidateApp, webContents }, { contentsId, downloadDir }) => {
    const contents = webContents.fromId(contentsId);
    if (!contents || contents.isDestroyed()) throw new Error("Expected the owned candidate download page");
    candidateApp.setPath("downloads", downloadDir);
    if (candidateApp.getPath("downloads") !== downloadDir) throw new Error("The candidate download path is not confined");
    contents.session.setDownloadPath(downloadDir);
    const root = globalThis as { cdpDownloadProbe?: { count: number; unrefused: number; overflow: boolean; cleanup: () => void } };
    if (root.cdpDownloadProbe) throw new Error("The candidate download witness already has an owner");
    const probe = { count: 0, unrefused: 0, overflow: false, cleanup: () => contents.session.off("will-download", observe) };
    // The production listener is installed before this observer. It only
    // observes cancellation; a regression can write to this private sink.
    const observe = (event: Electron.Event, _item: Electron.DownloadItem, owner: Electron.WebContents) => {
      if (probe.count === 8) probe.overflow = true;
      else {
        probe.count++;
        if (!event.defaultPrevented || owner.id !== contents.id) probe.unrefused++;
      }
    };
    root.cdpDownloadProbe = probe;
    contents.session.on("will-download", observe);
    return candidateApp.getPath("downloads");
  }, { contentsId, downloadDir });
  const witness = () => app!.evaluate(() => {
    const probe = (globalThis as { cdpDownloadProbe?: { count: number; unrefused: number; overflow: boolean } }).cdpDownloadProbe;
    if (!probe) throw new Error("The candidate download witness ended");
    return { count: probe.count, unrefused: probe.unrefused, overflow: probe.overflow };
  });
  const nativeAttempts = () => app!.evaluate(({ webContents }, { contentsId, url }) => {
    const contents = webContents.fromId(contentsId);
    if (!contents || contents.isDestroyed()) throw new Error("The owned download generation ended");
    for (let attempt = 0; attempt < 2; attempt++) contents.downloadURL(url);
  }, { contentsId, url: `${origin}/owned-download` });
  try {
    expect(configuredPath).toBe(downloadDir);
    expect(refusals()).toHaveLength(0);
    await withCdp(capability.browser_ws_url, async (send) => {
      const attached = await send("Target.attachToTarget", { targetId: inventory[0]!.id, flatten: true });
      expect(attached.error).toBeUndefined();
      const installed = await send("Runtime.evaluate", {
        expression: "addEventListener('scoped-download-attempt', () => { const anchor=document.createElement('a'); anchor.href='data:text/plain,OWNED_DOWNLOAD_FIXTURE'; anchor.download='owned-runtime-download.txt'; document.body.append(anchor); anchor.click(); anchor.remove(); }); 'installed'",
        returnByValue: true,
      }, attached.result!.sessionId as string);
      expect(installed.result).toMatchObject({ result: { value: "installed" } });
      await nativeAttempts();
      await expect.poll(witness).toEqual({ count: 2, unrefused: 0, overflow: false });
      expect(refusals()).toHaveLength(1);
    });
    await expect.poll(() => app!.evaluate(({ webContents }, id) => webContents.fromId(id)?.debugger.isAttached(), contentsId)).toBe(false);
    await app.evaluate(async ({ webContents }, id) => {
      const contents = webContents.fromId(id);
      if (!contents || contents.isDestroyed()) throw new Error("The owned download generation ended");
      await contents.executeJavaScript("dispatchEvent(new Event('scoped-download-attempt'))", true);
    }, contentsId);
    await expect.poll(witness).toEqual({ count: 3, unrefused: 0, overflow: false });
    await nativeAttempts();
    await expect.poll(witness).toEqual({ count: 5, unrefused: 0, overflow: false });
    expect(downloadRequests).toBe(MAX_DOWNLOAD_REQUESTS);
    expect(refusals()).toHaveLength(1);
    for (const key of ["url", "path", "filename", "token", "contents"]) expect(refusals()[0]).not.toHaveProperty(key);
    // Chromium starts a download in an unnamed intermediate file in the sink,
    // and may write its first bytes there, before the refusal reaches it; the
    // cancellation removes that file on Chromium's download thread after the
    // witness has counted the refusal. The sink is read once that removal is
    // done, and a write the refusal missed stays there.
    await expect.poll(() => fs.readdirSync(downloadDir)).toEqual([]);
  } finally {
    await app.evaluate(() => {
      const root = globalThis as { cdpDownloadProbe?: { cleanup: () => void } };
      root.cdpDownloadProbe?.cleanup();
      delete root.cdpDownloadProbe;
    });
  }
});
