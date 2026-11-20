import { describe, expect, it } from "vitest";
import type { WhatsNewState } from "../../../web/src/host";
import type { HostLog } from "./log";
import { fetchReleasesFile, REFETCH_AFTER_MS, RELEASES_BYTES_CAP, WhatsNew } from "./whatsNew";

const file = (...versions: string[]) => ({
  schema: 1,
  versions: versions.map((version) => ({ version, date: "2026-11-20", highlights: [], changes: [{ kind: "fix", text: { en: `${version} fix` } }] })),
});

/** A host over files kept in memory and a network the test answers. */
function harness(options: { cache?: unknown; record?: unknown; earlierLaunch?: boolean; failWrites?: boolean; answers: (() => Promise<unknown>)[] }) {
  let clock = 1_000;
  const events: [string, Record<string, unknown>][] = [];
  const disk: { cache: unknown; record: unknown } = { cache: options.cache ?? null, record: options.record ?? null };
  const log = { event: (event: string, fields: Record<string, unknown> = {}) => events.push([event, fields]) } as unknown as HostLog;
  const answers = [...options.answers];
  const write = (apply: () => void) => {
    if (options.failWrites) throw new Error("ENOSPC: no space left on device");
    apply();
  };
  const whatsNew = new WhatsNew("1.2.11", {
    fetch: () => answers.shift()!(),
    now: () => clock,
    readCache: () => disk.cache,
    writeCache: (value) => write(() => { disk.cache = value; }),
    readRecord: () => disk.record,
    writeRecord: (record) => write(() => { disk.record = record; }),
    earlierLaunch: options.earlierLaunch ?? true,
  }, log);
  const states: WhatsNewState[] = [];
  whatsNew.onChange((state) => states.push(state));
  return { whatsNew, events, disk, states, advance: (ms: number) => { clock += ms; } };
}

const versions = (state: WhatsNewState) => state.feed?.versions.map((row) => row.version) ?? null;
const eventNames = (events: [string, Record<string, unknown>][]) => events.map(([event]) => event);

