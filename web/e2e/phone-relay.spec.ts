import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test as base, type Page } from "@playwright/test";
import { WebSocket } from "ws";
import qrcode from "qrcode-generator";
import { screenshot } from "./wire";
import { startPhoneRelay } from "./phone-relay-fixture";
import { installBrowserPush } from "./phone-push-platform";
import { PHONE_PROTOCOL } from "../src/mobile/phoneVersion";
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

test.describe("mission graph touch", () => {
  test.use({ viewport: { width: 393, height: 852 }, hasTouch: true, isMobile: true });

  // UI_BEHAVIOR.md: a finger pans from a card, two fingers pinch, a tap opens its worker.
  // Native input matters: dispatching synthetic pointer events skips implicit capture transfer.
  test("card touches keep panning and pinching across successive moves, then a tap opens the worker", async ({ page, relay }) => {
    await page.goto(`${relay.origin}/p/${PHONE_PROTOCOL}/?demo=1`);
    await page.locator('[data-phone-tab="missions"]').tap();
    await page.locator('[data-phone-mission="mission-release"]').tap();
    const canvas = page.locator("[data-mission-canvas]");
    const card = page.locator('[data-mission-node="rehearse"]');
    await expect(canvas.locator('[data-dependency-layout="layered"]')).toBeVisible();
    await expect(card).toBeVisible();
    for (let step = 0; step < 3; step++) await page.locator('[data-mission-zoom-step="in"]').tap();
    await card.scrollIntoViewIfNeeded();
    const bounds = await card.boundingBox();
    expect(bounds).not.toBeNull();
    const at = { x: bounds!.x + bounds!.width / 2, y: bounds!.y + bounds!.height / 2 };
    // Ensure the gesture really starts inside the card, rather than its surrounding canvas.
    expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-mission-node]")?.dataset.missionNode, at)).toBe("rehearse");
    const input = await page.context().newCDPSession(page);
    const touch = (type: "touchStart" | "touchMove" | "touchEnd", points: { x: number; y: number }[]) => input.send("Input.dispatchTouchEvent", {
      type, touchPoints: points.map((point, id) => ({ ...point, id, radiusX: 4, radiusY: 4, force: 1 })),
    });
    try {
      let top = await canvas.evaluate((element) => element.scrollTop);
      const direction = top >= 45 ? 1 : -1;
      await touch("touchStart", [at]);
      for (const distance of [15, 30, 45]) {
        await touch("touchMove", [{ x: at.x, y: at.y + direction * distance }]);
        await expect.poll(async () => Math.abs(await canvas.evaluate((element) => element.scrollTop) - top)).toBeGreaterThan(10);
        top = await canvas.evaluate((element) => element.scrollTop);
      }
      await touch("touchEnd", []);
      await expect(page.locator("[data-phone-detail]")).toHaveCount(0);
      await card.scrollIntoViewIfNeeded();
      const next = await card.boundingBox();
      expect(next).not.toBeNull();
      const middle = { x: next!.x + next!.width / 2, y: next!.y + next!.height / 2 };
      const fingers = (spread: number) => [{ x: middle.x - spread, y: middle.y }, { x: middle.x + spread, y: middle.y }];
      for (const point of fingers(20)) {
        expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-mission-node]")?.dataset.missionNode, point)).toBe("rehearse");
      }
      let scale = Number(await canvas.getAttribute("data-mission-scale"));
      await touch("touchStart", fingers(20));
      for (const spread of [25, 30, 35]) {
        await touch("touchMove", fingers(spread));
        await expect.poll(async () => Number(await canvas.getAttribute("data-mission-scale"))).toBeGreaterThan(scale + 0.05);
        scale = Number(await canvas.getAttribute("data-mission-scale"));
      }
      await touch("touchEnd", []);
      await expect(page.locator("[data-phone-detail]")).toHaveCount(0);
      await page.locator('[data-mission-zoom-step="fit"]').tap();
      await card.tap();
      await expect(page.locator('[data-phone-detail="local|a3"]')).toBeVisible();
    } finally { await input.detach(); }
  });
});

async function pair(page: import("@playwright/test").Page, relay: Awaited<ReturnType<typeof startPhoneRelay>>) {
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
}

