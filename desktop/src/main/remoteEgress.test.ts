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
    const { subject, calls, fetch } = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", state: "ready" }));
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
    const { subject } = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", state: "ready" }));
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
    const paused = egress(() => answer(200, { port: 54321, secret: "0123456789abcdef0123456789abcdef", state: "paused_there" }));
    expect(await paused.subject.ensure("mini", KEY)).toBe("paused_there");
    // The listener is kept through a pause: its proxy is set and reached again once resumed.
    expect(paused.subject.ready("mini", KEY)).toBe(true);
    expect(await egress(() => answer(200, { port: 70000, secret: "x", state: "ready" })).subject.ensure("mini", KEY)).toBe("not_connected");
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

describe("a remote screen's login storage", () => {
  it("is cleared in order, the auth cache with it, once its pair ends, and a re-pair of the same key waits for the clear and starts empty", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const cleared = sessions();
    cleared.held.wait = true;
    const closed: string[] = [];
    const storage = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: cleared.of, closePages: (device) => closed.push(device) });
    expect(storage.admit("mini", KEY)).toBe("live");
    storage.keep(new Set());
    expect(closed).toEqual(["mini"]);
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries[0].state).toBe("clearing");
    expect(storage.admit("mini", KEY)).toBe("clearing");
    await vi.waitFor(() => expect(cleared.steps).toHaveLength(3));
    cleared.release();
    await storage.settled();
    const partition = remotePagePartition("mini", KEY);
    expect(cleared.steps).toEqual([`close ${partition}`, `auth ${partition}`, `storage ${partition}`, `cache ${partition}`]);
    expect(JSON.parse(fs.readFileSync(file, "utf8")).entries).toEqual([]);
    expect(storage.admit("mini", KEY)).toBe("live");
  });

  it("stays failed when a clear fails, keeps every page off it, and is cleared at the next launch", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const failing = sessions(true);
    const storage = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: failing.of, closePages: () => undefined });
    storage.admit("mini", KEY);
    storage.keep(new Set());
    await storage.settled();
    expect(storage.state("mini", KEY)).toBe("failed");
    expect(storage.admit("mini", KEY)).toBe("clearing");
    const next = sessions();
    const launched = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: next.of, closePages: () => undefined });
    await launched.settled();
    expect(next.steps.map((step) => step.split(" ")[0])).toEqual(["close", "auth", "storage", "cache"]);
    expect(launched.state("mini", KEY)).toBeNull();
  });

  it("clears an entry a quit left clearing before any page uses it", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    fs.writeFileSync(file, JSON.stringify({ entries: [{ device: "mini", key: KEY, state: "clearing" }] }));
    const cleared = sessions();
    const storage = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: cleared.of, closePages: () => undefined });
    expect(storage.admit("mini", KEY)).toBe("clearing");
    await storage.settled();
    expect(cleared.steps).toHaveLength(4);
    expect(storage.admit("mini", KEY)).toBe("live");
  });

  it("keeps a live pair's storage across a launch and a set that names it", async () => {
    const file = path.join(folder(), "remote-partitions.json");
    const first = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: sessions().of, closePages: () => undefined });
    first.admit("mini", KEY);
    const cleared = sessions();
    const second = new RemoteStorage({ file, log: { event: vi.fn() } as unknown as HostLog, session: cleared.of, closePages: () => undefined });
    second.keep(new Set([`mini\u0000${KEY}`]));
    await second.settled();
    expect(cleared.steps).toEqual([]);
    expect(second.state("mini", KEY)).toBe("live");
  });

  it("refuses a new pair's storage when the index is full rather than drop one that still needs clearing", () => {
    const file = path.join(folder(), "remote-partitions.json");
    const log = { event: vi.fn() };
    const storage = new RemoteStorage({ file, log: log as unknown as HostLog, session: sessions().of, closePages: () => undefined });
    for (let n = 0; n < MAX_STORED; n++) expect(storage.admit(`machine-${n}`, KEY)).toBe("live");
    expect(storage.admit("one-more", KEY)).toBe("full");
    expect(log.event).toHaveBeenCalledWith("remote.storage_full", {});
  });
});
