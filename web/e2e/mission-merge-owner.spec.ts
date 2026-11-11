// A node a mission's plan leaves to the operator to merge (PRD
// mission-destination B18-B20, D-10, D-11, D-35), on an isolated hided, a
// real repository and a fake `gh` that answers its pull request green and
// approved: the Conductor's plan names the node under the head, the node is
// done once its pull request is ready, and the merge is the operator's turn
// on the mission's map, the Mission list and Needs You; a mission whose
// scope stops at the pull request asks nothing for the same pull request.
// The runtime tests state each rule; only this crosses the CLI, GitHub's
// read and the screen together.

import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import type { HideAnswer } from "./agent-pane";
import { checkoutAt, withControl } from "./daemon-actions";
import { startFixture, type AgentFixture } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chooseTheme, enterWorkspace, screenshot } from "./wire";
import { initGithubRepository } from "./github-remote";
import { fakeGh, git, writes } from "./work-prs-fixture";

type Hide = (args: string[]) => Promise<HideAnswer>;
type Answer = { ok: boolean; result?: Record<string, unknown> };

async function ok(hide: Hide, args: string[]): Promise<Answer> {
  const ran = await hide(args);
  const got = JSON.parse(ran.out.trim().split("\n").at(-1) ?? "null") as Answer;
  expect(got?.ok, `${args.join(" ")}: ${ran.out}`).toBe(true);
  return got;
}

// The operator reads Hide in Korean, the words the plan and the captures are checked in.
test.use({ locale: "ko-KR" });

test.describe.configure({ timeout: 180_000 }); // wait-ok: a daemon, a family of two stand-ins and GitHub's read each finish their real startup

const WHY = "DB 변경이라 직접 보고 머지";

/** A plan whose change opens pull request 31, which the fake `gh` answers clean, green and approved, beside a research node still open. */
function plan(scope: "merge" | "pr", operatorMerge: boolean) {
  return {
    scope,
    nodes: [
      { key: "schema", title: "DB 스키마 변경", kind: "change", delivery: "pr", criteria: ["마이그레이션이 돈다"], ...(operatorMerge ? { operator_merge: WHY } : {}) },
      { key: "guide", title: "운영 가이드", kind: "research", criteria: ["가이드에 적는다"] },
    ],
  };
}

/**
 * The fixture's repository with a GitHub remote the fake `gh` answers for,
 * a daemon that finds that `gh` first, and the Conductor's mission planned
 * with `plan`, started, its change at work on pull request 31 with its line
 * met; then GitHub is read for the project.
 */