test("QR clears its secret, pairs over Noise and uses the shared conversation, terminal and reply UI", async ({ page, relay }) => {
  // A camera/link can return to a root page that is already open (same-document navigation).
  await page.goto(relay.origin + "/");
  await expect(page.getByRole("heading", { name: "Connect your computer" })).toBeVisible();
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await expect(page.getByRole("heading", { name: relay.pairing.name, exact: true })).toBeVisible();
  await expect(page).not.toHaveURL(/#pair=/);
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/p/${PHONE_PROTOCOL}/\\?host=`));
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
  await page.locator('[data-phone-agent="test-device|pane-one"]').click();
  await expect(page.getByText("The encrypted connection is ready.", { exact: true })).toBeVisible();
  await page.clock.install();
  relay.state.dropInputReply = true;
  await page.locator("[data-phone-reply]").fill("Apply this reply once");
  await page.locator("[data-phone-send]").click();
  await expect.poll(() => relay.received.filter((frame) => frame.type === "input").length).toBe(1);
  await expect(page.locator("[data-phone-unreachable]")).toContainText("Hide turned off or is asleep");
  await expect(page.locator("[data-phone-unreachable]")).not.toContainText("Tailscale");
  relay.state.offline = false;
  await page.clock.fastForward(15_000);
  await expect(page.locator("[data-phone-reply]")).toBeEnabled();
  await expect(page.locator("[data-phone-unreachable]")).toHaveCount(0);
  expect(relay.appliedInputs).toEqual(["Apply this reply once"]);
  await page.goto(`${relay.origin}/?host=${relay.pairing.room}&open=test-device%7Cpane-one`);
  await expect(page.locator('[data-phone-detail="test-device|pane-one"]')).toBeVisible();
  expect(relay.received.filter((frame) => frame.type === "input")).toMatchObject([{ text: "Apply this reply once" }]);
  expect(relay.appliedInputs).toEqual(["Apply this reply once"]);
});

for (const surface of ["root", "picker"]) {
  test(`the ${surface} refuses a saved device's changed protocol before Noise or authentication`, async ({ page, relay }) => {
    const second = await startPhoneRelay();
    relay.relayEndpoints.set(second.pairing.room, second.wsUrl);
    try {
      await pair(page, relay);
      await page.goto("about:blank");
      relay.state.protocol = 9;
      relay.received.length = relay.plaintext.length = relay.wire.binaryFrames = 0;
      if (surface === "root") await page.goto(relay.origin + "/?list=1");
      else {
        await page.goto(relay.origin + "/" + new URL(second.qr).hash);
        await page.getByRole("button", { name: "Connect", exact: true }).click();
        await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
        await page.getByRole("button", { name: second.pairing.name, exact: true }).click();
      }
      await expect.poll(() => relay.plaintext.length).toBeGreaterThan(0);
      await expect(page.getByText(`Phone app ${PHONE_PROTOCOL} · Device 9`, { exact: true })).toBeVisible();
      expect(relay.wire.binaryFrames).toBe(0);
      expect(relay.received).toEqual([]);
      await page.getByRole("button", { name: new RegExp(`Phone app ${PHONE_PROTOCOL} · Device 9`) }).click();
      await expect(page.getByRole("heading", { name: "Update Hide on this device" })).toBeVisible();
      expect(relay.wire.binaryFrames).toBe(0);
      expect(relay.received).toEqual([]);
    } finally { await page.goto("about:blank"); await second.stop(); }
  });
}

