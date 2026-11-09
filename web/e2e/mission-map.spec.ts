// A Conductor's mission as the operator meets it, on either engine and an
// isolated hided (PRD software-factory-conductor B14, B15, B19, B21, B48):
// the real `hide mission` commands from the pane's own shell change the
// core's record, the first plan opens nothing and, with the operator looking
// at the Conductor pane, pins that pane's popover once (PRD mission-pane-chip
// B11, B12); the pane's chip opens its mission's map as a card in the lane's
// 보기 slot beside the pane (operator decision 2026-11-09, replacing PRD
// mission-screen D-09, B12, B14), whose map follows each change of the
// record, a card opens its node's panel with the events the core sends for
// the card in front, and Escape lets the pick go and leaves the lane open
// (PRD mission-map-git-criteria B25, B28, B30, B31); the sidebar's `미션` row
// and the card's `모든 미션` open the screen's table (B1, B2), whose rows open
// the screen's own detail, and Settings › Mission keeps the cap the next
// `hide mission settings` reads.

import { expect, test, type Page } from "@playwright/test";
import type { HideAnswer } from "./agent-pane";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { enterWorkspace, placedBoxes, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

type Answer = { ok: boolean; reason?: string; result?: Record<string, unknown>; value?: Record<string, unknown> };

type Hide = (args: string[]) => Promise<HideAnswer>;

/** One `hide` command run as the Conductor agent; every one here is expected to succeed. */
async function mission(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  const answer = JSON.parse(line) as Answer;
  expect(answer.ok, `${args.join(" ")}: ${line}`).toBe(true);
  return answer;
}

const PLAN = {
  scope: "pr",
  check: "true",
  nodes: [
    // Every node names its own finish line (PRD mission-map-git-criteria B1).
    // A long Korean title, so the narrow window below has something to keep inside its card (PRD mission-node-simple B30).
    { key: "cause", title: "원인 조사: 재시작 뒤 미션 지도가 마지막 노드를 다시 그리지 않는 이유를 기록에서 찾는다", kind: "research", criteria: ["원인을 한 줄로 적는다"] },
    { key: "api", title: "API 추가", kind: "change", delivery: "pr", criteria: ["엔드포인트가 응답한다"], worker: { agent: "claude", model: "opus" } },
    { key: "screen", title: "API 쓰는 화면", kind: "change", delivery: "pr", criteria: ["화면이 API 값을 보인다"], depends_on: ["api"] },
  ],
};

/**
 * The fixture's repository Project, since a mission belongs to one, with the
 * Conductor's agent pane in it (the fixture's first agent, a Claude Code
 * stand-in that runs `hide` commands as a real agent does) and the page open
 * on it. The Conductor's one worker is the fixture's second agent, its
 * delegated child. A daemon that started is stopped when the page fails to open.
 */
async function conductorRepo(page: Page, fixture: AgentFixture, label: string): Promise<{ daemon: Daemon; conductor: Hide; pane: string; worker: string }> {
  const daemon = await startHided(fixture, label);
  try {
    const [pane, worker] = fixture.panes;
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    return { daemon, conductor: fixture.runAs(pane), pane, worker };
  } catch (error) {
    daemon.stop();
    throw error;
  }
}

/** The core reads this machine's projects just after it starts; until then it answers that it is still reading. */
async function createMission(conductor: Hide) {
  await expect.poll(async () => (await conductor(["mission", "create", "--title", "이슈 정리"])).out, { timeout: 20_000 }).toContain('"ok":true');
}

test("the Conductor's first plan shows on its pane, and the mission's card beside it follows the record", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    const opened = await conductorRepo(page, fixture, "mission-map");
    daemon = opened.daemon;
    const { conductor, worker } = opened;
    const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    await createMission(conductor);
    // A mission without a plan opens nothing; its pane's chip says it is being planned.
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(map).toHaveCount(0);
    const paneChip = page.locator("[data-pane-mission-chip]");
    await expect(paneChip).toHaveAttribute("data-mission-chip-state", "planning");
    const small = page.locator("[data-pane-mission]");
    await expect(small).toHaveCount(0);

    // The first plan opens nothing in the lane; the operator looks at the
    // Conductor pane, so its popover opens pinned (mission-pane-chip B11, B12).
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify(PLAN)]);
    await expect(paneChip).toHaveAttribute("data-mission-chip-state", "running");
    await expect(paneChip).toContainText("0/3");
    await expect(small).toBeVisible();
    // No worker is at a node yet: no row, and the tally counts none done and three ahead (mission-plan-visibility B10).
    await expect(small.locator("[data-mission-mini-node]")).toHaveCount(0);
    await expect(small.locator("[data-mission-mini-tally]")).toHaveAttribute("data-mission-mini-tally", "0 3");
    await expect(map).toHaveCount(0);
    await screenshot(page, "mission-pane-first-plan");
    // A press on the chip opens the map as a card in the 보기 slot, the pane staying beside it, and closes the popover.
    await paneChip.click();
    await expect(small).toHaveCount(0);
    await expect(map).toBeVisible();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(page.locator(`[data-pane-view="${opened.pane}"]`)).toBeVisible();
    await expect(page.locator("[data-mission-screen]")).toHaveCount(0);
    await expect(map.locator("[data-detail-title]")).toHaveAttribute("data-detail-title", /이슈 정리/);
    await expect(map.locator("[data-mission-status]")).toHaveAttribute("data-mission-status", "planning");
    await expect(map.locator("[data-mission-node]")).toHaveCount(3);
    // The node that waits on another is drawn below it (down flow).
    const [api, screen] = await placedBoxes(map, '[data-mission-node="api"]', '[data-mission-node="screen"]');
    expect(screen.y).toBeGreaterThan(api.y + api.height);

    await mission(conductor, ["mission", "start"]);
    // The node's worker is a live agent row, the Conductor's delegated child, as a spawned worker's is.
    const listed = (await mission(conductor, ["agent", "list"])).value as { items: { id: string; pane: string }[] };
    const workerId = listed.items.find((agent) => agent.pane === worker)!.id;
    await mission(conductor, ["mission", "node", "api", "--status", "working", "--agent-id", workerId, "--line", "엔드포인트 작성 중"]);
    await expect(map.locator("[data-mission-status]")).toHaveAttribute("data-mission-status", "running");
    const node = map.locator('[data-mission-node="api"]');
    await expect(node).toHaveAttribute("data-node-status", "working");
    // Where a moving node is now is the summary's, beside the map, not the card's (PRD mission-tool B16, B19).
    await expect(map.locator('[data-mission-summary-line="api"]')).toHaveText("엔드포인트 작성 중");
    await mission(conductor, ["mission", "decide", "--line", "화면 PR 방식 선택"]);
    // The decision is the operator's turn, above the map.
    await expect(map.locator('[data-mission-turn] [data-mission-banner="decision"]')).toContainText("화면 PR 방식 선택");
    // A card opens its node's panel in the app too: its worker's planned model, since the pane is not one Hide launched with a choice,
    // and the core's events for it, which the core sends for the mission card in front of the slot (B31).
    await node.click();
    const panel = map.locator('[data-mission-panel="api"] [data-mission-detail="api"]');
    await expect(panel).toBeVisible();
    // Its finish lines are counted in the panel, not on the card (mission-node-simple B4): the one the plan gave and the mission's check command, none passed yet.
    await expect(panel.locator('[data-panel-part="criteria"] h4')).toHaveText(/ 0\/2$/);
    await expect(panel.locator("[data-panel-meta] [data-run]")).toHaveAttribute("title", "opus (planned)");
    await panel.locator("[data-panel-events-toggle]").click();
    await expect(panel.locator('[data-panel-event="node"]').first()).toBeVisible();
    // Escape lets the pick go and leaves the lane open (B30).
    await page.keyboard.press("Escape");
    await expect(map.locator("[data-mission-panel]")).toHaveCount(0);
    await expect(map).toBeVisible();
    await screenshot(page, "mission-map-running");

    // The card belongs to the Agent tab it opened in: another tab in front shows none, and back in that tab it is as it was (operator decision 2026-11-09).
    const home = (await page.locator("[data-tab][aria-selected=true]").getAttribute("data-tab"))!;
    await page.keyboard.press(chord("new_tab"));
    await expect(page.locator("[data-tab][aria-selected=true]")).not.toHaveAttribute("data-tab", home);
    await expect(map).toHaveCount(0);
    await screenshot(page, "mission-card-other-tab");
    await page.locator(`[data-tab="${home}"]`).click();
    await expect(map).toBeVisible();
    await expect(map.locator('[data-mission-node="api"]')).toHaveAttribute("data-node-status", "working");
    await screenshot(page, "mission-card-back");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("the mission's card closes and opens again from the sidebar, the screen keeps the table, and its gear keeps the cap the Conductor reads", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    const opened = await conductorRepo(page, fixture, "mission-screen");
    daemon = opened.daemon;
    const { conductor } = opened;
    const card = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    const map = page.locator("[data-mission-screen-detail]");
    const small = page.locator("[data-pane-mission]");
    await createMission(conductor);
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify(PLAN)]);
    await mission(conductor, ["mission", "start"]);
    await mission(conductor, ["mission", "decide", "--line", "화면 PR 방식 선택"]);
    await page.locator("[data-pane-mission-chip]").click();
    await expect(card).toBeVisible();

    // Closed by the operator, a later plan does not open it again, nor the pane's popover (B12).
    await card.locator("[data-detail-close]").click();
    await expect(card).toHaveCount(0);
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify({ ...PLAN, nodes: [...PLAN.nodes, { key: "together", title: "함께 확인", kind: "verify", criteria: ["두 변경이 함께 동작한다"], depends_on: ["screen"] }] })]);
    const shown = await mission(conductor, ["mission", "show", "--json"]);
    expect(JSON.stringify(shown.result)).toContain("together");
    await expect(card).toHaveCount(0);
    await expect(small).toHaveCount(0);
    // The sidebar's 미션 row opens the table, the mission in its group, never the last mission shown (B1, B2; D-04).
    await page.locator("[data-sidebar-missions]").click();
    await expect(page.locator('[data-mission-screen-view="table"]')).toBeVisible();
    await expect(page.locator("[data-sidebar-missions]")).toHaveAttribute("aria-current", "page");
    await expect(page.locator('[data-mission-row-group="turn"]')).toHaveCount(1);
    await expect(map).toHaveCount(0);
    // The Conductor's row carries the mission mark, and pressing it leaves the screen for the card beside the pane.
    // Its open decision also lifts the row into the tree's 내 차례, so the row stands there and in its
    // project, each chip keeping its progress with the decision's `!` (needs-you-cleanup B1, B5).
    const chip = page.locator('nav[data-sidebar] [data-mission-chip][data-mission-chip-decision="true"]').first();
    await expect(chip).toHaveAttribute("data-mission-chip-status", "running");
    await expect(chip).toContainText("0/4");
    await chip.click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(card).toBeVisible();
    await expect(card.locator("[data-mission-node]")).toHaveCount(4);
    // The card's `모든 미션` opens the screen's table; a row there opens the screen's own detail.
    await card.locator("[data-detail-all-missions]").click();
    await expect(page.locator('[data-mission-screen-view="table"]')).toBeVisible();
    await page.locator('[data-mission-row-group="turn"] [data-slot="row-overlay-button"]').first().click();
    await expect(map).toBeVisible();
    await expect(map.locator("[data-mission-node]")).toHaveCount(4);

    // The detail's gear opens Settings › Mission, whose controls write the cap the Conductor reads (PRD mission-screen B10).
    await map.locator("[data-mission-screen-settings]").click();
    const sheet = page.getByRole("dialog").filter({ has: page.locator("[data-conductor-cap]") });
    await expect(sheet).toBeVisible();
    await sheet.locator("[data-conductor-cap]").click();
    await page.getByRole("option", { name: "6", exact: true }).click();
    await expect(sheet.locator("[data-conductor-cap]")).toHaveAttribute("data-conductor-cap", "6");
    await expect.poll(async () => JSON.stringify((await mission(conductor, ["mission", "settings"])).result)).toContain('"concurrency":6');
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

