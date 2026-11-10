import { beforeEach, expect, it, vi } from "vitest";
import { APP_HOSTS_KEY, appStore, browserStore, bundlePath, HOSTS_KEY, MAX_HOSTS, readHosts, type PairedHost } from "./hosts";

// The Keychain the app's plugin keeps, as a map: what a call returns is what the native side would.
const held = vi.hoisted(() => new Map<string, { noiseKey: string; credential: string | null }>());
// How long a native call takes, so a test can interleave two operations the way a slow Keychain does.
const delay = vi.hoisted(() => ({ save: 0, list: 0 }));
const later = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
vi.mock("./appBridge", () => ({
  keychain: {
    save: async (room: string, item: { noiseKey: string; credential: string | null }) => { await later(delay.save); held.set(room, item); },
    read: async (room: string) => held.get(room) ?? null,
    remove: async (room: string) => { held.delete(room); },
    list: async () => { const rooms = [...held.keys()]; await later(delay.list); return rooms.concat([...held.keys()].filter((room) => !rooms.includes(room))); },
  },
}));
beforeEach(() => { held.clear(); delay.save = 0; delay.list = 0; });

function storage() {
  const entries = new Map<string, string>();
  return { getItem: (key: string) => entries.get(key) ?? null, setItem: (key: string, value: string) => { entries.set(key, value); } } as Storage;
}
const host: PairedHost = { room: "a".repeat(43), name: "기기 <img src=x> 日本語", noiseKey: "b".repeat(43), credential: "credential", protocol: 1, lastSeen: null };

it("retains each device's credential and protocol, replaces that device, and never stores the QR secret", async () => {
  const store = storage();
  const hosts = browserStore(store);
  await hosts.save({ ...host, code: "single-use secret" } as PairedHost);
  await hosts.save({ ...host, room: "c".repeat(43), protocol: 2 });
  await hosts.save({ ...host, lastSeen: 123 });
  expect(readHosts(store)).toEqual([{ ...host, lastSeen: 123 }, { ...host, room: "c".repeat(43), protocol: 2 }]);
  expect(store.getItem(HOSTS_KEY)).not.toContain("single-use secret");
  await hosts.forget(host.room);
  expect(readHosts(store).map((entry) => entry.room)).toEqual(["c".repeat(43)]);
});

it("keeps a revoked device's name and room without its credential, and a new pairing replaces it", async () => {
  const store = storage();
  // A list written before revoked rows existed reads unchanged.
  store.setItem(HOSTS_KEY, JSON.stringify([host, { ...host, room: "c".repeat(43) }]));
  await browserStore(store).revoke(host.room);
  expect(readHosts(store)).toEqual([{ ...host, credential: null }, { ...host, room: "c".repeat(43) }]);
  await browserStore(store).save({ ...host, credential: "fresh" });
  expect(readHosts(store)[0]).toEqual({ ...host, credential: "fresh" });
  expect(() => readHosts({ getItem: () => JSON.stringify([{ ...host, credential: "" }]) })).toThrow("host_storage_invalid");
});

it("rejects corrupt or unavailable storage and a seventeenth device without losing the existing list", async () => {
  const store = storage();
  for (let n = 0; n < MAX_HOSTS; n++) await browserStore(store).save({ ...host, room: String.fromCharCode(65 + n).repeat(43) });
  await expect(browserStore(store).save(host)).rejects.toThrow("host_limit");
  expect(readHosts(store)).toHaveLength(MAX_HOSTS);
  expect(() => readHosts({ getItem: () => "{" })).toThrow();
  expect(() => readHosts({ getItem: () => JSON.stringify([{ ...host, noiseKey: "broken" }]) })).toThrow("host_storage_invalid");
  await expect(browserStore({ ...store, getItem: store.getItem, setItem: () => { throw new Error("quota"); } } as Storage).save({ ...host, room: "A".repeat(43) })).rejects.toThrow("quota");
});

it("opens only explicitly available protocol bundles, with no newest-version fallback", () => {
  expect(bundlePath(1, [1, 3])).toBe("/p/1/");
  expect(bundlePath(3, [1, 3])).toBe("/p/3/");
  for (const version of [0, 2, -1, 1.5, NaN, Infinity]) expect(bundlePath(version, [1, 3])).toBeNull();
});

it("keeps the app's secrets in the Keychain and nothing secret in the web storage", async () => {
  const store = storage();
  const hosts = appStore(store);
  await hosts.save({ ...host, credential: "the-credential" });
  await hosts.save({ ...host, room: "c".repeat(43), noiseKey: "d".repeat(43), credential: "other" });
  const written = store.getItem(APP_HOSTS_KEY) ?? "";
  for (const secret of ["the-credential", "other", host.noiseKey, "d".repeat(43)]) expect(written).not.toContain(secret);
  expect(store.getItem(HOSTS_KEY)).toBeNull();
  expect(await hosts.list()).toEqual([{ ...host, credential: "the-credential" }, { ...host, room: "c".repeat(43), noiseKey: "d".repeat(43), credential: "other" }]);
  await hosts.revoke(host.room);
  expect((await hosts.list())[0]).toEqual({ ...host, credential: null });
  expect(held.get(host.room)).toEqual({ noiseKey: host.noiseKey, credential: null });
  await hosts.forget(host.room);
  expect((await hosts.list()).map((entry) => entry.room)).toEqual(["c".repeat(43)]);
  expect([...held.keys()]).toEqual(["c".repeat(43)]);
});

it("drops a row whose Keychain item is gone and removes an item no row names", async () => {
  const store = storage();
  const hosts = appStore(store);
  await hosts.save(host);
  await hosts.save({ ...host, room: "c".repeat(43) });
  held.delete(host.room);
  held.set("z".repeat(43), { noiseKey: "e".repeat(43), credential: "orphan" });
  expect((await hosts.list()).map((entry) => entry.room)).toEqual(["c".repeat(43)]);
  expect(JSON.parse(store.getItem(APP_HOSTS_KEY) ?? "[]").map((row: PairedHost) => row.room)).toEqual(["c".repeat(43)]);
  expect([...held.keys()]).toEqual(["c".repeat(43)]);
});

it("refuses a seventeenth computer in the app before it writes a secret", async () => {
  const hosts = appStore(storage());
  for (let n = 0; n < MAX_HOSTS; n++) await hosts.save({ ...host, room: String.fromCharCode(65 + n).repeat(43) });
  await expect(hosts.save(host)).rejects.toThrow("host_limit");
  expect(held.size).toBe(MAX_HOSTS);
});

it("runs a list that was asked while a computer is being saved after the save, so the new item is not taken for an orphan", async () => {
  const store = storage();
  const hosts = appStore(store);
  delay.save = 20;
  delay.list = 30;
  const saving = hosts.save(host);
  // Asked while the Keychain write is in flight: its answer is read after the item exists, its row list before the row does.
  const listing = hosts.list();
  await saving;
  expect((await listing).map((entry) => entry.room)).toEqual([host.room]);
  expect([...held.keys()]).toEqual([host.room]);
  expect(JSON.parse(store.getItem(APP_HOSTS_KEY) ?? "[]").map((row: PairedHost) => row.room)).toEqual([host.room]);
});

it("keeps answering in order after an operation was refused", async () => {
  const hosts = appStore(storage());
  for (let n = 0; n < MAX_HOSTS; n++) await hosts.save({ ...host, room: String.fromCharCode(65 + n).repeat(43) });
  const refused = hosts.save(host);
  const listing = hosts.list();
  await expect(refused).rejects.toThrow("host_limit");
  expect(await listing).toHaveLength(MAX_HOSTS);
});
