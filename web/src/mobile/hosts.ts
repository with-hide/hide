/** Only paired-device identity and credentials persist, never a QR code or conversation. */
export interface PairedHost {
  room: string;
  name: string;
  noiseKey: string;
  credential: string;
  protocol: number;
  lastSeen: number | null;
}
export const HOSTS_KEY = "hide.phone.hosts.v1";
export const MAX_HOSTS = 16;
const KEY = /^[A-Za-z0-9_-]{43}$/;

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
        typeof h.credential !== "string" || !h.credential || h.credential.length > 512 ||
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

export async function removeHost(room: string): Promise<void> {
  if ("serviceWorker" in navigator) {
    const worker = await navigator.serviceWorker.getRegistration(`/push/${room}/`);
    if (worker && new URL(worker.scope).pathname === `/push/${room}/`) {
      await (await worker.pushManager.getSubscription())?.unsubscribe();
      if (!(await worker.unregister())) throw new Error("worker_remove_failed");
    }
    if ("caches" in window) await (await caches.open("hide-phone-words")).delete(`/push/${room}/words.json`);
  }
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
