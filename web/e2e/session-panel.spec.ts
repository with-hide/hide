// The sidebar's 태스크 list on a real isolated Herdr and daemon (PRD sidebar-task-lens).
import { expect, test, type Page } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { declareParent, labelAgent, setFixtureLifecycle, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { chord } from "./chords";
import { chooseTheme, countSent, openTaskFolds, screenshot, showTasks } from "./wire";
import { quietFor, unchangedForFrames } from "./wait";

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

/** A Herdr workspace at `cwd` running the fake `claude`. */
async function agentAt(herdr: HerdrFixture, cwd: string): Promise<string> {
  const created = herdr.run(["workspace", "create", "--cwd", cwd, "--label", path.basename(cwd), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await prompt(herdr, pane);
  herdr.run(["agent", "start", `agent-${path.basename(cwd)}`, "--kind", "claude", "--pane", pane]);
  return pane;
}

const LONG_TITLE = "세션 패널에서 긴 한국어 제목과 브랜치를 한 줄로 안전하게 표시하는 작업";
const QUESTION = "긴 한국어 요청을 읽은 뒤 질문 표시가 흐려지는지 확인해 주세요";

const sentTotal = (sent: Map<string, number>) => [...sent.values()].reduce((sum, count) => sum + count, 0);
const resolved = (daemon: Daemon, pane: string) => () => Boolean(JSON.parse(fs.readFileSync(path.join(daemon.stateDir, "core-state.json"), "utf8")).resolved_sessions?.[pane]);

/** The 프로젝트 tree, where a resolved agent with a live pane stays under its tab, dimmed. */
async function showProjects(page: Page) {
  await page.locator('[data-sidebar-lens-option="projects"]').click();
  await expect(page.locator('[data-sidebar-lens="projects"]')).toBeVisible();
}

test("the 태스크 list groups the device's tasks, folds children and resting ones, reads questions, and resolves durably", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    git(repo, ["commit", "--allow-empty", "-m", "initial"]);
    const tree = (name: string) => path.join(herdr.root, `repo-${name}`);
    for (const name of ["child", "asking", "question", "stopped"]) {
      git(repo, ["worktree", "add", "-b", `prd/${name}`, tree(name)]);
    }
    const parent = await agentAt(herdr, repo);
    labelAgent(herdr, parent, { task: LONG_TITLE, progress: "자식의 결과를 기다리는 중" });
    const child = await agentAt(herdr, tree("child"));
    labelAgent(herdr, child, { task: "하위 작업 검증" });
    declareParent(herdr, child, parent);
    await setFixtureLifecycle(herdr, child, "working");
    const asking = await agentAt(herdr, tree("asking"));
    labelAgent(herdr, asking, { task: "권한 승인 요청", progress: "검증 명령을 승인해 주세요" });
    await setFixtureLifecycle(herdr, asking, "blocked");
    const question = await agentAt(herdr, tree("question"));
    labelAgent(herdr, question, { task: "AI 질문 읽음 확인", reply: QUESTION, question: true });
    const stopped = await agentAt(herdr, tree("stopped"));
    labelAgent(herdr, stopped, { task: "설정 화면 스위치 추가", progress: "테스트 환경이 없어 멈췄어요", end: "unfinished" });
    daemon = await startHided(herdr, "session-panel");
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await page.locator(`nav[data-sidebar] [data-agent-open="${parent}"]`).first().click();
    // The rail has no Work tool; after the viewing column's documents button, the Project tool comes first, then Memory (B23; lane-two-columns B2, B25).
    await expect.poll(() => page.locator("[data-lane-tool]").evaluateAll((tools) => tools.map((tool) => tool.getAttribute("aria-label")))).toEqual(["Documents", "Project", "Memory", "Explorer", "Changes"]);
    const tasks = await showTasks(page);
    const row = (pane: string) => tasks.locator(`[data-task-row="${pane}"]`);
    const group = (name: string) => tasks.locator(`[data-task-group="${name}"]`);
    // B4, B7, B8: Needs You's second line is the question, the waiting parent stands for its child, unfinished work rests with its line.
    const ask = (pane: string) => group("needs_you").locator(`[data-task-row="${pane}"] [data-task-line="needs_you"]`);
    await expect(ask(asking)).toContainText("검증 명령을 승인해 주세요", { timeout: 30_000 });
    await expect(ask(question)).toContainText(QUESTION, { timeout: 30_000 });
    // A parent waiting on its working child stands in 대기 (status-marks D-22, B32).
    await expect(group("waiting").locator(`[data-task-row="${parent}"]`)).toBeVisible();
    await expect(row(child)).toHaveCount(0);
    // A folded parent counts no children (status-marks B35).
    await expect(row(parent).locator("[data-task-children]")).toHaveCount(0);
    await openTaskFolds(page);
    await expect(row(stopped)).toHaveAttribute("data-task-state", "resting");
    await expect(row(stopped)).toContainText("테스트 환경이 없어 멈췄어요");
    // B10: the chevron opens the children in place and closes them again.
    await tasks.locator(`[data-task-chevron="${parent}"]`).click();
    await expect(tasks.locator(`[data-task-child="${child}"]`)).toContainText("하위 작업 검증");
    await tasks.locator(`[data-task-chevron="${parent}"]`).click();
    await expect(tasks.locator(`[data-task-child="${child}"]`)).toHaveCount(0);
    // B12: Enter goes to the pane; a read question leaves Needs You.
    await row(question).locator('[data-task-focus="row"]').focus();
    await page.keyboard.press("Enter");
    await expect(page.locator(`[data-pane-view="${question}"]`)).toHaveAttribute("data-focused", "true");
    await expect(group("needs_you").locator(`[data-task-row="${question}"]`)).toHaveCount(0);
    await expect(row(question).locator('[data-task-line="needs_you"]')).toHaveCount(0);
    await row(asking).locator('[data-task-focus="row"]').click();
    await expect(page.locator(`[data-pane-view="${asking}"]`)).toHaveAttribute("data-focused", "true");
    await expect(row(asking)).toHaveAttribute("data-selected", "true");
    await expect(ask(asking)).toContainText("검증 명령을 승인해 주세요");
    // B12: Home reaches the first group head and the arrows the next row.
    await row(stopped).locator('[data-task-focus="row"]').focus();
    await page.keyboard.press("Home");
    await expect(tasks.locator('[data-task-list] [data-task-focus]').first()).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(row(asking).locator('[data-task-focus="row"]')).toBeFocused();
    // B18: Resolve from the ⋯ menu is saved, and the row stays under 쉬는 중.
    await row(stopped).hover();
    await tasks.locator(`[data-task-more="${stopped}"]`).click();
    await page.locator(`[data-task-menu="${stopped}"] [data-menu-item="resolve"]`).click();
    await expect.poll(resolved(daemon, stopped)).toBe(true);
    await expect(row(stopped)).toHaveAttribute("data-task-state", "resting");
    // B3: a restart keeps the 태스크 list, and the resolution keeps the agent in the 프로젝트 tree, dimmed (sidebar-every-tab-line B3).
    daemon = await daemon.restart();
    await page.goto("about:blank");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator('[data-sidebar-lens="tasks"]')).toBeVisible();
    await showProjects(page);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${parent}"]`).first()).toBeVisible();
    // Its worktree has no unresolved agent, so it stands in the `Worktrees with no agents` fold, where the resolved row is dimmed under its tab.
    await page.locator("nav[data-sidebar] [data-empty-worktrees]").click();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${stopped}"]`).first()).toHaveClass(/opacity-\(--opacity-dimmed\)/);
    await showTasks(page);
    await openTaskFolds(page);
    await row(stopped).locator('[data-task-focus="row"]').click();
    await expect(page.locator(`[data-pane-view="${stopped}"]`)).toHaveAttribute("data-focused", "true");
    // Input brings a resolved task back.
    await page.locator(`[data-pane-view="${stopped}"] .xterm-helper-textarea`).pressSequentially("continue");
    await expect.poll(resolved(daemon, stopped)).toBe(false);
    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      await screenshot(page, `task-list-${theme}`);
    }
    await showProjects(page);
    await expect(page.locator(`nav[data-sidebar] [data-pane="${stopped}"]`).first()).toBeVisible();
    await expect(page.locator(`nav[data-sidebar] [data-pane="${stopped}"]`).first()).not.toHaveClass(/opacity-\(--opacity-dimmed\)/);
    // The approved Overview exposes work, projects, mixed backlog, retained history and account usage.
    await page.keyboard.press(chord("overview"));
    const modal = page.getByRole("dialog", { name: "Overview", exact: true });
    expect(await modal.locator("[data-main-tab]").evaluateAll((tabs) => tabs.map((tab) => tab.getAttribute("data-main-tab")))).toEqual(["work", "projects", "backlog", "history", "usage"]);
    await page.keyboard.press("Escape");
    await showTasks(page);
    await page.mouse.move(2, 998);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await unchangedForFrames(page, () => sentTotal(sent));
    const quiet = sentTotal(sent);
    await quietFor(page, 3_000, "an idle 태스크 list sends no events");
    expect(sentTotal(sent)).toBe(quiet);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
