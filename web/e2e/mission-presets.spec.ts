// A mission's general nodes by their presets, from a plan the Conductor writes in its pane to the desktop's map and a
// paired phone (PRD node-presets B1-B3, B5, B10, B12): the preset table, the start gate and the old kind names live in
// the core, and the card only reads them off the snapshot and the phone's frame, so this crosses that command/socket
// boundary as the operator meets it. The card's own drawing of each state is MissionMap.test.tsx's.
import { devices, expect, test, type Browser, type BrowserContext, type Page } from "@playwright/test";
import { startFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { FakeTailscale } from "./fake-tailscale";
import { chord } from "./chords";
import { enterWorkspace, screenshot } from "./wire";
import { withControl } from "./daemon-actions";
import type { HideAnswer } from "./agent-pane";

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; line: string };
async function run(hide: Hide, args: string[]): Promise<Answer> {
  const { out } = await hide(["mission", ...args]);
  const line = out.trim().split("\n").at(-1)!;
  return { ok: (JSON.parse(line) as { ok: boolean }).ok, line };
}
async function mission(hide: Hide, args: string[]): Promise<Answer> {
  const answer = await run(hide, args);
  expect(answer.ok, `${args.join(" ")}: ${answer.line}`).toBe(true);
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

const PLAN = {
  scope: "pr",
  nodes: [
    // The kind names from before general nodes still plan, each the general node of that preset (B8).
    { key: "cause", title: "재시도 잠금 원인 찾기", kind: "research", criteria: ["원인을 한 줄로 적는다"] },
    { key: "check", title: "합친 변경 검증", kind: "general", preset: "verify", criteria: ["일곱 항목을 돌린다"] },
    { key: "tidy", title: "사이트 문구 정리", kind: "general", criteria: ["세 곳을 고친다"] },
    // A deploy node asks before it starts, the preset's own gate since the plan names none (B3).
    { key: "ship", title: "1.2.8 배포", kind: "general", preset: "deploy", criteria: ["다운로드가 된다"], depends_on: ["cause"] },
    { key: "wait", title: "로그인 수정 기다림", kind: "external", external: { pr: 77 } },
  ],
};

// The two registered stand-ins, daemon and paired browser each need to finish
// their real startup; all waits below name observable state, not elapsed time.
test.describe.configure({ timeout: 120_000 }); // wait-ok: two registered stand-ins, daemon startup and a paired phone reading the same core

test("general nodes end in their preset's icon and name, a deploy node's start gate asks before it starts, and the phone draws the same (PRD node-presets B1-B3, B5, B10, B12)", async ({ page, browser }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  const tailscale = new FakeTailscale();
  tailscale.install();
  tailscale.ready();
  let daemon: Daemon | null = null;
  let phoneContext: BrowserContext | null = null;
  try {
    daemon = await startHided(fixture, "mission-presets", undefined, { HIDE_TAILSCALE_BIN: tailscale.bin });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const conductor = fixture.runAs(fixture.panes[0]);
    await withControl(daemon, async (control) => {
      await control.until((rest) => rest.git_worktrees_loading === false, "the project catalog read");
    });
    await mission(conductor, ["create", "--title", "1.2.8 출시"]);
    await mission(conductor, ["plan", "--json", JSON.stringify(PLAN)]);
    await page.locator("[data-pane-mission-chip]").click();
    const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    await expect(map.locator("[data-mission-node]")).toHaveCount(5);
    const foot = (key: string) => map.locator(`[data-mission-node="${key}"] [data-node-foot] [data-node-line="result"]`);
    // Planned, each general node's foot is its preset's icon and name, grey (B1, B5).
    for (const [key, icon, name] of [["cause", "research", "Research"], ["check", "verify", "Verify"], ["tidy", "general", "General"], ["ship", "deploy", "Deploy"]] as const) {
      await expect(foot(key).locator("[data-line-icon]")).toHaveAttribute("data-line-icon", icon);
      await expect(foot(key)).toHaveText(name);
      await expect(foot(key)).toHaveAttribute("data-line-tone", "planned");
    }
    // The deploy node's start gate shows ahead as the faint band, the operator's turn not yet (B3).
    const ship = map.locator('[data-mission-node="ship"]');
    await expect(ship.locator('[data-node-gate="ahead"]')).toHaveText("배포해도 될지");
    await expect(ship.locator("[data-node-turn], [data-node-gate-tag]")).toHaveCount(0);
    // The waiting node has no worker: an hourglass and its word where one would be, and its pull request as its line (B12).
    const wait = map.locator('[data-mission-node="wait"]');
    await expect(wait.locator("[data-node-run] [data-node-waiting]")).toHaveText("Watching");
    await expect(wait.locator("[data-node-foot] [data-line-pr]")).toHaveText("↗#77");
    await screenshot(page, "mission-presets-plan-desktop");

    await mission(conductor, ["start"]);
    await mission(conductor, ["node", "check", "--status", "working"]);
    await expect(foot("check")).toHaveAttribute("data-line-tone", "working");
    // A finished node shows its result line itself; a verdict shows on any preset, a failed one red with a red mark (B1, B2).
    await mission(conductor, ["record", "cause", "c1", "--pass", "--evidence", "로그 비교"]);
    await mission(conductor, ["node", "cause", "--status", "done", "--line", "재시도 때 잠금이 먼저 풀림"]);
    await expect(foot("cause")).toHaveAttribute("data-line-shows", "conclusion");
    await expect(foot("cause")).toHaveText("재시도 때 잠금이 먼저 풀림");
    await mission(conductor, ["record", "check", "c1", "--pass", "--evidence", "일곱 항목"]);
    await mission(conductor, ["node", "check", "--status", "done", "--verdict", "fail", "--checks", "5/7"]);
    await expect(foot("check")).toHaveText("Failed 5/7");
    await expect(foot("check")).toHaveAttribute("data-line-tone", "failed");
    await expect(map.locator('[data-mission-node="check"] [data-node-mark-failed]')).toBeVisible();
    // Its dependency done, the deploy node asks: the band is the operator's turn, and no worker binds before the pass (B3).
    await expect(ship.locator('[data-node-gate="open"]')).toHaveText("배포해도 될지");
    await expect(ship.locator('[data-node-gate-tag="open"]')).toHaveText("Needs you");
    const early = await run(conductor, ["node", "ship", "--status", "working"]);
    expect([early.ok, early.line.includes("mission_gate_before_start")], early.line).toEqual([false, true]);
    await screenshot(page, "mission-presets-asking-desktop");

    // The phone reads the same presets and gate from its frame, never a result line or a verdict (B10).
    const paired = await pair(browser, daemon);
    phoneContext = paired.context;
    const phone = paired.phone;
    await phone.locator('[data-phone-tab="missions"]').tap();
    await phone.locator("[data-phone-mission]").tap();
    const phoneFoot = (key: string) => phone.locator(`[data-mission-node="${key}"] [data-node-foot] [data-node-line="result"]`);
    for (const [key, icon, name] of [["cause", "research", "Research"], ["check", "verify", "Verify"], ["tidy", "general", "General"], ["ship", "deploy", "Deploy"]] as const) {
      await expect(phoneFoot(key).locator("[data-line-icon]")).toHaveAttribute("data-line-icon", icon);
      await expect(phoneFoot(key)).toHaveText(name);
    }
    await expect(phone.locator('[data-mission-node="ship"] [data-node-gate="open"]')).toHaveText("배포해도 될지");
    await expect(phone.locator('[data-mission-node="wait"] [data-node-waiting]')).toHaveText("Watching");
    await screenshot(phone, "mission-presets-asking-phone");
  } finally {
    await phoneContext?.close();
    daemon?.stop();
    fixture.stop();
    tailscale.remove();
  }
});
