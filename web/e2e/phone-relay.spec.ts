import { expect, test as base } from "@playwright/test";
import { WebSocket } from "ws";
import { startPhoneRelay } from "./phone-relay-fixture";

const test = base.extend<{ relay: Awaited<ReturnType<typeof startPhoneRelay>> }>({
  relay: async ({ page }, use, info) => {
    const relay = await startPhoneRelay();
    // Replace only the external relay endpoint; frames pass through its local server and native Noise peer.
    await page.routeWebSocket("wss://relay.withhide.dev/ws", (socket) => {
      let upstream: WebSocket | null = null;
      const pending: (string | Buffer)[] = [];
      socket.onMessage((frame) => {
        if (!upstream) {
          const url = relay.relayEndpoints.get(JSON.parse(String(frame)).room);
          if (!url) { socket.close(); return; }
          const next = new WebSocket(url); upstream = next;
          next.on("open", () => { for (const item of pending.splice(0)) next.send(item); });
          next.on("message", (bytes, binary) => socket.send(binary ? Buffer.from(bytes as Buffer) : bytes.toString()));
          next.on("close", () => socket.close());
          next.on("error", () => socket.close());
        }
        if (upstream.readyState === WebSocket.OPEN) upstream.send(frame);
        else if (pending.length < 16) pending.push(frame);
        else socket.close();
      });
      socket.onClose(() => upstream?.close());
    });
    try { await use(relay); } finally {
      if (info.status !== info.expectedStatus) console.warn("phone fixture failure", {
        types: relay.received.slice(-16).map((message) => message.type ?? message.client_kind), errors: relay.errors.slice(-8),
        browser: await page.evaluate(async () => ({ permission: typeof Notification === "undefined" ? "unsupported" : Notification.permission,
          workers: (await navigator.serviceWorker.getRegistrations()).map((worker) => ({ scope: new URL(worker.scope).pathname.startsWith("/push/") ? "device" : "root", active: worker.active?.state, installing: worker.installing?.state })) })).catch(() => "page unavailable"),
      });
      await page.goto("about:blank"); await relay.stop();
    }
  },
});
test.use({ viewport: { width: 390, height: 844 }, locale: "en-US" });

async function pair(page: import("@playwright/test").Page, relay: Awaited<ReturnType<typeof startPhoneRelay>>) {
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
}

test("QR clears its secret, pairs over Noise and uses the shared conversation, terminal and reply UI", async ({ page, relay }) => {
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await expect(page.getByRole("heading", { name: `Connect to ${relay.pairing.name}`, exact: true })).toBeVisible();
  await expect(page).not.toHaveURL(/#pair=/);
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page).toHaveURL(/\/p\/1\/\?host=/);
  await expect(page.locator('[data-phone-agent="test-device|pane-one"]')).toBeVisible();
  await expect(page.locator("[data-phone-install-hint]")).toBeVisible();
  await page.locator('[data-phone-agent="test-device|pane-one"]').click();
  await expect(page.getByText("The encrypted connection is ready.", { exact: true })).toBeVisible();
  await page.locator('[data-phone-view="terminal"]').click();
  await expect(page.getByText("terminal output from device", { exact: true })).toBeVisible();
  await page.locator("[data-phone-reply]").fill("Proceed once");
  await page.locator("[data-phone-send]").click();
  await expect(page.locator("[data-phone-reply]")).toHaveValue("");
  await page.locator('[data-phone-key="enter"]').click();
  await expect.poll(() => relay.received.filter((frame) => frame.type === "input").length).toBe(2);
  expect(relay.received.filter((frame) => frame.type === "input")).toMatchObject([{ text: "Proceed once" }, { key: "enter" }]);
  expect(relay.plaintext.every((frame) => JSON.parse(frame).type === "phone")).toBe(true);
  expect(relay.plaintext.join("")).not.toContain(relay.pairing.code);
  expect(relay.plaintext.join("")).not.toContain(relay.credential);
  expect(relay.errors).toEqual([]);
});

