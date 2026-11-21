import { serviceWorkerAllowed, appHost } from "./appHost";
import { keychain } from "./appBridge";

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
/** The app's list: the same rows without the two secrets (the credential and the computer's key), which are in the Keychain. */
export const APP_HOSTS_KEY = "hide.phone.hosts.app.v1";
export const MAX_HOSTS = 16;
const KEY = /^[A-Za-z0-9_-]{43}$/;
const pushWorkers = new Map<string, Promise<ServiceWorkerRegistration>>();

/** Where paired computers are kept: a browser keeps all of a row in its storage, the app keeps the secrets in the Keychain. */
export interface HostStore {
  list(): Promise<PairedHost[]>;
  save(host: PairedHost): Promise<void>;
  forget(room: string): Promise<void>;
  /** The device refused this phone's credential: keep its name and room, drop the credential. */
  revoke(room: string): Promise<void>;
}

type Row = Omit<PairedHost, "noiseKey" | "credential"> & Partial<Pick<PairedHost, "noiseKey" | "credential">>;

function checked(hosts: unknown, secrets: boolean): PairedHost[] | Row[] {
  if (!Array.isArray(hosts) || hosts.length > MAX_HOSTS) throw new Error("host_storage_invalid");
  const rooms = new Set<string>();
  for (const h of hosts) {
    if (!h || typeof h !== "object" || typeof h.room !== "string" || !KEY.test(h.room) ||
        (secrets && (typeof h.noiseKey !== "string" || !KEY.test(h.noiseKey))) ||
        (secrets && h.credential !== null && (typeof h.credential !== "string" || !h.credential || h.credential.length > 512)) ||
        typeof h.name !== "string" || !h.name.trim() || new TextEncoder().encode(h.name).length > 128 ||
        !Number.isSafeInteger(h.protocol) || h.protocol < 1 ||
        (h.lastSeen !== null && (!Number.isSafeInteger(h.lastSeen) || h.lastSeen < 0)) || rooms.has(h.room)) {
      throw new Error("host_storage_invalid");
    }
    rooms.add(h.room);
  }
  return hosts as PairedHost[];
}

function parse(storage: Pick<Storage, "getItem">, key: string, secrets: boolean): PairedHost[] | Row[] {
  const raw = storage.getItem(key);
  if (raw === null) return [];
  if (raw.length > 32_768) throw new Error("host_storage_invalid");
  return checked(JSON.parse(raw), secrets);
}

/** Every row of a browser's list, validated; a bad list is an error, never an empty one. */
export function readHosts(storage: Pick<Storage, "getItem">): PairedHost[] {
  return parse(storage, HOSTS_KEY, true) as PairedHost[];
}

/** The site's store: the whole row, secrets included, in this browser's storage (the address stays the only other holder). */
export function browserStore(storage: Storage): HostStore {
  const write = (hosts: PairedHost[]) => {
    // Serialize named fields only; even a caller passing a QR cannot persist its code.
    const encoded = JSON.stringify(hosts.map(({ room, name, noiseKey, credential, protocol, lastSeen }) => ({ room, name, noiseKey, credential, protocol, lastSeen })));
    checked(JSON.parse(encoded), true);
    storage.setItem(HOSTS_KEY, encoded);
  };
  const save = (host: PairedHost) => {
    const hosts = readHosts(storage);
    const index = hosts.findIndex((h) => h.room === host.room);
    if (index < 0) {
      if (hosts.length >= MAX_HOSTS) throw new Error("host_limit");
      hosts.push(host);
    } else hosts[index] = host;
    write(hosts);
  };
  return {
    list: async () => readHosts(storage),
    save: async (host) => save(host),
    forget: async (room) => write(readHosts(storage).filter((h) => h.room !== room)),
    revoke: async (room) => {
      const host = readHosts(storage).find((h) => h.room === room);
      if (host?.credential) save({ ...host, credential: null });
    },
  };
}

/**
 * The app's store: the host list (name, room, protocol, last seen) in the web view's storage, where no secret is, and
 * each computer's credential and key in one Keychain item under its room. A row without its item cannot connect and a
 * item without its row belongs to nothing a person can see, so a list read drops both. Every operation runs on one
 * queue, in the order it was asked: a read that ran between a save's Keychain write and its row write would take the new
 * item for an orphan and delete it.
 */
