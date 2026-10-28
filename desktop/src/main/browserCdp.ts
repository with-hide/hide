// One app-owned loopback CDP server. A capability names an area, never the
// Chromium process: targets come exclusively from BrowserViews' inventory.
import { createHash, randomBytes, timingSafeEqual } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { Socket } from "node:net";
import { WebSocket, WebSocketServer, type RawData } from "ws";

type Json = Record<string, unknown>;
type DebuggerMessage = (event: unknown, method: string, params: Json, sessionId: string) => void;
type DebuggerDetach = (event: unknown, reason: string) => void;
/** The Electron boundary, also usable without loading Electron in server tests. */
export interface PageDebugger {
  isAttached(): boolean;
  attach(version?: string): void;
  detach(): void;
  sendCommand(method: string, params?: Json, sessionId?: string): Promise<unknown>;
  on(event: "message", listener: DebuggerMessage): unknown;
  on(event: "detach", listener: DebuggerDetach): unknown;
  removeListener(event: "message", listener: DebuggerMessage): unknown;
  removeListener(event: "detach", listener: DebuggerDetach): unknown;
}
export type CdpPage = {
  workspace: string;
  area_id: string;
  id: string;
  /** A page of the file partition: only a comment capability reaches it (`BROWSER_DISPLAYS.md`, Comment mode). */
  local?: boolean;
  contents: { id: number; debugger: PageDebugger; isDestroyed(): boolean; getURL(): string; getTitle(): string };
};
export type CdpScope = { workspace: string; area_id: string; display_id?: string };
/** What a capability is for. None is an agent's; `comment` is the operator's Comment mode, held by hided alone. */
export type CdpPurpose = "comment";
export type CdpAction = { action: "open" | "close" | "select"; url?: string; display_id?: string; request_id: string };
export type CdpActionResult = { view_id: string; area_id: string | null; load: number | null };
export type CdpRetirement = { contentsId: number; reason: "closed" | "evicted" | "replaced" | "window_closed" };
type Options = {
  pages: () => readonly CdpPage[];
  incarnation: (scope: CdpScope) => number | null;
  changed: (listener: (retirement?: CdpRetirement) => void) => () => void;
  action: (scope: CdpScope, action: CdpAction, signal: AbortSignal) => Promise<CdpActionResult>;
  attached: (page: CdpPage, attached: boolean) => void;
  /** Loads a display's unloaded page again, hidden, for an agent's connection to it (`BrowserViews.cdpRevive`). */
  revive?: (scope: CdpScope) => Promise<"none" | "loaded" | "failed">;
  log: (event: string, fields: Record<string, string | number | boolean>) => void;
  chromeVersion: string;
  /** Wall clock is a platform boundary; protocol/server timers stay native. */
  now?: () => number;
};
type Capability = { path: string; scope: CdpScope; purpose?: CdpPurpose; incarnation: number; generations: Map<string, number> };
type Command = { id: number; method: string; params: Json; sessionId?: string };
type Session = { id: string; targetId: string; kind: "page" | "tab" | "iframe"; lease: Lease; nativeId?: string; parent?: Session; browserParent?: string; flattened: boolean };
type Lease = {
  page: CdpPage;
  client: Client;
  root: Session;
  message: DebuggerMessage;
  detach: DebuggerDetach;
  pageAttaches: { targetId: string }[];
  ready: Promise<void>;
  mainFrameId: string;
};
type Intent = {
  fingerprint: string; issued: number; requestId: string; method: string; attempts: number;
  state: "pending" | "complete" | "retryable" | "fatal";
  result: Promise<unknown>; action?: CdpAction;
  nested?: { parentKey: string; targetId: string; parent?: Session };
};
type Client = {
  intents: Map<string, Intent>;
  closingPages: Set<number>;
  draining: boolean;
  socket: WebSocket;
  capability: Capability;
  sessions: Map<string, Session>;
  // Protocol-only parents. They have the capability's authority, never a
  // Chromium browser debugger or an additional native browser context.
  browserSessions: Set<string>;
  leases: Map<number, Lease>;
  abort: AbortController;
  direct?: Session;
  discover: boolean;
  autoAttach: boolean;
  autoKind: "page" | "tab";
  autoFlattened: boolean;
  known: Map<string, string>;
  /** Execution contexts of the isolated worlds this client's comment made. */
  commentWorlds: Set<number>;
  pending: number;
  minute: number;
  requests: number;
};

// Caps are app constants, not environment options. ws bounds incoming frames;
// pending and bufferedAmount also bound work and outbound backpressure.
const MAX_CAPABILITIES = 64;
const MAX_SOCKETS = 32;
const MAX_CLIENTS = 8;
const MAX_HTTP_REQUESTS = 16;
const MAX_PENDING = 32;
const MAX_SESSIONS = 64;
const MAX_BODY_BYTES = 16 * 1024;
const MAX_MESSAGE_BYTES = 4 * 1024 * 1024;
const MAX_BUFFER_BYTES = 8 * 1024 * 1024;
const MAX_REQUESTS_PER_MINUTE = 600;
const MAX_INTENTS = 128;
// Core accepts an intent for ten minutes; leave a margin at the boundary.
const MAX_INTENT_AGE_MS = 9 * 60_000;
const MUTATIONS = new Set(["Target.createTarget", "Target.closeTarget", "Target.activateTarget", "Target.attachToTarget", "Target.attachToBrowserTarget", "Target.detachFromTarget", "Page.close", "Page.bringToFront"]);
const REQUEST_TIMEOUT_MS = 10_000;
const PAGE_DOMAINS = new Set(["Accessibility", "Animation", "Audits", "CSS", "DOM", "DOMSnapshot", "Debugger", "Emulation", "Fetch", "Input", "Inspector", "Log", "Network", "Overlay", "Page", "Performance", "PerformanceTimeline", "Profiler", "Runtime", "Schema", "Security"]);
// A comment capability reaches a local file page, so it carries only what the
// comment session sends (`hided/src/browser_page/comment.rs`, `page.rs` attach)
// and the events it reads. Anything else, navigation, Fetch, Network, Input,
// new targets, is refused for it, and its evaluations run in an isolated world.
const COMMENT_METHODS = new Set([
  "Target.getTargets", "Target.attachToTarget", "Page.enable", "Page.getFrameTree", "Page.createIsolatedWorld",
  "Runtime.enable", "Runtime.addBinding", "Runtime.evaluate", "Page.captureScreenshot",
]);
// The parameters each of those methods takes from the session, and no others: a
// world made with universal access, an evaluation with a user gesture or the
// command-line API, a binding in the page's own world would each be more than the session asks for.
const COMMENT_PARAMS: Record<string, readonly string[]> = {
  "Target.getTargets": [], "Target.attachToTarget": ["targetId", "flatten"], "Page.enable": [], "Page.getFrameTree": [], "Runtime.enable": [],
  "Page.createIsolatedWorld": ["frameId", "worldName"], "Runtime.addBinding": ["name", "executionContextName"],
  "Runtime.evaluate": ["expression", "contextId", "returnByValue", "awaitPromise"], "Page.captureScreenshot": ["format", "clip"],
};
const MAX_COMMENT_WORLDS = 64;
const COMMENT_EVENTS = new Set(["Runtime.bindingCalled", "Page.frameNavigated", "Page.navigatedWithinDocument", "Inspector.detached"]);
const SCOPED_BROWSER_METHODS = new Set(["Browser.getVersion", "Target.getTargets", "Target.getTargetInfo", "Target.getBrowserContexts", "Target.attachToTarget", "Target.detachFromTarget"]);
// These act on the shared Chromium session or the host filesystem, even when
// sent through a page debugger. They never reach Electron.
const DENIED_PAGE_METHODS = new Set([
  "DOM.setFileInputFiles", "DOM.getFileInfo", "Page.setDownloadBehavior", "Page.generateTestReport",
  "Network.getAllCookies", "Network.clearBrowserCookies", "Network.clearBrowserCache",
  "Network.setCookie", "Network.setCookies", "Network.deleteCookies",
  "Network.loadNetworkResource", "Fetch.takeResponseBodyAsStream",
  "Security.setIgnoreCertificateErrors", "Security.setOverrideCertificateErrors",
  "Emulation.setScriptExecutionDisabled",
  "Page.crash",
]);