test("unsupported device shows both protocol versions and lets the user choose another", async ({ page, relay }) => {
  relay.state.protocol = 9;
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Update Hide on this device" })).toBeVisible();
  await expect(page.getByText("Phone app", { exact: true })).toBeVisible();
  await expect(page.getByText("Device", { exact: true })).toBeVisible();
  await expect(page.getByText("9", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Choose another device" }).click();
  await expect(page.getByRole("heading", { name: "Connect your computer" })).toBeVisible();
});

// Opening the app with one saved computer goes straight to it, so the device list must still be reachable and stay put
// once chosen: the loader's decision and the bundle's way back cross a page load, which no unit test sees.
test("one saved device opens at launch, and Choose another device keeps the list", async ({ page, relay }) => {
  await pair(page, relay);
  await page.goto("about:blank");
  await page.goto(relay.origin + "/");
  await expect(page).toHaveURL(new RegExp(`/p/${PHONE_PROTOCOL}/\\?host=${relay.pairing.room}`));
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
  // A device the app cannot open: its screen's way back is the list, which does not open the device again.
  relay.state.protocol = 9;
  await page.goto("about:blank");
  await page.goto(relay.origin + "/");
  await expect(page.getByRole("heading", { name: "Update Hide on this device" })).toBeVisible();
  await page.getByRole("button", { name: "Choose another device" }).click();
  await expect(page).toHaveURL(relay.origin + "/?list=1");
  await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
  await expect(page.locator("[data-phone-host]")).toContainText(relay.pairing.name);
  await expect(page.locator("[data-phone-add]")).toBeVisible();
  await expect(page.locator("[data-phone-demo-entry]")).toBeVisible();
  await expect(page).toHaveURL(relay.origin + "/?list=1");
});

for (const [reason, copy] of [["code_expired", "expired"], ["phone_limit", "4"]]) {
  test(`pairing refusal ${reason} uses the existing guidance`, async ({ page, relay }) => {
    relay.state.refusal = reason!;
    await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.locator("[data-phone-guidance]")).toContainText(copy!);
  });
}

// A Safari tab's workers have no push; the revoked device's cleanup must not read as a storage failure.
test("a revoked device stays listed as disconnected, never dials again and pairs anew", async ({ page, relay }) => {
  await page.addInitScript(() => { Object.defineProperty(ServiceWorkerRegistration.prototype, "pushManager", { configurable: true, get: () => undefined }); });
  await pair(page, relay);
  relay.state.refusal = "revoked";
  await page.reload();
  await expect(page.locator('[data-phone-guidance="revoked"]')).toContainText("revoked");
  await expect(page.getByText("Device storage is unavailable", { exact: false })).toHaveCount(0);
  const hosts = () => page.evaluate(() => JSON.parse(localStorage.getItem("hide.phone.hosts.v1") ?? "[]") as { room: string; name: string; credential: string | null }[]);
  await expect.poll(hosts).toEqual([expect.objectContaining({ room: relay.pairing.room, name: relay.pairing.name, credential: null })]);
  await page.getByRole("button", { name: "Choose another device" }).click();
  const row = page.locator(`[data-phone-host-revoked="${relay.pairing.room}"]`);
  await expect(row).toContainText(relay.pairing.name);
  await expect(row).toContainText("Disconnected");
  await expect(row).toContainText("Unused for 7 days, or disconnected on this device");
  await expect(row.getByRole("button", { name: "Remove device" })).toBeVisible();
  // A link to the device shows the refusal without dialing it.
  relay.plaintext.length = 0;
  await page.goto(`${relay.origin}/p/${PHONE_PROTOCOL}/?host=${relay.pairing.room}`);
  await expect(page.locator('[data-phone-guidance="revoked"]')).toBeVisible();
  expect(relay.plaintext).toEqual([]);
  await page.goto(relay.origin + "/");
  await row.getByRole("button", { name: "Reconnect" }).click();
  await expect(page).toHaveURL(relay.origin + "/?add=1");
  relay.state.refusal = "";
  await pair(page, relay);
  await expect.poll(hosts).toEqual([expect.objectContaining({ room: relay.pairing.room, credential: relay.credential })]);
  await page.goto(relay.origin + "/?list=1");
  await expect(page.locator("[data-phone-host-revoked]")).toHaveCount(0);
});

test("the device list marks a device that refuses its status check as disconnected", async ({ page, relay }) => {
  const second = await startPhoneRelay();
  relay.relayEndpoints.set(second.pairing.room, second.wsUrl);
  try {
    await pair(page, relay);
    await page.goto("about:blank");
    await page.goto(relay.origin + "/" + new URL(second.qr).hash);
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    relay.state.refusal = "revoked";
    await page.getByRole("button", { name: second.pairing.name, exact: true }).click();
    await expect(page.locator(`[data-phone-host-revoked="${relay.pairing.room}"]`)).toContainText("Disconnected");
    // The root list watches the live device to its counts and never dials the revoked one.
    relay.plaintext.length = 0;
    await page.goto(relay.origin + "/");
    await expect(page.getByText("Needs you 1 · Working 2", { exact: true })).toBeVisible();
    await expect(page.locator(`[data-phone-host-revoked="${relay.pairing.room}"]`)).toBeVisible();
    expect(relay.plaintext).toEqual([]);
  } finally { await page.goto("about:blank"); await second.stop(); }
});

test("the in-app link scanner adds a device and CSP refuses external scripts", async ({ page, relay }) => {
  await page.goto(relay.origin);
  // No computer yet: the first screen's big button opens the browser's add page, whose pasted link is one row under the scan.
  await page.getByRole("button", { name: "Scan QR code", exact: true }).click();
  await page.getByRole("button", { name: "Paste QR link", exact: true }).click();
  await page.getByRole("textbox", { name: "Paste QR link" }).fill(relay.qr);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.getByRole("heading", { name: relay.pairing.name, exact: true })).toBeVisible();
  const blocked = await page.evaluate(() => new Promise<boolean>((resolve) => {
    document.addEventListener("securitypolicyviolation", (event) => resolve(event.blockedURI === "https://example.com/external.js"), { once: true });
    const script = document.createElement("script"); script.src = "https://example.com/external.js"; document.head.append(script);
  }));
  expect(blocked).toBe(true);
  await expect(page.locator("script[src^='https://']")).toHaveCount(1); // Only the rejected test script.
});

test("the in-app camera decodes a QR inside its frame and releases its video track", async ({ page, relay }) => {
  const qr = qrcode(0, "M"); qr.addData(relay.qr); qr.make();
  const matrix = Array.from({ length: qr.getModuleCount() }, (_, row) =>
    Array.from({ length: qr.getModuleCount() }, (_, column) => qr.isDark(row, column)));
  await page.addInitScript((cells) => {
    navigator.mediaDevices.getUserMedia = async () => {
      // A wide camera picture with the code where the frame is, as a person aims it.
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1920;
      const context = canvas.getContext("2d")!;
      const left = 960 - ((cells.length + 8) * 4) / 2;
      context.fillStyle = "white"; context.fillRect(0, 0, canvas.width, canvas.height);
      context.fillStyle = "black";
      cells.forEach((row, y) => row.forEach((dark, x) => { if (dark) context.fillRect(left + (x + 4) * 4, left + (y + 4) * 4, 4, 4); }));
      const stream = canvas.captureStream(4);
      (window as unknown as { cameraTrack: MediaStreamTrack }).cameraTrack = stream.getVideoTracks()[0]!;
      return stream;
    };
  }, matrix);
  await page.goto(`${relay.origin}/?add=1`);
  await page.getByRole("button", { name: "Scan QR code", exact: true }).click();
  await expect(page.getByRole("heading", { name: relay.pairing.name, exact: true })).toBeVisible();
  await expect(page.locator("video")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { cameraTrack: MediaStreamTrack }).cameraTrack.readyState)).toBe("ended");
});

