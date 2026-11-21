// What each released version brought (docs/UI_BEHAVIOR.md, What's new and
// Update history): the releases file built into the app
// (`web/src/releases.json`), and the record of which version's window closed
// (`shown`) and which versions the history opened (`seen`, `opened`), kept
// apart in the profile (operator decision m228). A profile no earlier launch
// wrote to is a new install: its version is recorded at once, so the
// first-run wizard alone introduces Hide (B8). It shows nothing itself: the
// shell decides and draws (`web/src/whatsNew.ts`). A failure is an event in
// the host's log; nothing on screen changes.

import fs from "node:fs";
import path from "node:path";
import type { WhatsNewState } from "../../../web/src/host";
import bundledReleases from "../../../web/src/releases.json";
import { announces, mergeRecord, parseReleaseFeed, parseVersion, VERSION_LIMIT, type ReleaseFeed, type WhatsNewRecord } from "../../../web/src/releases";
import { rememberedCliPath } from "./cli";
import { readJsonFile, writeJsonFile } from "./jsonFile";
import type { HostLog } from "./log";
import { windowStatePath } from "./windowState";

export const whatsNewRecordFile = (userData: string) => path.join(userData, "whats-new.json");
/**
 * Files an earlier launch leaves in the profile, named by their owners: the
 * window's bounds, kept when it closes, and the `hide` CLI that attached.
 * Neither exists before the window opens on a new install; a first launch
 * killed before either was written reads as new again, which only records
 * its version again.
 */
const earlierLaunchFiles = (userData: string) => [windowStatePath(userData), rememberedCliPath(userData)];
export type WhatsNewIo = {
  /** The releases file: the one built in, or the one `HIDE_RELEASES_FILE` names. */
  releases: unknown;
  readRecord: () => unknown;
  writeRecord: (record: WhatsNewRecord) => void;
  /** An earlier launch left files in the profile: false on a new install. */
  earlierLaunch: boolean;
};

const release = (value: unknown): string | null => (typeof value === "string" && parseVersion(value) ? value : null);
const detail = (error: unknown) => (error instanceof Error ? error.message : String(error)).slice(0, 300);

export class WhatsNew {
  private readonly feed: ReleaseFeed;
  private record: WhatsNewRecord | null;
  private readonly listeners = new Set<(state: WhatsNewState) => void>();

  /** Reads the file and the record; a file that does not parse throws, and the host logs it. */
  constructor(
    private readonly appVersion: string,
    private readonly io: WhatsNewIo,
    private readonly log: HostLog,
  ) {
    this.feed = this.parse(io.releases);
    this.record = this.readRecord();
    if (this.record === null && !io.earlierLaunch && announces(appVersion) && io.readRecord() === null) {
      this.log.event("whats_new.new_install", { version: appVersion });
      this.keep({ shown: appVersion, seen: appVersion, opened: [] });
    }
  }

  state(): WhatsNewState {
    return { appVersion: this.appVersion, feed: this.feed, record: this.record };
  }

  onChange(listener: (state: WhatsNewState) => void): void {
    this.listeners.add(listener);
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

/** The io a packaged app uses: the built-in file, or `file` read whole when one is named, and the record in `userData`. */
export function whatsNewIo(userData: string, file: string | null): WhatsNewIo {
  return {
    releases: file === null ? bundledReleases : JSON.parse(fs.readFileSync(file, "utf8")),
    readRecord: () => readJsonFile(whatsNewRecordFile(userData)),
    writeRecord: (record) => writeJsonFile(whatsNewRecordFile(userData), record),
    earlierLaunch: earlierLaunchFiles(userData).some((file) => fs.existsSync(file)),
  };
}
