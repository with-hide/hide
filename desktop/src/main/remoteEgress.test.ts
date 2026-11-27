// Where a remote screen's pages send their requests and keep their logins
// (docs/BROWSER_DISPLAYS.md, A page in another computer's screen).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { remotePagePartition } from "./browserSync";
import type { HostLog } from "./log";
import { MAX_STORED, RemoteEgress, RemoteStorage, type Clearable } from "./remoteEgress";

vi.mock("electron", () => ({ session: {} }));

const KEY = "ab".repeat(32);
const P1 = "11".repeat(16);
const P2 = "22".repeat(16);
const CORE = "laptop-core";
const folders: string[] = [];
afterEach(() => {
  for (const folder of folders.splice(0)) fs.rmSync(folder, { recursive: true, force: true });
});

function answer(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

function egress(fetched: (url: string, init: RequestInit) => Response) {
  const calls: string[] = [];
  const pages = { setProxy: vi.fn(async (config: unknown) => { calls.push(`proxy ${JSON.stringify(config)}`); }), closeAllConnections: vi.fn(async () => { calls.push("close"); }) };
  const fetch = vi.fn(async (url: string, init: RequestInit) => fetched(url, init));
  const subject = new RemoteEgress({ daemon: () => ({ origin: "http://127.0.0.1:7", token: "t" }), log: { event: vi.fn() } as unknown as HostLog, session: () => pages, owner: 4242, fetch: fetch as unknown as typeof globalThis.fetch });
  return { subject, pages, calls, fetch };
}

describe("a remote screen's proxy", () => {
  it("is set exactly, before the pair counts as ready, with its connections closed after", async () => {
    const { subject, calls, fetch } = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", pairing: P1, state: "ready" }));
    expect(subject.ready("mini", KEY)).toBe(false);
    expect(await subject.ensure("mini", KEY)).toBe("ready");
    expect(fetch.mock.calls[0]![0]).toBe(`http://127.0.0.1:7/peer-screen/mini/net?key=${KEY}&owner=4242`);
    expect(calls).toEqual([`proxy ${JSON.stringify({ mode: "fixed_servers", proxyRules: "http://127.0.0.1:54321", proxyBypassRules: "<-loopback>" })}`, "close"]);
    expect(subject.ready("mini", KEY)).toBe(true);
    // The same listener again sets nothing again.
    expect(await subject.ensure("mini", KEY)).toBe("ready");
    expect(calls).toHaveLength(2);
  });

  it("answers its proxy login by port, and no other", async () => {
    const { subject } = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", pairing: P1, state: "ready" }));
    await subject.ensure("mini", KEY);
    expect(subject.credentials("127.0.0.1", 54321)).toEqual({ username: "hide", password: "0123456789abcdef0123456789abcdef" });
    expect(subject.credentials("127.0.0.1", 54322)).toBeNull();
    expect(subject.credentials("proxy.example", 54321)).toBeNull();
    subject.release("mini", KEY);
    expect(subject.credentials("127.0.0.1", 54321)).toBeNull();
    expect(subject.ready("mini", KEY)).toBe(false);
  });

  it("reports why a request fails now, and an older daemon here", async () => {
    expect(await egress(() => answer(503, { reason: "not_paired" })).subject.ensure("mini", KEY)).toBe("not_paired");
    expect(await egress(() => answer(404, {})).subject.ensure("mini", KEY)).toBe("remote_update_here");
    const paused = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", pairing: P1, state: "paused_there" }));
    expect(await paused.subject.ensure("mini", KEY)).toBe("paused_there");
    // The listener is kept through a pause: its proxy is set and reached again once resumed.
    expect(paused.subject.ready("mini", KEY)).toBe(true);
    expect(await egress(() => answer(200, { port: 70000, secret: "x", pairing: P1, state: "ready" })).subject.ensure("mini", KEY)).toBe("not_connected");
    // An answer that names no pairing has no storage to give its pages.
    expect(await egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", state: "ready" })).subject.ensure("mini", KEY)).toBe("not_connected");
  });

  it("names its pages' partition by the pairing, and moves them to a new pairing's with its proxy set first", async () => {
    let pairing = P1;
    const repaired: string[] = [];
    const proxied: string[] = [];
    const pages = { setProxy: vi.fn(async () => undefined), closeAllConnections: vi.fn(async () => undefined) };
    const subject = new RemoteEgress({
      daemon: () => ({ origin: "http://127.0.0.1:7", token: "t" }), log: { event: vi.fn() } as unknown as HostLog, owner: 4242,
      session: (partition) => { proxied.push(partition); return pages; },
      fetch: (async () => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", pairing, state: "ready" })) as unknown as typeof fetch,
      repaired: (device) => repaired.push(device),
    });
    expect(subject.partition("mini", KEY)).toBeNull();
    await subject.ensure("mini", KEY);
    expect(subject.partition("mini", KEY)).toBe(remotePagePartition("mini", KEY, P1));
    // A pause, a restart of the daemon: the same pairing, the same storage.
    await subject.ensure("mini", KEY);
    expect(proxied).toEqual([remotePagePartition("mini", KEY, P1)]);
    expect(repaired).toEqual([]);
    pairing = P2;
    await subject.ensure("mini", KEY);
    expect(subject.partition("mini", KEY)).toBe(remotePagePartition("mini", KEY, P2));
    expect(proxied).toEqual([remotePagePartition("mini", KEY, P1), remotePagePartition("mini", KEY, P2)]);
    expect(subject.ready("mini", KEY)).toBe(true);
    expect(repaired).toEqual(["mini"]);
  });

  it("lets go of a listener its ask was opening when released, and asks again only after that", async () => {
    const events: string[] = [];
    let answerFirst: () => void = () => undefined;
    let asks = 0;
    const subject = new RemoteEgress({
      daemon: () => ({ origin: "http://127.0.0.1:7", token: "t" }), log: { event: vi.fn() } as unknown as HostLog, owner: 4242,
      session: () => ({ setProxy: vi.fn(async () => undefined), closeAllConnections: vi.fn(async () => undefined) }),
      fetch: (async (_url: string, init: RequestInit) => {
        events.push(init.method ?? "GET");
        if (init.method === "POST" && asks++ === 0) await new Promise<void>((done) => { answerFirst = done; });
        return answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", pairing: P1, state: "ready" });
      }) as unknown as typeof fetch,
    });
    const first = subject.ensure("mini", KEY);
    await vi.waitFor(() => expect(events).toEqual(["POST"]));
    subject.release("mini", KEY);
    const again = subject.ensure("mini", KEY);
    expect(events).toEqual(["POST"]);
    answerFirst();
    expect(await first).toBe("not_connected");
    expect(await again).toBe("ready");
    expect(events).toEqual(["POST", "DELETE", "POST"]);
    expect(subject.ready("mini", KEY)).toBe(true);
  });
});

function folder(): string {
  const made = fs.mkdtempSync(path.join(os.tmpdir(), "hide-remote-storage-"));
  folders.push(made);
  return made;
}

/** A session whose clear the test settles, step by step. */
function sessions(fail = false) {
  const steps: string[] = [];
  let release: () => void = () => undefined;
  const held = { wait: false };
  const made = new Map<string, Clearable>();
  const of = (partition: string): Clearable => {
    let session = made.get(partition);
    if (!session) {
      const step = (name: string) => async () => {
        steps.push(`${name} ${partition}`);
        if (name === "storage" && held.wait) await new Promise<void>((done) => { release = done; });
        if (name === "storage" && fail) throw new Error("disk full");
      };
      session = { closeAllConnections: step("close"), clearAuthCache: step("auth"), clearStorageData: step("storage"), clearCache: step("cache") } as unknown as Clearable;
      made.set(partition, session);
    }
    return session;
  };
  return { of, steps, held, release: () => release() };
}


/** A storage on `file`, its clears through `cleared`, its pages' closes into `closed`. */
function storageAt(file: string, cleared = sessions(), extra: Partial<ConstructorParameters<typeof RemoteStorage>[0]> = {}) {
  const closed: string[] = [];
  const log = { event: vi.fn() };
  const storage = new RemoteStorage({ file, log: log as unknown as HostLog, session: cleared.of, closePages: (device) => closed.push(device), ...extra });
  return { storage, closed, cleared, log };
}

const PAIRED = new Set([`mini\u0000${KEY}`]);

describe("a remote screen's login storage", () => {
  it("admits no pair before the shell names its core's set, then lets each paired one make its pages", () => {
    const retried: string[] = [];
    const { storage } = storageAt(path.join(folder(), "remote-partitions.json"), sessions(), { cleared: (device) => retried.push(device) });
    expect(storage.admit("mini", KEY, P1)).toBe("waiting");
    storage.keep(CORE, PAIRED);
    expect(retried).toEqual(["mini"]);
    expect(storage.admit("mini", KEY, P1)).toBe("live");
    storage.keep(CORE, PAIRED);
    expect(retried).toEqual(["mini"]);
  });

  it("is cleared in order, the auth cache with it, once its pair ends, and a re-pair of the same key waits for the clear and starts empty", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const { storage, closed, cleared } = storageAt(file);
    cleared.held.wait = true;
    storage.keep(CORE, PAIRED);
    expect(storage.admit("mini", KEY, P1)).toBe("live");
    storage.keep(CORE, new Set());
    expect(closed).toEqual(["mini"]);
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries[0]).toMatchObject({ device: "mini", key: KEY, pairing: P1, core: CORE, state: "clearing" });
    expect(storage.admit("mini", KEY, P1)).toBe("clearing");
    await vi.waitFor(() => expect(cleared.steps).toHaveLength(3));
    cleared.release();
    await storage.settled();
    const partition = remotePagePartition("mini", KEY, P1);
    expect(cleared.steps).toEqual([`close ${partition}`, `auth ${partition}`, `storage ${partition}`, `cache ${partition}`]);
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries).toEqual([]);
    expect(storage.admit("mini", KEY, P1)).toBe("live");
  });

  it("stays failed when a clear fails, keeps every page off it, and is cleared at the next launch", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const { storage } = storageAt(file, sessions(true));
    storage.keep(CORE, PAIRED);
    storage.admit("mini", KEY, P1);
    storage.keep(CORE, new Set());
    await storage.settled();
    expect(storage.state("mini", KEY, P1)).toBe("failed");
    expect(storage.admit("mini", KEY, P1)).toBe("clearing");
    const next = sessions();
    const launched = storageAt(file, next).storage;
    await launched.settled();
    expect(next.steps.map((step) => step.split(" ")[0])).toEqual(["close", "auth", "storage", "cache"]);
    expect(launched.state("mini", KEY, P1)).toBeNull();
  });

  // The order the review asked to close: the clear's own record and the
  // clear both fail, the app quits with the entry still live on disk, and
  // the machine is paired again under the same key. The new pairing's pages
  // get a partition of their own, and the old one is cleared, never used.
  it("never hands an ended pairing's storage to a new pairing of the same key, whatever its clear left", async () => {
    const dir = folder();
    const file = path.join(dir, "remote-partitions.json");
    const partitions = path.join(dir, "Partitions");
    const old = remotePagePartition("mini", KEY, P1);
    fs.mkdirSync(path.join(partitions, old.slice("persist:".length)), { recursive: true });
    const first = storageAt(file, sessions(true), { partitions });
    first.storage.keep(CORE, PAIRED);
    expect(first.storage.admit("mini", KEY, P1)).toBe("live");
    // The index cannot be written any more: a folder stands where it is staged.
    fs.mkdirSync(`${file}.${process.pid}.tmp`);
    first.storage.keep(CORE, new Set());
    await first.storage.settled();
    expect(first.storage.state("mini", KEY, P1)).toBe("failed");
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries[0].state).toBe("live");
    expect(first.log.event).toHaveBeenCalledWith("remote.storage_index_failed", expect.anything());
    fs.rmdirSync(`${file}.${process.pid}.tmp`);

    // Quit; launched again with the pair, paired again under the same key.
    const next = sessions();
    const launched = storageAt(file, next, { partitions });
    launched.storage.keep(CORE, PAIRED);
    expect(next.steps).toEqual([]);
    expect(launched.storage.admit("mini", KEY, P2)).toBe("live");
    expect(remotePagePartition("mini", KEY, P2)).not.toBe(old);
    await launched.storage.settled();
    expect(next.steps).toEqual([`close ${old}`, `auth ${old}`, `storage ${old}`, `cache ${old}`]);
    expect(launched.closed).toEqual(["mini"]);
    expect(launched.storage.state("mini", KEY, P1)).toBeNull();
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries.map((entry: { pairing: string }) => entry.pairing)).toEqual([P2]);
  });

  it("keeps a live pair's storage across a launch and a set that names it", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const first = storageAt(file).storage;
    first.keep(CORE, PAIRED);
    first.admit("mini", KEY, P1);
    const cleared = sessions();
    const second = storageAt(file, cleared).storage;
    second.keep(CORE, PAIRED);
    expect(second.admit("mini", KEY, P1)).toBe("live");
    await second.settled();
    expect(cleared.steps).toEqual([]);
    expect(second.state("mini", KEY, P1)).toBe("live");
  });

  // The same profile attached to another core (another state folder) is told
  // that core's set, which names none of the first core's machines.
  it("clears only the logins of the core that named the set", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const { storage, cleared } = storageAt(file);
    storage.keep(CORE, PAIRED);
    storage.admit("mini", KEY, P1);
    storage.keep("another-core", new Set());
    await storage.settled();
    expect(cleared.steps).toEqual([]);
    expect(storage.state("mini", KEY, P1)).toBe("live");
    storage.keep(CORE, new Set());
    await storage.settled();
    expect(storage.state("mini", KEY, P1)).toBeNull();
  });

  it("deletes at launch the partition folders its index does not name, and only those", () => {
    const dir = folder();
    const file = path.join(dir, "remote-partitions.json");
    const partitions = path.join(dir, "Partitions");
    const first = storageAt(file).storage;
    first.keep(CORE, PAIRED);
    first.admit("mini", KEY, P1);
    const named = remotePagePartition("mini", KEY, P1).slice("persist:".length);
    const unnamed = remotePagePartition("pc", KEY, P2).slice("persist:".length);
    for (const name of [named, unnamed, "hide-browser-web", "hide-remote-page-not-ours"]) fs.mkdirSync(path.join(partitions, name), { recursive: true });
    storageAt(file, sessions(), { partitions });
    expect(fs.readdirSync(partitions).sort()).toEqual([named, "hide-browser-web", "hide-remote-page-not-ours"].sort());
    // An index that cannot be read names none.
    fs.writeFileSync(file, "{ not json");
    storageAt(file, sessions(), { partitions });
    expect(fs.readdirSync(partitions).sort()).toEqual(["hide-browser-web", "hide-remote-page-not-ours"]);
  });

  it("refuses a new pair's storage when the index is full rather than drop one that still needs clearing, and says so once", () => {
    const { storage, log } = storageAt(path.join(folder(), "remote-partitions.json"));
    storage.keep(CORE, new Set());
    for (let n = 0; n < MAX_STORED; n++) expect(storage.admit(`machine-${n}`, KEY, P1)).toBe("live");
    expect(storage.admit("one-more", KEY, P1)).toBe("full");
    expect(storage.admit("one-more", KEY, P1)).toBe("full");
    expect(log.event.mock.calls.filter(([kind]) => kind === "remote.storage_full")).toHaveLength(1);
  });
});