test("the detail keeps its newest line above the keyboard, unless the reader scrolled up", async ({ page, relay }) => {
  relay.state.rows = Array.from({ length: 120 }, (_, index) => `row ${index + 1}`).join("\n");
  await pair(page, relay);
  await page.locator('[data-phone-agent="test-device|pane-one"]').click();
  await page.locator('[data-phone-view="terminal"]').click();
  const newest = page.getByText("row 120", { exact: true });
  await expect(newest).toBeInViewport();
  // The keyboard takes the lower part of the screen, as the app's web view and a browser's visual viewport both shrink.
  await page.setViewportSize({ width: 390, height: 480 });
  await expect(newest).toBeInViewport();
  await expect(page.locator("[data-phone-reply]")).toBeInViewport();
  // Someone reading older rows keeps their place when the keyboard comes up.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator("[data-phone-scrollback]").evaluate((element) => { element.scrollTop = 0; element.dispatchEvent(new Event("scroll")); });
  await page.setViewportSize({ width: 390, height: 480 });
  await expect(page.getByText("row 1", { exact: true })).toBeInViewport();
  await expect(newest).not.toBeInViewport();
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
  expect(relay.received.filter((frame) => frame.type === "start_agent")).toMatchObject([{ text: "Start the next task", target: "checkout-one" }]);
});

test("an HTML file is drawn in the bundle's sandboxed look frame, running its script in an origin of its own", async ({ page, relay }) => {
  relay.state.html = `<!doctype html><html><head><meta charset="utf-8"><style>h1 { color: rgb(67, 56, 202) }</style></head><body>
<h1 id="title">알림 설정 목업</h1><img id="img" src="data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==">
<p id="script">not run</p><p id="storage">?</p><p id="parent">?</p>
<script>document.getElementById("script").textContent = "script ran";
try { localStorage.length; document.getElementById("storage").textContent = "storage open"; } catch { document.getElementById("storage").textContent = "storage refused"; }
try { parent.document.title; document.getElementById("parent").textContent = "parent reachable"; } catch { document.getElementById("parent").textContent = "parent isolated"; }</script></body></html>`;
  await pair(page, relay);
  await page.locator('[data-phone-agent="test-device|pane-one"]').click();
  await page.locator('[data-phone-view="looks"]').click();
  await expect(page.locator('[data-phone-look="v-html"]')).toContainText("HTML page");
  await page.locator('[data-phone-look="v-html"]').click();
  const frame = page.frameLocator("[data-phone-look-html]");
  await expect(frame.locator("#title")).toHaveText("알림 설정 목업");
  await expect(frame.locator("#title")).toHaveCSS("color", "rgb(67, 56, 202)");
  await expect(frame.locator("#script")).toHaveText("script ran");
  await expect(frame.locator("#storage")).toHaveText("storage refused");
  await expect(frame.locator("#parent")).toHaveText("parent isolated");
  expect(await frame.locator("#img").evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(1);
  // The bundle's frame sits beside its page.
  await expect(page.locator("[data-phone-look-html]")).toHaveAttribute("src", "look-frame.html");
  expect(page.frames().map((child) => new URL(child.url(), relay.origin).pathname)).toContain(`/p/${PHONE_PROTOCOL}/look-frame.html`);
  expect(relay.received.filter((frame) => frame.type === "look_seen")).toMatchObject([{ view_id: "v-html" }]);
});

// Reload, worker scopes and encrypted reconnect cross boundaries a component test cannot observe.
test("the phone keeps receiving Off across reload and reconnect, and On registers again", async ({ page, relay }) => {
  // The older computer's global off must not own this phone's receiving choice.
  relay.state.push = false;
  await page.addInitScript(() => {
    Object.defineProperty(Notification, "permission", { configurable: true, get: () => localStorage.getItem("fixture.permission") ?? "default" });
    Notification.requestPermission = async () => { localStorage.setItem("fixture.permission", "granted"); return "granted"; };
    const subscription = () => ({
      options: { applicationServerKey: new Uint8Array(JSON.parse(localStorage.getItem("fixture.subscription")!) as number[]).buffer, userVisibleOnly: true },
      toJSON: () => ({ endpoint: "https://push.example/subscription", keys: { p256dh: "public", auth: "secret" } }),
      unsubscribe: async () => { localStorage.removeItem("fixture.subscription"); return true; },
    }) as PushSubscription;
    PushManager.prototype.getSubscription = async () => localStorage.getItem("fixture.subscription") ? subscription() : null;
    PushManager.prototype.subscribe = async (options) => {
      const key = options!.applicationServerKey as BufferSource;
      const bytes = ArrayBuffer.isView(key) ? new Uint8Array(key.buffer, key.byteOffset, key.byteLength) : new Uint8Array(key);
      localStorage.setItem("fixture.subscription", JSON.stringify([...bytes])); return subscription();
    };
  });
  await pair(page, relay);
  await page.locator('[data-phone-tab="settings"]').click();
  const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
  await receive.click();
  await expect(receive).toHaveAttribute("aria-checked", "true");
  await screenshot(page, "phone-receiving-on-en");
  for (const [language, label] of [["ko", "알림 받기"], ["zh-CN", "接收通知"], ["ja", "通知を受け取る"], ["en", "Receive notifications"]]) {
    await page.locator('[data-phone-setting="language"]').click();
    await page.locator(`[data-phone-language-option="${language}"]`).click();
    await expect(page.getByRole("switch", { name: label, exact: true })).toHaveAttribute("aria-checked", "true");
    await screenshot(page, `phone-receiving-on-${language}`);
  }
  const subscriptions = () => relay.received.filter((frame) => frame.type === "push_subscription").length;
  const off = () => relay.received.filter((frame) => frame.type === "push_permission" && frame.state === "denied").length;
  await expect.poll(subscriptions).toBe(1);
  const scopes = () => page.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).map((worker) => new URL(worker.scope).pathname).sort());
  await expect.poll(scopes).toEqual(["/", `/push/${relay.pairing.room}/`]);
  await receive.click();
  await expect(page.locator('[data-phone-notifications="off"]')).toBeVisible();
  await screenshot(page, "phone-receiving-off-en");
  await expect.poll(off).toBe(1);
  expect(await page.evaluate(() => localStorage.getItem("fixture.subscription"))).toBeNull();
  await page.reload();
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
  await expect.poll(off).toBe(2);
  expect(subscriptions()).toBe(1);
  relay.disconnect();
  await expect.poll(off).toBe(3);
  expect(subscriptions()).toBe(1);
  await page.locator('[data-phone-tab="settings"]').click();
  await expect(receive).toHaveAttribute("aria-checked", "false");
  await receive.click();
  await expect.poll(subscriptions).toBe(2);
  await expect(receive).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Remove device", exact: true }).click();
  await page.getByRole("button", { name: "Remove device", exact: true }).last().click();
  await expect(page).toHaveURL(relay.origin + "/");
  await expect(page.getByRole("heading", { name: "Connect your computer" })).toBeVisible();
  await expect.poll(scopes).toEqual(["/"]);
});

