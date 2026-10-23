// The mobile companion on an isolated pinned Herdr and a debug hided (PRD
// mobile-companion D-11): Settings > Mobile walks a fake `tailscale` through
// its four checklist states to a QR; an iPhone-sized touch page pairs with the
// QR's code, lists the agents in four groups, opens a detail, and its reply
// and quick keys reach the pane's PTY; a loopback push endpoint decrypts
// what hided sends and proves the three modes, the viewing rule and the Seen
// clear; revoke, the phone limit, the seven-day revoke, the unreachable line
// and the empty list follow; the start sheet starts an agent in a checkout and
// in Home and keeps its text while the phone is away; and no code, credential
// or key reaches the log.
//
// Nothing here runs the operator's Tailscale: HIDE_TAILSCALE_BIN names a
// script in the test's own directory, and the push endpoint is a local server
// only a debug hided accepts.

import { devices, expect, test, type Browser, type BrowserContext, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { elsewhereTab, finishFixtureTurn, labelAgent, labelMarker, setFixtureLifecycle, setFixtureSession, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { DNS, FakeTailscale } from "./fake-tailscale";
import { startHided, type Daemon } from "./hided-fixture";
import { screenshot, THIS_MACHINE } from "./wire";
import { chord } from "./chords";

test.describe.configure({ timeout: 240_000 });

type Push = { authorization: string; payload: { title: string; state: "needs_you" | "done"; place: string; tag: string; device_id: string; pane_id: string; clear: string[] } };

/** A push service on loopback that decrypts RFC 8291 aes128gcm with the phone's own keys. */
async function pushService() {
  const ecdh = crypto.createECDH("prime256v1");
  ecdh.generateKeys();
  const auth = crypto.randomBytes(16);
  const received: Push[] = [];
  const decrypt = (body: Buffer) => {
    const salt = body.subarray(0, 16);
    const idLength = body[20] ?? 0;
    const serverKey = body.subarray(21, 21 + idLength);
    const record = body.subarray(21 + idLength);
    const secret = ecdh.computeSecret(serverKey);
    const info = Buffer.concat([Buffer.from("WebPush: info\0"), ecdh.getPublicKey(), serverKey]);
    const ikm = Buffer.from(crypto.hkdfSync("sha256", secret, auth, info, 32));
    const cek = Buffer.from(crypto.hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: aes128gcm\0"), 16));
    const nonce = Buffer.from(crypto.hkdfSync("sha256", ikm, salt, Buffer.from("Content-Encoding: nonce\0"), 12));
    const decipher = crypto.createDecipheriv("aes-128-gcm", cek, nonce);
    decipher.setAuthTag(record.subarray(record.length - 16));
    const plain = Buffer.concat([decipher.update(record.subarray(0, record.length - 16)), decipher.final()]);
    return plain.subarray(0, plain.lastIndexOf(2)).toString("utf8");
  };
  const server = http.createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => {
      try {
        received.push({ authorization: String(request.headers.authorization ?? ""), payload: JSON.parse(decrypt(Buffer.concat(chunks))) as Push["payload"] });
        response.writeHead(201);
      } catch {
        response.writeHead(400);
      }
      response.end();
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as { port: number }).port;
  const b64 = (bytes: Buffer) => bytes.toString("base64url");
  return {
    endpoint: `http://127.0.0.1:${port}/push/phone-one`,
    p256dh: b64(ecdh.getPublicKey()),
    auth: b64(auth),
    received,
    close: () => server.close(),
  };
}

/**
 * An iPhone-sized touch page. Chromium has no push service of its own here,
 * so the page's PushManager hands out the loopback endpoint's subscription
 * and the permission answer is "granted" once asked. The phone's own language
 * is English unless a test asks for another.
 */
async function phoneContext(browser: Browser, push?: { endpoint: string; p256dh: string; auth: string }, locale = "en-US"): Promise<BrowserContext> {
  const iphone = devices["iPhone 13"];
  const context = await browser.newContext({
    viewport: iphone.viewport,
    deviceScaleFactor: iphone.deviceScaleFactor,
    userAgent: iphone.userAgent,
    isMobile: true,
    hasTouch: true,
    colorScheme: "dark",
    locale,
  });
  if (push) {
    await context.addInitScript((subscription) => {
      const record = { subscribed: 0, unsubscribed: 0, permission: "default" as NotificationPermission };
      (window as unknown as { __push: typeof record }).__push = record;
      let current: unknown = null;
      const fake = {
        endpoint: subscription.endpoint,
        toJSON: () => ({ endpoint: subscription.endpoint, keys: { p256dh: subscription.p256dh, auth: subscription.auth } }),
        unsubscribe: async () => {
          record.unsubscribed += 1;
          current = null;
          return true;
        },
      };
      Object.defineProperty(Notification, "permission", { configurable: true, get: () => record.permission });
      Notification.requestPermission = async () => {
        record.permission = "granted";
        return "granted";
      };
      PushManager.prototype.subscribe = async function () {
        record.subscribed += 1;
        current = fake;
        return fake as unknown as PushSubscription;
      };
      PushManager.prototype.getSubscription = async function () {
        return current as PushSubscription | null;
      };
    }, push);
  }
  return context;
}

const SESSION = "0f0e0d0c-0b0a-4000-8000-000000000001";

/** One Claude transcript record: an operator's prompt or the agent's text. */
function turn(type: "user" | "assistant", text: string, minute: number): string {
  const timestamp = `2026-09-29T${String(9 + Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}:00Z`;
  return JSON.stringify(
    type === "user"
      ? { type, sessionId: SESSION, userType: "external", promptId: `prompt-${minute}`, timestamp, message: { role: "user", content: text } }
      : { type, sessionId: SESSION, timestamp, message: { role: "assistant", content: [{ type: "text", text }] } },
  );
}

function toolOutput(text: string): string {
  return JSON.stringify({ type: "user", sessionId: SESSION, timestamp: "2026-09-29T10:05:00Z", message: { role: "user", content: [{ type: "tool_result", content: text }] } });
}

/** The fixture's titles, which the core keeps on every label it makes. */
function title(herdr: HerdrFixture, pane: string): string {
  return pane === herdr.panes[0] ? "Agent one" : "Agent two";
}

/** The agent stops and its label asks the operator `question`. */
async function ask(herdr: HerdrFixture, pane: string, question: string): Promise<void> {
  await setFixtureLifecycle(herdr, pane, "idle");
  labelAgent(herdr, pane, { task: title(herdr, pane), reply: question, question: true });
}

async function work(herdr: HerdrFixture, pane: string): Promise<void> {
  await setFixtureLifecycle(herdr, pane, "working");
}

async function openMobileSettings(page: Page, daemon: Daemon): Promise<void> {
  // A fresh document: the same address with only a new fragment would not reload.
  await page.goto("about:blank");
  await page.goto(`${daemon.origin}/#token=${daemon.token}`);
  await expect(page.locator("[data-sidebar-overview]")).toBeVisible({ timeout: 20_000 });
  await page.keyboard.press(chord("settings"));
  await page.locator('[data-settings-tab="mobile"]').click();
  await expect(page.locator('[data-mobile-tab="true"]')).toBeVisible();
}

/** Show QR: the one action that makes a pairing code; opening the tab makes none (B58). */
async function showPairing(page: Page): Promise<void> {
  await page.locator('[data-mobile-show-code="true"]').click({ timeout: 20_000 });
}

/** Picks a push mode in the one dropdown on Settings > Notifications (PRD settings-left-nav B6), and returns to Mobile. */
async function choosePush(page: Page, mode: "off" | "app_closed" | "always"): Promise<void> {
  await page.locator('[data-settings-tab="notifications"]').click();
  await page.locator("[data-push-select]").click();
  await page.locator(`[data-push-choice="${mode}"]`).click();
  await expect(page.locator(`[data-push-detail="${mode}"]`)).toBeVisible();
  await page.locator('[data-settings-tab="mobile"]').click();
  await expect(page.locator('[data-mobile-tab="true"]')).toBeVisible();
}

/** The push mode Settings > Notifications shows, read there before returning to Mobile. */
async function expectPush(page: Page, label: string): Promise<void> {
  await page.locator('[data-settings-tab="notifications"]').click();
  await expect(page.locator("[data-push-select]")).toHaveText(label);
  await page.locator('[data-settings-tab="mobile"]').click();
  await expect(page.locator('[data-mobile-tab="true"]')).toBeVisible();
}

/** The QR's `#pair=...` fragment, opened on this loopback daemon instead of the ts.net address. */
async function pairingUrl(page: Page, daemon: Daemon): Promise<string> {
  const qr = page.locator("[data-mobile-qr]");
  await expect(qr).toBeVisible({ timeout: 20_000 });
  const url = (await qr.getAttribute("data-mobile-qr")) ?? "";
  expect(url.startsWith(`https://${DNS}/m/#pair=`)).toBe(true);
  return `${daemon.origin}/m/${url.slice(url.indexOf("#"))}`;
}

/** Presses Show QR, then reads the pairing address it shows. */
async function shownPairingUrl(page: Page, daemon: Daemon): Promise<string> {
  await showPairing(page);
  return pairingUrl(page, daemon);
}

function coreLog(daemon: Daemon): string {
  const dir = path.join(daemon.stateDir, "Logs");
  if (!fs.existsSync(dir)) return "";
  return fs
    .readdirSync(dir)
    .filter((name) => name.startsWith("core"))
    .map((name) => fs.readFileSync(path.join(dir, name), "utf8"))
    .join("\n");
}

/** The group the phone was last told a pane is in, from its own `agents` frames. */
function groupOf(frames: string[], pane: string): string | null {
  for (let index = frames.length - 1; index >= 0; index -= 1) {
    const frame = JSON.parse(frames[index] ?? "{}") as { type?: string; groups?: { group: string; agents: { pane_id: string }[] }[] };
    if (frame.type !== "agents") continue;
    return frame.groups?.find((group) => group.agents.some((agent) => agent.pane_id === pane))?.group ?? null;
  }
  return null;
}

async function noPushFor(received: Push[], count: number, ms = 3_000): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
  expect(received).toHaveLength(count);
}

test("Settings > Mobile to a paired phone: list, detail, reply, quick keys, push rules and revoke", async ({ browser, page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr();
  const tailscale = new FakeTailscale();
  const push = await pushService();
  let daemon: Daemon | null = null;
  const contexts: BrowserContext[] = [];
  try {
    const [one, two] = herdr.panes;
    // Herdr's clients look here while a turn ends, so it ends unseen: done.
    const elsewhere = elsewhereTab(herdr);
    const finish = (pane: string) => finishFixtureTurn(herdr, pane, elsewhere);
    daemon = await startHided(herdr, "mobile", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });

    // B1: off by default, and nothing asked Tailscale anything.
    await openMobileSettings(page, daemon);
    const toggle = page.locator('[data-mobile-switch="true"]');
    await expect(toggle).toHaveAttribute("aria-checked", "false");
    await expect(page.locator('[data-mobile-tab="true"]')).toContainText("Over Tailscale, only inside your tailnet.");
    await expect(page.locator("[data-push-select]")).toHaveCount(0);
    await expectPush(page, "Off");
    expect(tailscale.calls()).toBe("");

    // B2: the CLI is missing, so the first step carries the download link and the rest wait.
    await toggle.click();
    const step = (id: string) => page.locator(`[data-mobile-step="${id}"]`);
    await expect(step("installed")).toHaveAttribute("data-step-state", "failed", { timeout: 20_000 });
    await expect(page.locator('[data-mobile-step-link="installed"]')).toHaveAttribute("href", "https://tailscale.com/download");
    // B57: only the failing step shows; the ones after it wait unseen, and there is nothing to pair with yet.
    for (const id of ["logged_in", "https"]) await expect(step(id)).toHaveCount(0);
    await expect(page.locator("[data-mobile-pairing]")).toHaveCount(0);

    // B3: installed and logged out, then HTTPS off, each seen without reopening the tab.
    tailscale.loggedOut();
    tailscale.install();
    await expect(step("installed")).toHaveCount(0, { timeout: 20_000 });
    await expect(step("logged_in")).toHaveAttribute("data-step-state", "failed");
    await expect(step("logged_in")).toContainText("Sign in in the Tailscale app");
    tailscale.httpsOff();
    await expect(step("https")).toHaveAttribute("data-step-state", "failed", { timeout: 20_000 });
    await expect(page.locator('[data-mobile-step-link="https"]')).toHaveAttribute("href", "https://login.tailscale.com/admin/dns");
    await screenshot(page, "mobile-settings-blocked");

    // B4, B57, B58: every Mac step passes, the checks fold into one line, hided adds its serve entry, and the QR appears only on Show QR.
    tailscale.ready();
    await expect(page.locator('[data-mobile-ready="true"]')).toContainText("Tailscale is ready", { timeout: 20_000 });
    expect(tailscale.proxy()).toBe(daemon.origin);
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);
    const firstUrl = await shownPairingUrl(page, daemon);
    await expect(page.locator("[data-mobile-url]")).toHaveAttribute("data-mobile-url", `https://${DNS}`);
    await expect(page.locator("[data-mobile-countdown]")).toHaveText(/Code expires in [45]:\d\d/);

    // B10, B58: Hide QR takes the code out of the page, and the next Show QR makes a new one that voids the one before it.
    await page.locator('[data-mobile-hide-code="true"]').click();
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);
    const pairUrl = await shownPairingUrl(page, daemon);
    expect(pairUrl).not.toBe(firstUrl);
    const stale = await (await phoneContext(browser, undefined, "ko-KR")).newPage();
    contexts.push(stale.context());
    await stale.goto(firstUrl);
    await stale.locator('[data-phone-pair="true"]').tap();
    await expect(stale.locator('[data-phone-guidance="code_expired"]')).toHaveText("코드가 만료됐어요. Mac에서 QR을 다시 여세요.", { timeout: 20_000 });

    // Agent states before the phone opens: one asks, two works.
    await ask(herdr, one, "배포 전에 테스트를 다시 돌릴까요?");
    await work(herdr, two);

    // B11: the QR opens the pairing page; Connect lands on the list and the Mac lists the phone.
    const phoneContextOne = await phoneContext(browser, push);
    contexts.push(phoneContextOne);
    const phone = await phoneContextOne.newPage();
    const frames: string[] = [];
    phone.on("websocket", (socket) => socket.on("framereceived", (frame) => frames.push(String(frame.payload))));
    await phone.goto(pairUrl);
    await expect(phone.getByRole("heading", { name: "Connect to mac" })).toBeVisible();
    await expect(phone.getByText("The code expires in 5 minutes. If it expires, reopen the QR code on your Mac.")).toBeVisible();
    await screenshot(phone, "mobile-phone-pair");
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 20_000 });
    await expect(phone.locator('[data-phone-install-hint="true"]')).toBeVisible();
    await expect(page.locator("[data-mobile-phones]")).toHaveAttribute("data-mobile-phones", "1");
    await expect(page.locator("[data-mobile-phone-line]")).toHaveText("Just now");
    await expect(page.getByText("Connected phones · 1 / 4")).toBeVisible();
    // The spent code is gone from the page; the next phone needs Show QR (B58).
    await expect(page.locator("[data-mobile-qr]")).toHaveCount(0);
    // B10: the code that paired is spent.
    const again = await (await phoneContext(browser)).newPage();
    contexts.push(again.context());
    await again.goto(pairUrl);
    await again.locator('[data-phone-pair="true"]').tap();
    await expect(again.locator('[data-phone-guidance="code_expired"]')).toBeVisible({ timeout: 20_000 });
    // The credential is in the page address for Add to Home Screen, never the code.
    expect(new URL(phone.url()).hash).toMatch(/^#k=[0-9a-f]{64}$/);

    // B19, B20: the header and the four groups in order, each row's request line.
    await expect(phone.locator("[data-phone-header]")).toHaveAttribute("data-phone-header", "mac");
    const oneRow = phone.locator(`[data-phone-agent$="|${one}"]`);
    const twoRow = phone.locator(`[data-phone-agent$="|${two}"]`);
    await expect(phone.locator('[data-phone-group="needs_you"]').locator(`[data-phone-agent$="|${one}"]`)).toBeVisible({ timeout: 20_000 });
    await expect(oneRow.locator("[data-phone-line]")).toHaveText("Question · 배포 전에 테스트를 다시 돌릴까요?");
    await expect(phone.locator('[data-phone-group="working"]').locator(`[data-phone-agent$="|${two}"]`)).toBeVisible();
    await expect(phone.locator("[data-phone-group] h2").first()).toContainText("Needs You");
    // B21: live; two asks and joins one under Needs You without a reload.
    await ask(herdr, two, "어느 브랜치에 올릴까요?");
    await expect(phone.locator('[data-phone-group="needs_you"] [data-phone-group-count]')).toHaveText("2", { timeout: 20_000 });
    // B41: an idle list sends nothing; the phone hears only changes.
    const agentFrames = () => frames.filter((frame) => frame.includes('"type":"agents"')).length;
    const idleStart = agentFrames();
    await new Promise((resolve) => setTimeout(resolve, 3_000));
    expect(agentFrames() - idleStart).toBeLessThanOrEqual(1);
    await screenshot(phone, "mobile-phone-list");

    // The agent's conversation: Herdr reports one's Claude session, whose
    // transcript holds 70 turns with tool output and injected context among them.
    const transcript = path.join(daemon.home, ".claude", "projects", "-fixture", `${SESSION}.jsonl`);
    fs.mkdirSync(path.dirname(transcript), { recursive: true });
    const turns = Array.from({ length: 70 }, (_, index) => turn(index % 2 === 0 ? "user" : "assistant", `turn ${String(index).padStart(2, "0")}`, index));
    // The first request carries what the fixture provider answers, so the
    // session keeps the agent's title.
    turns[0] = turn("user", `turn 00 ${labelMarker({ task: "Agent one" })}`, 0);
    turns[69] = turn("assistant", "turn 69 **굵게**\n\n- 항목 하나\n- 항목 둘\n\n```\ncode line\n```", 69);
    turns.splice(66, 0, toolOutput("SECRET-TOOL-OUTPUT"), turn("user", "<system-reminder>INJECTED-CONTEXT</system-reminder>", 66));
    fs.writeFileSync(transcript, `${turns.join("\n")}\n`);
    setFixtureSession(herdr, one, SESSION);

    // B24: the detail shows the head and the agent's newest 30 messages, newest at the bottom.
    const history = [...Array.from({ length: 260 }, (_, index) => `line ${String(index + 1).padStart(3, "0")}`), `wide ${"w".repeat(300)}`].join("\n");
    execFileSync(herdr.bin, ["pane", "send-text", one, `${history}\n`], { env: herdr.env, timeout: 30_000 });
    await oneRow.tap();
    const detail = phone.locator("[data-phone-detail]");
    await expect(detail).toHaveAttribute("data-phone-detail", new RegExp(`\\|${one}$`));
    await expect(detail).toContainText("Agent one");
    const conversation = phone.locator("[data-phone-conversation]");
    await expect(conversation).toHaveAttribute("data-phone-conversation", "30", { timeout: 20_000 });
    await expect(conversation.locator("[data-phone-message]").last()).toContainText("turn 69");
    await expect(conversation).not.toContainText("turn 39");
    await expect(conversation.locator('[data-phone-message="you"]').first()).toContainText("turn 40");
    // The agent's Markdown is drawn, not shown as marks.
    const newest = conversation.locator("[data-phone-message]").last();
    await expect(newest.locator("strong")).toHaveText("굵게");
    await expect(newest.locator("li")).toHaveCount(2);
    await expect(newest.locator("pre")).toHaveText("code line");
    await expect(newest).not.toContainText("**");
    // Pulling to the top brings the older pages, down to the first turn.
    await conversation.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(conversation).toHaveAttribute("data-phone-conversation", "60", { timeout: 20_000 });
    await conversation.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(conversation).toHaveAttribute("data-phone-conversation", "70", { timeout: 20_000 });
    await expect(conversation.locator("[data-phone-message]").first()).toContainText("turn 00");
    await expect(phone.locator("[data-phone-older]")).toHaveCount(0);
    // What the agent writes next arrives on its own.
    fs.appendFileSync(transcript, `${turn("assistant", "turn 70 arrived", 70)}\n`);
    await expect(conversation.locator("[data-phone-message]").last()).toContainText("turn 70 arrived", { timeout: 20_000 });
    // Tool output and injected context never leave the Mac.
    expect(frames.some((frame) => frame.includes("SECRET-TOOL-OUTPUT") || frame.includes("INJECTED-CONTEXT"))).toBe(false);
    expect(await conversation.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await conversation.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await screenshot(phone, "mobile-phone-conversation");

    // Terminal: the pane's recent rows.
    await phone.locator('[data-phone-view="terminal"]').tap();
    const rows = phone.locator('[aria-label="Recent terminal output"]');
    await expect(rows).toContainText("line 260", { timeout: 20_000 });
    await expect(rows).not.toContainText("line 001");
    const scrollback = phone.locator("[data-phone-scrollback]");
    expect(await scrollback.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight)).toBeLessThan(48);
    // B24: a row as wide as the desktop pane wraps at the phone's width; nothing scrolls sideways.
    await expect(rows).toContainText("wide www");
    expect(await scrollback.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    expect(await phone.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
    // B25: pulling to the top brings the older rows, up to what the pane holds.
    await expect(phone.getByText("Pull up to load more")).toBeVisible();
    await scrollback.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(rows).toContainText("line 001", { timeout: 20_000 });
    await expect(scrollback).toHaveAttribute("data-phone-scrollback", "400");
    await expect(phone.getByText("Pull up to load more")).toHaveCount(0);
    await expect(rows).toContainText("fixture %");
    for (const name of ["Enter", "Escape", "Up arrow", "Down arrow", "Ctrl-C"]) await expect(phone.getByRole("button", { name, exact: true })).toBeVisible();
    // B38: fingers fit every control.
    for (const control of [phone.locator('[data-phone-key="enter"]'), phone.locator('[data-phone-send="true"]'), phone.locator('[data-phone-reply="true"]')]) {
      const box = await control.boundingBox();
      expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    }

    // B26: an empty reply cannot be sent; a reply lands with Enter and the field clears.
    await expect(phone.locator('[data-phone-send="true"]')).toBeDisabled();
    await phone.locator('[data-phone-reply="true"]').fill("yes please");
    await phone.locator('[data-phone-send="true"]').tap();
    await expect(phone.locator('[data-phone-reply="true"]')).toHaveValue("", { timeout: 20_000 });
    await expect.poll(() => (fs.existsSync(herdr.inputLogs[0]) ? fs.readFileSync(herdr.inputLogs[0], "utf8") : "")).toMatch(/yes please[\r\n]/);
    // B27: exactly once.
    expect(fs.readFileSync(herdr.inputLogs[0], "utf8").split("yes please").length - 1).toBe(1);
    // B25: the echoed reply reaches the scrollback on its own.
    await expect(phone.locator('[aria-label="Recent terminal output"]')).toContainText("yes please", { timeout: 20_000 });
    // B26: one key each.
    const logLength = fs.readFileSync(herdr.inputLogs[0], "utf8").length;
    await phone.locator('[data-phone-key="escape"]').tap();
    await expect.poll(() => fs.readFileSync(herdr.inputLogs[0], "utf8").slice(logLength)).toBe("\u001b");
    await phone.locator('[data-phone-key="up"]').tap();
    // Up is CSI A, or SS3 A in the application cursor mode a TUI may set.
    await expect.poll(() => ["\u001b\u001b[A", "\u001b\u001bOA"].includes(fs.readFileSync(herdr.inputLogs[0], "utf8").slice(logLength))).toBe(true);
    // B27: a reply over the limit is named before it is sent.
    await phone.locator('[data-phone-reply="true"]').fill("x".repeat(2001));
    await expect(phone.locator('[data-phone-input-error="true"]')).toContainText("up to 2,000 characters");
    await expect(phone.locator('[data-phone-send="true"]')).toBeDisabled();
    await phone.locator('[data-phone-reply="true"]').fill("");
    await scrollback.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await screenshot(phone, "mobile-phone-detail");

    // B30: with push on, the list offers Turn on notifications; allowing it registers the subscription.
    await choosePush(page, "always");
    await phone.locator('[data-phone-back="true"]').tap();
    const enable = phone.locator('[data-phone-notifications="enable"]');
    await expect(enable).toBeVisible({ timeout: 20_000 });
    await enable.tap();
    await expect(enable).toHaveCount(0, { timeout: 20_000 });
    await expect(page.locator("[data-mobile-phone-line]")).toHaveText("Just now · Receiving notifications");

    // B31, B33 (always): two finishes; one notification for it, with no terminal content.
    await finish(two);
    await expect.poll(() => push.received.length, { timeout: 20_000 }).toBe(1);
    const done = push.received[0]!;
    expect(done.payload.title).toBe("Agent two");
    expect(done.payload.state).toBe("done");
    expect(done.payload.tag).toBe(`${done.payload.device_id}|${two}`);
    expect(done.payload.pane_id).toBe(two);
    expect(JSON.stringify(done.payload)).not.toContain("fixture %");
    expect(done.authorization).toMatch(/^vapid t=[\w-]+\.[\w-]+\.[\w-]+, k=[\w-]+$/);

    // B33: while the phone views one, one's request sends nothing.
    await oneRow.tap();
    await expect(detail).toBeVisible();
    await work(herdr, one);
    await expect.poll(() => groupOf(frames, one)).toBe("working");
    await ask(herdr, one, "정말 배포할까요?");
    await expect.poll(() => groupOf(frames, one)).toBe("needs_you");
    await noPushFor(push.received, 1);
    await expect.poll(() => coreLog(daemon as Daemon)).toContain('"reason":"viewing"');
    await phone.locator('[data-phone-back="true"]').tap();

    // B33 (off): nothing.
    await choosePush(page, "off");
    await work(herdr, two);
    await expect.poll(() => groupOf(frames, two)).toBe("working");
    await ask(herdr, two, "끔에서는 조용히");
    await expect.poll(() => groupOf(frames, two)).toBe("needs_you");
    await noPushFor(push.received, 1);

    // B33 (only while the app is closed): nothing while the desktop is connected. The
    // desktop then reads two, which clears its notification (B34).
    await choosePush(page, "app_closed");
    await work(herdr, two);
    await expect.poll(() => groupOf(frames, two)).toBe("working");
    await finish(two);
    await expect.poll(() => groupOf(frames, two)).toBe("done");
    await noPushFor(push.received, 1);
    await expect.poll(() => coreLog(daemon as Daemon)).toContain('"mode":"app_closed","renderers":1');
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-mobile-tab="true"]')).toHaveCount(0);
    await page.locator(`nav[data-sidebar] [data-project] [data-agent-open="${two}"]`).click();
    await expect(phone.locator('[data-phone-group="seen"]').locator(`[data-phone-agent$="|${two}"]`)).toBeVisible({ timeout: 20_000 });
    await page.close();
    await work(herdr, one);
    await expect.poll(() => groupOf(frames, one)).toBe("working");
    await ask(herdr, one, "데스크톱이 닫혔을 때");
    await expect.poll(() => push.received.length, { timeout: 20_000 }).toBe(2);
    const closed = push.received[1]!;
    expect(closed.payload.title).toBe("Agent one");
    expect(closed.payload.state).toBe("needs_you");
    expect(closed.payload.clear).toContain(`${closed.payload.device_id}|${two}`);

    // B28: the pane closes under an open detail; the reply bar goes inert.
    await twoRow.tap();
    await phone.locator('[data-phone-view="terminal"]').tap();
    await expect(phone.locator("[data-phone-scrollback]")).toBeVisible({ timeout: 20_000 });
    execFileSync(herdr.bin, ["pane", "close", two], { env: herdr.env, timeout: 30_000 });
    await expect(phone.locator('[data-phone-rows-state="gone"]')).toHaveText("This pane is no longer open.", { timeout: 20_000 });
    await expect(phone.locator('[data-phone-reply="true"]')).toBeDisabled();
    await screenshot(phone, "mobile-phone-gone");
    await phone.locator('[data-phone-back="true"]').tap();

    // B15, B36: revoke from the Mac closes the phone at once and drops its subscription.
    const desk = await page.context().newPage();
    await openMobileSettings(desk, daemon);
    await desk.locator("[data-mobile-revoke]").click();
    await expect(desk.locator("[data-mobile-phone]")).toHaveCount(0);
    await expect(phone.locator('[data-phone-guidance="revoked"]')).toHaveText("This phone's connection was revoked. Reopen the QR code on your Mac.", { timeout: 20_000 });
    await expect.poll(() => phone.evaluate(() => (window as unknown as { __push: { unsubscribed: number } }).__push.unsubscribed)).toBe(1);
    const phones = JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "phones.json"), "utf8")) as { phones: unknown[] };
    expect(phones.phones).toHaveLength(0);
    await screenshot(phone, "mobile-phone-revoked");

    // B7: switching off removes only hide's own serve entry.
    await desk.locator('[data-mobile-switch="true"]').click();
    await expect.poll(() => tailscale.proxy(), { timeout: 20_000 }).toBeNull();
    expect(tailscale.calls()).toContain("serve --yes --https=443 --set-path=/ off");

    // B40: each kind is its own record, and no secret reaches the log.
    const log = coreLog(daemon);
    for (const component of ["mobile_transport", "mobile_pairing", "mobile_phone", "mobile_push"]) expect(log).toContain(`"component":"${component}"`);
    const code = new URLSearchParams(pairUrl.slice(pairUrl.indexOf("#") + 1)).get("pair") ?? "";
    const credential = new URL(phone.url()).hash.slice(3);
    for (const secret of [code, JSON.parse(Buffer.from(code, "base64url").toString("utf8")).code as string, credential, push.p256dh, push.auth]) {
      if (secret) expect(log).not.toContain(secret);
    }
  } finally {
    for (const context of contexts) await context.close();
    push.close();
    daemon?.stop();
    herdr.stop();
    tailscale.remove();
  }
});

