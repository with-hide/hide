import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import type { WhatsNewState } from "../../../web/src/host";
import { rememberedCliPath } from "./cli";
import type { HostLog } from "./log";
import { WhatsNew, whatsNewIo } from "./whatsNew";
import { windowStatePath } from "./windowState";

const file = (...versions: string[]) => ({
  schema: 1,
  versions: versions.map((version) => ({ version, date: "2026-11-20", highlights: [], changes: [{ kind: "fix", text: { en: `${version} fix` } }] })),
});

/** A host over a releases file and a record kept in memory. */
function harness(options: { releases?: unknown; record?: unknown; earlierLaunch?: boolean; failWrites?: boolean }) {
  const events: [string, Record<string, unknown>][] = [];
  const disk: { record: unknown } = { record: options.record ?? null };
  const log = { event: (event: string, fields: Record<string, unknown> = {}) => events.push([event, fields]) } as unknown as HostLog;
  const whatsNew = new WhatsNew("1.2.11", {
    releases: options.releases ?? file("1.2.9", "1.2.10", "1.2.11"),
    readRecord: () => disk.record,
    writeRecord: (record) => {
      if (options.failWrites) throw new Error("ENOSPC: no space left on device");
      disk.record = record;
    },
    earlierLaunch: options.earlierLaunch ?? true,
  }, log);
  return { whatsNew, events, disk };
}

const versions = (state: WhatsNewState) => state.feed.versions.map((row) => row.version);
const eventNames = (events: [string, Record<string, unknown>][]) => events.map(([event]) => event);

describe("WhatsNew", () => {
  it("records a new install's version at once, so no window ever opens for it (B8)", () => {
    const { whatsNew, disk, events } = harness({ earlierLaunch: false });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.11", opened: [] });
    expect(whatsNew.state().record).toEqual({ shown: "1.2.11", seen: "1.2.11", opened: [] });
    expect(eventNames(events)).toContain("whats_new.new_install");
  });

  it("leaves an app that ran before without a record, so the shell offers it this version", () => {
    const { whatsNew, disk } = harness({});
    expect(disk.record).toBeNull();
    expect(whatsNew.state().record).toBeNull();
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10", "1.2.9"]);
  });

  it("throws on a file that is not a releases file, for the host to log", () => {
    expect(() => harness({ releases: { schema: 9, versions: [] } })).toThrow();
  });

  it("keeps the record for the launch when the profile cannot be written", () => {
    const { whatsNew, events } = harness({ failWrites: true });
    whatsNew.save({ shown: "1.2.11" });
    expect(whatsNew.state().record).toEqual({ shown: "1.2.11", seen: null, opened: [] });
    expect(eventNames(events)).toContain("whats_new.record_write_failed");
  });

  it("moves the record forward through the shell's saves and refuses one past the running version", () => {
    const { whatsNew, disk, events } = harness({ record: { shown: "1.2.10", seen: "1.2.9" } });
    const states: WhatsNewState[] = [];
    whatsNew.onChange((state) => states.push(state));
    whatsNew.save({ shown: "1.2.11" });
    whatsNew.save({ opened: "1.2.11" });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.9", opened: ["1.2.11"] });
    expect(states.at(-1)?.record).toEqual({ shown: "1.2.11", seen: "1.2.9", opened: ["1.2.11"] });
    whatsNew.save({ shown: "99.0.0" });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.9", opened: ["1.2.11"] });
    expect(events.at(-1)).toEqual(["whats_new.save_refused", { reason: "ahead" }]);
  });

  it("reads an unreadable record as none, says so in the log, and does not take it for a new install", () => {
    const { whatsNew, events, disk } = harness({ record: { unreadable: "not JSON" }, earlierLaunch: false });
    expect(whatsNew.state().record).toBeNull();
    expect(disk.record).toEqual({ unreadable: "not JSON" });
    expect(events[0]).toEqual(["whats_new.record_unreadable", { detail: "not JSON" }]);
  });
});

describe("whatsNewIo", () => {
  const profile = () => fs.mkdtempSync(path.join(os.tmpdir(), "whats-new-profile-"));

  it("reads a profile with no file of an earlier launch as a new install, and one with the window's bounds or the CLI it found as not", () => {
    for (const [file, earlier] of [[null, false], [windowStatePath, true], [rememberedCliPath, true]] as const) {
      const dir = profile();
      try {
        if (file) fs.writeFileSync(file(dir), "{}");
        expect(whatsNewIo(dir, null).earlierLaunch, String(file?.name)).toBe(earlier);
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    }
  });

  it("reads the built-in file, or the file named in its place", () => {
    const dir = profile();
    try {
      expect(new WhatsNew("1.2.11", whatsNewIo(dir, null), { event: () => undefined } as unknown as HostLog).state().feed.versions.length).toBeGreaterThan(0);
      const named = path.join(dir, "releases.json");
      fs.writeFileSync(named, JSON.stringify(file("1.2.11")));
      expect(versions(new WhatsNew("1.2.11", whatsNewIo(dir, named), { event: () => undefined } as unknown as HostLog).state())).toEqual(["1.2.11"]);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