class ProtocolError extends Error {
  constructor(message: string, readonly code = -32000) { super(message); }
}
/** The trusted action may have committed before its response was lost. */
export class CdpActionUncertain extends ProtocolError {
  constructor() { super("Workspace action completion is unknown; retry the same CDP request ID"); }
}
function object(value: unknown): value is Json { return value !== null && typeof value === "object" && !Array.isArray(value); }
function secret(): string { return randomBytes(32).toString("hex"); }
function sameToken(a: string, b: string): boolean {
  const left = Buffer.from(a), right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}
function targetId(page: CdpPage): string { return `page-${page.contents.id}`; }
function identifier(value: unknown): value is string { return typeof value === "string" && value.length > 0 && value.length <= 256 && !value.includes("\0") && !value.includes("\u0001"); }
function cdpAddress(value: string): boolean {
  try { const url = new URL(value); return url.protocol === "http:" || url.protocol === "https:" || url.href === "about:blank"; }
  catch { return false; }
}
/** The daemon's success envelope and optional core fields, not raw top-level
 * view fields. Close/select omit load; already-closed receipts may omit area. */
export function parseBrowserControlResult(value: unknown): CdpActionResult | null {
  if (!object(value) || value.ok !== true || !object(value.result)) return null;
  const { view_id, area_id, load } = value.result;
  if (!identifier(view_id) || area_id !== undefined && area_id !== null && !identifier(area_id)
    || load !== undefined && load !== null && (!Number.isSafeInteger(load) || (load as number) < 0)) return null;
  return { view_id, area_id: (area_id ?? null) as string | null, load: (load ?? null) as number | null };
}
function fileAddress(value: string): boolean {
  try { return new URL(value).protocol === "file:"; }
  catch { return false; }
}
/** The purpose a `/connect` names: none, `comment` (for one display), or invalid (null). */
function purposeOf(value: unknown, scope: CdpScope): CdpPurpose | undefined | null {
  if (!object(value) || value.purpose === undefined) return undefined;
  return value.purpose === "comment" && scope.display_id ? "comment" : null;
}
function scopeOf(value: unknown): CdpScope | null {
  if (!object(value) || typeof value.workspace !== "string" || value.workspace.length > 8192 || !/^[^\0]+\0[^\0]+$/.test(value.workspace)) return null;
  if (!identifier(value.area_id)) return null;
  if (value.display_id !== undefined && !identifier(value.display_id)) return null;
  return { workspace: value.workspace, area_id: value.area_id, ...(value.display_id === undefined ? {} : { display_id: value.display_id }) };
}
function commandOf(value: unknown): Command {
  if (!object(value) || !Number.isSafeInteger(value.id) || (value.id as number) < 0 || typeof value.method !== "string" || value.method.length > 256 || !/^[A-Za-z]+\.[A-Za-z]+$/.test(value.method)) throw new ProtocolError("Invalid CDP request", -32600);
  if (value.params !== undefined && !object(value.params)) throw new ProtocolError("Invalid parameters", -32602);
  if (value.sessionId !== undefined && typeof value.sessionId !== "string") throw new ProtocolError("Invalid session", -32602);
  return { id: value.id as number, method: value.method, params: (value.params ?? {}) as Json, ...(value.sessionId === undefined ? {} : { sessionId: value.sessionId }) };
}
function text(params: Json, key: string): string {
  const value = params[key];
  if (typeof value !== "string" || !value || value.length > 16384) throw new ProtocolError(`Invalid ${key}`, -32602);
  return value;
}

export class BrowserCdpGateway {
  private readonly server = createServer((request, response) => { void this.http(request, response); });
  private readonly ws = new WebSocketServer({ noServer: true, maxPayload: MAX_MESSAGE_BYTES, perMessageDeflate: false });
  private readonly sockets = new Set<Socket>();
  private readonly clients = new Set<Client>();
  private readonly capabilities = new Map<string, Capability>();
  private readonly owners = new Map<number, Lease>();
  private readonly controlToken = secret();
  // Client metadata for the existing context, never native context authority.
  private readonly browserContextId = secret();
  private endpoint = "";
  private listening: Promise<{ endpoint: string; token: string }> | null = null;
  private rejectStart: ((error: Error) => void) | null = null;
  private readonly listenAbort = new AbortController();
  private closing: Promise<void> | null = null;
  private stopped = false;
  private available = false;
  private httpPending = 0;
  private readonly unsubscribe: () => void;
  private readonly now: () => number;

  constructor(private readonly options: Options) {
    this.now = options.now ?? Date.now;
    this.server.requestTimeout = REQUEST_TIMEOUT_MS;
    this.server.headersTimeout = REQUEST_TIMEOUT_MS;
    this.server.keepAliveTimeout = 1000;
    this.server.maxHeadersCount = 32;
    this.server.on("connection", (socket) => {
      if (this.stopped || this.sockets.size >= MAX_SOCKETS) {
        this.options.log("browser.cdp_limit", { resource: "connections", sockets: this.sockets.size });
        socket.destroy(); return;
      }
      this.sockets.add(socket);
      socket.on("close", () => this.sockets.delete(socket));
      socket.setTimeout(REQUEST_TIMEOUT_MS, () => socket.destroy());
    });
    this.server.on("clientError", (_error, socket) => socket.destroy());
    this.server.on("upgrade", (request, socket, head) => this.upgrade(request, socket as Socket, head));
    this.server.on("error", () => this.options.log("browser.cdp_server_failed", { reason: "listen" }));
    this.unsubscribe = options.changed((retirement) => this.refresh(retirement));
  }

  start(): Promise<{ endpoint: string; token: string }> {
    if (this.stopped) return Promise.reject(new Error("CDP gateway is closed"));
    this.listening ??= new Promise((resolve, reject) => {
      this.rejectStart = reject;
      const fail = (error: Error) => { this.rejectStart = null; this.server.removeListener("listening", ready); reject(error); };
      const ready = () => {
        this.rejectStart = null;
        this.server.removeListener("error", fail);
        const address = this.server.address();
        if (!address || typeof address === "string" || this.stopped) { reject(new Error("CDP gateway is closed")); return; }
        this.endpoint = `http://127.0.0.1:${address.port}`;
        this.options.log("browser.cdp_started", { port: address.port });
        resolve({ endpoint: this.endpoint, token: this.controlToken });
      };
      this.server.once("error", fail);
      this.server.once("listening", ready);
      this.server.listen({ port: 0, host: "127.0.0.1", signal: this.listenAbort.signal });
    });
    return this.listening;
  }

  /** A lost/replaced daemon revokes every capability before another attach. */
  setAvailable(value: boolean): void {
    if (this.available === value) return;
    this.available = value;
    if (!value) this.revokeAll();
  }

  private revokeAll(): void {
    this.capabilities.clear();
    for (const client of [...this.clients]) this.end(client);
  }