test("a narrow window keeps every card's words inside it, a long title ending in an ellipsis (PRD mission-node-simple B30)", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    const opened = await conductorRepo(page, fixture, "mission-map-narrow");
    daemon = opened.daemon;
    await createMission(opened.conductor);
    await mission(opened.conductor, ["mission", "plan", "--json", JSON.stringify(PLAN)]);
    await page.locator("[data-pane-mission-chip]").click();
    const map = page.locator('aside[data-detail="mission"]');
    await expect(map).toBeVisible();
    await expect(map.locator("[data-mission-node]")).toHaveCount(3);
    const spills = await map.locator("[data-mission-node]").evaluateAll((cards) =>
      cards.flatMap((card) => {
        const box = card.getBoundingClientRect();
        const title = card.querySelector<HTMLElement>("[title]");
        const out = card.scrollWidth > card.clientWidth + 1 || (title !== null && title.getBoundingClientRect().right > box.right + 1);
        return out ? [card.getAttribute("data-mission-node")] : [];
      }),
    );
    expect(spills).toEqual([]);
    // The long title is cut, not wrapped or spilled: its whole is on hover and in the card's name.
    expect(await map.locator('[data-mission-node="cause"] [title]').evaluate((title) => title.scrollWidth > title.clientWidth)).toBe(true);
    await screenshot(page, "mission-map-narrow");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});
