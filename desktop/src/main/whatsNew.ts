// What each released version brought (docs/UI_BEHAVIOR.md, What's new and
// Update history): the file withhide.dev serves (`site/public/app/releases.json`),
// fetched at launch and again when the history opens on a copy an hour old,
// kept in the profile so the history reads offline, and the record of which
// version's window closed (`shown`) and which versions the history opened
// (`seen`, `opened`), kept apart (operator decision m228). A profile no
// earlier launch wrote to is a new install: its version is recorded at once,
// so the first-run wizard alone introduces Hide (B8). It shows nothing
// itself: the shell decides and draws (`web/src/whatsNew.ts`). A failure is
// an event in the host's log and a `failed` flag the history's empty state
// reads; nothing else on screen changes.

import fs from "node:fs";
import path from "node:path";
import type { WhatsNewState } from "../../../web/src/host";
import { announces, mergeRecord, parseReleaseFeed, parseVersion, VERSION_LIMIT, type ReleaseFeed, type WhatsNewRecord } from "../../../web/src/releases";
import { fetchBoundedJson, type FetchImpl } from "./boundedJson";
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
/** Files every earlier launch leaves in the profile; none of them before the window opens on a new install. */
const EARLIER_LAUNCH_FILES = ["window-state.json", "cli-path.json"];

export type WhatsNewIo = {
  fetch: () => Promise<unknown>;
  now: () => number;
  readCache: () => unknown;
  writeCache: (value: { fetchedAt: number; file: unknown }) => void;
  readRecord: () => unknown;
  writeRecord: (record: WhatsNewRecord) => void;
  /** An earlier launch left files in the profile: false on a new install. */
  earlierLaunch: boolean;
};

const release = (value: unknown): string | null => (typeof value === "string" && parseVersion(value) ? value : null);
const detail = (error: unknown) => (error instanceof Error ? error.message : String(error)).slice(0, 300);

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

  /** Reads the kept copy and the record, then fetches; the promise settles when that fetch has. */
  start(): Promise<void> {
    this.record = this.readRecord();
    if (this.record === null && !this.io.earlierLaunch && announces(this.appVersion) && this.io.readRecord() === null) {
      this.log.event("whats_new.new_install", { version: this.appVersion });
      this.keep({ shown: this.appVersion, seen: this.appVersion, opened: [] });
    }
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
    return this.refresh(true);
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
    // A copy dated ahead of the clock (the clock was set back) counts as old.
    const age = this.fetchedAt === null ? null : this.io.now() - this.fetchedAt;
    if (!always && age !== null && age >= 0 && age < REFETCH_AFTER_MS) return;
    this.fetching = true;
    this.emit();
    try {
      const file = await this.io.fetch();
      const feed = this.parse(file);
      const fetchedAt = this.io.now();
      this.feed = feed;
      this.fetchedAt = fetchedAt;
      this.failed = false;
      this.log.event("whats_new.fetched", { versions: feed.versions.length });
      // A copy that cannot be kept leaves this launch's file in use.
      try {
        this.io.writeCache({ fetchedAt, file });
      } catch (error) {
        this.log.event("whats_new.cache_write_failed", { detail: detail(error) });
      }
    } catch (error) {
      this.failed = true;
      this.log.event("whats_new.fetch_failed", { detail: detail(error) });
    } finally {
      this.fetching = false;
      this.emit();
    }
  }

  /** Moves the record forward, never back: a version's window closed, or a version opened in the history. */
  save(update: { shown?: unknown; seen?: unknown; opened?: unknown }): void {
    const merged = mergeRecord(this.record, update, this.feed, this.appVersion);
    if ("refused" in merged) {
      this.log.event("whats_new.save_refused", { reason: merged.refused });
      return;
    }
    const next = merged.record;
    const before = this.record;
    if (before !== null && next.shown === before.shown && next.seen === before.seen && next.opened.join() === before.opened.join()) return;
    this.keep(next);
  }

  /** Holds `record` for this launch and writes it; a write that fails is the log's, and this launch still keeps it. */
  private keep(record: WhatsNewRecord): void {
    this.record = record;
    try {
      this.io.writeRecord(record);
    } catch (error) {
      this.log.event("whats_new.record_write_failed", { detail: detail(error) });
    }
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
    const opened = Array.isArray(row.opened) ? row.opened.map(release).filter((value): value is string => value !== null).slice(0, VERSION_LIMIT) : [];
    return { shown: release(row.shown), seen: release(row.seen), opened };
  }

  private emit(): void {
    const state = this.state();
    for (const listener of this.listeners) listener(state);
  }
}

/** The file over Electron's network stack (the system's proxy), bounded in time and size. */
export function fetchReleasesFile(url: string, fetchImpl: FetchImpl): Promise<unknown> {
  return fetchBoundedJson(fetchImpl, url, { headers: { Accept: "application/json" }, cap: RELEASES_BYTES_CAP, timeoutMs: RELEASES_TIMEOUT_MS, what: "releases file" });
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
    earlierLaunch: EARLIER_LAUNCH_FILES.some((file) => fs.existsSync(path.join(userData, file))),
  };
}
