import { expect, test as base } from "@playwright/test";
import { WebSocket } from "ws";
import qrcode from "qrcode-generator";
import { startPhoneRelay } from "./phone-relay-fixture";
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

async function pair(page: import("@playwright/test").Page, relay: Awaited<ReturnType<typeof startPhoneRelay>>) {
  await page.goto(relay.origin + "/" + new URL(relay.qr).hash);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.locator('[data-phone-connected="true"]')).toBeVisible();
}

test("QR clears its secret, pairs over Noise and uses the shared conversation, terminal and reply UI", async ({ page, relay }) => {
  // A camera/link can return to a root page that is already open (same-document navigation).
  await page.goto(relay.origin + "/");
  await expect(page.getByRole("button", { name: "Connect another device" })).toBeVisible();
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
      if (surface === "root") await page.goto(relay.origin);
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
  await page.goto(relay.origin + "/");
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
  // No computer yet: the first screen's big button opens the browser's scan and paste page.
  await page.getByRole("button", { name: "Scan QR code", exact: true }).click();
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

test("the in-app camera decodes a QR and releases its video track", async ({ page, relay }) => {
  const qr = qrcode(0, "M"); qr.addData(relay.qr); qr.make();
  const matrix = Array.from({ length: qr.getModuleCount() }, (_, row) =>
    Array.from({ length: qr.getModuleCount() }, (_, column) => qr.isDark(row, column)));
  await page.addInitScript((cells) => {
    navigator.mediaDevices.getUserMedia = async () => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = (cells.length + 8) * 4;
      const context = canvas.getContext("2d")!;
      context.fillStyle = "white"; context.fillRect(0, 0, canvas.width, canvas.height);
      context.fillStyle = "black";
      cells.forEach((row, y) => row.forEach((dark, x) => { if (dark) context.fillRect((x + 4) * 4, (y + 4) * 4, 4, 4); }));
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
  // Notifications and the paired computers live in the Settings tab.
  await page.locator('[data-phone-tab="settings"]').click();
  await page.getByRole("button", { name: "Turn on notifications", exact: true }).click();
  await expect(page.locator('[data-phone-notifications="enable"]')).toHaveCount(0);
  await expect.poll(() => relay.received.filter((frame) => frame.type === "push_subscription").length).toBe(1);
  const scopes = () => page.evaluate(async () => (await navigator.serviceWorker.getRegistrations()).map((worker) => new URL(worker.scope).pathname).sort());
  await expect.poll(scopes).toEqual(["/", `/push/${relay.pairing.room}/`]);
  await page.getByRole("button", { name: "Remove device", exact: true }).click();
  await page.getByRole("button", { name: "Remove device", exact: true }).last().click();
  await expect(page).toHaveURL(relay.origin + "/");
  await expect(page.getByRole("heading", { name: "Connect your computer" })).toBeVisible();
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
  await page.locator('[data-phone-option="1"]').click();
  await expect(page.locator("[data-phone-decision]")).toHaveAttribute("data-phone-decision", "sending");
  const answers = () => relay.received.filter((frame) => frame.type === "answer_mission");
  await expect.poll(() => answers().length).toBe(2);
  expect(answers()).toMatchObject([{ mission: "mission-one", key: 2, text: "sign it after lunch" }, { mission: "mission-one", key: 2, option: 1 }]);
  expect(answers()[0]!.request_id).not.toBe(answers()[1]!.request_id);
});
