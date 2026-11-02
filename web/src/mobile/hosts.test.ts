import { expect, it } from "vitest";
import { bundlePath, forgetHost, HOSTS_KEY, MAX_HOSTS, readHosts, saveHost, type PairedHost } from "./hosts";

function storage() {
  const entries = new Map<string, string>();
  return { getItem: (key: string) => entries.get(key) ?? null, setItem: (key: string, value: string) => { entries.set(key, value); } };
}
const host: PairedHost = { room: "a".repeat(43), name: "기기 <img src=x> 日本語", noiseKey: "b".repeat(43), credential: "credential", protocol: 1, lastSeen: null };

it("retains each device's credential and protocol, replaces that device, and never stores the QR secret", () => {
  const store = storage();
  saveHost(store, { ...host, code: "single-use secret" } as PairedHost);
  saveHost(store, { ...host, room: "c".repeat(43), protocol: 2 });
  saveHost(store, { ...host, lastSeen: 123 });
  expect(readHosts(store)).toEqual([{ ...host, lastSeen: 123 }, { ...host, room: "c".repeat(43), protocol: 2 }]);
  expect(store.getItem(HOSTS_KEY)).not.toContain("single-use secret");
  forgetHost(store as Storage, host.room);
  expect(readHosts(store).map((entry) => entry.room)).toEqual(["c".repeat(43)]);
});

it("rejects corrupt or unavailable storage and a seventeenth device without losing the existing list", () => {
  const store = storage();
  for (let n = 0; n < MAX_HOSTS; n++) saveHost(store, { ...host, room: String.fromCharCode(65 + n).repeat(43) });
  expect(() => saveHost(store, host)).toThrow("host_limit");
  expect(readHosts(store)).toHaveLength(MAX_HOSTS);
  expect(() => readHosts({ getItem: () => "{" })).toThrow();
  expect(() => readHosts({ getItem: () => JSON.stringify([{ ...host, noiseKey: "broken" }]) })).toThrow("host_storage_invalid");
  expect(() => saveHost({ ...store, setItem: () => { throw new Error("quota"); } }, { ...host, room: "A".repeat(43) })).toThrow("quota");
});

it("opens only explicitly available protocol bundles, with no newest-version fallback", () => {
  expect(bundlePath(1, [1, 3])).toBe("/p/1/");
  expect(bundlePath(3, [1, 3])).toBe("/p/3/");
  for (const version of [0, 2, -1, 1.5, NaN, Infinity]) expect(bundlePath(version, [1, 3])).toBeNull();
});
