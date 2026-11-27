// Where the browser pages of another computer's screen send their requests,
// and the login storage they keep here (docs/BROWSER_DISPLAYS.md, A page in
// another computer's screen).
//
// Every request of such a page leaves through that computer: its session's
// proxy is a listener this computer's daemon keeps on its own loopback for
// the machine and its key (`hided/src/screen_net.rs`), whose connections
// each ride one stream of the connection between the two machines to that
// computer's proxy. The proxy is set before the first page of the session is
// made, with no `direct` fallback and loopback not bypassed, so this
// computer's own loopback, LAN and resolver are never asked. The listener
// needs a secret, answered here to Chromium's proxy login for its port.
//
// The pages of one machine under one key share one persistent session,
// kept across restarts and cleared once that pair ends: the index file names
// each partition with its state, written before the step it names, so a
// clear a quit cut short runs again before anything else uses it.

import { session as electronSession, type Session } from "electron";
import path from "node:path";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import { remotePagePartition } from "./browserSync";
import type { HostLog } from "./log";

/** This computer's daemon as the window is attached to it. */
export type EgressDaemon = { origin: string; token: string };

/**
 * Where requests of a machine's pages stand: `ready`, or the reason they
 * fail now, in the daemon's words (`not_connected`, `paused`,
 * `paused_there`, `no_grant`, `unsupported_there`, `relay_budget`,
 * `net_limit`), or `remote_update_here` when this computer's daemon is
 * older than its app.
 */
export type EgressState = string;

/** The failure words a page shows for each state other than `ready`, keys of `native.browser.*`. */
export function egressWords(state: EgressState): string {
  switch (state) {
    case "paused":
    case "paused_there":
    case "no_grant":
      return "remotePaused";
    case "unsupported_there":
      return "remoteUpdateThere";
    case "remote_update_here":
      return "remoteUpdateHere";
    case "relay_budget":
      return "relayBudget";
    case "net_limit":
      return "remoteNetLimit";
    default:
      return "remoteNetUnavailable";
  }
}

type Listener = { port: number; secret: string };
type Pair = { device: string; key: string; listener: Listener | null; asking: Promise<EgressState> | null };

/** The machine and key a pair is named by, as the paired set names it. */
export function pairName(device: string, key: string): string {
  return `${device}\u0000${key}`;
}

export type EgressDeps = {
  daemon: () => EgressDaemon | null;
  log: HostLog;
  /** The session of a partition; Electron's unless a test hands its own. */
  session?: (partition: string) => Pick<Session, "setProxy" | "closeAllConnections">;
  /** The owner the daemon keeps a listener for: this process. */
  owner?: number;
  fetch?: typeof fetch;
};

export class RemoteEgress {
  private readonly pairs = new Map<string, Pair>();

  constructor(private readonly deps: EgressDeps) {}

  private sessionOf(partition: string): Pick<Session, "setProxy" | "closeAllConnections"> {
    return this.deps.session ? this.deps.session(partition) : electronSession.fromPartition(partition);
  }

  /** Whether the pair's pages may make requests: its proxy is set. */
  ready(device: string, key: string): boolean {
    return this.pairs.get(pairName(device, key))?.listener != null;
  }

  /**
   * Asks the daemon for the pair's listener and where its requests stand.
   * The first answer, and one naming another port or secret, sets the
   * session's proxy and closes its connections before the pair counts as
   * ready; one ask at a time per pair.
   */
  ensure(device: string, key: string): Promise<EgressState> {
    const name = pairName(device, key);
    let pair = this.pairs.get(name);
    if (!pair) {
      pair = { device, key, listener: null, asking: null };
      this.pairs.set(name, pair);
    }
    const asked = pair;
    asked.asking ??= this.ask(asked).finally(() => {
      asked.asking = null;
    });
    return asked.asking;
  }

