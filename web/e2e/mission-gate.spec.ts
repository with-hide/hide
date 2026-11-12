// The operator's review gate from plan to phone answer (mission-human-gate
// B1, B2, B5, B6, B9, B13, B23): real pane commands, the core's gate arrival
// and its phone projection must agree. Component tests cannot observe that
// command/socket boundary or an answer reaching the same core from the phone.
import { devices, expect, test, type Browser, type BrowserContext, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { FakeTailscale } from "./fake-tailscale";
import { chord } from "./chords";
import { enterWorkspace, screenshot } from "./wire";
import { withControl } from "./daemon-actions";
import type { HideAnswer } from "./agent-pane";

type Hide = (args: string[]) => Promise<HideAnswer>;
async function mission(hide: Hide, args: string[]) {
  const { out } = await hide(["mission", ...args]);
  const answer = JSON.parse(out.trim().split("\n").at(-1)!);
  expect(answer.ok, out).toBe(true);
  return answer;
}

/** The direct connection's existing fake-Tailscale boundary, never the operator's service. */
async function pair(browser: Browser, daemon: Daemon): Promise<{ context: BrowserContext; phone: Page }> {
  const settings = await browser.newPage();
  const context = await browser.newContext({ ...devices["iPhone 13"], locale: "en-US", colorScheme: "dark" });
  try {
    await settings.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(settings.locator("[data-sidebar-overview]")).toBeVisible();
    await settings.keyboard.press(chord("settings"));
    await settings.locator('[data-settings-tab="mobile"]').click();
    const tab = settings.locator('[data-mobile-tab="true"]');
    await tab.getByText("Advanced · Connect directly with Tailscale", { exact: true }).click();
    await tab.getByRole("switch", { name: "Use Tailscale", exact: true }).click();
    await settings.locator('[data-mobile-switch="true"]').click();
    await settings.locator('[data-mobile-show-code="true"]').click();
    const qr = settings.locator("[data-mobile-qr]");
    await expect(qr).toBeVisible();
    const address = (await qr.getAttribute("data-mobile-qr"))!;
    const phone = await context.newPage();
    await phone.goto(`${daemon.origin}/m/${new URL(address).hash}`);
    await phone.locator('[data-phone-pair="true"]').tap();
    await expect(phone.locator('[data-phone-connected="true"]')).toBeVisible();
    return { context, phone };
  } catch (error) {
    await context.close();
    throw error;
  } finally { await settings.close(); }
}

// The two registered stand-ins, daemon and paired browser each need to finish
// their real startup; all waits below name observable state, not elapsed time.
test.describe.configure({ timeout: 120_000 }); // wait-ok: two registered stand-ins, daemon startup and a paired phone answer reaching the same core

test("a planned review stays quiet until arrival and the phone's answer passes the desktop gate", async ({ page, browser }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  const tailscale = new FakeTailscale();
  tailscale.install();
  tailscale.ready();
  let daemon: Daemon | null = null;
  let phoneContext: BrowserContext | null = null;
  try {
    daemon = await startHided(fixture, "mission-gate", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const conductor = fixture.runAs(fixture.panes[0]);
    // Observe the catalog's readiness before issuing the single create.
    await withControl(daemon, async (control) => {
      await control.until((rest) => rest.git_worktrees_loading === false, "the project catalog read");
    });
    await mission(conductor, ["create", "--title", "화면 검토"]);
    await mission(conductor, ["plan", "--json", JSON.stringify({ scope: "pr", nodes: [
      { key: "screen", title: "화면 확인", kind: "research", criteria: ["화면이 준비됨"], gate: "계획과 결과 화면" },
    ] })]);
    await page.locator("[data-pane-mission-chip]").click();
    const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    await expect(map.locator("[data-mission-gates]")).toHaveAttribute("data-mission-gates", "1");
    await expect(map.locator("[data-mission-gate-when]")).toHaveText("once each step passes its other finish lines");
    await expect(map.locator('[data-node-gate="ahead"]')).toHaveText("계획과 결과 화면");
    await expect(map.locator("[data-node-gate-tag]")).toHaveCount(0);
    await screenshot(page, "mission-gate-plan-desktop");
    const paired = await pair(browser, daemon);
    phoneContext = paired.context;
    const phone = paired.phone;
    await phone.locator('[data-phone-tab="missions"]').tap();
    await expect(phone.locator("[data-mission-row-gates]")).toHaveAttribute("data-mission-row-gates", "1");
    await phone.locator("[data-phone-mission]").tap();
    await expect(phone.locator("[data-mission-gates]")).toHaveAttribute("data-mission-gates", "1");
    await screenshot(phone, "mission-gate-plan-phone");
    // The Conductor starts only after the operator's conversational approval.
    await mission(conductor, ["start"]);
    await mission(conductor, ["node", "screen", "--status", "working"]);
    await expect(map.locator("[data-node-gate]")).toHaveAttribute("data-node-gate", "ahead");
    await mission(conductor, ["record", "screen", "c1", "--pass", "--evidence", "screen ready"]);
    await expect(map.locator('[data-node-gate="open"]')).toHaveText("계획과 결과 화면");
    await expect(map.locator('[data-node-gate-tag="open"]')).toHaveText("Needs you");
    const decision = map.locator('[data-mission-banner="decision"]');
    await expect(decision).toContainText("Check 계획과 결과 화면 and tell me");
    await map.locator('[data-mission-node="screen"]').click();
    await expect(map.locator("[data-panel-gate]")).toHaveCount(0);
    await expect(phone.locator('[data-node-gate="open"]')).toHaveText("계획과 결과 화면");
    await screenshot(page, "mission-gate-open-desktop");
    await screenshot(phone, "mission-gate-open-phone");
    await phone.locator('[data-phone-option="1"]').tap();
    await phone.locator("[data-phone-choice-send]").tap();
    await expect(map.locator('[data-node-gate="passed"]')).toHaveText("계획과 결과 화면");
    await expect(map.locator('[data-panel-gate-via="phone"]')).toContainText("Go on");
    // The graph keeps the completed step visible, including when the last
    // research line closes the mission (UI_BEHAVIOR.md, The phone app).
    await expect(phone.locator('[data-mission-node="screen"][data-node-look="done"]')).toBeVisible();
    await expect(phone.locator('[data-node-gate="passed"]')).toBeVisible();
    await screenshot(page, "mission-gate-passed-desktop");
    await screenshot(phone, "mission-gate-passed-phone");
  } finally {
    await phoneContext?.close();
    daemon?.stop();
    fixture.stop();
    tailscale.remove();
  }
});