test("the phone caches group and row status words in the desktop language", async ({ browser, page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ agents: false });
  const tailscale = new FakeTailscale();
  tailscale.install();
  tailscale.ready();
  let daemon: Daemon | null = null;
  let context: BrowserContext | null = null;
  try {
    daemon = await startHided(herdr, "mobile-language", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await openMobileSettings(page, daemon);
    await page.locator('[data-mobile-switch="true"]').click();
    await choosePush(page, "app_closed");
    context = await phoneContext(browser);
    const phone = await context.newPage();
    await phone.goto(await shownPairingUrl(page, daemon));
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator("[data-phone-empty]")).toHaveText("No agents are running", { timeout: 20_000 });

    // The core's explicit language reaches the phone and gives way to the phone's own once unset.
    await page.locator('[data-settings-tab="general"]').click();
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="ko"]').click();
    await expect(phone.locator("[data-phone-empty]")).toHaveText("실행 중인 에이전트가 없어요", { timeout: 20_000 });
    await expect(phone.locator("html")).toHaveAttribute("lang", "ko");
    // Group and row status words stay cached for notifications while the app is closed.
    await expect
      .poll(() =>
        phone.evaluate(async () => {
          const hit = await (await caches.open("hide-phone-words")).match("/m/words.json");
          return hit ? ((await hit.json()) as unknown) : null;
        }),
      )
      .toEqual({
        needs_you: "내 차례",
        done: "읽을 것",
        observer_unconfirmed: "관찰자가 경고를 확인하지 않았어요",
        letter_undelivered: "편지가 전달되지 않았어요",
        status_question: "질문",
        status_approval: "승인",
        status_error: "막힘",
        status_working: "작업 중",
        status_done: "끝남",
        status_idle: "쉬는 중",
        status_unknown: "상태 모름",
        status_waiting: "대기",
        status_stopped: "멈춤",
        status_unfinished: "일 남음",
        subject_plan_review: "계획 검토 · {{count}}단계",
      });
    await page.locator("[data-interface-language]").click();
    await page.locator('[data-language-option="system"]').click();
    await expect(phone.locator("[data-phone-empty]")).toHaveText("No agents are running", { timeout: 20_000 });
    await expect(phone.locator("html")).toHaveAttribute("lang", "en");
  } finally {
    await context?.close();
    daemon?.stop();
    herdr.stop();
    tailscale.remove();
  }
});