  /** Synchronous revocation/detach precedes asynchronous server close. */
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.stopped = true;
    this.rejectStart?.(new Error("CDP gateway is closed"));
    this.rejectStart = null;
    this.listenAbort.abort();
    this.setAvailable(false);
    this.unsubscribe();
    for (const client of [...this.clients]) this.end(client);
    for (const socket of this.sockets) socket.destroy();
    this.ws.close();
    this.closing = new Promise((resolve) => this.server.close(() => resolve()));
    return this.closing;
  }

  private eligible(capability: Capability): readonly CdpPage[] {
    const scope = capability.scope;
    if (this.options.incarnation(scope) !== capability.incarnation) { this.revoke(capability); return []; }
    const comment = capability.purpose === "comment";
    // A local file page exists only for a comment capability; for any other it is absent, as it always was.
    const scoped = this.options.pages().filter((page) => page.workspace === scope.workspace && page.area_id === scope.area_id && (!scope.display_id || page.id === scope.display_id) && !page.contents.isDestroyed() && (comment || !page.local));
    const reachable = (page: CdpPage) => page.local ? fileAddress(page.contents.getURL()) : cdpAddress(page.contents.getURL());
    if (scoped.some((page) => capability.generations.has(page.id) && !reachable(page))) { this.revoke(capability); return []; }
    const pages = scoped.filter(reachable);
    for (const page of pages) {
      const generation = capability.generations.get(page.id);
      if (generation !== undefined && generation !== page.contents.id || generation === undefined && capability.generations.size >= MAX_SESSIONS) { this.revoke(capability); return []; }
      capability.generations.set(page.id, page.contents.id);
    }
    return pages;
  }
  private revoke(capability: Capability, ownClose?: number): void {
    this.capabilities.delete(capability.path);
    for (const client of [...this.clients]) if (client.capability === capability) {
      if (ownClose !== undefined && client.closingPages.has(ownClose)) this.drain(client);
      else this.end(client);
    }
  }
  private page(capability: Capability, id: string): CdpPage {
    const page = this.eligible(capability).find((page) => targetId(page) === id || `tab-${page.contents.id}` === id);
    if (!page) throw new ProtocolError("Target is outside this capability");
    return page;
  }
  private info(page: CdpPage, kind: "page" | "tab" = "page"): Json {
    return { targetId: `${kind}-${page.contents.id}`, type: kind, title: page.contents.getTitle(), url: page.contents.getURL(), attached: this.owners.has(page.contents.id), canAccessOpener: false, browserContextId: this.browserContextId };
  }
  private validHeaders(request: IncomingMessage): boolean {
    if (!this.endpoint || request.headers.host !== new URL(this.endpoint).host || request.headers["x-forwarded-host"] || request.headers["x-forwarded-for"]) return false;
    return request.headers.origin === undefined || request.headers.origin === this.endpoint;
  }
  private resolveCapability(path: string): { capability: Capability; suffix: string } | null {
    for (const capability of this.capabilities.values()) {
      if (path.startsWith(`${capability.path}/`)) return { capability, suffix: path.slice(capability.path.length) };
    }
    return null;
  }
  private reply(response: ServerResponse, status: number, value: unknown): void {
    response.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" });
    response.end(JSON.stringify(value));
  }
  private async body(request: IncomingMessage): Promise<unknown> {
    const chunks: Buffer[] = [];
    let bytes = 0;
    for await (const chunk of request) {
      bytes += Buffer.byteLength(chunk as Buffer);
      if (bytes > MAX_BODY_BYTES) throw new ProtocolError("Request body too large", 413);
      chunks.push(Buffer.from(chunk as Buffer));
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8")) as unknown;
  }
  private async http(request: IncomingMessage, response: ServerResponse): Promise<void> {
    if (!this.validHeaders(request)) { this.reply(response, 403, { error: "Invalid host or origin" }); return; }
    if (this.stopped || !this.available) { this.reply(response, 503, { error: "Desktop is not attached" }); return; }
    if (this.httpPending >= MAX_HTTP_REQUESTS) { this.reply(response, 429, { error: "Request limit exceeded" }); return; }
    this.httpPending++;
    try {
      const path = request.url ?? "";
      if (path === "/revoke" && request.method === "POST") {
        if (!sameToken(request.headers.authorization ?? "", `Bearer ${this.controlToken}`)) { this.reply(response, 401, { error: "Unauthorized" }); return; }
        // A node daemon whose link to its core ended: what the core authorized
        // through that link stops working, and the gateway stays available.
        this.revokeAll();
        this.reply(response, 200, {});
        return;
      }
      if (path === "/connect" && request.method === "POST") {
        if (!sameToken(request.headers.authorization ?? "", `Bearer ${this.controlToken}`)) { this.reply(response, 401, { error: "Unauthorized" }); return; }
        const body = await this.body(request);
        const scope = scopeOf(body);
        if (!scope) { this.reply(response, 400, { error: "Invalid scope" }); return; }
        const purpose = purposeOf(body, scope);
        if (purpose === null) { this.reply(response, 400, { error: "Invalid purpose" }); return; }
        const incarnation = this.options.incarnation(scope);
        if (incarnation === null || !Number.isSafeInteger(incarnation) || incarnation < 0) { this.reply(response, 409, { error: "Workspace area is no longer authorized" }); return; }
        if (this.stopped || !this.available) { this.reply(response, 503, { error: "Desktop is not attached" }); return; }
        let existing = [...this.capabilities.values()].find((item) => JSON.stringify(item.scope) === JSON.stringify(scope) && item.purpose === purpose);
        if (existing) { this.eligible(existing); if (!this.capabilities.has(existing.path)) existing = undefined; }
        if (!existing && this.capabilities.size >= MAX_CAPABILITIES) { this.reply(response, 429, { error: "Capability limit exceeded" }); return; }
        const capability: Capability = existing ?? { path: `/cdp/${secret()}`, scope, ...(purpose ? { purpose } : {}), incarnation, generations: new Map() };
        let pages = this.eligible(capability);
        // An agent's display whose page the host unloaded loads again before it is driven.
        if (scope.display_id && pages.length === 0 && !purpose && this.options.revive) {
          const revived = await this.options.revive(scope);
          if (this.stopped || !this.available) { this.reply(response, 503, { error: "Desktop is not attached" }); return; }
          if (revived === "failed") { this.reply(response, 409, { error: "Display page failed to load" }); return; }
          pages = this.eligible(capability);
        }
        if (scope.display_id && pages.length === 0) { this.reply(response, 409, { error: purpose ? "Display has no eligible browser target" : "Display has no eligible HTTP(S) browser target; native files are unsupported" }); return; }
        this.capabilities.set(capability.path, capability);
        this.reply(response, 200, { cdp_http_url: `${this.endpoint}${capability.path}`, browser_ws_url: this.browserUrl(capability), ...(scope.display_id ? { display_id: scope.display_id } : {}) });
        return;
      }
      const resolved = this.resolveCapability(path);
      if (!resolved || request.method !== "GET") { this.reply(response, 404, { error: "Not found" }); return; }
      const { capability, suffix } = resolved;
      this.eligible(capability);
      if (!this.capabilities.has(capability.path)) { this.reply(response, 404, { error: "Not found" }); return; }
      // A comment capability is dialed at its browser socket by hided; it has no discovery.
      if (capability.purpose === "comment") this.reply(response, 404, { error: "Not found" });
      else if (suffix === "/json/version" || suffix === "/json/version/") this.reply(response, 200, { Browser: `Chrome/${this.options.chromeVersion}`, "Protocol-Version": "1.3", webSocketDebuggerUrl: this.browserUrl(capability) });
      else if (["/json", "/json/", "/json/list", "/json/list/"].includes(suffix)) this.reply(response, 200, this.eligible(capability).map((page) => ({ ...this.info(page), id: targetId(page), webSocketDebuggerUrl: `${this.endpoint.replace("http:", "ws:")}${capability.path}/devtools/page/${targetId(page)}` })));
      else this.reply(response, 404, { error: "Not found" });
    } catch (error) {
      if (!response.headersSent) this.reply(response, error instanceof ProtocolError && error.code === 413 ? 413 : 400, { error: "Invalid request or body limit exceeded" });
    }
    finally { this.httpPending--; }
  }
  private browserUrl(capability: Capability): string { return `${this.endpoint.replace("http:", "ws:")}${capability.path}/devtools/browser`; }
  private upgrade(request: IncomingMessage, socket: Socket, head: Buffer): void {
    const resolved = this.resolveCapability(request.url ?? "");
    if (!this.validHeaders(request) || !this.available || this.stopped || !resolved) { socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n"); return; }
    if (this.clients.size >= MAX_CLIENTS) {
      this.options.log("browser.cdp_limit", { resource: "clients", clients: this.clients.size });
      socket.end("HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\n\r\n"); return;
    }
    const { capability, suffix } = resolved;
    this.eligible(capability);
    if (!this.capabilities.has(capability.path)) { socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n"); return; }
    let page: CdpPage | undefined;
    if (suffix !== "/devtools/browser") {
      const match = /^\/devtools\/page\/(page-\d+)$/.exec(suffix);
      try { if (match?.[1]) page = this.page(capability, match[1]); else throw new ProtocolError("Invalid target"); }
      catch { socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n"); return; }
    }
    // Node's HTTP idle timeout must not kill a healthy upgraded CDP socket.
    socket.setTimeout(0);
    this.ws.handleUpgrade(request, socket, head, (ws) => {
      const client: Client = { intents: new Map(), closingPages: new Set(), draining: false, socket: ws, capability, sessions: new Map(), browserSessions: new Set(), leases: new Map(), abort: new AbortController(), discover: false, autoAttach: false, autoKind: "page", autoFlattened: true, known: new Map(), commentWorlds: new Set(), pending: 0, minute: Date.now(), requests: 0 };
      this.clients.add(client);
      ws.on("close", () => this.end(client));
      ws.on("error", () => this.end(client));
      ws.on("message", (data, binary) => { void this.receive(client, data, binary); });
      if (page) {
        try { client.direct = this.attach(client, page, true); }
        catch { this.end(client); }
      }
      this.options.log("browser.cdp_connected", { clients: this.clients.size, sessions: this.owners.size });
    });
  }
  private send(client: Client, value: Json): void {
    if (client.socket.readyState !== WebSocket.OPEN) return;
    const data = JSON.stringify(value);
    if (Buffer.byteLength(data) > MAX_MESSAGE_BYTES || client.socket.bufferedAmount + Buffer.byteLength(data) > MAX_BUFFER_BYTES) { this.limit(client, "outbound_bytes"); return; }
    client.socket.send(data, (error) => { if (error) this.end(client); });
  }
  private event(client: Client, method: string, params: Json, session?: Session | string): void {
    if (typeof session === "string") this.send(client, { method, params, sessionId: session });
    else if (!session || session === client.direct) this.send(client, { method, params });
    else if (session.flattened) this.send(client, { method, params, sessionId: session.id });
    else this.send(client, { method: "Target.receivedMessageFromTarget", params: { sessionId: session.id, targetId: session.targetId, message: JSON.stringify({ method, params }) } });
  }
  private async receive(client: Client, data: RawData, binary: boolean): Promise<void> {
    if (client.abort.signal.aborted) return;
    if (Date.now() - client.minute >= 60_000) { client.minute = Date.now(); client.requests = 0; }
    if (binary) { this.end(client, 1003, "Text CDP messages are required"); return; }
    if (client.pending >= MAX_PENDING || ++client.requests > MAX_REQUESTS_PER_MINUTE) { this.limit(client, "requests"); return; }
    let command: Command;
    try { command = commandOf(JSON.parse(data.toString()) as unknown); }
    catch { this.send(client, { id: null, error: { code: -32600, message: "Invalid CDP request" } }); this.end(client); return; }
    client.pending++;
    const deadline = Date.now() + REQUEST_TIMEOUT_MS;
    try {
      const execute = (requestId?: string) => {
        const nested = this.nestedReceipt(client, command, command.sessionId ?? client.direct?.id ?? "browser", deadline);
        if (nested) return nested;
        const session = command.sessionId ? client.browserSessions.has(command.sessionId) ? undefined : this.getSession(client, command.sessionId) : client.direct;
        return this.dispatch(client, command, session, 0, requestId, deadline);
      };
      const result = await (MUTATIONS.has(command.method) ? this.intent(client, command, command.sessionId ?? client.direct?.id ?? "browser", execute, undefined, deadline) : execute());
      this.send(client, { id: command.id, result: result ?? {}, ...(command.sessionId ? { sessionId: command.sessionId } : {}) });
    } catch (error) {
      const failure = error instanceof ProtocolError ? error : new ProtocolError("CDP command failed");
      this.send(client, { id: command.id, error: { code: failure.code, message: failure.message }, ...(command.sessionId ? { sessionId: command.sessionId } : {}) });
      this.options.log("browser.cdp_request_failed", { method: command.method, code: failure.code, pending: client.pending });
    } finally {
      client.pending--;
      if (client.draining && client.pending === 0) this.end(client, 1000, "Scoped target closed");
    }
  }
  private intent(client: Client, command: Command, sessionKey: string, execute: (requestId: string) => Promise<unknown>, nested?: { parentKey: string; targetId: string; parent?: Session }, deadline = Date.now() + REQUEST_TIMEOUT_MS): Promise<unknown> {
    const key = `${sessionKey}:${command.id}`;
    const fingerprint = createHash("sha256").update(JSON.stringify({ method: command.method, params: command.params, scope: client.capability.scope })).digest("hex");
    const previous = client.intents.get(key);
    if (previous) {
      if (previous.fingerprint !== fingerprint) throw new ProtocolError("Request ID was reused for a different intent");
      if (this.now() - previous.issued > MAX_INTENT_AGE_MS) throw new ProtocolError("Intent expired; use a new CDP request ID");
      if (previous.state !== "retryable") return previous.result;
      if (previous.attempts >= 3) throw new ProtocolError("Action outcome remains unknown; inspect browser displays before issuing a new intent");
      if (client.draining || client.abort.signal.aborted || !this.available || this.options.incarnation(client.capability.scope) !== client.capability.incarnation || !this.capabilities.has(client.capability.path)) throw new ProtocolError("Intent no longer has workspace authority");
      return this.runIntent(previous, () => previous.action ? this.retryAction(client, previous, deadline) : execute(previous.requestId));
    }
    if (client.intents.size >= MAX_INTENTS) throw new ProtocolError("Intent limit exceeded; open a new CDP connection");
    const issued = this.now(), requestId = `${issued}-${secret().slice(0, 32)}`;
    const entry: Intent = { fingerprint, issued, requestId, method: command.method, attempts: 0, state: "pending", result: Promise.resolve(), nested };
    client.intents.set(key, entry);
    return this.runIntent(entry, () => execute(requestId));
  }
  private runIntent(entry: Intent, execute: () => Promise<unknown>): Promise<unknown> {
    entry.state = "pending";
    entry.attempts++;
    entry.result = Promise.resolve().then(execute).then((result) => {
      entry.state = "complete"; return result;
    }, (error: unknown) => {
      entry.state = error instanceof CdpActionUncertain ? "retryable" : "fatal"; throw error;
    });
    return entry.result;
  }
  private async retryAction(client: Client, intent: Intent, deadline: number): Promise<unknown> {
    const result = await this.perform(client, intent.action!, deadline);
    if (intent.action!.action === "open") {
      const page = await this.waitForPage(client, result.view_id, deadline);
      this.refreshClient(client);
      return { targetId: targetId(page) };
    }
    return intent.method === "Target.closeTarget" ? { success: true } : {};
  }
  /** Receipt replay has no native authority. Its original envelope binding
   * is checked before looking for the now-retired child session. */
  private nestedReceipt(client: Client, command: Command, parentKey: string, deadline: number): Promise<unknown> | undefined {
    if (command.method !== "Target.sendMessageToTarget") return;
    const sessionId = text(command.params, "sessionId");
    let nested: Command;
    try { nested = commandOf(JSON.parse(text(command.params, "message")) as unknown); } catch { return; }
    if (nested.sessionId || !MUTATIONS.has(nested.method)) return;
    const previous = client.intents.get(`${sessionId}:${nested.id}`);
    if (!previous?.nested || previous.nested.parentKey !== parentKey) return;
    return this.nestedResult(client, nested, sessionId, previous.nested.targetId, previous.nested.parent,
      () => this.intent(client, nested, sessionId, async () => { throw new ProtocolError("Receipt has no native authority"); }, undefined, deadline));
  }
  private async nestedResult(client: Client, command: Command, sessionId: string, target: string, parent: Session | undefined, execute: () => Promise<unknown>): Promise<Json> {
    let reply: Json;
    try { reply = { id: command.id, result: await execute() ?? {} }; }
    catch (error) {
      const failure = error instanceof ProtocolError ? error : new ProtocolError("CDP command failed");
      reply = { id: command.id, error: { code: failure.code, message: failure.message } };
    }
    this.event(client, "Target.receivedMessageFromTarget", { sessionId, targetId: target, message: JSON.stringify(reply) }, parent);
    return {};
  }
  private getSession(client: Client, id: string): Session {
    const session = client.sessions.get(id);
    if (!session) throw new ProtocolError("Unknown scoped session");
    if (session.browserParent && !client.browserSessions.has(session.browserParent)) throw new ProtocolError("Scoped browser parent was released");
    this.page(client.capability, targetId(session.lease.page));
    return session;
  }
  private sessionCount(client: Client): number { return client.sessions.size + client.browserSessions.size; }
  private attach(client: Client, page: CdpPage, flattened: boolean, kind: "page" | "tab" = "page"): Session {
    const existing = client.leases.get(page.contents.id);
    if (existing) {
      if (existing.root.kind === kind) return existing.root;
      if (kind === "page") return this.attachPageChild(existing.root);
      throw new ProtocolError("Reconnect to change the attachment profile");
    }
    if (this.sessionCount(client) >= MAX_SESSIONS) throw new ProtocolError("Session limit exceeded");
    if (this.owners.has(page.contents.id) || page.contents.debugger.isAttached()) throw new ProtocolError("Target already has a debugger");
    page.contents.debugger.attach("1.3");
    const root = { id: secret(), targetId: `${kind}-${page.contents.id}`, kind, flattened } as Session;
    const lease: Lease = { page, client, root, pageAttaches: [], ready: Promise.resolve(), mainFrameId: "", message: (_event, method, params, sessionId) => this.nativeEvent(lease, method, params, sessionId), detach: () => this.release(lease, false) };
    root.lease = lease;
    page.contents.debugger.on("message", lease.message);
    page.contents.debugger.on("detach", lease.detach);
    client.leases.set(page.contents.id, lease);
    client.sessions.set(root.id, root);
    this.owners.set(page.contents.id, lease);
    this.options.attached(page, true);
    // Chromium identifies a page's main frame by its native target ID. Our
    // public target is stable for the Hide display generation, so translate
    // that protocol identity after this exact debugger attests its frame.
    lease.ready = this.bounded(client, page.contents.debugger.sendCommand("Page.getFrameTree", {}), lease).then((answer) => {
      if (!object(answer) || !object(answer.frameTree) || !object(answer.frameTree.frame)
        || !identifier(answer.frameTree.frame.id) || answer.frameTree.frame.parentId !== undefined
        || this.owners.get(page.contents.id) !== lease) throw new ProtocolError("The native main frame could not be identified");
      lease.mainFrameId = answer.frameTree.frame.id;
    }).catch((error: unknown) => {
      this.options.log("browser.cdp_attach_failed", { display_id: page.id, reason: "main_frame" });
      this.release(lease);
      throw error;
    });
    // Auto-attach may have no pending caller when preparation fails. Release
    // still delivers the detached event; later commands receive a typed error.
    void lease.ready.catch(() => {});
    return root;
  }
  private attachPageChild(tab: Session): Session {
    const { client, page } = tab.lease;
    const existing = [...client.sessions.values()].find((session) => session.parent === tab && session.kind === "page");
    if (existing) return existing;
    if (this.sessionCount(client) >= MAX_SESSIONS) throw new ProtocolError("Session limit exceeded");
    const child: Session = { id: secret(), targetId: targetId(page), kind: "page", lease: tab.lease, parent: tab, flattened: tab.flattened };
    client.sessions.set(child.id, child);
    this.event(client, "Target.attachedToTarget", { sessionId: child.id, targetInfo: this.info(page), waitingForDebugger: false }, tab);
    return child;
  }
  /** A second standard CDP session attaches only to the exact native target
   * this WebContents debugger attests, never a target ID from the client. */
  private async attachAdditionalPage(lease: Lease, flattened: boolean, browserParent: string | undefined, deadline: number): Promise<Session> {
    const { client } = lease;
    if (this.sessionCount(client) + lease.pageAttaches.length >= MAX_SESSIONS) throw new ProtocolError("Session limit exceeded");
    if (browserParent && !client.browserSessions.has(browserParent)) throw new ProtocolError("Scoped browser parent was released");
    const answer: unknown = await this.native(lease.root, "Target.getTargetInfo", {}, deadline);
    if (!object(answer) || !object(answer.targetInfo) || answer.targetInfo.type !== "page" || typeof answer.targetInfo.targetId !== "string") throw new ProtocolError("The native page cannot create an additional session");
    if (browserParent && !client.browserSessions.has(browserParent)) throw new ProtocolError("Scoped browser parent was released");
    const intent = { targetId: answer.targetInfo.targetId };
    lease.pageAttaches.push(intent);
    let nativeId: string | undefined;
    try {
      const result: unknown = await this.native(lease.root, "Target.attachToTarget", { targetId: intent.targetId, flatten: true }, deadline);
      if (!object(result) || typeof result.sessionId !== "string") throw new ProtocolError("The native page could not create an additional session");
      nativeId = result.sessionId;
      const existing = [...client.sessions.values()].find((session) => session.lease === lease && session.nativeId === result.sessionId);
      if (existing) return existing;
      return this.additionalPageSession(lease, result.sessionId, flattened, browserParent);
    } catch (error) {
      if (nativeId && this.owners.get(lease.page.contents.id) === lease && ![...client.sessions.values()].some((session) => session.lease === lease && session.nativeId === nativeId)) {
        try { await this.native(lease.root, "Target.detachFromTarget", { sessionId: nativeId }, deadline); }
        catch { this.release(lease); }
      }
      throw error;
    } finally {
      const index = lease.pageAttaches.indexOf(intent);
      if (index >= 0) lease.pageAttaches.splice(index, 1);
      this.releaseUnusedLease(lease);
    }
  }
  private releaseUnusedLease(lease: Lease): void {
    if (lease.pageAttaches.length === 0 && ![...lease.client.sessions.values()].some((session) => session.lease === lease)) this.release(lease);
  }
  private additionalPageSession(lease: Lease, nativeId: string, flattened: boolean, browserParent?: string): Session {
    const { client, page } = lease;
    if (client.abort.signal.aborted || this.owners.get(page.contents.id) !== lease || this.sessionCount(client) >= MAX_SESSIONS
      || browserParent !== undefined && !client.browserSessions.has(browserParent)) throw new ProtocolError("Session limit exceeded or connection closed");
    const session: Session = { id: secret(), targetId: targetId(page), kind: "page", lease, nativeId, flattened, browserParent };
    client.sessions.set(session.id, session);
    this.event(client, "Target.attachedToTarget", { sessionId: session.id, targetInfo: this.info(page), waitingForDebugger: false }, browserParent);
    return session;
  }
  private release(lease: Lease, detach = true): void {
    const { page, client } = lease;
    if (this.owners.get(page.contents.id) !== lease) return;
    this.owners.delete(page.contents.id);
    client.leases.delete(page.contents.id);
    page.contents.debugger.removeListener("message", lease.message);
    page.contents.debugger.removeListener("detach", lease.detach);
    if (detach && page.contents.debugger.isAttached()) {
      try { page.contents.debugger.detach(); } catch { /* Destroyed contents already ended the native session. */ }
    }
    for (const session of [...client.sessions.values()]) {
      if (session.lease !== lease) continue;
      client.sessions.delete(session.id);
      this.event(client, "Target.detachedFromTarget", { sessionId: session.id, targetId: session.targetId }, session.parent ?? session.browserParent);
    }
    this.options.attached(page, false);
    if (client.direct?.lease === lease) {
      if (client.closingPages.has(page.contents.id)) this.drain(client);
      else if (!client.draining) this.end(client);
    }
  }
  private drain(client: Client): void {
    if (client.draining) return;
    client.draining = true;
    for (const lease of [...client.leases.values()]) this.release(lease);
  }
  private limit(client: Client, resource: string): void {
    this.options.log("browser.cdp_limit", { resource, clients: this.clients.size, pending: client.pending, sessions: client.sessions.size, buffered_bytes: client.socket.bufferedAmount });
    this.end(client, 1013, "CDP resource limit exceeded");
  }
  private end(client: Client, code?: number, reason?: string): void {
    if (client.abort.signal.aborted) return;
    client.abort.abort();
    this.clients.delete(client);
    for (const lease of [...client.leases.values()]) this.release(lease);
    client.browserSessions.clear();
    if (code && client.socket.readyState === WebSocket.OPEN || client.socket.readyState === WebSocket.CLOSING) {
      if (client.socket.readyState === WebSocket.OPEN) client.socket.close(code, reason);
      const timer = setTimeout(() => client.socket.terminate(), 1000);
      timer.unref();
      client.socket.once("close", () => clearTimeout(timer));
    } else client.socket.terminate();
    this.options.log("browser.cdp_disconnected", { clients: this.clients.size, sessions: this.owners.size, pending: client.pending });
  }
  private async bounded<T>(client: Client, operation: Promise<T>, lease?: Lease, ambiguous = false, deadline = Date.now() + REQUEST_TIMEOUT_MS): Promise<T> {
    let timer: NodeJS.Timeout | undefined;
    let abort: () => void = () => {};
    try {
      return await Promise.race([operation, new Promise<never>((_resolve, reject) => {
        abort = () => reject(new ProtocolError("CDP connection closed"));
        client.abort.signal.addEventListener("abort", abort, { once: true });
        timer = setTimeout(() => {
          if (lease) this.release(lease);
          reject(ambiguous ? new CdpActionUncertain() : new ProtocolError("CDP request timed out"));
        }, Math.max(0, deadline - Date.now()));
        if (client.abort.signal.aborted) abort();
      })]);
    } finally { clearTimeout(timer); client.abort.signal.removeEventListener("abort", abort); }
  }
  /** Rewrite protocol frame references only, never evaluated page values. */
  private frameFields(lease: Lease, value: Json, outgoing: boolean): Json {
    if (!lease.mainFrameId) return value;
    const from = outgoing ? lease.mainFrameId : targetId(lease.page);
    const to = outgoing ? targetId(lease.page) : lease.mainFrameId;
    const fields = { ...value };
    for (const key of ["frameId", "parentFrameId"]) if (fields[key] === from) fields[key] = to;
    return fields;
  }
  private nativePayload(lease: Lease, method: string, value: Json): Json {
    const result = this.frameFields(lease, value, true);
    const frame = (value: Json) => {
      const from = lease.mainFrameId, to = targetId(lease.page);
      return { ...value, ...(value.id === from ? { id: to } : {}), ...(value.parentId === from ? { parentId: to } : {}) };
    };
    if (method === "Page.getFrameTree" && object(value.frameTree) && object(value.frameTree.frame)) {
      const tree = value.frameTree;
      result.frameTree = { ...tree, frame: frame(tree.frame as Json),
        ...(Array.isArray(tree.childFrames) ? { childFrames: tree.childFrames.map((child: unknown) => object(child) && object(child.frame) ? { ...child, frame: frame(child.frame) } : child) } : {}) };
    }
    if (method === "Page.frameNavigated" && object(value.frame)) result.frame = frame(value.frame);
    if (method === "Runtime.executionContextCreated" && object(value.context) && object(value.context.auxData)) {
      result.context = { ...value.context, auxData: this.frameFields(lease, value.context.auxData, true) };
    }
    return result;
  }
  private async native(session: Session, method: string, params: Json, deadline = Date.now() + REQUEST_TIMEOUT_MS): Promise<unknown> {
    const { client, page } = session.lease;
    return this.bounded(client, (async () => {
      await session.lease.ready;
      this.page(client.capability, targetId(page));
      if (this.owners.get(page.contents.id) !== session.lease) throw new ProtocolError("Debugger session was released");
      if (session.browserParent && !client.browserSessions.has(session.browserParent)) throw new ProtocolError("Scoped browser parent was released");
      if (Date.now() >= deadline) { this.release(session.lease); throw new ProtocolError("CDP request timed out"); }
      const result = await page.contents.debugger.sendCommand(method, this.frameFields(session.lease, params, false), session.nativeId);
      return object(result) ? this.nativePayload(session.lease, method, result) : result;
    })(), session.lease, false, deadline);
  }
  private async dispatch(client: Client, command: Command, session: Session | undefined, depth: number, intentId: string | undefined, deadline: number): Promise<unknown> {
    if (client.abort.signal.aborted || client.draining || !this.available) throw new ProtocolError("CDP connection closed");
    if (depth > 4) throw new ProtocolError("Nested CDP request limit exceeded");
    if (Date.now() >= deadline) throw new ProtocolError("CDP request timed out");
    const { method, params } = command;
    if (session) this.getSession(client, session.id);
    if (client.capability.purpose === "comment") this.commentGate(client, method, params);
    if (command.sessionId && client.browserSessions.has(command.sessionId) && !SCOPED_BROWSER_METHODS.has(method)) throw new ProtocolError("Command is outside the scoped browser session boundary");
    const requestId = intentId ?? `${Date.now()}-${secret().slice(0, 32)}`;
    if (method.startsWith("Target.")) return this.target(client, method, params, session, depth, requestId,
      command.sessionId && client.browserSessions.has(command.sessionId) ? command.sessionId : undefined, deadline);
    if (method === "Browser.getVersion") return { protocolVersion: "1.3", product: `Chrome/${this.options.chromeVersion}`, revision: "", userAgent: `Chrome/${this.options.chromeVersion}`, jsVersion: "" };
    if (method.startsWith("Browser.")) throw new ProtocolError("Browser-wide command is not available in a scoped connection");
    if (!session) throw new ProtocolError("A scoped page session is required");
    if (session.kind === "tab") {
      if (method === "Runtime.runIfWaitingForDebugger") return {};
      throw new ProtocolError("A scoped page session is required");
    }
    if (method === "Page.close") { await this.perform(client, { action: "close", display_id: session.lease.page.id, request_id: requestId }, deadline); return {}; }
    if (method === "Page.bringToFront") { await this.perform(client, { action: "select", display_id: session.lease.page.id, request_id: requestId }, deadline); return {}; }
    const domain = method.split(".")[0] ?? "";
    if (!PAGE_DOMAINS.has(domain) || DENIED_PAGE_METHODS.has(method)) throw new ProtocolError("Command is outside the scoped page boundary");
    // A drag that carries files drops them on the page as an upload does.
    if (method === "Input.dispatchDragEvent" && object(params.data) && params.data.files !== undefined
      && !(Array.isArray(params.data.files) && params.data.files.length === 0)) throw new ProtocolError("Dragging files is not available through CDP");
    if (method === "Page.navigate") {
      if (!cdpAddress(text(params, "url"))) throw new ProtocolError("Native files and non-HTTP(S) navigation are not supported through CDP");
    }
    if (["Fetch.continueRequest", "Page.getResourceContent"].includes(method)
      && typeof params.url === "string" && !cdpAddress(params.url)) throw new ProtocolError("Native file resources are not supported through CDP");
    if (method === "Network.getCookies") {
      const origin = new URL(session.lease.page.contents.getURL()).origin;
      if (origin === "null" || params.urls !== undefined && (!Array.isArray(params.urls) || params.urls.some((url) => typeof url !== "string" || new URL(url).origin !== origin))) throw new ProtocolError("Cookies must belong to the scoped page origin");
      return this.native(session, method, { ...params, urls: params.urls ?? [session.lease.page.contents.getURL()] }, deadline);
    }
    if (method === "Page.printToPDF" && params.transferMode === "ReturnAsStream") throw new ProtocolError("Filesystem streams are not available");
    const answer = await this.native(session, method, params, deadline);
    // The worlds a comment made are the only ones it may evaluate in.
    if (client.capability.purpose === "comment" && method === "Page.createIsolatedWorld" && object(answer) && typeof answer.executionContextId === "number") {
      if (client.commentWorlds.size >= MAX_COMMENT_WORLDS) client.commentWorlds.delete(client.commentWorlds.values().next().value as number);
      client.commentWorlds.add(answer.executionContextId);
    }
    return answer;
  }
  /** What a comment capability may send; the evaluation must name a world, never the page's own. */
  private commentGate(client: Client, method: string, params: Json): void {
    const allowed = COMMENT_PARAMS[method];
    if (!COMMENT_METHODS.has(method) || !allowed) throw new ProtocolError("Command is outside the comment boundary");
    if (Object.keys(params).some((key) => !allowed.includes(key))) throw new ProtocolError("Parameter is outside the comment boundary");
    if (method === "Runtime.addBinding" && typeof params.executionContextName !== "string") throw new ProtocolError("A comment binding belongs to its own world");
    if (method === "Runtime.evaluate" && !(typeof params.contextId === "number" && client.commentWorlds.has(params.contextId))) throw new ProtocolError("A comment evaluates only in a world it made");
  }
  private async perform(client: Client, action: CdpAction, deadline: number): Promise<CdpActionResult> {
    if (Date.now() >= deadline) throw new CdpActionUncertain();
    this.eligible(client.capability);
    if (client.draining || !this.capabilities.has(client.capability.path)) throw new ProtocolError("Intent no longer has workspace authority");
    const intent = [...client.intents.values()].find((entry) => entry.requestId === action.request_id);
    if (intent) intent.action ??= action;
    const closing = action.action === "close" ? this.eligible(client.capability).find((page) => page.id === action.display_id)?.contents.id : undefined;
    if (closing !== undefined) client.closingPages.add(closing);
    try {
      const result = await this.bounded(client, this.options.action(client.capability.scope, action, client.abort.signal), undefined, true, deadline);
      if ((result.area_id !== client.capability.scope.area_id && !(action.action === "close" && result.area_id === null)) || !identifier(result.view_id)
        || action.display_id !== undefined && result.view_id !== action.display_id || action.action === "open" && result.load === null) throw new ProtocolError("Workspace action returned an invalid target");
      // A committed core close can precede the host's native retirement. End
      // its authority now, but let receive deliver the successful receipt
      // before the normal close handshake; no timing grace is involved.
      if (closing !== undefined) {
        if (client.capability.scope.display_id) this.revoke(client.capability, closing);
        else if (client.direct?.lease.page.contents.id === closing) this.drain(client);
      }
      return result;
    } finally { if (closing !== undefined) client.closingPages.delete(closing); }
  }
  private async target(client: Client, method: string, params: Json, parent: Session | undefined, depth: number, requestId: string, browserParent: string | undefined, deadline: number): Promise<unknown> {
    if (method === "Target.getTargets") return { targetInfos: this.eligible(client.capability).flatMap((page) => [this.info(page), this.info(page, "tab")]) };
    if (method === "Target.getTargetInfo") {
      if (params.targetId === undefined) return { targetInfo: parent
        ? { targetId: parent.targetId, type: parent.kind, url: parent.lease.page.contents.getURL(), title: parent.lease.page.contents.getTitle(), attached: true, canAccessOpener: false, browserContextId: this.browserContextId }
        : { targetId: "scoped-browser", type: "browser", title: "", url: "", attached: true, canAccessOpener: false } };
      const id = text(params, "targetId");
      return { targetInfo: this.info(this.page(client.capability, id), id.startsWith("tab-") ? "tab" : "page") };
    }
    if (method === "Target.getBrowserContexts") return { browserContextIds: [] };
    if (["Target.createBrowserContext", "Target.disposeBrowserContext", "Target.exposeDevToolsProtocol", "Target.openDevTools"].includes(method)) throw new ProtocolError("Additional browser contexts are not supported");
    if (method === "Target.attachToBrowserTarget") {
      if (parent || browserParent) throw new ProtocolError("A scoped browser session requires the root connection");
      this.eligible(client.capability);
      if (!this.capabilities.has(client.capability.path)) throw new ProtocolError("Intent no longer has workspace authority");
      if (this.sessionCount(client) >= MAX_SESSIONS) throw new ProtocolError("Session limit exceeded");
      const id = secret();
      client.browserSessions.add(id);
      return { sessionId: id };
    }
    if (method === "Target.setDiscoverTargets") {
      if (params.discover === true && !client.discover) client.known.clear();
      client.discover = params.discover === true;
      this.refreshClient(client);
      return {};
    }
    if (method === "Target.setAutoAttach") {
      if (parent) {
        if (parent.kind === "tab") {
          if (params.autoAttach === true && this.filterAllows(params.filter, "page")) this.attachPageChild(parent);
          return {};
        }
        // Chromium may discover unrelated targets. Only iframe descendants
        // of this exact debugger session are admitted by nativeEvent.
        return this.native(parent, method, { ...params, flatten: true, filter: [{ type: "iframe", exclude: false }, { exclude: true }] }, deadline);
      }
      const pages = this.filterAllows(params.filter, "page"), tabs = this.filterAllows(params.filter, "tab");
      client.autoKind = pages ? "page" : "tab";
      client.autoFlattened = params.flatten === true;
      client.autoAttach = params.autoAttach === true && (pages || tabs);
      this.refreshClient(client);
      return {};
    }
    if (method === "Target.attachToTarget") {
      if (parent) throw new ProtocolError("Use auto-attach for scoped iframe descendants");
      const id = text(params, "targetId");
      const page = this.page(client.capability, id);
      const existed = client.leases.has(page.contents.id);
      const kind = id.startsWith("tab-") ? "tab" : "page";
      if (browserParent && kind !== "page") throw new ProtocolError("Scoped browser sessions attach only page targets");
      let lease = client.leases.get(page.contents.id);
      if (browserParent && !lease) {
        // The physical debugger belongs to the connection. Every virtual
        // parent's public page gets an independent native child session.
        const root = this.attach(client, page, true);
        client.sessions.delete(root.id);
        lease = root.lease;
      }
      let session: Session;
      try { session = lease && kind === "page" ? await this.attachAdditionalPage(lease, params.flatten === true, browserParent, deadline) : this.attach(client, page, params.flatten === true, kind); }
      finally { if (lease) this.releaseUnusedLease(lease); }
      if (!existed && !browserParent) this.event(client, "Target.attachedToTarget", { sessionId: session.id, targetInfo: this.info(page, kind), waitingForDebugger: false });
      return { sessionId: session.id };
    }
    if (method === "Target.detachFromTarget") {
      const id = text(params, "sessionId");
      if (client.browserSessions.has(id)) {
        if (parent || browserParent && browserParent !== id) throw new ProtocolError("Session is not a child of this target");
        client.browserSessions.delete(id);
        const children = [...client.sessions.values()].filter((session) => session.browserParent === id);
        try {
          for (const session of children) if (client.sessions.has(session.id)) await this.detachSession(session, deadline);
        } finally {
          // A failed/expired native detach must not leave a revoked parent's
          // sessions live. Release each still-attested affected debugger.
          for (const session of children) if (client.sessions.has(session.id)) this.release(session.lease);
        }
        this.event(client, "Target.detachedFromTarget", { sessionId: id, targetId: "scoped-browser" });
        return {};
      }
      const session = this.getSession(client, id);
      if (parent && session.parent !== parent) throw new ProtocolError("Session is not a child of this target");
      if (browserParent && session.browserParent !== browserParent) throw new ProtocolError("Session is not a child of this target");
      await this.detachSession(session, deadline);
      return {};
    }
    if (method === "Target.sendMessageToTarget") {
      const receipt = this.nestedReceipt(client, { id: 0, method, params }, parent?.id ?? client.direct?.id ?? "browser", deadline);
      if (receipt) return receipt;
      const session = this.getSession(client, text(params, "sessionId"));
      if (parent && session.parent !== parent && session !== parent) throw new ProtocolError("Session is not a child of this target");
      if (browserParent) throw new ProtocolError("Scoped browser sessions require flattened child commands");
      let nested: Command;
      try { nested = commandOf(JSON.parse(text(params, "message")) as unknown); } catch { throw new ProtocolError("Invalid nested CDP request", -32602); }
      if (nested.sessionId) throw new ProtocolError("Nested session override is forbidden");
      if (nested.method === "Target.sendMessageToTarget") throw new ProtocolError("Multilevel legacy CDP envelopes are not supported; use flattened sessions");
      const execute = (requestId?: string) => this.dispatch(client, nested, session, depth + 1, requestId, deadline);
      return this.nestedResult(client, nested, session.id, session.targetId, parent, () => MUTATIONS.has(nested.method)
        ? this.intent(client, nested, session.id, execute, { parentKey: parent?.id ?? client.direct?.id ?? "browser", targetId: session.targetId, parent }, deadline) : execute());
    }
    if (method === "Target.createTarget") {
      if (client.capability.scope.display_id) throw new ProtocolError("A display capability cannot create another display");
      if (params.browserContextId !== undefined || params.newWindow === true || params.forTab === true || params.hidden === true || params.background === true || params.enableBeginFrameControl === true || ["left", "top", "width", "height", "windowState"].some((key) => params[key] !== undefined)) throw new ProtocolError("Additional browser contexts or windows are not supported");
      const url = text(params, "url");
      if (!cdpAddress(url)) throw new ProtocolError("Native files and non-HTTP(S) targets are not supported through CDP");
      const result = await this.perform(client, { action: "open", url, request_id: requestId }, deadline);
      const page = await this.waitForPage(client, result.view_id, deadline);
      this.refreshClient(client);
      return { targetId: targetId(page) };
    }
    if (method === "Target.closeTarget" || method === "Target.activateTarget") {
      const page = this.page(client.capability, text(params, "targetId"));
      await this.perform(client, { action: method === "Target.closeTarget" ? "close" : "select", display_id: page.id, request_id: requestId }, deadline);
      return method === "Target.closeTarget" ? { success: true } : {};
    }
    throw new ProtocolError("Target command is not available in a scoped connection", -32601);
  }
  private async detachSession(session: Session, deadline: number): Promise<void> {
    if (session.nativeId) {
      await this.native(session.parent ?? session.lease.root, "Target.detachFromTarget", { sessionId: session.nativeId }, deadline);
      // Native detach events can arrive after the command reply. Retire the
      // admitted subtree exactly once before granting another command.
      this.retireSession(session);
      this.releaseUnusedLease(session.lease);
    } else this.release(session.lease);
  }
  private retireSession(child: Session): void {
    const { client } = child.lease;
    for (const session of [...client.sessions.values()]) {
      for (let ancestor: Session | undefined = session; ancestor; ancestor = ancestor.parent) {
        if (ancestor === child) {
          client.sessions.delete(session.id);
          this.event(client, "Target.detachedFromTarget", { sessionId: session.id, targetId: session.targetId }, session.parent ?? session.browserParent);
          break;
        }
      }
    }
  }
  private filterAllows(value: unknown, type: string): boolean {
    if (value === undefined) return type !== "tab";
    if (!Array.isArray(value) || value.length > 16 || value.some((entry) => !object(entry))) throw new ProtocolError("Invalid target filter", -32602);
    const match = (value as Json[]).find((entry) => entry.type === undefined || entry.type === type);
    return match !== undefined && match.exclude !== true;
  }
  private waitForPage(client: Client, displayId: string, deadline: number): Promise<CdpPage> {
    return new Promise((resolve, reject) => {
      let timer: NodeJS.Timeout;
      const finish = (page?: CdpPage, uncertain = false) => {
        clearTimeout(timer);
        client.abort.signal.removeEventListener("abort", abort);
        if (page) resolve(page); else reject(uncertain ? new CdpActionUncertain() : new ProtocolError("The workspace did not show the new browser target"));
      };
      const abort = () => finish();
      const poll = () => {
        if (client.abort.signal.aborted) { finish(); return; }
        const page = this.eligible(client.capability).find((page) => page.id === displayId);
        if (client.abort.signal.aborted) { finish(); return; }
        if (page) finish(page);
        else if (Date.now() >= deadline) finish(undefined, true);
        else timer = setTimeout(poll, Math.min(100, Math.max(0, deadline - Date.now())));
      };
      client.abort.signal.addEventListener("abort", abort, { once: true });
      poll();
    });
  }
  private refresh(retirement?: CdpRetirement): void {
    for (const capability of this.capabilities.values()) {
      if (this.options.incarnation(capability.scope) !== capability.incarnation) { this.revoke(capability); continue; }
      if (retirement) {
        const retired = [...capability.generations].find(([, contents]) => contents === retirement.contentsId);
        if (retired) {
          if (retirement.reason !== "closed" || capability.scope.display_id) { this.revoke(capability, retirement.reason === "closed" ? retirement.contentsId : undefined); continue; }
          capability.generations.delete(retired[0]);
        }
      }
      const count = this.eligible(capability).length;
      if (capability.scope.display_id && capability.generations.size > 0 && count === 0) this.revoke(capability);
    }
    for (const client of this.clients) this.refreshClient(client);
  }
  private refreshClient(client: Client): void {
    if (client.abort.signal.aborted || client.draining) return;
    const pages = this.eligible(client.capability);
    const ids = new Set(pages.flatMap((page) => [targetId(page), `tab-${page.contents.id}`]));
    for (const lease of [...client.leases.values()]) if (!ids.has(targetId(lease.page))) this.release(lease);
    for (const id of client.known.keys()) if (!ids.has(id)) { client.known.delete(id); if (client.discover) this.event(client, "Target.targetDestroyed", { targetId: id }); }
    for (const page of pages) {
      for (const kind of ["page", "tab"] as const) {
        const info = this.info(page, kind), id = `${kind}-${page.contents.id}`, previous = client.known.get(id);
        const encoded = JSON.stringify(info);
        client.known.set(id, encoded);
        if (client.discover && previous !== encoded) this.event(client, previous ? "Target.targetInfoChanged" : "Target.targetCreated", { targetInfo: info });
      }
      if (client.autoAttach && !client.leases.has(page.contents.id) && !this.owners.has(page.contents.id)) {
        try {
          const session = this.attach(client, page, client.autoFlattened, client.autoKind);
          this.event(client, "Target.attachedToTarget", { sessionId: session.id, targetInfo: this.info(page, client.autoKind), waitingForDebugger: false });
        } catch { this.options.log("browser.cdp_attach_failed", { display_id: page.id, reason: "debugger_busy" }); }
      }
    }
  }
  private nativeEvent(lease: Lease, method: string, params: Json, nativeId: string): void {
    const { client, page } = lease;
    try { this.page(client.capability, targetId(page)); } catch { this.release(lease); return; }
    const parent = nativeId ? [...client.sessions.values()].find((session) => session.lease === lease && session.nativeId === nativeId)
      : lease.root.kind === "tab" ? [...client.sessions.values()].find((session) => session.lease === lease && session.kind === "page" && !session.nativeId) : lease.root;
    if (!parent) return;
    if (method === "Target.attachedToTarget") {
      const info = object(params.targetInfo) ? params.targetInfo : {};
      if (typeof params.sessionId !== "string") return;
      if (parent.browserParent && !client.browserSessions.has(parent.browserParent)) {
        // This exact debugger attested the late native attachment. It grants
        // no public authority after its virtual parent has been revoked.
        void this.native(lease.root, "Target.detachFromTarget", { sessionId: params.sessionId }).catch(() => this.release(lease));
        return;
      }
      if (typeof info.url === "string" && info.url.toLowerCase().startsWith("file:") && client.capability.purpose !== "comment") { this.revoke(client.capability); return; }
      if (info.type === "page") {
        if ([...client.sessions.values()].some((session) => session.lease === lease && session.nativeId === params.sessionId)) return;
        const index = lease.pageAttaches.findIndex((intent) => intent.targetId === info.targetId);
        // The awaiting native attach result owns admission and cleanup. It
        // rechecks the virtual parent before publishing this exact session.
        if (index >= 0) return;
      }
      let depth = 0;
      for (let ancestor: Session | undefined = parent; ancestor; ancestor = ancestor.parent) depth++;
      if (info.type !== "iframe" || client.capability.purpose === "comment" || this.sessionCount(client) >= MAX_SESSIONS || depth >= 8) {
        void this.native(parent, "Target.detachFromTarget", { sessionId: params.sessionId }).catch(() => this.release(lease));
        return;
      }
      if (!identifier(info.targetId)) { this.release(lease); return; }
      const child: Session = { id: secret(), targetId: info.targetId, kind: "iframe", lease, parent, browserParent: parent.browserParent, nativeId: params.sessionId, flattened: parent.flattened };
      client.sessions.set(child.id, child);
      this.event(client, method, { sessionId: child.id, targetInfo: this.frameFields(lease, { targetId: child.targetId, type: "iframe", title: typeof info.title === "string" ? info.title : "", url: typeof info.url === "string" ? info.url : "", attached: true, canAccessOpener: false, browserContextId: this.browserContextId, ...(identifier(info.parentFrameId) ? { parentFrameId: info.parentFrameId } : {}) }, true), waitingForDebugger: params.waitingForDebugger === true }, parent);
    } else if (method === "Target.detachedFromTarget") {
      const child = [...client.sessions.values()].find((session) => session.lease === lease && session.nativeId === params.sessionId);
      if (child) this.retireSession(child);
    } else if (client.sessions.has(parent.id) && (!parent.browserParent || client.browserSessions.has(parent.browserParent))
      && !method.startsWith("Target.") && !method.startsWith("Browser.")
      && (client.capability.purpose !== "comment" || COMMENT_EVENTS.has(method))) this.event(client, method, this.nativePayload(lease, method, params), parent);
  }
}