for (const cleanup of [true, false]) test(`turning receiving Off handles late subscription cleanup ${cleanup ? "success" : "failure"}`, async ({ page, relay }) => {
  await page.addInitScript((succeeds) => {
    Object.defineProperty(Notification, "permission", { configurable: true, get: () => "granted" });
    let subscription: PushSubscription | null = null;
    PushManager.prototype.getSubscription = async () => subscription;
    PushManager.prototype.subscribe = (options) => {
      const key = options!.applicationServerKey as BufferSource;
      const bytes = ArrayBuffer.isView(key) ? new Uint8Array(key.buffer, key.byteOffset, key.byteLength) : new Uint8Array(key);
      const applicationServerKey = bytes.slice().buffer;
      return new Promise((resolve) => {
        Object.assign(window, { finishSubscription: () => { subscription = {
          options: { applicationServerKey, userVisibleOnly: true },
          toJSON: () => ({ endpoint: "https://push.example/late", keys: { p256dh: "public", auth: "secret" } }),
          unsubscribe: async () => { localStorage.setItem("fixture.late-unsubscribed", "true"); if (succeeds) subscription = null; return succeeds; },
        } as PushSubscription; resolve(subscription); } });
      });
    };
  }, cleanup);
  await pair(page, relay);
  await expect.poll(() => page.evaluate(() => "finishSubscription" in window)).toBe(true);
  await page.locator('[data-phone-tab="settings"]').click();
  const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
  await expect(receive).toHaveAttribute("aria-checked", "true");
  await receive.click();
  await expect(page.locator('[data-phone-notifications="stopping"]')).toBeVisible();
  await page.evaluate(() => (window as unknown as { finishSubscription: () => void }).finishSubscription());
  await expect.poll(() => page.evaluate(() => localStorage.getItem("fixture.late-unsubscribed"))).toBe("true");
  expect(relay.received.filter((frame) => frame.type === "push_subscription")).toHaveLength(0);
  await expect(receive).toHaveAttribute("aria-checked", String(!cleanup));
  if (!cleanup) await expect(page.locator('[data-phone-notifications="failed"] [role="alert"]')).toContainText("Try again");
});

test("a receiving choice that cannot be saved survives visibility and Settings remount", async ({ page, relay }) => {
  await installBrowserPush(page);
  await pair(page, relay);
  await expect.poll(() => relay.state.notifications).toBe("on");
  await page.evaluate(() => { localStorage.setItem("hide.phone.web-push", "on"); localStorage.setItem("fixture.storage-fail", "true"); });
  await page.locator('[data-phone-tab="settings"]').click();
  const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
  await receive.click();
  await expect(page.locator('[data-phone-notifications="off"]')).toContainText("Could not save on this phone");
  const registrations = relay.received.filter((frame) => frame.type === "push_subscription").length;
  await page.evaluate(() => { document.dispatchEvent(new Event("visibilitychange")); window.dispatchEvent(new StorageEvent("storage")); });
  await page.locator('[data-phone-tab="agents"]').click();
  await page.locator('[data-phone-tab="settings"]').click();
  await expect(receive).toHaveAttribute("aria-checked", "false");
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_permission" && frame.state === "denied").length).toBeGreaterThan(1);
  expect(relay.received.filter((frame) => frame.type === "push_subscription")).toHaveLength(registrations);
});