test("an empty list, the unreachable line, the phone limit and the seven-day revoke", async ({ browser, page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ agents: false });
  const tailscale = new FakeTailscale();
  tailscale.install();
  tailscale.ready();
  let daemon: Daemon | null = null;
  const contexts: BrowserContext[] = [];
  try {
    daemon = await startHided(herdr, "mobile-limits", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await openMobileSettings(page, daemon);
    await page.locator('[data-mobile-switch="true"]').click();
    await choosePush(page, "app_closed");
    const phoneContextA = await phoneContext(browser);
    contexts.push(phoneContextA);
    const phone = await phoneContextA.newPage();
    await phone.goto(await shownPairingUrl(page, daemon));
    await phone.locator('[data-phone-pair="true"]').tap();

    // B22: no agents, one line.
    await expect(phone.locator('[data-phone-empty="true"]')).toHaveText("No agents are running", { timeout: 20_000 });
    await screenshot(phone, "mobile-phone-empty");
    // B38: the phone's own light or dark setting decides the theme.
    await expect(phone.locator("html")).toHaveClass(/dark/);
    await phone.emulateMedia({ colorScheme: "light" });
    await expect(phone.locator("html")).not.toHaveClass(/dark/);
    await screenshot(phone, "mobile-phone-empty-light");
    await phone.emulateMedia({ colorScheme: "dark" });

    // B23, B8: the daemon goes away; the list dims under the unreachable line,
    // and the phone comes back on its own with the same credential.
    daemon = await daemon.restart(async () => {
      await expect(phone.locator('[data-phone-unreachable="true"]')).toContainText("Not connected · hide on your Mac or Tailscale on your phone is off. Retrying", { timeout: 20_000 });
      await screenshot(phone, "mobile-phone-unreachable");
    });
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 30_000 });
    await expect(phone.locator('[data-phone-unreachable="true"]')).toHaveCount(0);
    const restarted = daemon.origin;
    await expect.poll(() => tailscale.proxy(), { timeout: 20_000 }).toBe(restarted);

    // A Mac gone to sleep: hided stops answering and the phone's socket stays open. The phone asks
    // after 20 s of silence, hears nothing for 10 s, and shows the same line (30 s at most), then
    // connects again by itself once hided runs. Windows has no SIGSTOP and no mobile daemon (#315).
    if (process.platform !== "win32") {
      process.kill(daemon.pid, "SIGSTOP");
      try {
        await expect(phone.locator('[data-phone-unreachable="true"]')).toContainText("Not connected · hide on your Mac or Tailscale on your phone is off. Retrying", { timeout: 45_000 });
        await expect(phone.locator('[data-phone-connected="false"]')).toBeVisible();
        await screenshot(phone, "mobile-phone-silent-hided");
      } finally {
        process.kill(daemon.pid, "SIGCONT");
      }
      await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 45_000 });
      await expect(phone.locator('[data-phone-unreachable="true"]')).toHaveCount(0);
    }

    // B23: opened with no network, the cached shell shows the same line instead of a blank page.
    await phone.reload();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 20_000 });
    await phoneContextA.setOffline(true);
    await phone.reload();
    await expect(phone.locator('[data-phone-unreachable="true"]')).toBeVisible({ timeout: 20_000 });
    await phoneContextA.setOffline(false);
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 30_000 });

    // B29: the push mode survived the restart.
    await openMobileSettings(page, daemon);
    await expectPush(page, "Only when the app is closed");

    // B14: three more phones on record make four; a fifth is refused with the reason.
    const record = (id: string, lastSeen: number) => ({
      id,
      name: `폰 ${id}`,
      credential_sha256: crypto.createHash("sha256").update(id).digest("hex"),
      paired_at_ms: lastSeen,
      last_seen_ms: lastSeen,
      notifications: "unasked",
      push: null,
    });
    daemon = await daemon.restart((stateDir) => {
      const file = path.join(stateDir, "phones.json");
      const current = JSON.parse(fs.readFileSync(file, "utf8")) as { phones: unknown[] };
      current.phones.push(record("b", Date.now()), record("c", Date.now()), record("d", Date.now()));
      fs.writeFileSync(file, JSON.stringify(current));
    });
    await openMobileSettings(page, daemon);
    await expect(page.getByText("Connected phones · 4 / 4")).toBeVisible({ timeout: 20_000 });
    const fifth = await (await phoneContext(browser)).newPage();
    contexts.push(fifth.context());
    await fifth.goto(await shownPairingUrl(page, daemon));
    await fifth.locator('[data-phone-pair="true"]').tap();
    await expect(fifth.locator('[data-phone-guidance="phone_limit"]')).toHaveText("You can connect up to 4 phones. Revoke one in Settings > Mobile on your Mac.", { timeout: 20_000 });
    await screenshot(page, "mobile-settings-ready");

    // B16: a phone away for eight days is revoked at start and told so when it opens.
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 30_000 });
    const phoneId = (JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "phones.json"), "utf8")) as { phones: { id: string; name: string }[] }).phones.find((entry) => entry.name === "iPhone")?.id;
    expect(phoneId).toBeTruthy();
    await phoneContextA.setOffline(true);
    daemon = await daemon.restart((stateDir) => {
      const file = path.join(stateDir, "phones.json");
      const current = JSON.parse(fs.readFileSync(file, "utf8")) as { phones: { id: string; last_seen_ms: number }[] };
      for (const entry of current.phones) if (entry.id === phoneId) entry.last_seen_ms = Date.now() - 8 * 24 * 60 * 60 * 1000;
      fs.writeFileSync(file, JSON.stringify(current));
    });
    await phoneContextA.setOffline(false);
    await expect(phone.locator('[data-phone-guidance="revoked"]')).toHaveText("This phone's connection was revoked. Reopen the QR code on your Mac.", { timeout: 30_000 });
    await openMobileSettings(page, daemon);
    await expect(page.getByText("Connected phones · 3 / 4")).toBeVisible({ timeout: 20_000 });
  } finally {
    for (const context of contexts) await context.close();
    daemon?.stop();
    herdr.stop();
    tailscale.remove();
  }
});

