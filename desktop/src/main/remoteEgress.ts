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
// The pages of one pairing share one persistent session, named by the
// machine, its key and the pairing (the daemon's hash of the grant the
// machine gave), so it is kept across restarts and a pause, and pairing
// again starts empty whatever happened to the clear of the one before. The
// index file names each partition with the core whose pairing it was and
// its state, written before the step it names, so a clear a quit cut short
// runs again before anything else uses it; a partition folder the index
// does not name is deleted at launch.

import { session as electronSession, type Session } from "electron";
import fs from "node:fs";
import path from "node:path";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import { remoteFilePartition, remotePagePartition } from "./browserSync";
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
type Pair = {
  device: string;
  key: string;
  listener: Listener | null;
  /** The pairing the daemon last named, which the pages' partition is named by. */
  pairing: string | null;
  /** The partition whose proxy is the listener now. */
  proxied: string | null;
  /** Whether an ask reached the daemon, which may hold a listener for it whatever came of the answer. */
  asked: boolean;
  asking: Promise<EgressState> | null;
};

/** The machine and key a pair is named by, as the paired set names it. */
export function pairName(device: string, key: string): string {
  return `${device}\u0000${key}`;
}

const PAIRING = /^[0-9a-f]{32}$/;

export type EgressDeps = {
  daemon: () => EgressDaemon | null;
  log: HostLog;
  /** The session of a partition; Electron's unless a test hands its own. */
  session?: (partition: string) => Pick<Session, "setProxy" | "closeAllConnections">;
  /** The owner the daemon keeps a listener for: this process. */
  owner?: number;
  fetch?: typeof fetch;
  /**
   * The machine was paired again under the same key while its screen showed:
   * its pages, on the ended pairing's partition, close and are made again on
   * the new one, whose proxy is set now.
   */
  repaired?: (device: string, key: string) => void;
};

export class RemoteEgress {
  private readonly pairs = new Map<string, Pair>();
  /** The listener a released pair may still hold, being let go. */
  private readonly releasing = new Map<string, Promise<void>>();

  constructor(private readonly deps: EgressDeps) {}

  private sessionOf(partition: string): Pick<Session, "setProxy" | "closeAllConnections"> {
    return this.deps.session ? this.deps.session(partition) : electronSession.fromPartition(partition);
  }

  /** The pairing the daemon last named for the pair; null before its first answer. */
  pairing(device: string, key: string): string | null {
    return this.pairs.get(pairName(device, key))?.pairing ?? null;
  }

  /** The partition the pair's pages keep their logins in now; null before its first answer. */
  partition(device: string, key: string): string | null {
    const pairing = this.pairing(device, key);
    return pairing === null ? null : remotePagePartition(device, key, pairing);
  }

  /** The partition the pair's file pages load in now (`remoteFilePartition`); null before its first answer. */
  filePartition(device: string, key: string): string | null {
    const pairing = this.pairing(device, key);
    return pairing === null ? null : remoteFilePartition(device, key, pairing);
  }

  /** Whether the pair's pages may make requests: their partitions' proxy is the listener. */
  ready(device: string, key: string): boolean {
    const pair = this.pairs.get(pairName(device, key));
    return pair?.listener != null && pair.proxied === this.partition(device, key);
  }

  /**
   * Asks the daemon for the pair's listener and where its requests stand.
   * The first answer, and one naming another port, secret or pairing, sets
   * the proxy of both its partitions, the logins' and the file pages', and
   * closes their connections before the pair counts as ready; one ask at a time per pair, after the release of the pair
   * before it has let its listener go.
   */
  ensure(device: string, key: string): Promise<EgressState> {
    const name = pairName(device, key);
    let pair = this.pairs.get(name);
    if (!pair) {
      pair = { device, key, listener: null, pairing: null, proxied: null, asked: false, asking: null };
      this.pairs.set(name, pair);
    }
    const asked = pair;
    asked.asking ??= (async () => {
      await this.releasing.get(name);
      return this.ask(asked);
    })().finally(() => {
      asked.asking = null;
    });
    return asked.asking;
  }

