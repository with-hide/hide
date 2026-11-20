import { describe, expect, it } from "vitest";
import type { WhatsNewState } from "../../../web/src/host";
import type { HostLog } from "./log";
import { fetchReleasesFile, REFETCH_AFTER_MS, RELEASES_BYTES_CAP, WhatsNew } from "./whatsNew";

const file = (...versions: string[]) => ({
  schema: 1,
  versions: versions.map((version) => ({ version, date: "2026-11-20", highlights: [], changes: [{ kind: "fix", text: { en: `${version} fix` } }] })),
});

/** A host over files kept in memory and a network the test answers. */
function harness(options: { cache?: unknown; record?: unknown; answers: (() => Promise<unknown>)[] }) {
  let clock = 1_000;
  const events: [string, Record<string, unknown>][] = [];
  const disk: { cache: unknown; record: unknown } = { cache: options.cache ?? null, record: options.record ?? null };
  const log = { event: (event: string, fields: Record<string, unknown> = {}) => events.push([event, fields]) } as unknown as HostLog;
  const answers = [...options.answers];
  const whatsNew = new WhatsNew("1.2.11", {
    fetch: () => answers.shift()!(),
    now: () => clock,
    readCache: () => disk.cache,
    writeCache: (value) => { disk.cache = value; },
    readRecord: () => disk.record,
    writeRecord: (record) => { disk.record = record; },
  }, log);
  const states: WhatsNewState[] = [];
  whatsNew.onChange((state) => states.push(state));
  return { whatsNew, events, disk, states, advance: (ms: number) => { clock += ms; } };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const versions = (state: WhatsNewState) => state.feed?.versions.map((row) => row.version) ?? null;

describe("WhatsNew", () => {
  it("reads the kept copy at once, then fetches and keeps the new file", async () => {
    const { whatsNew, disk, states } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => file("1.2.10", "1.2.11")] });
    whatsNew.start();
    expect(versions(states[0]!)).toEqual(["1.2.10"]);
    await settle();
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10"]);
    expect(disk.cache).toMatchObject({ file: file("1.2.10", "1.2.11") });
  });

  it("keeps the copy and says the fetch failed when withhide.dev cannot be reached (B11, B19)", async () => {
    const { whatsNew, events } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => { throw new Error("getaddrinfo ENOTFOUND"); }] });
    whatsNew.start();
    await settle();
    expect(whatsNew.state()).toMatchObject({ failed: true, fetching: false });
    expect(versions(whatsNew.state())).toEqual(["1.2.10"]);
    expect(events.at(-1)).toEqual(["whats_new.fetch_failed", { detail: "getaddrinfo ENOTFOUND" }]);
  });

  it("throws away an answer of the wrong shape and keeps the copy (B22)", async () => {
    const { whatsNew, disk } = harness({ cache: { fetchedAt: 10, file: file("1.2.10") }, answers: [async () => ({ schema: 9, versions: [] })] });
    whatsNew.start();
    await settle();
    expect(whatsNew.state().failed).toBe(true);
    expect(disk.cache).toMatchObject({ file: file("1.2.10") });
  });

  it("fetches again on the history's open only once the copy is an hour old (B20)", async () => {
    const { whatsNew, advance } = harness({ answers: [async () => file("1.2.10"), async () => file("1.2.10", "1.2.11")] });
    whatsNew.start();
    await settle();
    await whatsNew.refresh();
    expect(versions(whatsNew.state())).toEqual(["1.2.10"]);
    advance(REFETCH_AFTER_MS);
    await whatsNew.refresh();
    expect(versions(whatsNew.state())).toEqual(["1.2.11", "1.2.10"]);
  });

  it("moves the record forward only, keeping the closed window apart from the opened history", () => {
    const { whatsNew, disk } = harness({ record: { shown: "1.2.10", seen: "1.2.9" }, answers: [async () => file()] });
    whatsNew.start();
    whatsNew.save({ shown: "1.2.11" });
    whatsNew.save({ shown: "1.2.9", seen: "1.2.10" });
    expect(disk.record).toEqual({ shown: "1.2.11", seen: "1.2.10" });
  });

  it("refuses a record value that is no release version and changes nothing", () => {
    const { whatsNew, disk, events } = harness({ record: { shown: "1.2.10", seen: null }, answers: [async () => file()] });
    whatsNew.start();
    whatsNew.save({ shown: "../../etc" });
    expect(disk.record).toEqual({ shown: "1.2.10", seen: null });
    expect(events.some(([event]) => event === "whats_new.save_refused")).toBe(true);
  });

  it("reads an unreadable record as none and says so in the log", () => {
    const { whatsNew, events } = harness({ record: { unreadable: "not JSON" }, answers: [async () => file()] });
    whatsNew.start();
    expect(whatsNew.state().record).toBeNull();
    expect(events[0]).toEqual(["whats_new.record_unreadable", { detail: "not JSON" }]);
  });
});

describe("fetchReleasesFile", () => {
  const answer = (body: string, init: ResponseInit = {}) => async () => new Response(body, init);

  it("refuses a file over 1 MiB, by its declared length and by its body", async () => {
    const big = "x".repeat(RELEASES_BYTES_CAP + 1);
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer(big))).rejects.toThrow("exceeds 1 MiB");
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer("{}", { headers: { "content-length": String(RELEASES_BYTES_CAP + 1) } }))).rejects.toThrow("exceeds 1 MiB");
  });

  it("says the HTTP status of a failed answer", async () => {
    await expect(fetchReleasesFile("https://withhide.dev/app/releases.json", answer("missing", { status: 404 }))).rejects.toThrow("HTTP 404");
  });
});