  private async ask(pair: Pair): Promise<EgressState> {
    const daemon = this.deps.daemon();
    if (!daemon) return "not_connected";
    const owner = this.deps.owner ?? process.pid;
    const address = `${daemon.origin}/peer-screen/${encodeURIComponent(pair.device)}/net?key=${encodeURIComponent(pair.key)}&owner=${owner}`;
    let answer: Response;
    try {
      answer = await (this.deps.fetch ?? fetch)(address, { method: "POST", headers: { authorization: `Bearer ${daemon.token}` }, signal: AbortSignal.timeout(10_000) });
    } catch {
      return "not_connected";
    }
    if (answer.status === 404) return "remote_update_here";
    const body: unknown = await answer.json().catch(() => null);
    const fields = typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
    if (!answer.ok) return typeof fields.reason === "string" && fields.reason.length <= 64 ? fields.reason : "not_connected";
    const { port, secret, state } = fields;
    if (typeof port !== "number" || !Number.isInteger(port) || port < 1 || port > 65_535 || typeof secret !== "string" || !/^[0-9a-f]{32}$/.test(secret) || typeof state !== "string") {
      this.deps.log.event("remote.egress_invalid", {});
      return "not_connected";
    }
    if (this.pairs.get(pairName(pair.device, pair.key)) !== pair) return "not_connected";
    if (pair.listener?.port !== port || pair.listener.secret !== secret) {
      pair.listener = null;
      const pages = this.sessionOf(remotePagePartition(pair.device, pair.key));
      try {
        await pages.setProxy({ mode: "fixed_servers", proxyRules: `http://127.0.0.1:${port}`, proxyBypassRules: "<-loopback>" });
        await pages.closeAllConnections();
      } catch (error) {
        this.deps.log.event("remote.egress_failed", { detail: String(error) });
        return "not_connected";
      }
      if (this.pairs.get(pairName(pair.device, pair.key)) !== pair) return "not_connected";
      pair.listener = { port, secret };
      this.deps.log.event("remote.egress_ready", { device: pair.device });
    }
    return state;
  }

  /** Chromium's proxy login for one of the pairs' listeners: `hide` and its secret; null for any other proxy. */
  credentials(host: string, port: number): { username: string; password: string } | null {
    if (host !== "127.0.0.1") return null;
    for (const pair of this.pairs.values()) {
      if (pair.listener?.port === port) return { username: "hide", password: pair.listener.secret };
    }
    return null;
  }

  /** The pair's pages are gone: its listener is let go, and its pages may make no request until asked again. */
  release(device: string, key: string): void {
    const name = pairName(device, key);
    const pair = this.pairs.get(name);
    if (!pair) return;
    this.pairs.delete(name);
    const daemon = this.deps.daemon();
    if (!daemon || !pair.listener) return;
    const owner = this.deps.owner ?? process.pid;
    void (this.deps.fetch ?? fetch)(`${daemon.origin}/peer-screen/${encodeURIComponent(device)}/net?key=${encodeURIComponent(key)}&owner=${owner}`, {
      method: "DELETE", headers: { authorization: `Bearer ${daemon.token}` }, signal: AbortSignal.timeout(3_000),
    }).catch(() => this.deps.log.event("remote.egress_release_failed", {}));
  }

  /** This computer's daemon is another one now: every listener it held is gone with it. */
  forgetAll(): void {
    this.pairs.clear();
  }
}

/** The partitions the index may hold; a new pair past it keeps no storage. */
export const MAX_STORED = 32;

type StoredState = "live" | "clearing" | "failed";
type Stored = { device: string; key: string; state: StoredState };

/** What a session's clear needs. */
export type Clearable = Pick<Session, "closeAllConnections" | "clearAuthCache" | "clearStorageData" | "clearCache">;

export type StorageDeps = {
  /** The index file. */
  file: string;
  log: HostLog;
  session?: (partition: string) => Clearable;
  /** Closes every page of the pair before its storage is cleared. */
  closePages: (device: string, key: string) => void;
  /** The pair's storage was cleared: a screen of it may make its pages again. */
  cleared?: (device: string, key: string) => void;
};