  private async ask(pair: Pair): Promise<EgressState> {
    const daemon = this.deps.daemon();
    if (!daemon) return "not_connected";
    const name = pairName(pair.device, pair.key);
    if (this.pairs.get(name) !== pair) return "not_connected";
    let answer: Response;
    // From here the daemon may hold a listener for the pair, whether or not
    // the answer arrives or the proxy is set: `release` lets it go.
    pair.asked = true;
    try {
      answer = await (this.deps.fetch ?? fetch)(this.address(daemon, pair.device, pair.key), { method: "POST", headers: { authorization: `Bearer ${daemon.token}` }, signal: AbortSignal.timeout(10_000) });
    } catch {
      return "not_connected";
    }
    if (answer.status === 404) return "remote_update_here";
    const body: unknown = await answer.json().catch(() => null);
    const fields = typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
    if (!answer.ok) return typeof fields.reason === "string" && fields.reason.length <= 64 ? fields.reason : "not_connected";
    const { port, secret, pairing, state } = fields;
    if (typeof port !== "number" || !Number.isInteger(port) || port < 1 || port > 65_535 || typeof secret !== "string" || !/^[0-9a-f]{32}$/.test(secret) || typeof pairing !== "string" || !PAIRING.test(pairing) || typeof state !== "string") {
      this.deps.log.event("remote.egress_invalid", {});
      return "not_connected";
    }
    if (this.pairs.get(name) !== pair) return "not_connected";
    const repaired = pair.pairing !== null && pair.pairing !== pairing;
    // From here the pages of an ended pairing are refused every request (`ready`).
    pair.pairing = pairing;
    const partition = remotePagePartition(pair.device, pair.key, pairing);
    if (pair.listener?.port !== port || pair.listener.secret !== secret || pair.proxied !== partition) {
      pair.listener = null;
      pair.proxied = null;
      try {
        for (const proxied of [partition, remoteFilePartition(pair.device, pair.key, pairing)]) {
          const pages = this.sessionOf(proxied);
          await pages.setProxy({ mode: "fixed_servers", proxyRules: `http://127.0.0.1:${port}`, proxyBypassRules: "<-loopback>" });
          await pages.closeAllConnections();
        }
      } catch (error) {
        this.deps.log.event("remote.egress_failed", { detail: String(error) });
        return "not_connected";
      }
      if (this.pairs.get(name) !== pair || pair.pairing !== pairing) return "not_connected";
      pair.listener = { port, secret };
      pair.proxied = partition;
      this.deps.log.event("remote.egress_ready", { device: pair.device });
    }
    if (repaired) {
      this.deps.log.event("remote.egress_repaired", { device: pair.device });
      this.deps.repaired?.(pair.device, pair.key);
    }
    return state;
  }

  private address(daemon: EgressDaemon, device: string, key: string): string {
    const owner = this.deps.owner ?? process.pid;
    return `${daemon.origin}/peer-screen/${encodeURIComponent(device)}/net?key=${encodeURIComponent(key)}&owner=${owner}`;
  }

  /** Chromium's proxy login for one of the pairs' listeners: `hide` and its secret; null for any other proxy. */
  credentials(host: string, port: number): { username: string; password: string } | null {
    if (host !== "127.0.0.1") return null;
    for (const pair of this.pairs.values()) {
      if (pair.listener?.port === port) return { username: "hide", password: pair.listener.secret };
    }
    return null;
  }

  /**
   * The pair's pages are gone: its pages may make no request until asked
   * again, and the daemon lets go of any listener an ask of the pair may
   * have opened (one answered, one whose proxy could not be set, one whose
   * answer was lost), once an ask still on its way has its answer.
   */
  release(device: string, key: string): void {
    const name = pairName(device, key);
    const pair = this.pairs.get(name);
    if (!pair) return;
    this.pairs.delete(name);
    if (!pair.asked && !pair.asking) return;
    const asked = pair.asking ?? Promise.resolve("");
    const done: Promise<void> = asked
      .catch(() => "")
      .then(() => this.letGo(device, key))
      .finally(() => {
        if (this.releasing.get(name) === done) this.releasing.delete(name);
      });
    this.releasing.set(name, done);
  }

