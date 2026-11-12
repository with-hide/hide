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

import fs from "node:fs";
import path from "node:path";
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
    await mission(conductor, ["mission", "decide", "--line", "화면 PR 방식 선택", "--option", "API PR 위에 쌓기", "--option", "API PR 머지 뒤 main에서 시작하기", "--recommend", "1"]);
    // The decision is the operator's turn, above the map.
    const decision = map.locator('[data-mission-turn] [data-mission-banner="decision"]');
    await expect(decision).toContainText("화면 PR 방식 선택");
    // Its choices are radio rows: the recommended one is tagged, a press only marks a row, and 답하기 sends (PRD mission-human-gate B19).
    await expect(decision.locator("[data-mission-option]")).toHaveCount(2);
    await expect(decision.locator('[data-mission-option="1"]')).toHaveAttribute("data-mission-recommended", "true");
    await expect(decision.locator("[data-mission-answer-send]")).toBeDisabled();
    await decision.locator('[data-mission-option="2"]').click();
    await expect(decision.locator('[data-mission-option="2"]')).toHaveAttribute("aria-checked", "true");
    await expect(decision.locator("[data-mission-answer-send]")).toBeEnabled();
    await expect(decision).toHaveAttribute("data-mission-answer", "open");
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
    const chip = page.locator('nav[data-sidebar] [data-mission-chip][data-mission-chip-turn="decision"]').first();
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

test("a narrow window keeps every card's words inside it, a long title ending in an ellipsis, and each planned card's five rows: its plan to the branch the mission goes to, the operator's merge before it, or its kind (PRD mission-node-simple B30; PRD mission-destination B48, B53, B54, B55, B57, B58)", async ({ page }) => {
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
    // Nothing of a card, its title, its worker or its bottom line, reaches past its right edge.
    const spills = await map.locator("[data-mission-node]").evaluateAll((cards) =>
      cards.flatMap((card) => {
        const right = card.getBoundingClientRect().right + 1;
        const parts = [...card.querySelectorAll<HTMLElement>("[data-node-title], [data-node-run], [data-node-line], [data-node-line] > *")];
        const out = card.scrollWidth > card.clientWidth + 1 || parts.some((part) => part.getBoundingClientRect().right > right);
        return out ? [card.getAttribute("data-mission-node")] : [];
      }),
    );
    expect(spills).toEqual([]);
    // The long title is cut, not wrapped or spilled: its whole is on hover and in the card's name.
    const cause = map.locator('[data-mission-node="cause"]');
    expect(await cause.locator("[data-node-title]").evaluate((title) => title.scrollWidth > title.clientWidth)).toBe(true);
    expect(await cause.getAttribute("aria-label")).toContain(PLAN.nodes[0]!.title);
    // Research changes no code: it ends in its kind until it is done. Each change says its plan, a pull request to the branch its checkout was on.
    await expect(cause.locator("[data-node-foot] [data-node-line]")).toHaveAttribute("data-node-line", "result");
    await expect(cause.locator("[data-node-line]")).toHaveAttribute("data-line-shows", "kind");
    for (const key of ["api", "screen"]) {
      const card = map.locator(`[data-mission-node="${key}"]`);
      const line = card.locator("[data-node-foot] [data-node-line]");
      // The scope `pr` mission leaves the merge to the operator: a faint person before the destination, never the operator's turn yet.
      await expect(line).toHaveAttribute("data-line-mark", "plan_pr");
      await expect(line).toHaveAttribute("data-line-person", "ahead");
      await expect(line.locator("[data-line-to]")).toHaveText("→main");
      await expect(card.locator("[data-node-merge-turn], [data-node-turn]")).toHaveCount(0);
      await expect(card).toHaveAttribute("aria-label", /(Where it goes|어디로 들어가나): PR → main · (You merge it|머지는 내가)/);
      // A planned card: dashed, its worker marked planned on its own row, and no time.
      await expect(card).toHaveAttribute("data-node-look", "waiting");
      await expect(card.locator("[data-node-took]")).toHaveCount(0);
    }
    await expect(map.locator('[data-mission-node="api"] [data-node-run] [data-node-model]')).toHaveText("opus");
    // The head names the branch and, merge first, what the mission applies; the scope `pr` mission leaves the merges to the operator.
    await expect(map.locator("[data-mission-destination]")).toHaveAttribute("data-mission-destination", "main");
    await expect(map.locator('[data-mission-options] [data-mission-head-option="merge"]')).toHaveText(/PR까지 · 머지는 전부 내가|Up to PRs · you merge them all/);
    await screenshot(page, "mission-map-narrow");
  } finally {
    daemon?.stop();
    fixture.stop();
  }
});

// One restart gives the real CLI an older shelved id and an open caller's
// node. Both positional forms must survive parser -> owner thread -> core.
test("positional show reads a shelved mission and keeps node-key precedence", async ({ page }) => {
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    const opened = await conductorRepo(page, fixture, "mission-show-shelf");
    daemon = opened.daemon;
    const { conductor } = opened;
    await createMission(conductor);
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify(PLAN)]);
    daemon = await daemon.restart((state) => {
      // Extend a file actually written by this candidate, retaining identities
      // and schema fields instead of hand-writing a partial mission shape.
      const saved = JSON.parse(fs.readFileSync(path.join(state, "missions", "mission-1.json"), "utf8"));
      const missions = Array.from({ length: 130 }, (_, at) => {
        const record = structuredClone(saved.missions[0]);
        record.id = `mission-${at + 1}`;
        record.title = at === 0 ? "Shelved original" : `Work ${at + 1}`;
        if (at < 129) {
          record.status = "done";
          record.closed_at_unix_ms = at + 1;
          for (const node of record.nodes) node.status = "done";
        }
        return record;
      });
      fs.writeFileSync(path.join(state, "missions.json"), JSON.stringify({ schema_version: 4, next: 130, missions }));
      fs.rmSync(path.join(state, "missions"), { recursive: true });
    });
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const shown = await mission(conductor, ["mission", "show", "mission-1", "--json"]);
    expect(shown.result?.mission).toMatchObject({ id: "mission-1", title: "Shelved original" });
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify({ nodes: [
      { key: "mission-1", title: "A node named like a mission", kind: "research", criteria: ["Read this node"] },
    ] })]);
    const node = await mission(conductor, ["mission", "show", "mission-1", "--json"]);
    expect(node.result).toMatchObject({ mission_id: "mission-130", node: { key: "mission-1" } });
  } finally {
    try { daemon?.stop(); } finally { fixture.stop(); }
  }
});