async function readyPull(page: Page, label: string, planned: ReturnType<typeof plan>, fn: (stack: { conductor: Hide; daemon: Daemon; gh: { state: string }; fixture: AgentFixture }) => Promise<void>): Promise<void> {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const fixture = await startFixture({ family: true });
  let daemon: Daemon | null = null;
  try {
    // The fixture's checkout, which the first daemon makes a repository, starts with a remote naming GitHub, as the catalog reads remotes once.
    fs.mkdirSync(path.join(fixture.root, "fixture"), { recursive: true });
    initGithubRepository(path.join(fixture.root, "fixture"));
    const gh = fakeGh(fixture.root);
    daemon = await startHided(fixture, label, undefined, { PATH: `${gh.bin}${path.delimiter}${fixture.fixturePath}` });
    const live = daemon;
    const repo = fs.realpathSync(path.join(fixture.root, "fixture"));
    const origin = path.join(fixture.root, "origin.git");
    git(fixture.root, ["init", "--bare", origin]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-u", "origin", "main"]);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const conductor = fixture.runAs(fixture.panes[0]);
    await withControl(live, (control) => control.until((rest) => rest.git_worktrees_loading === false || undefined, "the project catalog read"));
    await ok(conductor, ["mission", "create", "--title", "결제 테이블 정리"]);
    await ok(conductor, ["mission", "plan", "--json", JSON.stringify(planned)]);
    await ok(conductor, ["mission", "start"]);
    if (planned.nodes[0]?.operator_merge) {
      // Before a PR is ready, the plan names its owner and asks nothing.
      const shown = (await conductor(["mission", "show"])).out;
      const planFile = path.join(repo, "merge-owner-plan.txt");
      fs.writeFileSync(planFile, shown);
      await ok(conductor, ["file", "open", planFile]);
      await expect(page.getByText("머지까지 · schema만 👤 내가 머지", { exact: false }).first()).toBeVisible();
      await chooseTheme(page, "dark");
      await screenshot(page, "merge-owner-plan-dark");
      await chooseTheme(page, "light");
      await screenshot(page, "merge-owner-plan-light");
      await page.locator("[data-pane-mission-chip]").click();
      const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
      await expect(map.locator('[data-mission-turn-item="merge"]')).toHaveCount(0);
      await expect(page.locator("nav[data-sidebar] [data-mission-ask]")).toHaveCount(0);
      await screenshot(page, "merge-owner-before-light");
      await chooseTheme(page, "dark");
      await screenshot(page, "merge-owner-before-dark");
    }
    await ok(conductor, ["mission", "node", "schema", "--status", "working", "--pr", "31", "--line", "PR #31 열림"]);
    await ok(conductor, ["mission", "record", "schema", "c1", "--pass", "--evidence", "마이그레이션이 돌았다"]);
    await withControl(live, async (control) => {
      const at = await control.until((rest) => checkoutAt(rest, repo), "the fixture checkout");
      control.send("github_request", { workspace_id: at.workspace, refresh: true });
    });
    await fn({ conductor, daemon: live, gh, fixture });
  } finally {
    daemon?.stop();
    fixture.stop();
  }
}

/** Where the Conductor's schema node stands, as `hide mission show --json` reads it. */
async function schemaNode(conductor: Hide): Promise<{ status?: string; merge?: { by?: string; why?: string; state?: string; prs?: number[] } }> {
  const shown = (await ok(conductor, ["mission", "show", "--json"])).result as { mission: { nodes: { key: string }[] } };
  return shown.mission.nodes.find((node) => node.key === "schema") as { status?: string; merge?: { by?: string; why?: string; state?: string; prs?: number[] } };
}

/** The `hide mission show` text the Conductor reads, kept beside the captures. */
function keepText(name: string, text: string): void {
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (dir) fs.writeFileSync(path.join(dir, `${name}.txt`), text);
}

test("a node the plan leaves to the operator is done once its pull request is ready, and its merge is their turn", async ({ page }) => {
  await readyPull(page, "mission-merge-owner", plan("merge", true), async ({ conductor, gh }) => {
    // The Conductor's plan names the node under the head and on its own line.
    const shown = (await conductor(["mission", "show"])).out;
    expect(shown).toContain("머지까지 · schema만 👤 내가 머지");
    keepText("merge-owner-plan-text", shown);

    // GitHub answers pull request 31 ready: the node is done without its merge, and the merge is the operator's now.
    await expect.poll(async () => (await schemaNode(conductor)).merge?.state, { timeout: 30_000 }).toBe("due"); // wait-ok: the fresh GitHub read finishes the node and publishes its operator merge turn
    const node = await schemaNode(conductor);
    expect(node.status).toBe("done");
    expect(node.merge).toEqual({ by: "operator", why: WHY, state: "due", prs: [31] });
    const detail = (await conductor(["mission", "show", "schema"])).out;
    expect(detail).toContain("👤 머지할 차례");
    keepText("merge-owner-node-text", detail);

    // F1: the last held PR is already merged while #31 stays open and ready.
    await ok(conductor, ["mission", "node", "schema", "--pr", "37"]);
    await expect.poll(async () => (await schemaNode(conductor)).merge?.prs, { timeout: 30_000 }).toEqual([31]); // wait-ok: the added held PR receives GitHub's merged state before the turn is read

    // The mission's map: its turn names the merge as the operator's.
    await page.locator("[data-pane-mission-chip]").click();
    const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    const turn = map.locator('[data-mission-turn] [data-mission-turn-item="merge"]');
    await expect(turn).toContainText("머지할 차례");
    await expect(turn).toContainText("#31");
    await expect(turn).not.toContainText("#37");
    // Needs You lists the mission for the merge.
    const ask = page.locator('nav[data-sidebar] [data-mission-ask][data-mission-ask-node="schema"]').first();
    await expect(ask).toContainText("머지할 차례 PR #31");
    await expect(ask).not.toContainText("#37");
    await chooseTheme(page, "dark");
    await screenshot(page, "merge-owner-turn-dark");
    await chooseTheme(page, "light");
    await screenshot(page, "merge-owner-turn-light");

    // The Mission list stands it in 내 차례 with 머지하기.
    await page.locator("[data-sidebar-missions]").click();
    await expect(page.locator('[data-mission-screen-view="table"]')).toBeVisible();
    const row = page.locator('[data-mission-row-group="turn"]');
    await expect(row).toHaveCount(1);
    await expect(row.locator('[data-mission-row-pill="landing"]')).toHaveText("머지하기");
    await expect(row.locator('[data-mission-row-line="merge"]')).toContainText("#31");
    await expect(row.locator('[data-mission-row-line="merge"]')).not.toContainText("#37");
    await screenshot(page, "merge-owner-list-light");
    await chooseTheme(page, "dark");
    await screenshot(page, "merge-owner-list-dark");

    // The same PR identity survives the close and is the one named in the report.
    await ok(conductor, ["mission", "node", "guide", "--status", "dropped"]);
    await expect.poll(async () => {
      const shown = (await ok(conductor, ["mission", "show", "--json"])).result as { mission: { report?: string } };
      return shown.mission.report?.split("Left for the operator to merge (1):\n")[1];
    }, { timeout: 30_000 }).toBe(`- DB 스키마 변경 (schema) · PR #31 · ${WHY}`); // wait-ok: the mission closes merge_wait after its last unfinished node is dropped

    // Nothing in Hide merged it: the operator's merge is theirs to make.
    expect(writes(gh.state).filter((args) => args[0] === "pr" && args[1] === "merge")).toEqual([]);
  });
});

test("a mission whose scope stops at the pull request asks nothing for a ready one", async ({ page }) => {
  await readyPull(page, "mission-merge-owner-pr", plan("pr", false), async ({ conductor }) => {
    // The same ready pull request finishes the node, and no merge is the operator's turn.
    await expect.poll(async () => (await schemaNode(conductor)).status, { timeout: 30_000 }).toBe("done"); // wait-ok: the fresh GitHub read finishes the PR-scope node without adding a turn
    const node = await schemaNode(conductor);
    expect(node.merge).toEqual({ by: "operator", state: "ahead", prs: [31] });
    expect((await conductor(["mission", "show"])).out).not.toContain("👤");

    await page.locator("[data-pane-mission-chip]").click();
    const map = page.locator('aside[data-lane="detail"][data-detail="mission"]');
    await expect(map.locator('[data-mission-node="schema"]')).toHaveAttribute("data-node-status", "done");
    await expect(map.locator('[data-mission-turn-item="merge"]')).toHaveCount(0);
    await expect(page.locator("nav[data-sidebar] [data-mission-ask]")).toHaveCount(0);
    await chooseTheme(page, "dark");
    await screenshot(page, "scope-pr-quiet-dark");
    await chooseTheme(page, "light");
    await screenshot(page, "scope-pr-quiet-light");

    await page.locator("[data-sidebar-missions]").click();
    await expect(page.locator('[data-mission-screen-view="table"]')).toBeVisible();
    await expect(page.locator('[data-mission-row-group="turn"]')).toHaveCount(0);
    await screenshot(page, "scope-pr-quiet-list-light");
  });
});