describe("WhatsNew", () => {
  it("reads the kept copy at once, then fetches and keeps the new file", async () => {
    const { whatsNew, disk, states } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => file("1.2.10", "1.2.11")] });
    const fetched = whatsNew.start();
    expect(versions(states[0]!)).toEqual(["1.2.10"]);
    await fetched;
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10"]);
    expect(disk.cache).toMatchObject({ file: file("1.2.10", "1.2.11") });
  });

  it("records a new install's version at once, before any file, so no window ever opens for it (B8)", async () => {
    const { whatsNew, disk, events } = harness({ earlierLaunch: false, answers: [async () => { throw new Error("offline"); }] });
    await whatsNew.start();
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.11", opened: [] });
    expect(whatsNew.state().record).toEqual({ shown: "1.2.11", seen: "1.2.11", opened: [] });
    expect(eventNames(events)).toContain("whats_new.new_install");
  });

  it("leaves an app that ran before without a record, so the shell offers it this version", async () => {
    const { whatsNew, disk } = harness({ answers: [async () => file("1.2.11")] });
    await whatsNew.start();
    expect(disk.record).toBeNull();
    expect(whatsNew.state().record).toBeNull();
  });

  it("keeps the copy and says the fetch failed when withhide.dev cannot be reached (B11, B19)", async () => {
    const { whatsNew, events } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => { throw new Error("getaddrinfo ENOTFOUND"); }] });
    await whatsNew.start();
    expect(whatsNew.state()).toMatchObject({ failed: true, fetching: false });
    expect(versions(whatsNew.state())).toEqual(["1.2.10"]);
    expect(events.at(-1)).toEqual(["whats_new.fetch_failed", { detail: "getaddrinfo ENOTFOUND" }]);
  });

  it("throws away an answer of the wrong shape and keeps the copy (B22)", async () => {
    const { whatsNew, disk } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => ({ schema: 9, versions: [] })] });
    await whatsNew.start();
    expect(whatsNew.state().failed).toBe(true);
    expect(disk.cache).toMatchObject({ file: file("1.2.10") });
  });

  it("uses a fetched file and keeps the record for the launch when the profile cannot be written", async () => {
    const { whatsNew, events } = harness({ failWrites: true, answers: [async () => file("1.2.11")] });
    await whatsNew.start();
    expect(whatsNew.state()).toMatchObject({ failed: false });
    expect(versions(whatsNew.state())).toEqual(["1.2.11"]);
    whatsNew.save({ shown: "1.2.11" });
    expect(whatsNew.state().record).toEqual({ shown: "1.2.11", seen: null, opened: [] });
    expect(eventNames(events)).toEqual(expect.arrayContaining(["whats_new.cache_write_failed", "whats_new.record_write_failed"]));
  });

  it("fetches again on the history's open only once the copy is an hour old, and at once on Try again (B19, B20)", async () => {
    const { whatsNew, advance } = harness({ answers: [async () => file("1.2.10"), async () => file("1.2.10", "1.2.11"), async () => file("1.2.9", "1.2.10", "1.2.11")] });
    await whatsNew.start();
    await whatsNew.refresh();
    expect(versions(whatsNew.state())).toEqual(["1.2.10"]);
    advance(REFETCH_AFTER_MS);
    await whatsNew.refresh();
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10"]);
    await whatsNew.refresh(true);
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10", "1.2.9"]);
  });

  it("counts a copy dated ahead of the clock as old", async () => {
    const { whatsNew } = harness({ cache: { fetchedAt: 10 * REFETCH_AFTER_MS, file: file("1.2.10") }, answers: [async () => { throw new Error("offline"); }, async () => file("1.2.11")] });
    await whatsNew.start();
    await whatsNew.refresh();
    expect(versions(whatsNew.state())).toEqual(["1.2.11"]);
  });

  it("moves the record forward through the shell's saves and refuses one past the running version", async () => {
    const { whatsNew, disk, events } = harness({ record: { shown: "1.2.10", seen: "1.2.9" }, answers: [async () => file("1.2.9", "1.2.10", "1.2.11")] });
    await whatsNew.start();
    whatsNew.save({ shown: "1.2.11" });
    whatsNew.save({ opened: "1.2.11" });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.9", opened: ["1.2.11"] });
    whatsNew.save({ shown: "99.0.0" });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.9", opened: ["1.2.11"] });
    expect(events.at(-1)).toEqual(["whats_new.save_refused", { reason: "ahead" }]);
  });

  it("reads an unreadable record as none, says so in the log, and does not take it for a new install", async () => {
    const { whatsNew, events, disk } = harness({ record: { unreadable: "not JSON" }, earlierLaunch: false, answers: [async () => file()] });
    await whatsNew.start();
    expect(whatsNew.state().record).toBeNull();
    expect(disk.record).toEqual({ unreadable: "not JSON" });
    expect(events[0]).toEqual(["whats_new.record_unreadable", { detail: "not JSON" }]);
  });
});

describe("fetchReleasesFile", () => {
  const answer = (body: BodyInit, init: ResponseInit = {}) => async () => new Response(body, init);

  it("refuses a file over 1 MiB, by its declared length and as its body arrives", async () => {
    const big = "x".repeat(RELEASES_BYTES_CAP + 1);
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer(big))).rejects.toThrow("exceeds 1 MiB");
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer("{}", { headers: { "content-length": String(RELEASES_BYTES_CAP + 1) } }))).rejects.toThrow("exceeds 1 MiB");
    // An answer that never ends is cut at the cap instead of being read whole.
    let sent = 0;
    const endless = new ReadableStream<Uint8Array>({ pull: (controller) => { sent += 64 * 1024; controller.enqueue(new Uint8Array(64 * 1024)); } });
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer(endless))).rejects.toThrow("exceeds 1 MiB");
    expect(sent).toBeLessThan(2 * RELEASES_BYTES_CAP);
  });

  it("reads a file that arrives in pieces", async () => {
    const text = JSON.stringify(file("1.2.11"));
    const pieces = [text.slice(0, 10), text.slice(10)].map((part) => new TextEncoder().encode(part));
    const stream = new ReadableStream<Uint8Array>({ start: (controller) => { for (const part of pieces) controller.enqueue(part); controller.close(); } });
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer(stream))).resolves.toEqual(file("1.2.11"));
  });

  it("says the HTTP status of a failed answer", async () => {
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer("missing", { status: 404 }))).rejects.toThrow("HTTP 404");
  });
});