test("receiving Off retries an inactive computer's failed cleanup after reload", async ({ page, relay }) => {
  const second = await startPhoneRelay(); relay.relayEndpoints.set(second.pairing.room, second.wsUrl);
  try {
    await installBrowserPush(page);
    await pair(page, relay);
    await expect.poll(() => relay.state.notifications).toBe("on");
    await pair(page, { ...second, origin: relay.origin });
    await expect.poll(() => second.state.notifications).toBe("on");
    const scope = `/push/${relay.pairing.room}/`;
    await page.evaluate((scope) => localStorage.setItem("fixture.cleanup-refuse", scope), scope);
    await page.locator('[data-phone-tab="settings"]').click();
    const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
    await receive.click();
    await expect(page.locator('[data-phone-notifications="failed"] [role="alert"]')).toContainText("Try again");
    expect(await page.evaluate((room) => localStorage.getItem(`fixture.push:/push/${room}/`), second.pairing.room)).toBeNull();
    await page.reload();
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    await page.locator('[data-phone-tab="settings"]').click();
    await expect.poll(() => page.evaluate((scope) => Number(localStorage.getItem(`fixture.cleanup:${scope}`)), scope)).toBeGreaterThan(1);
    await expect(page.locator('[data-phone-notifications="failed"] [role="alert"]')).toContainText("Try again");
    await page.evaluate(() => localStorage.removeItem("fixture.cleanup-refuse"));
    await receive.click();
    await expect(receive).toHaveAttribute("aria-checked", "false");
    expect(await page.evaluate((scope) => localStorage.getItem(`fixture.push:${scope}`), scope)).toBeNull();
    expect(relay.state.notifications).toBe("on"); // Inactive computer was never told Off; its endpoint had to be removed here.
    expect(second.received.filter((frame) => frame.type === "push_permission" && frame.state === "denied").length).toBeGreaterThan(1);
  } finally { await page.goto("about:blank"); await second.stop(); }
});

test("receiving On restores the inactive computer even when its saved state is on", async ({ page, relay }) => {
  const second = await startPhoneRelay(); relay.relayEndpoints.set(second.pairing.room, second.wsUrl);
  try {
    await installBrowserPush(page);
    await pair(page, relay);
    await expect.poll(() => relay.state.notifications).toBe("on");
    await pair(page, { ...second, origin: relay.origin });
    await expect.poll(() => second.state.notifications).toBe("on");
    await page.locator('[data-phone-tab="settings"]').click();
    const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
    await receive.click();
    await expect(receive).toHaveAttribute("aria-checked", "false");
    await receive.click();
    await expect.poll(() => second.state.notifications).toBe("on");
    expect(relay.state.notifications).toBe("on");
    expect(await page.evaluate((room) => localStorage.getItem(`fixture.push:/push/${room}/`), relay.pairing.room)).toBeNull();
    await page.goto(`${relay.origin}/p/${PHONE_PROTOCOL}/?host=${relay.pairing.room}`);
    await expect.poll(() => relay.received.filter((frame) => frame.type === "push_subscription").length).toBe(2);
    expect(await page.evaluate((room) => localStorage.getItem(`fixture.push:/push/${room}/`), relay.pairing.room)).not.toBeNull();
  } finally { await page.goto("about:blank"); await second.stop(); }
});

test("Off cleanup is shared across reconnects and finishes before a newer On", async ({ page, relay }) => {
  await installBrowserPush(page);
  await pair(page, relay);
  await expect.poll(() => relay.state.notifications).toBe("on");
  const scope = `/push/${relay.pairing.room}/`;
  await page.evaluate((scope) => localStorage.setItem("fixture.cleanup-hold", scope), scope);
  await page.locator('[data-phone-tab="settings"]').click();
  const receive = page.getByRole("switch", { name: "Receive notifications", exact: true });
  await receive.click();
  await expect(page.locator('[data-phone-notifications="stopping"]')).toBeVisible();
  await expect.poll(() => page.evaluate(() => "finishCleanup" in window)).toBe(true);
  relay.disconnect();
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_permission" && frame.state === "denied").length).toBe(2);
  await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
  await expect(receive).toBeDisabled();
  expect(await page.evaluate((scope) => localStorage.getItem(`fixture.cleanup:${scope}`), scope)).toBe("1");
  await page.evaluate(() => (window as unknown as { finishCleanup: (ok: boolean) => void }).finishCleanup(false));
  await expect(page.locator('[data-phone-notifications="failed"] [role="alert"]')).toContainText("Try again");
  await page.evaluate(() => localStorage.removeItem("fixture.cleanup-hold"));
  await receive.click();
  await expect(receive).toHaveAttribute("aria-checked", "false");
  await receive.click();
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_subscription").length).toBe(2);
  expect(await page.evaluate((scope) => localStorage.getItem(`fixture.push:${scope}`), scope)).not.toBeNull();
});