test("the phone's start sheet starts an agent in a checkout and in Home, and keeps its text while the phone is away", async ({ browser, page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const herdr = await startHerdr({ agents: false });
  const tailscale = new FakeTailscale();
  tailscale.install();
  tailscale.ready();
  let daemon: Daemon | null = null;
  const contexts: BrowserContext[] = [];
  try {
    daemon = await startHided(herdr, "mobile-start", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await openMobileSettings(page, daemon);
    await page.locator('[data-mobile-switch="true"]').click();
    const phoneContextA = await phoneContext(browser);
    contexts.push(phoneContextA);
    const phone = await phoneContextA.newPage();
    await phone.goto(await shownPairingUrl(page, daemon));
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 20_000 });

    // B42: + opens the sheet; the target is This Mac's Home, the kind and model the remembered choice.
    await phone.locator('[data-phone-start="true"]').tap();
    const sheet = phone.locator('[data-phone-start-sheet="true"]');
    await expect(sheet).toBeVisible();
    const target = sheet.locator('[data-phone-start-target="true"]');
    await expect(target).toHaveValue(`home:${daemon.node}`, { timeout: 20_000 });
    await expect(target.locator("option:checked")).toHaveText(`${THIS_MACHINE} · Home`);
    await expect(sheet.locator('[data-phone-start-kind="true"]')).toHaveValue("claude");
    await expect(sheet.locator('[data-phone-start-submit="true"]')).toBeDisabled();
    // B46: the Korean text and a long checkout name stay inside the phone's width.
    await sheet.locator('[data-phone-start-text="true"]').fill("첫 지시 확인용 문장입니다. 아주 긴 한국어 문장이 줄을 바꿔도 시트 밖으로 나가지 않아야 합니다.");
    for (const control of ["target", "kind", "model"]) {
      const box = await sheet.locator(`[data-phone-start-${control}="true"]`).boundingBox();
      expect(box && box.x >= 0 && box.x + box.width <= 390).toBe(true);
    }
    await screenshot(phone, "mobile-phone-start-sheet");

    // B43: a checkout target with the model the catalog lists; the phone lands on that agent's detail.
    const checkout = target.locator("option", { hasText: "fixture" }).first();
    await target.selectOption(await checkout.getAttribute("value") as string);
    const model = sheet.locator('[data-phone-start-model="true"]');
    await expect(model).toBeEnabled({ timeout: 20_000 });
    await model.selectOption("opus");
    await sheet.locator('[data-phone-start-submit="true"]').tap();
    const detail = phone.locator("[data-phone-detail]");
    await expect(detail).toBeVisible({ timeout: 45_000 });
    const pane = ((await detail.getAttribute("data-phone-detail")) ?? "").split("|").slice(1).join("|");
    expect(pane).not.toBe("");
    await expect
      .poll(
        () => {
          const info = spawnSync(herdr.bin, ["pane", "process-info", "--pane", pane], { env: herdr.env, encoding: "utf8", timeout: 10_000 }).stdout;
          return info.includes("--model") && info.includes("opus");
        },
        { timeout: 30_000 },
      )
      .toBe(true);
    await screenshot(phone, "mobile-phone-start-detail");

    // B44: the text was spent by the start; the remembered model is now the sheet's; offline keeps a new draft.
    await phone.locator('[data-phone-back="true"]').tap();
    await phone.locator('[data-phone-start="true"]').tap();
    await expect(sheet.locator('[data-phone-start-text="true"]')).toHaveValue("");
    await expect(sheet.locator('[data-phone-start-model="true"]')).toHaveValue("opus", { timeout: 20_000 });
    await sheet.locator('[data-phone-start-text="true"]').fill("Home에서 시작");
    await phoneContextA.setOffline(true);
    await expect(sheet.locator('[data-phone-start-unreachable="true"]')).toBeVisible({ timeout: 30_000 });
    await expect(sheet.locator('[data-phone-start-submit="true"]')).toBeDisabled();
    await expect(sheet.locator('[data-phone-start-text="true"]')).toHaveValue("Home에서 시작");
    await screenshot(phone, "mobile-phone-start-offline");
    await phoneContextA.setOffline(false);
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible({ timeout: 30_000 });

    // B42, B43: This Mac's Home is the default target; the start lands on its agent.
    await expect(target).toHaveValue(`home:${daemon.node}`);
    await sheet.locator('[data-phone-start-submit="true"]').tap();
    await expect(detail).toBeVisible({ timeout: 45_000 });
    await expect(detail).not.toHaveAttribute("data-phone-detail", new RegExp(`\\|${pane}$`));
    await expect(sheet).toHaveCount(0);
  } finally {
    for (const context of contexts) await context.close();
    daemon?.stop();
    herdr.stop();
    tailscale.remove();
  }
});
