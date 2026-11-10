import { serviceWorkerAllowed } from "./appHost";

/** Only paired-device identity and credentials persist, never a QR code or conversation. */
export interface PairedHost {
  room: string;
  name: string;
  noiseKey: string;
  /** Null once the device refused it: a revoked credential never connects again, and the row stays until removed or paired anew. */
  credential: string | null;
  protocol: number;
  lastSeen: number | null;
}
export const HOSTS_KEY = "hide.phone.hosts.v1";
export const MAX_HOSTS = 16;
const KEY = /^[A-Za-z0-9_-]{43}$/;
const pushWorkers = new Map<string, Promise<ServiceWorkerRegistration>>();

export function readHosts(storage: Pick<Storage, "getItem">): PairedHost[] {
  const raw = storage.getItem(HOSTS_KEY);
  if (raw === null) return [];
  if (raw.length > 32_768) throw new Error("host_storage_invalid");
  const hosts: unknown = JSON.parse(raw);
  if (!Array.isArray(hosts) || hosts.length > MAX_HOSTS) throw new Error("host_storage_invalid");
  const rooms = new Set<string>();
  for (const h of hosts) {
    if (!h || typeof h !== "object" || typeof h.room !== "string" || typeof h.noiseKey !== "string" || !KEY.test(h.room) || !KEY.test(h.noiseKey) ||
        typeof h.name !== "string" || !h.name.trim() || new TextEncoder().encode(h.name).length > 128 ||
        (h.credential !== null && (typeof h.credential !== "string" || !h.credential || h.credential.length > 512)) ||
        !Number.isSafeInteger(h.protocol) || h.protocol < 1 ||
        (h.lastSeen !== null && (!Number.isSafeInteger(h.lastSeen) || h.lastSeen < 0)) || rooms.has(h.room)) {
      throw new Error("host_storage_invalid");
    }
    rooms.add(h.room);
  }
  return hosts as PairedHost[];
}

export function saveHost(storage: Pick<Storage, "getItem" | "setItem">, host: PairedHost): void {
  const hosts = readHosts(storage);
  const index = hosts.findIndex((h) => h.room === host.room);
  if (index < 0) {
    if (hosts.length >= MAX_HOSTS) throw new Error("host_limit");
    hosts.push(host);
  } else hosts[index] = host;
  // Serialize named fields only; even a caller passing a QR cannot persist its code.
  const encoded = JSON.stringify(hosts.map(({ room, name, noiseKey, credential, protocol, lastSeen }) =>
    ({ room, name, noiseKey, credential, protocol, lastSeen })));
  readHosts({ getItem: () => encoded });
  storage.setItem(HOSTS_KEY, encoded);
}

export function forgetHost(storage: Storage, room: string): void {
  storage.setItem(HOSTS_KEY, JSON.stringify(readHosts(storage).filter((h) => h.room !== room)));
}

/** The device refused this phone's credential: keep its name and room, drop the credential. */
export function revokeHost(storage: Pick<Storage, "getItem" | "setItem">, room: string): void {
  const host = readHosts(storage).find((h) => h.room === room);
  if (host?.credential) saveHost(storage, { ...host, credential: null });
}

/** One registration request per saved device, shared by words, notices and subscription. */
export function registerHostPush(room: string): Promise<ServiceWorkerRegistration> {
  if (!readHosts(localStorage).some((host) => host.room === room && host.credential)) return Promise.reject(new Error("host_not_saved"));
  const existing = pushWorkers.get(room);
  if (existing) return existing;
  if (pushWorkers.size >= MAX_HOSTS) return Promise.reject(new Error("host_limit"));
  const pending = navigator.serviceWorker.register("/sw.js", { scope: `/push/${room}/` })
    .catch((error) => { pushWorkers.delete(room); throw error; });
  pushWorkers.set(room, pending);
  return pending;
}

async function removeHostPush(room: string): Promise<void> {
  if (serviceWorkerAllowed()) {
    // A removal waits for its admitted registration; it cannot leave a late worker behind.
    const pending = pushWorkers.get(room);
    if (pending) await pending.catch(() => undefined);
    const worker = await navigator.serviceWorker.getRegistration(`/push/${room}/`);
    if (worker && new URL(worker.scope).pathname === `/push/${room}/`) {
      // A Safari tab has workers without push; such a worker holds no subscription.
      const push = worker.pushManager as PushManager | undefined;
      if (push) await (await push.getSubscription())?.unsubscribe();
      if (!(await worker.unregister())) throw new Error("worker_remove_failed");
    }
    if ("caches" in window) await (await caches.open("hide-phone-words")).delete(`/push/${room}/words.json`);
  }
  pushWorkers.delete(room);
}

/** A revoked device keeps no notification worker; a failed removal is not the operator's to act on, so it goes to the log. */
export function dropHostPush(room: string): Promise<void> {
  return removeHostPush(room).catch((error: unknown) => { console.warn("phone.push_remove_failed", { room, error: String(error) }); });
}

export async function removeHost(room: string): Promise<void> {
  await removeHostPush(room);
  forgetHost(localStorage, room);
}

export function bundlePath(protocol: number, supported: readonly number[]): string | null {
  return Number.isSafeInteger(protocol) && protocol > 0 && supported.includes(protocol) ? `/p/${protocol}/` : null;
}

export function hostLink(host: PairedHost, tag?: string): string {
  const query = new URLSearchParams({ host: host.room });
  if (tag) query.set("open", tag);
  return `/?${query}`;
}
