// The sidebar 태스크 list's tree past one level on a real isolated Herdr and
// daemon: every opened row opens its children, the indent stops at the third
// level, a child in another checkout than its parent names that branch, and
// the row of the focused pane is selected (PRD sidebar-task-lens B10, B11, B13).
import { expect, test } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { declareParent, labelAgent, setFixtureLifecycle, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { measure } from "./session-panel-geometry.mjs";
import { chooseTheme, screenshot, showTasks } from "./wire";

test.describe.configure({ timeout: 240_000 });
test.use({ actionTimeout: 15_000 });

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

async function prompt(herdr: HerdrFixture, pane: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    const read = spawnSync(herdr.bin, ["pane", "read", pane, "--source", "visible", "--format", "text"], { env: herdr.env, encoding: "utf8", timeout: 10_000 });
    if (read.status === 0 && read.stdout.includes("fixture %")) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`no prompt in pane ${pane}`);
}

/** A Herdr workspace at `cwd` running the fake `claude`, named `name`. */
async function agentAt(herdr: HerdrFixture, cwd: string, name: string): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", cwd, "--label", name, "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await prompt(herdr, pane);
  herdr.run(["agent", "start", `agent-${name}`, "--kind", "claude", "--pane", pane]);
  return pane;
}

test("the 태스크 list opens a five-level tree, stops the indent at three and names a child's other checkout", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    git(repo, ["commit", "--allow-empty", "-m", "initial"]);
    const tree = (name: string) => path.join(herdr.root, `repo-${name}`);
    git(repo, ["worktree", "add", "-b", "prd/child", tree("child")]);
    git(repo, ["worktree", "add", "-b", "fix/deep-fixture", tree("deep")]);
    // root (main) > child (prd/child) > grandchild (same checkout) > deep (fix/deep-fixture) > deeper (same checkout)
    const root = await agentAt(herdr, repo, "root");
    labelAgent(herdr, root, { task: "다섯 단계 트리 조율", progress: "자식의 결과를 기다리는 중" });
    const chain: { pane: string; task: string }[] = [];
    for (const [name, cwd, task] of [["child", tree("child"), "하위 작업 검증"], ["grandchild", tree("child"), "같은 체크아웃 손자"], ["deep", tree("deep"), "다른 체크아웃 증손"], ["deeper", tree("deep"), "넷째 단계 로그 확인"]] as const) {
      const pane = await agentAt(herdr, cwd, name);
      labelAgent(herdr, pane, { task, progress: `${task} 진행 중` });
      declareParent(herdr, pane, chain.at(-1)?.pane ?? root);
      await setFixtureLifecycle(herdr, pane, "working");
      chain.push({ pane, task });
    }
    const [child, grandchild, deep, deeper] = chain.map((entry) => entry.pane) as [string, string, string, string];
    daemon = await startHided(herdr, "session-tree");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator(`nav[data-sidebar] [data-agent-open="${root}"]`).first().click();
    const panel = await showTasks(page);
    const childRow = (pane: string) => panel.locator(`[data-task-child="${pane}"]`);
    const open = (pane: string) => panel.locator(`[data-task-chevron="${pane}"]`).click();
    // Four working descendants draw no `● N`: the root's row stands alone until it is opened.
    await expect(panel.locator(`[data-task-row="${root}"]`)).toBeVisible({ timeout: 30_000 });
    await expect(panel.locator(`[data-task-row="${root}"] [data-descendant-mark]`)).toHaveCount(0);

    // Each chevron opens one more level; nothing below a closed row is drawn.
    await open(root);
    // Labels come from the core's label worker, which reads each session once it settles.
    await expect(childRow(child)).toContainText("하위 작업 검증", { timeout: 30_000 });
    await expect(childRow(grandchild)).toHaveCount(0);
    for (const pane of [child, grandchild, deep]) await open(pane);
    await expect(childRow(deeper)).toContainText("넷째 단계 로그 확인", { timeout: 30_000 });
    expect(await panel.locator("[data-task-child]").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-depth")))).toEqual(["1", "2", "3", "4"]);

    // The fourth level keeps the third level's column: as many rail columns, and it hangs from the rail above.
    const columns = (pane: string) => childRow(pane).locator("[data-tree-rails] > span").count();
    expect(await columns(deeper)).toBe(await columns(deep));
    await expect(childRow(deeper).locator("[data-tree-pass]")).toHaveCount(1);
    await expect(childRow(deep).locator("[data-tree-elbow]")).toHaveCount(1);

    // A child shows the full branch only where the checkout differs from the parent's.
    await expect(childRow(child).locator(`[data-task-branch="${child}"]`)).toHaveText("prd/child");
    await expect(childRow(child).locator(`[data-task-branch="${child}"]`)).toHaveAttribute("title", new RegExp(`^prd/child\\n.*repo-child$`));
    await expect(childRow(grandchild).locator("[data-task-branch]")).toHaveCount(0);
    await expect(childRow(deep).locator(`[data-task-branch="${deep}"]`)).toHaveText("fix/deep-fixture");
    await expect(childRow(deeper).locator("[data-task-branch]")).toHaveCount(0);
    // Branch, label and facts stay inside each row, with no part over another.
    expect(await measure(page)).toEqual({ partProblems: [], rowsFit: [], overflow: [] });
    // Under the pointer the row's buttons take the time's place and cover nothing on the line.
    await panel.locator(`[data-task-row="${root}"]`).hover();
    await expect(panel.locator(`[data-task-more="${root}"]`)).toBeVisible();
    await expect(panel.locator(`[data-task-row="${root}"] [data-agent-elapsed]`)).toBeHidden();
    expect(await measure(page)).toEqual({ partProblems: [], rowsFit: [], overflow: [] });
    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      await screenshot(page, `task-tree-${theme}`);
    }

    // The focused pane's row is selected; folded away, its nearest drawn ancestor takes the selection.
    await childRow(deeper).locator('[data-task-focus="row"]').click();
    await expect(page.locator(`[data-pane-view="${deeper}"]`)).toHaveAttribute("data-focused", "true");
    await expect(childRow(deeper).locator('[data-task-focus="row"]')).toHaveAttribute("aria-current", "true");
    await open(grandchild);
    await expect(childRow(deeper)).toHaveCount(0);
    await expect(childRow(grandchild)).toHaveAttribute("data-selected", "true");
    await expect(panel.locator('[aria-current="true"]')).toHaveCount(1);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