for (const reason of ["unchanged", "gone", "key_changed"]) test(`reconnecting reconciles a subscription after ${reason}`, async ({ page, relay }) => {
  await installBrowserPush(page);
  await pair(page, relay);
  await expect.poll(() => relay.state.notifications).toBe("on");
  const before = relay.received.find((frame) => frame.type === "push_subscription")!.subscription;
  if (reason === "gone") relay.state.notifications = "unasked";
  else if (reason === "key_changed") await page.evaluate((room) => {
    const key = `fixture.push:/push/${room}/`;
    const saved = JSON.parse(localStorage.getItem(key)!) as { key: number[] };
    saved.key[1] ^= 1; localStorage.setItem(key, JSON.stringify(saved));
  }, relay.pairing.room);
  const connections = relay.received.filter((frame) => frame.client_kind === "phone").length;
  relay.disconnect();
  await expect.poll(() => relay.received.filter((frame) => frame.client_kind === "phone").length).toBeGreaterThan(connections);
  if (reason === "unchanged") {
    await page.locator('[data-phone-tab="settings"]').click();
    await expect(page.getByRole("switch", { name: "Receive notifications", exact: true })).toHaveAttribute("aria-checked", "true");
    expect(await page.evaluate((room) => localStorage.getItem(`fixture.cleanup:/push/${room}/`), relay.pairing.room)).toBeNull();
    expect(relay.received.filter((frame) => frame.type === "push_subscription")).toHaveLength(1);
    return;
  }
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_subscription").length).toBe(2);
  const after = relay.received.filter((frame) => frame.type === "push_subscription").at(-1)!.subscription;
  expect(after).not.toEqual(before);
  expect(await page.evaluate((room) => localStorage.getItem(`fixture.cleanup:/push/${room}/`), relay.pairing.room)).toBe("1");
});

for (const permission of ["denied", "unsupported"]) {
  test(`hosted notification ${permission} retains its one-line guidance`, async ({ page, relay }) => {
    relay.state.push = true;
    await page.addInitScript((answer) => {
      if (answer === "unsupported") { Reflect.deleteProperty(window, "PushManager"); Reflect.deleteProperty(window, "Notification"); }
      else Object.defineProperty(Notification, "permission", { configurable: true, get: () => "denied" });
    }, permission);
    await pair(page, relay);
    await page.locator('[data-phone-tab="settings"]').click();
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
    await expect(page).toHaveURL(new RegExp(`/p/${PHONE_PROTOCOL}/\\?host=${relay.pairing.room}`));
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
  } finally { await page.goto("about:blank"); await second.stop(); }
});

// Approved B1: the QR names the device, and the device picker is a labelled
// sheet in the phone's language. The hostile name remains an escaping fixture.
test.describe("Korean phone screens", () => {
  test.use({ locale: "ko-KR" });
  test("QR and device sheet keep all actions and counts in Korean", async ({ page, relay }) => {
    await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
    await expect(page.getByRole("heading", { name: relay.pairing.name, exact: true })).toBeVisible();
    await expect(page.getByText("에이전트를 보고 답할 수 있어요.", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "연결", exact: true }).click();
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    const trigger = page.getByRole("button", { name: relay.pairing.name, exact: true });
    await trigger.click();
    const sheet = page.getByRole("dialog", { name: "기기 고르기" });
    await expect(sheet).toBeVisible();
    await expect(sheet.getByText("내 차례 1 · 작업 중 2", { exact: true })).toBeVisible();
    await expect(sheet.getByRole("button", { name: "다른 기기 연결" })).toBeVisible();
    await sheet.getByRole("button", { name: "닫기", exact: true }).click();
    await expect(sheet).toHaveCount(0);
    await expect(trigger).toBeFocused();
  });
});

test("the missions tab lists a mission, and its answer reaches the device as one request that a refusal does not lose", async ({ page, relay }) => {
  await pair(page, relay);
  await expect(page.locator('[data-phone-tab="missions"]')).toHaveAttribute("data-phone-tab-count", "1");
  await page.locator('[data-phone-tab="missions"]').click();
  await expect(page.locator('[data-phone-mission-group="turn"] [data-phone-mission="mission-one"]')).toContainText("Ship the release");
  await expect(page.locator("[data-phone-tabs]")).toBeVisible();
  await page.locator('[data-phone-mission="mission-one"]').click();
  await expect(page.locator("[data-phone-tabs]")).toHaveCount(0);
  await expect(page.locator("[data-phone-question]")).toHaveText("Sign the build?");

  // A refusal comes back in words, the written answer stays, and the same press goes again.
  relay.state.refuseMission = "gone";
  await page.locator("[data-phone-answer-input]").fill("sign it after lunch");
  await page.locator("[data-phone-answer-send]").click();
  await expect(page.locator("[data-phone-answer-error]")).toContainText("no longer open");
  await expect(page.locator("[data-phone-answer-input]")).toHaveValue("sign it after lunch");

  relay.state.refuseMission = "";
  // A press only marks the choice; 답하기 sends it (PRD mission-human-gate B19).
  await expect(page.locator("[data-phone-choice-send]")).toBeDisabled();
  await page.locator('[data-phone-option="1"]').click();
  await expect(page.locator('[data-phone-option="1"]')).toHaveAttribute("aria-checked", "true");
  await page.locator("[data-phone-choice-send]").click();
  await expect(page.locator("[data-phone-decision]")).toHaveAttribute("data-phone-decision", "sending");
  const answers = () => relay.received.filter((frame) => frame.type === "answer_mission");
  await expect.poll(() => answers().length).toBe(2);
  expect(answers()).toMatchObject([{ mission: "mission-one", key: 2, text: "sign it after lunch" }, { mission: "mission-one", key: 2, option: 1 }]);
  expect(answers()[0]!.request_id).not.toBe(answers()[1]!.request_id);
});