/**
 * The login storage of the machines this window's operator paired with
 * (one partition per machine and key): which exist, and the clear of each
 * once its pair ends, in order: connections closed, the HTTP auth cache,
 * storage, then the cache. An entry goes only after all of them; one that
 * failed stays `failed` and no page uses it until a later clear empties it.
 */
export class RemoteStorage {
  private readonly entries = new Map<string, Stored>();
  private readonly clearing = new Map<string, Promise<void>>();

  constructor(private readonly deps: StorageDeps) {
    const read = readJsonFile(deps.file);
    const rows = typeof read === "object" && read !== null && Array.isArray((read as { entries?: unknown }).entries) ? (read as { entries: unknown[] }).entries : [];
    for (const row of rows.slice(0, MAX_STORED)) {
      if (typeof row !== "object" || row === null) continue;
      const { device, key, state } = row as Record<string, unknown>;
      if (typeof device !== "string" || typeof key !== "string" || (state !== "live" && state !== "clearing" && state !== "failed")) continue;
      this.entries.set(remotePagePartition(device, key), { device, key, state });
    }
    // A clear a quit cut short, or one that failed, runs again before any page.
    for (const [partition, entry] of this.entries) if (entry.state !== "live") void this.clear(partition);
  }

  static at(folder: string, log: HostLog, closePages: StorageDeps["closePages"], cleared: StorageDeps["cleared"]): RemoteStorage {
    return new RemoteStorage({ file: path.join(folder, "remote-partitions.json"), log, closePages, cleared });
  }

  /**
   * Whether the pair's pages may be made now: `live` once its partition is in
   * the index, `clearing` while an earlier clear of it runs or failed, `full`
   * when the index holds as many as it may.
   */
  admit(device: string, key: string): "live" | "clearing" | "full" {
    const partition = remotePagePartition(device, key);
    const entry = this.entries.get(partition);
    if (entry) return entry.state === "live" ? "live" : "clearing";
    if (this.entries.size >= MAX_STORED) {
      this.deps.log.event("remote.storage_full", {});
      return "full";
    }
    this.entries.set(partition, { device, key, state: "live" });
    this.save();
    return "live";
  }

  /**
   * The pairs this window's operator holds now, by `pairName`: every other
   * partition is cleared, and one still clearing or failed is cleared again.
   */
  keep(paired: ReadonlySet<string>): void {
    for (const [partition, entry] of this.entries) {
      if (entry.state !== "live" || !paired.has(pairName(entry.device, entry.key))) void this.clear(partition);
    }
  }

  /** Clears `partition`'s storage, once at a time. */
  private clear(partition: string): Promise<void> {
    const running = this.clearing.get(partition);
    if (running) return running;
    const entry = this.entries.get(partition);
    if (!entry) return Promise.resolve();
    entry.state = "clearing";
    this.save();
    this.deps.closePages(entry.device, entry.key);
    const pages = this.deps.session ? this.deps.session(partition) : electronSession.fromPartition(partition);
    const done = (async () => {
      try {
        await pages.closeAllConnections();
        await pages.clearAuthCache();
        await pages.clearStorageData();
        await pages.clearCache();
        if (this.entries.get(partition) === entry) this.entries.delete(partition);
        this.deps.log.event("remote.storage_cleared", {});
        this.save();
        this.deps.cleared?.(entry.device, entry.key);
        return;
      } catch (error) {
        entry.state = "failed";
        this.deps.log.event("remote.storage_clear_failed", { detail: String(error) });
      }
      this.save();
    })().finally(() => this.clearing.delete(partition));
    this.clearing.set(partition, done);
    return done;
  }

  /** Waits for every clear running now; for tests and for the quit. */
  async settled(): Promise<void> {
    await Promise.all([...this.clearing.values()]);
  }

  state(device: string, key: string): StoredState | null {
    return this.entries.get(remotePagePartition(device, key))?.state ?? null;
  }

  private save(): void {
    try {
      writeJsonFile(this.deps.file, { entries: [...this.entries.values()] });
    } catch (error) {
      this.deps.log.event("remote.storage_index_failed", { detail: String(error) });
    }
  }
}
