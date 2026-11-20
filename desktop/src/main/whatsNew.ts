// What each released version brought (docs/UI_BEHAVIOR.md, What's new and
// Update history): the file withhide.dev serves (`site/public/app/releases.json`),
// fetched at launch and again when the history opens on a copy an hour old,
// kept in the profile so the history reads offline, and the record of which
// version's window closed (`shown`) and which version was opened in the
// history (`seen`), kept apart (operator decision m228). It shows nothing
// itself: the shell decides and draws (`web/src/whatsNew.ts`). A failure is
// an event in the host's log and a `failed` flag the history's empty state
// reads; nothing else on screen changes.

import path from "node:path";
import type { WhatsNewState } from "../../../web/src/host";
import { compareVersions, parseReleaseFeed, parseVersion, type ReleaseFeed, type WhatsNewRecord } from "../../../web/src/releases";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import type { HostLog } from "./log";

/** The published file; `HIDE_RELEASES_FEED_URL` stands in for it in tests. */
export const RELEASES_URL = "https://withhide.dev/app/releases.json";
/** The file is a few KB a version; more is not withhide.dev's answer. */
export const RELEASES_BYTES_CAP = 1024 * 1024;
const RELEASES_TIMEOUT_MS = 20_000;
/** Opening the history fetches again once the copy is this old. */
export const REFETCH_AFTER_MS = 60 * 60 * 1000;

export const releasesCacheFile = (userData: string) => path.join(userData, "releases.json");
export const whatsNewRecordFile = (userData: string) => path.join(userData, "whats-new.json");

export type WhatsNewIo = {
  fetch: () => Promise<unknown>;
  now: () => number;
  readCache: () => unknown;
  writeCache: (value: { fetchedAt: number; file: unknown }) => void;
  readRecord: () => unknown;
  writeRecord: (record: WhatsNewRecord) => void;
};

const release = (value: unknown): string | null => (typeof value === "string" && parseVersion(value) ? value : null);
const newer = (a: string | null, b: string | null): string | null => (a === null ? b : b === null ? a : compareVersions(parseVersion(a)!, parseVersion(b)!) >= 0 ? a : b);

export class WhatsNew {
  private feed: ReleaseFeed | null = null;
  private fetchedAt: number | null = null;
  private fetching = false;
  private failed = false;
  private record: WhatsNewRecord | null = null;
  private readonly launchedAt: number;
  private readonly listeners = new Set<(state: WhatsNewState) => void>();

  constructor(
    private readonly appVersion: string,
    private readonly io: WhatsNewIo,
    private readonly log: HostLog,
  ) {
    this.launchedAt = io.now();
  }

  /** Reads the kept copy and the record, then fetches. */
  start(): void {
    this.record = this.readRecord();
    const cached = this.io.readCache();
    if (cached !== null) {
      const kept = typeof cached === "object" && cached !== null ? (cached as { fetchedAt?: unknown; file?: unknown }) : {};
      try {
        if (typeof kept.fetchedAt !== "number") throw new Error("no fetch time");
        this.feed = this.parse(kept.file);
        this.fetchedAt = kept.fetchedAt;
      } catch (error) {
        this.log.event("whats_new.cache_unreadable", { detail: String(error).slice(0, 200) });
      }
    }
    this.emit();
    void this.refresh(true);
  }

  state(): WhatsNewState {
    return { appVersion: this.appVersion, launchedAt: this.launchedAt, feed: this.feed, fetchedAt: this.fetchedAt, fetching: this.fetching, failed: this.failed, record: this.record };
  }

  onChange(listener: (state: WhatsNewState) => void): void {
    this.listeners.add(listener);
  }

  /**
   * Fetches the file unless a fetch is running or, without `always`, the copy
   * is younger than `REFETCH_AFTER_MS`. A bad answer keeps the copy (B22).
   */
  async refresh(always = false): Promise<void> {
    if (this.fetching) return;
    if (!always && this.fetchedAt !== null && this.io.now() - this.fetchedAt < REFETCH_AFTER_MS) return;
    this.fetching = true;
    this.emit();
    try {
      const file = await this.io.fetch();
      const feed = this.parse(file);
      const fetchedAt = this.io.now();
      this.io.writeCache({ fetchedAt, file });
      this.feed = feed;
      this.fetchedAt = fetchedAt;
      this.failed = false;
      this.log.event("whats_new.fetched", { versions: feed.versions.length });
    } catch (error) {
      this.failed = true;
      this.log.event("whats_new.fetch_failed", { detail: (error instanceof Error ? error.message : String(error)).slice(0, 300) });
    } finally {
      this.fetching = false;
      this.emit();
    }
  }

  /** Moves the record forward, never back: a version's window closed, or a version opened in the history. */
  save(update: { shown?: unknown; seen?: unknown }): void {
    const shown = update.shown === undefined ? undefined : release(update.shown);
    const seen = update.seen === undefined ? undefined : release(update.seen);
    if (shown === null || seen === null) {
      this.log.event("whats_new.save_refused", { reason: "version" });
      return;
    }
    const before = this.record ?? { shown: null, seen: null };
    const next: WhatsNewRecord = { shown: shown === undefined ? before.shown : newer(before.shown, shown), seen: seen === undefined ? before.seen : newer(before.seen, seen) };
    if (this.record !== null && next.shown === before.shown && next.seen === before.seen) return;
    this.io.writeRecord(next);
    this.record = next;
    this.emit();
  }

  private parse(file: unknown): ReleaseFeed {
    const { feed, dropped } = parseReleaseFeed(file);
    for (const row of dropped) this.log.event("whats_new.version_dropped", { version: row.version, reason: row.reason });
    return feed;
  }

  private readRecord(): WhatsNewRecord | null {
    const raw = this.io.readRecord();
    if (raw === null) return null;
    const row = typeof raw === "object" ? (raw as Record<string, unknown>) : {};
    if ("unreadable" in row) {
      this.log.event("whats_new.record_unreadable", { detail: String(row.unreadable).slice(0, 200) });
      return null;
    }
    return { shown: release(row.shown), seen: release(row.seen) };
  }

  private emit(): void {
    const state = this.state();
    for (const listener of this.listeners) listener(state);
  }
}

/** The file over Electron's network stack (the system's proxy), bounded in time and size. */
export async function fetchReleasesFile(url: string, fetchImpl: (url: string, init: RequestInit) => Promise<Response>): Promise<unknown> {
  const response = await fetchImpl(url, { headers: { Accept: "application/json" }, signal: AbortSignal.timeout(RELEASES_TIMEOUT_MS) });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(`releases file answered HTTP ${response.status}`);
  }
  const declared = Number(response.headers.get("content-length") ?? "0");
  if (declared > RELEASES_BYTES_CAP) {
    await response.body?.cancel();
    throw new Error("releases file exceeds 1 MiB");
  }
  const body = await response.arrayBuffer();
  if (body.byteLength > RELEASES_BYTES_CAP) throw new Error("releases file exceeds 1 MiB");
  return JSON.parse(new TextDecoder().decode(body));
}

/** The io a packaged app uses, files in `userData`. */
export function whatsNewIo(userData: string, fetch: () => Promise<unknown>): WhatsNewIo {
  return {
    fetch,
    now: Date.now,
    readCache: () => readJsonFile(releasesCacheFile(userData)),
    writeCache: (value) => writeJsonFile(releasesCacheFile(userData), value),
    readRecord: () => readJsonFile(whatsNewRecordFile(userData)),
    writeRecord: (record) => writeJsonFile(whatsNewRecordFile(userData), record),
  };
}