test("host picker reports counts, reconnects without repeating a reply and routes a push link to its agent", async ({ page, relay }) => {
  await pair(page, relay);
  await page.getByRole("button", { name: relay.pairing.name, exact: true }).click();
  await expect(page.getByText("Needs you 1 · Working 2", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Connect another device" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.clock.install();
  relay.state.offline = true; relay.disconnect();
  await expect(page.locator("[data-phone-unreachable]")).toContainText("Hide turned off or is asleep");
  await expect(page.locator("[data-phone-unreachable]")).not.toContainText("Tailscale");
  relay.state.offline = false;
  await page.clock.fastForward(15_000);
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
  await page.goto(`${relay.origin}/?host=${relay.pairing.room}&open=test-device%7Cpane-one`);
  await expect(page.locator('[data-phone-detail="test-device|pane-one"]')).toBeVisible();
  expect(relay.received.filter((frame) => frame.type === "input")).toHaveLength(0);
});

test("unsupported device shows both protocol versions and lets the user choose another", async ({ page, relay }) => {
  relay.state.protocol = 9;
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("heading", { name: `Update Hide on ${relay.pairing.name}` })).toBeVisible();
  await expect(page.getByText("Phone app 1 · Device 9", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Choose another device" }).click();
  await expect(page.getByRole("button", { name: "Connect another device" })).toBeVisible();
});

for (const [reason, copy] of [["code_expired", "expired"], ["phone_limit", "4"]]) {
  test(`pairing refusal ${reason} uses the existing guidance`, async ({ page, relay }) => {
    relay.state.refusal = reason!;
    await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.locator("[data-phone-guidance]")).toContainText(copy!);
  });
}

test("the in-app link scanner adds a device and CSP refuses external scripts", async ({ page, relay }) => {
  await page.goto(relay.origin);
  await page.getByRole("button", { name: "Connect another device" }).click();
  await page.getByRole("textbox", { name: "Paste QR link" }).fill(relay.qr);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("heading", { name: `Connect to ${relay.pairing.name}`, exact: true })).toBeVisible();
  const blocked = await page.evaluate(() => new Promise<boolean>((resolve) => {
    document.addEventListener("securitypolicyviolation", (event) => resolve(event.blockedURI === "https://example.com/external.js"), { once: true });
    const script = document.createElement("script"); script.src = "https://example.com/external.js"; document.head.append(script);
  }));
  expect(blocked).toBe(true);
  await expect(page.locator("script[src^='https://']")).toHaveCount(1); // Only the rejected test script.
});

test("start and wake use the same encrypted command path", async ({ page, relay }) => {
  relay.state.sleeping = true;
  await pair(page, relay);
  await page.locator('[data-phone-agent="test-device|pane-one"]').click();
  await page.locator('[data-phone-wake="wake"]').click();
  await expect(page.locator("[data-phone-sleep]")).toHaveCount(0);
  await page.locator("[data-phone-back]").click();
  await page.locator("[data-phone-start]").click();
  await page.locator("[data-phone-start-text]").fill("Start the next task");
  await page.locator("[data-phone-start-submit]").click();
  await expect(page.locator('[data-phone-detail="test-device|pane-one"]')).toBeVisible();
  expect(relay.received.filter((frame) => frame.type === "wake_agent")).toHaveLength(1);
  expect(relay.received.filter((frame) => frame.type === "start_agent")).toMatchObject([{ text: "Start the next task", target: "home:test-device" }]);
});

test("hosted push registers and removes only the device scope while keeping the root worker", async ({ page, relay }) => {
  relay.state.push = true;
  await page.addInitScript(() => {
    let permission: NotificationPermission = "default";
    Object.defineProperty(Notification, "permission", { configurable: true, get: () => permission });
    Notification.requestPermission = async () => { permission = "granted"; return permission; };
    PushManager.prototype.getSubscription = async () => null;
    PushManager.prototype.subscribe = async () => ({ toJSON: () => ({ endpoint: "https://push.example/subscription", keys: { p256dh: "public", auth: "secret" } }) }) as PushSubscription;
  });
  await pair(page, relay);
  await page.getByRole("button", { name: "Turn on notifications", exact: true }).click();
  await expect(page.locator('[data-phone-notifications="enable"]')).toHaveCount(0);
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_subscription").length).toBe(1);
  const scopes = () => page.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).map((worker) => new URL(worker.scope).pathname).sort());
  await expect.poll(scopes).toEqual(["/", `/push/${relay.pairing.room}/`]);
  await page.getByRole("button", { name: relay.pairing.name, exact: true }).click();
  await page.getByRole("button", { name: "Remove device", exact: true }).click();
  await page.getByRole("button", { name: "Remove device", exact: true }).last().click();
  await expect(page.getByRole("button", { name: "Connect another device" })).toBeVisible();
  await expect.poll(scopes).toEqual(["/"]);
});

for (const permission of ["denied", "unsupported"]) {
  test(`hosted notification ${permission} retains its one-line guidance`, async ({ page, relay }) => {
    relay.state.push = true;
    await page.addInitScript((answer) => {
      if (answer === "unsupported") { Reflect.deleteProperty(window, "PushManager"); Reflect.deleteProperty(window, "Notification"); }
      else Object.defineProperty(Notification, "permission", { configurable: true, get: () => "denied" });
    }, permission);
    await pair(page, relay);
    await expect(page.locator(`[data-phone-notifications="${permission === "denied" ? "denied" : "install_first"}"]`)).toBeVisible();
  });
}

test("one browser keeps two devices and switches its live list through the header", async ({ page, relay }) => {
  const second = await startPhoneRelay();
  relay.relayEndpoints.set(second.pairing.room, second.wsUrl);
  try {
    await pair(page, relay);
    await page.goto("about:blank");
    await page.goto(relay.origin + "/" + new URL(second.qr).hash);
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(second.pairing.room));
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    await page.getByRole("button", { name: second.pairing.name, exact: true }).click();
    await expect(page.getByText("Needs you 1 · Working 2", { exact: true })).toHaveCount(2);
    await page.getByRole("button", { name: /Needs you 1 · Working 2/ }).first().click();
    await expect(page).toHaveURL(new RegExp(`/p/1/\\?host=${relay.pairing.room}`));
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
  } finally { await page.goto("about:blank"); await second.stop(); }
});
