// Pulling older terminal output crosses the phone, daemon and real holder.
// A component test cannot tell whether styled, soft-wrapped PTY output is
// still available at the phone's paging boundary.
import fs from "node:fs";
import { devices, expect, test } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { sendKeys, viewPane, withControl } from "./daemon-actions";
import { FakeTailscale } from "./fake-tailscale";
import { chord } from "./chords";
import { screenshot } from "./wire";
import { animationsFinished } from "./wait";

test("the phone reaches terminal history and explains an alternate screen", async ({ browser, page }) => {
  test.setTimeout(120_000); // wait-ok: pairing and two native agent starts precede four page reads.
  const fixture = await startFixture();
  const tailscale = new FakeTailscale();
  const context = await browser.newContext({ ...devices["iPhone 13"], locale: "ko-KR", colorScheme: "dark" });
  let daemon: Daemon | null = null;
  try {
    tailscale.install(); tailscale.ready();
    daemon = await startHided(fixture, "mobile-scrollback", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-sidebar-overview]")).toBeVisible();
    await page.keyboard.press(chord("settings"));
    await page.locator('[data-settings-tab="mobile"]').click();
    const mobile = page.locator("[data-mobile-tab]");
    await mobile.getByText("Advanced · Connect directly with Tailscale", { exact: true }).click();
    await mobile.getByRole("switch", { name: "Use Tailscale", exact: true }).click();
    await page.locator("[data-mobile-switch]").click();
    await expect(page.locator("[data-mobile-ready]")).toBeVisible();
    await page.locator("[data-mobile-show-code]").click();
    const qr = page.locator("[data-mobile-qr]");
    await expect(qr).toBeVisible();
    const address = (await qr.getAttribute("data-mobile-qr"))!;
    const phone = await context.newPage();
    await phone.goto(`${daemon.origin}/m/${address.slice(address.indexOf("#"))}`);
    await phone.locator("[data-phone-pair]").tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible();
    const pane = fixture.panes[0];
    await withControl(daemon, async (control) => {
      viewPane(control, pane, { cols: 80, rows: 24 });
      // Each 150-character answer occupies two desktop rows, but one phone
      // paragraph. Cursor redraws replace the status below each answer.
      for (let start = 0; start < 600; start += 50) {
        const output = Array.from({ length: 50 }, (_, i) => `\x1b[2K\x1b[32mrow ${String(start + i).padStart(4, "0")} ${"answer ".repeat(20)}\x1b[0m\r\x1b[2Kworking\x1b[1G\x1b[2K`).join("");
        sendKeys(control, pane, output);
      }
      sendKeys(control, pane, "NEWEST-READY\r");
    });
    await phone.locator(`[data-phone-agent$="|${pane}"]`).tap();
    await phone.locator('[data-phone-view="terminal"]').tap();
    const scroll = phone.locator("[data-phone-scrollback]");
    const output = phone.locator("[data-phone-scrollback] pre");
    await expect(output).toContainText("NEWEST-READY");
    for (const lines of [400, 600, 800, 1000]) {
      await scroll.evaluate((element) => { element.scrollTop = 0; });
      await expect(scroll).toHaveAttribute("data-phone-scrollback", String(lines));
    }
    await scroll.evaluate((element) => { element.scrollTop = 0; });
    // 1,000 physical rows include 499 complete two-row answers and the ready row.
    await expect(output).toContainText("row 0101");
    await expect(phone.getByText("위로 당기면 더 불러와요", { exact: true })).toHaveCount(0);
    await expect(phone.locator('[data-phone-key="page_up"]')).toBeVisible();
    await expect(phone.locator('[data-phone-key="page_down"]')).toBeVisible();
    const hint = phone.locator("[data-phone-screen-history]");
    await expect(hint).toHaveCount(0);
    await screenshot(phone, "mobile-scrollback-oldest");
    await withControl(daemon, async (control) => {
      sendKeys(control, pane, "\x1b[?1049h\x1b[H\x1b[2JALTERNATE-SCREEN\r");
      for (let row = 0; row < 100; row++) sendKeys(control, pane, `alternate ${row}\r`);
      sendKeys(control, pane, "ALTERNATE-NEWEST\r");
    });
    await expect(output).toContainText("ALTERNATE-NEWEST");
    await expect(output).not.toContainText("alternate 0\n");
    await expect(phone.getByText("위로 당기면 더 불러와요", { exact: true })).toHaveCount(0);
    await expect(hint).toHaveText("전체 화면 프로그램이라 지난 줄은 여기 없어요. 지난 내용은 대화에서 보세요.");
    await scroll.evaluate((element) => { element.scrollTop = 0; });
    await expect(hint).toBeInViewport();
    expect(await hint.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    expect(await hint.evaluate((element) => {
      const text = element.firstChild!;
      let offset = 0;
      return element.textContent!.split(" ").every((word) => {
        const range = document.createRange();
        range.setStart(text, offset); range.setEnd(text, offset + word.length);
        offset += word.length + 1;
        return range.getClientRects().length === 1;
      });
    })).toBe(true);
    const keys = phone.getByRole("group", { name: "빠른 키" });
    const bounds = await keys.boundingBox();
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
    for (const button of await keys.getByRole("button").all()) {
      const box = await button.boundingBox();
      expect(box!.width).toBeGreaterThanOrEqual(44);
      expect(box!.height).toBeGreaterThanOrEqual(44);
      expect(box!.y).toBe(bounds!.y);
    }
    const before = fs.readFileSync(fixture.inputLogs[0], "utf8").length;
    await phone.locator('[data-phone-key="page_up"]').tap();
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0], "utf8").slice(before)).toBe("\x1b[5~");
    await phone.locator('[data-phone-key="page_down"]').tap();
    await expect.poll(() => fs.readFileSync(fixture.inputLogs[0], "utf8").slice(before)).toBe("\x1b[5~\x1b[6~");
    await phone.mouse.move(0, 0);
    await screenshot(phone, "mobile-scrollback-alternate-ko-dark");
    await phone.emulateMedia({ colorScheme: "light" });
    await animationsFinished(phone);
    await screenshot(phone, "mobile-scrollback-alternate-ko-light");
    await phone.evaluate(() => localStorage.setItem("hide.phone.settings", JSON.stringify({ language: "en", theme: "system" })));
    await phone.reload();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible();
    await phone.locator(`[data-phone-agent$="|${pane}"]`).tap();
    await phone.locator('[data-phone-view="terminal"]').tap();
    await expect(output).toContainText("ALTERNATE-NEWEST");
    await scroll.evaluate((element) => { element.scrollTop = 0; });
    await expect(hint).toBeInViewport();
    expect(await phone.getByRole("group", { name: "Quick keys" }).evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await animationsFinished(phone);
    await screenshot(phone, "mobile-scrollback-alternate-en-light");
    await phone.emulateMedia({ colorScheme: "dark" });
    await animationsFinished(phone);
    await screenshot(phone, "mobile-scrollback-alternate-en-dark");
    await phone.setViewportSize({ width: 320, height: 844 });
    await expect.poll(async () => Math.min(...await phone.getByRole("group", { name: "Quick keys" }).getByRole("button").evaluateAll((buttons) => buttons.map((button) => button.getBoundingClientRect().width))), { message: "the narrower phone keeps every quick key's touch width" }).toBeGreaterThanOrEqual(44);
    await hint.tap();
    await expect(phone.locator("[data-phone-conversation]")).toBeVisible();
  } finally {
    await context.close(); daemon?.stop(); fixture.stop(); tailscale.remove();
  }
});
