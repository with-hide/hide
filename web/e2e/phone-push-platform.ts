// Only browser-owned APIs are replaced. Workers, scopes, storage, connection and frames stay real.
import type { Page } from "@playwright/test";

export async function installBrowserPush(page: Page): Promise<void> {
  await page.addInitScript(() => {
    Object.defineProperty(Notification, "permission", { configurable: true, get: () => "granted" });
    Notification.requestPermission = async () => "granted";
    const write = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key === "hide.phone.web-push" && this.getItem("fixture.storage-fail")) throw new DOMException("Full", "QuotaExceededError");
      write.call(this, key, value);
    };
    const scopes = new WeakMap<PushManager, string>();
    const descriptor = Object.getOwnPropertyDescriptor(ServiceWorkerRegistration.prototype, "pushManager")!;
    Object.defineProperty(ServiceWorkerRegistration.prototype, "pushManager", { configurable: true, get() {
      const manager = descriptor.get!.call(this) as PushManager; scopes.set(manager, new URL(this.scope).pathname); return manager;
    } });
    type Saved = { endpoint: string; key: number[] };
    const subscription = (scope: string, saved: Saved) => ({
      options: { applicationServerKey: new Uint8Array(saved.key).buffer, userVisibleOnly: true },
      toJSON: () => ({ endpoint: saved.endpoint, keys: { p256dh: "public", auth: "secret" } }),
      unsubscribe: async () => {
        const count = `fixture.cleanup:${scope}`;
        localStorage.setItem(count, String(Number(localStorage.getItem(count) ?? 0) + 1));
        if (localStorage.getItem("fixture.cleanup-hold") === scope) {
          const answer = await new Promise<boolean>((resolve) => Object.assign(window, { finishCleanup: resolve }));
          if (!answer) return false;
        }
        if (localStorage.getItem("fixture.cleanup-refuse") === scope) return false;
        localStorage.removeItem(`fixture.push:${scope}`); return true;
      },
    }) as PushSubscription;
    PushManager.prototype.getSubscription = async function () {
      const scope = scopes.get(this)!;
      const saved = localStorage.getItem(`fixture.push:${scope}`);
      return saved ? subscription(scope, JSON.parse(saved) as Saved) : null;
    };
    PushManager.prototype.subscribe = async function (options) {
      const scope = scopes.get(this)!;
      const serial = Number(localStorage.getItem("fixture.push-serial") ?? 0) + 1;
      localStorage.setItem("fixture.push-serial", String(serial));
      const saved = { endpoint: `https://push.example/${serial}`, key: [...new Uint8Array(options!.applicationServerKey as ArrayBuffer)] };
      localStorage.setItem(`fixture.push:${scope}`, JSON.stringify(saved));
      return subscription(scope, saved);
    };
  });
}