export function appStore(storage: Storage): HostStore {
  const rows = () => parse(storage, APP_HOSTS_KEY, false) as Row[];
  const write = (hosts: Row[]) => {
    const encoded = JSON.stringify(hosts.map(({ room, name, protocol, lastSeen }) => ({ room, name, protocol, lastSeen })));
    checked(JSON.parse(encoded), false);
    storage.setItem(APP_HOSTS_KEY, encoded);
  };
  let tail: Promise<unknown> = Promise.resolve();
  const serial = <T>(operation: () => Promise<T>): Promise<T> => {
    const run = tail.then(operation, operation);
    tail = run.catch(() => undefined);
    return run;
  };
  const list = async (): Promise<PairedHost[]> => {
    const listed = rows();
    const held = new Set(await keychain.list());
    const found: PairedHost[] = [];
    for (const row of listed) {
      const item = held.has(row.room) ? await keychain.read(row.room) : null;
      if (item) found.push({ room: row.room, name: row.name, protocol: row.protocol, lastSeen: row.lastSeen, noiseKey: item.noiseKey, credential: item.credential });
    }
    if (found.length !== listed.length) write(found);
    for (const room of held) if (!found.some((host) => host.room === room)) await keychain.remove(room);
    return found;
  };
  const save = async (host: PairedHost) => {
    const listed = rows();
    const index = listed.findIndex((h) => h.room === host.room);
    if (index < 0 && listed.length >= MAX_HOSTS) throw new Error("host_limit");
    // The secret first: a failure between the two leaves an item with no row, which the next read removes.
    await keychain.save(host.room, { noiseKey: host.noiseKey, credential: host.credential });
    if (index < 0) listed.push(host); else listed[index] = host;
    write(listed);
  };
  return {
    list: () => serial(list),
    save: (host) => serial(() => save(host)),
    forget: (room) => serial(async () => {
      await keychain.remove(room);
      write(rows().filter((h) => h.room !== room));
    }),
    revoke: (room) => serial(async () => {
      const host = (await list()).find((h) => h.room === room);
      if (host?.credential) await save({ ...host, credential: null });
    }),
  };
}

let store: HostStore | undefined;
/** The store of this page: the Keychain-backed one inside the app, the browser's storage everywhere else. */
export function hostStore(): HostStore {
  return store ??= appHost() ? appStore(localStorage) : browserStore(localStorage);
}

/** One registration request per saved device, shared by words, notices and subscription. */
export async function registerHostPush(room: string): Promise<ServiceWorkerRegistration> {
  if (!(await hostStore().list()).some((host) => host.room === room && host.credential)) throw new Error("host_not_saved");
  const existing = pushWorkers.get(room);
  if (existing) return existing;
  if (pushWorkers.size >= MAX_HOSTS) throw new Error("host_limit");
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
  await hostStore().forget(room);
}

export function bundlePath(protocol: number, supported: readonly number[]): string | null {
  return Number.isSafeInteger(protocol) && protocol > 0 && supported.includes(protocol) ? `/p/${protocol}/` : null;
}

/** The start page as the list of saved computers: it never opens one by itself, so a way back to the list stays there. */
export const HOST_LIST_LINK = "/?list=1";

/**
 * The computer the start page opens without showing the list: the one its address names, or else the only computer
 * this phone keeps, unless the address asks for the list or the scanner, or that computer refused this phone.
 */
export function hostToOpen(query: URLSearchParams, hosts: readonly PairedHost[]): PairedHost | null {
  const named = query.get("host");
  if (named) return hosts.find((host) => host.room === named) ?? null;
  if (query.has("list") || query.has("add") || hosts.length !== 1) return null;
  return hosts[0]!.credential === null ? null : hosts[0]!;
}

export function hostLink(host: PairedHost, tag?: string): string {
  const query = new URLSearchParams({ host: host.room });
  if (tag) query.set("open", tag);
  return `/?${query}`;
}