  private async letGo(device: string, key: string): Promise<void> {
    const daemon = this.deps.daemon();
    if (!daemon) return;
    try {
      await (this.deps.fetch ?? fetch)(this.address(daemon, device, key), { method: "DELETE", headers: { authorization: `Bearer ${daemon.token}` }, signal: AbortSignal.timeout(3_000) });
    } catch {
      this.deps.log.event("remote.egress_release_failed", {});
    }
  }

  /** This computer's daemon is another one now: every listener it held is gone with it. */
  forgetAll(): void {
    this.pairs.clear();
    this.releasing.clear();
  }
}

/** The partitions the index may hold; a new pair past it keeps no storage. */
export const MAX_STORED = 32;

type StoredState = "live" | "clearing" | "failed";
type Stored = { device: string; key: string; pairing: string; core: string; state: StoredState };

/** What a session's clear needs. */
export type Clearable = Pick<Session, "closeAllConnections" | "clearAuthCache" | "clearStorageData" | "clearCache">;

/** Whether a pair's pages may be made now (`RemoteStorage.admit`). */
export type Admitted = "live" | "clearing" | "full" | "waiting" | "unrecorded";

export type StorageDeps = {
  /** The index file. */
  file: string;
  /** The profile's partition folders (`<userData>/Partitions`); none in a test that keeps no folders. */
  partitions?: string;
  log: HostLog;
  session?: (partition: string) => Clearable;
  /** Closes every page on `partition` before its storage is cleared, and no page of another pairing. */
  closePages: (partition: string) => void;
  /** The pair's storage was cleared, or may be admitted now: a screen of it may make its pages again. */
  cleared?: (device: string, key: string) => void;
};

/** The folder Electron keeps a remote page partition in, by its name. */
const PARTITION_FOLDER = /^hide-remote-page-[0-9a-f]{32}$/;

/**
 * The login storage of the machines this window's operator paired with
 * (one partition per machine, key and pairing): which exist, the core whose
 * pairing each was, and the clear of each once its pair ends, in order:
 * connections closed, the HTTP auth cache, storage, then the cache. An entry
 * goes only after all of them; one that failed stays `failed` and is cleared
 * again at the next launch and the next paired set. Its pairing in its name
 * keeps a failed clear, or a lost index, from ever reaching a later pairing.
 */
export class RemoteStorage {
  private readonly entries = new Map<string, Stored>();
  private readonly clearing = new Map<string, Promise<void>>();
  /** The core the shell last named the paired set of; nothing is admitted before it. */
  private core: string | null = null;
  private fullLogged = false;

  constructor(private readonly deps: StorageDeps) {
    const read = readJsonFile(deps.file);
    const rows = typeof read === "object" && read !== null && Array.isArray((read as { entries?: unknown }).entries) ? (read as { entries: unknown[] }).entries : [];
    for (const row of rows.slice(0, MAX_STORED)) {
      if (typeof row !== "object" || row === null) continue;
      const { device, key, pairing, core, state } = row as Record<string, unknown>;
      if (typeof device !== "string" || typeof key !== "string" || typeof pairing !== "string" || !PAIRING.test(pairing) || typeof core !== "string" || (state !== "live" && state !== "clearing" && state !== "failed")) continue;
      this.entries.set(remotePagePartition(device, key, pairing), { device, key, pairing, core, state });
    }
    this.deleteUnnamed();
    // A clear a quit cut short, or one that failed, runs again before any page.
    for (const [partition, entry] of this.entries) if (entry.state !== "live") void this.clear(partition);
  }