test.describe("the Android app's notifications", () => {
  // The page as the Android app serves it: the phone bundle at Capacitor's `https://localhost` origin, with a stand-in of
  // Capacitor's bridge in place of the app (its `HidePush` answers as `HidePushPlugin` does: Google's token, named by its
  // service, and no environment). Pairing goes through the app's own link hand-off (`HideApp.takeLink`); the connection,
  // the Noise session and the device are the fixture's real ones.
  const REGISTRATION = { token: "c".repeat(26) + ":APA91b" + "d".repeat(120), p256dh: "P".repeat(87), auth: "A".repeat(22) };
  const dist = fileURLToPath(new URL("../phone-dist/", import.meta.url));

  async function openAndroidApp(page: Page, relay: Awaited<ReturnType<typeof startPhoneRelay>>) {
    await page.route("https://localhost/**", async (route) => {
      const pathname = new URL(route.request().url()).pathname;
      const file = path.resolve(dist, "." + (pathname.endsWith("/") ? pathname + "index.html" : pathname));
      if (!file.startsWith(dist)) return route.fulfill({ status: 403 });
      return route.fulfill({ path: file, headers: { "Cache-Control": "no-store" } });
    });
    await page.addInitScript(({ link, registration }) => {
      const read = (key: string) => JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, { noiseKey: string; credential?: string }>;
      const granted = () => localStorage.getItem("fixture.permission") === "granted";
      const answer = () => ({ found: true, service: "fcm", ...registration });
      const native: Record<string, (options: Record<string, string>) => unknown> = {
        "HideApp.info": () => ({ version: "1.0.0", build: "1" }),
        "HideApp.takeLink": () => {
          if (sessionStorage.getItem("fixture.link-taken")) return {};
          sessionStorage.setItem("fixture.link-taken", "true");
          return { link };
        },
        "HideApp.appearance": () => ({}),
        "HideKeychain.save": ({ room, noiseKey, credential }) => {
          localStorage.setItem("fixture.keychain", JSON.stringify({ ...read("fixture.keychain"), [room]: { noiseKey, ...(credential ? { credential } : {}) } }));
          return {};
        },
        "HideKeychain.read": ({ room }) => { const item = read("fixture.keychain")[room]; return item ? { found: true, ...item } : { found: false }; },
        "HideKeychain.list": () => ({ rooms: Object.keys(read("fixture.keychain")) }),
        "HidePush.state": () => ({ permission: granted() ? "granted" : "unknown" }),
        "HidePush.enable": () => { localStorage.setItem("fixture.permission", "granted"); return answer(); },
        "HidePush.registration": () => granted() ? answer() : { found: false },
        "HidePush.forget": () => ({}),
        "HidePush.takeOpen": () => ({}),
      };
      Object.assign(window, { Capacitor: {
        getPlatform: () => "android",
        isPluginAvailable: (name: string) => ["HideApp", "HideKeychain", "HideScanner", "HidePush"].includes(name),
        nativePromise: async (plugin: string, method: string, options: Record<string, string> = {}) => {
          const call = native[`${plugin}.${method}`];
          if (!call) throw Object.assign(new Error(`${plugin}.${method}`), { code: "unimplemented" });
          return call(options);
        },
        addListener: () => ({ remove: () => undefined }),
      } });
    }, { link: relay.qr, registration: REGISTRATION });
    await page.goto("https://localhost/");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    expect(await page.evaluate(() => window.location.origin)).toBe("https://localhost");
    await page.locator('[data-phone-tab="settings"]').click();
    return page.getByRole("switch", { name: "Receive notifications", exact: true });
  }

  const registrations = (relay: Awaited<ReturnType<typeof startPhoneRelay>>) => relay.received.filter((frame) => String(frame.type).endsWith("_registration"));
  const sessions = (relay: Awaited<ReturnType<typeof startPhoneRelay>>) => relay.received.filter((frame) => frame.client_kind === "phone").length;

  test("the switch gives the computer Google's token and the phone's key, and reads on", async ({ page, relay }) => {
    const receive = await openAndroidApp(page, relay);
    await expect(page.locator('[data-phone-notifications="off"]')).toBeVisible();
    await receive.click();
    await expect.poll(() => registrations(relay)).toEqual([{ type: "fcm_registration", token: REGISTRATION.token, key: { p256dh: REGISTRATION.p256dh, auth: REGISTRATION.auth } }]);
    await expect(page.locator('[data-phone-notifications="on"]')).toBeVisible();
    await expect(receive).toHaveAttribute("aria-checked", "true");
  });

  test("a computer that refuses the registration is marked outdated and the session goes on", async ({ page, relay }) => {
    relay.state.refuseAppRegistration = true;
    const receive = await openAndroidApp(page, relay);
    await receive.click();
    await expect.poll(() => registrations(relay).map((frame) => frame.type)).toEqual(["fcm_registration"]);
    await expect(page.locator("[data-phone-notifications]")).toContainText("Update Hide on the computer");
    // The same session: the agents list is still live, and the phone never joined again.
    await page.locator('[data-phone-tab="agents"]').click();
    await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
    expect(sessions(relay)).toBe(1);
  });
});