  static at(folder: string, log: HostLog, closePages: StorageDeps["closePages"], cleared: StorageDeps["cleared"]): RemoteStorage {
    return new RemoteStorage({ file: path.join(folder, "remote-partitions.json"), partitions: path.join(folder, "Partitions"), log, closePages, cleared });
  }

  /**
   * Deletes, before any page is made, each remote page folder no entry
   * names: one whose index entry was lost, whose index could not be read,
   * or that was made past a full index. Only names of that one shape, in
   * that one folder.
   */
  private deleteUnnamed(): void {
    const folder = this.deps.partitions;
    if (!folder) return;
    let names: string[];
    try {
      names = fs.readdirSync(folder);
    } catch {
      return;
    }
    const named = new Set([...this.entries.keys()].map((partition) => partition.slice("persist:".length)));
    for (const name of names) {
      if (!PARTITION_FOLDER.test(name) || named.has(name)) continue;
      try {
        fs.rmSync(path.join(folder, name), { recursive: true, force: true });
        this.deps.log.event("remote.storage_unnamed_deleted", {});
      } catch (error) {
        this.deps.log.event("remote.storage_unnamed_failed", { detail: String(error) });
      }
    }
  }

  /**
   * Whether the pair's pages may be made now, on the partition of
   * `pairing`: `live` once it is in the index, `clearing` while an earlier
   * clear of it runs or failed, `full` when the index holds as many as it
   * may, `waiting` until the shell has named its core's paired set, and
   * `unrecorded` when its row could not be written: a session the index does
   * not name is deleted at the next launch, so none is made before its row
   * is on disk, and the next sync asks again. A live entry of the same
   * machine and key under another pairing of this core is cleared, its own
   * pages closed first; one whose clear failed is cleared again at the next
   * launch and paired set, never at each sync.
   */
  admit(device: string, key: string, pairing: string): Admitted {
    const core = this.core;
    if (core === null) return "waiting";
    const partition = remotePagePartition(device, key, pairing);
    for (const [held, entry] of this.entries) {
      if (held !== partition && entry.state === "live" && entry.core === core && entry.device === device && entry.key === key) void this.clear(held);
    }
    const entry = this.entries.get(partition);
    if (entry) return entry.state === "live" ? "live" : "clearing";
    if (this.entries.size >= MAX_STORED) {
      if (!this.fullLogged) this.deps.log.event("remote.storage_full", {});
      this.fullLogged = true;
      return "full";
    }
    this.fullLogged = false;
    const admitted: Stored = { device, key, pairing, core, state: "live" };
    this.entries.set(partition, admitted);
    if (this.save()) return "live";
    if (this.entries.get(partition) === admitted) this.entries.delete(partition);
    return "unrecorded";
  }

  /**
   * The pairs core `core` holds now, by `pairName`, as this window's shell
   * names them: each partition of that core's that none of them names is
   * cleared, and one of any core still clearing or failed is cleared again.
   * Another core's live entries stay: the same profile attached to another
   * state folder does not clear the logins of the core it left. The first
   * set lets every pair waiting on it make its pages.
   */
  keep(core: string, paired: ReadonlySet<string>): void {
    const first = this.core === null;
    this.core = core;
    for (const [partition, entry] of this.entries) {
      if (entry.state !== "live" || (entry.core === core && !paired.has(pairName(entry.device, entry.key)))) void this.clear(partition);
    }
    if (!first) return;
    for (const pair of paired) {
      const at = pair.indexOf("\u0000");
      this.deps.cleared?.(pair.slice(0, at), pair.slice(at + 1));
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
    this.deps.closePages(partition);
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

  state(device: string, key: string, pairing: string): StoredState | null {
    return this.entries.get(remotePagePartition(device, key, pairing))?.state ?? null;
  }

  /** Writes the index; false, and logged, when it could not be written. */
  private save(): boolean {
    try {
      writeJsonFile(this.deps.file, { entries: [...this.entries.values()] });
      return true;
    } catch (error) {
      this.deps.log.event("remote.storage_index_failed", { detail: String(error) });
      return false;
    }
  }
}
