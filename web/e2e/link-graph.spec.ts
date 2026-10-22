// The link record on an isolated pinned Herdr and hided (PRD link-graph D-34):
// Claude Code session files under the daemon's HOME, one that printed the
// pull request's address when GitHub made it and two that worked on its
// branch afterwards, and a fake `gh` that lists the pull request. With no
// pane open on any of them, the PR panel shows the three sessions, the maker
// with its `Created PR` chip (B6, B7, B32); a line's `View conversation` opens
// the Sessions tab at that session (B11); the Sessions row carries the PR
// chip back to the panel (B36); `Resume` starts that session in the branch's
// worktree (B12); and a session whose file is deleted stays as a dim line
// whose Resume says why (B15). Light and Dark captures land in
// HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { agentsIn, claudeProjects, startHerdr } from "./herdr-fixture";
import { OMP_ID, PI_ID, prepareNativeWriter } from "./session-reader-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { countSent, screenshot, showProjectRow, showSidebarPrs } from "./wire";
import { chord } from "./chords";
import { animationsFinished } from "./wait";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 240_000 });

function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", ...args], { cwd, encoding: "utf8" });
}

const HOUR = 3_600_000;
const CREATED = Date.now() - 2 * HOUR;
const BRANCH = "prd/link";

/** A `gh` logged in for `acme/repo` that lists one open pull request on `BRANCH`, made at `CREATED`. */
function fakeGh(root: string): string {
  const bin = path.join(root, "gh-bin");
  const pulls = [
    {
      number: 31,
      title: "연결 기록 패널을 PR 옆에 붙입니다",
      statusCheckRollup: [{ __typename: "CheckRun", name: "verify", status: "COMPLETED", conclusion: "SUCCESS" }],
      headRefName: BRANCH,
      headRefOid: "1".repeat(40),
      isCrossRepository: false,
      baseRefName: "main",
      state: "OPEN",
      reviewDecision: "REVIEW_REQUIRED",
      isDraft: false,
      url: "https://github.com/acme/repo/pull/31",
      mergedAt: null,
      closedAt: null,
      createdAt: new Date(CREATED).toISOString(),
      updatedAt: new Date(CREATED + HOUR).toISOString(),
      closingIssuesReferences: [],
    },
  ];
  fixtureProgram(
    bin,
    "gh",
    `const args = process.argv.slice(2);
const out = (value) => process.stdout.write(JSON.stringify(value) + "\\n");
const [a, b] = args;
if (a === "auth" && b === "status") process.exit(0);
if (a === "api" && b === "user") { process.stdout.write("example-user\\n"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { process.stdout.write("[]\\n"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { process.stdout.write('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}\\n'); process.exit(0); }
if (a === "repo" && b === "view") { out({ nameWithOwner: "acme/repo", id: "R_fixture" }); process.exit(0); }
if (a === "pr" && b === "list") { out(args.includes("merged") ? [] : ${JSON.stringify(pulls)}); process.exit(0); }
if (a === "issue" && b === "list") { out([]); process.exit(0); }
if (a === "api" && b === "graphql") { out({ data: {} }); process.exit(0); }
process.stderr.write("unsupported: " + args.join(" ") + "\\n");
process.exit(1);
`,
  );
  return bin;
}

const at = (ms: number) => new Date(ms).toISOString();

/** A person's turn as Claude Code writes it, on `BRANCH` in `cwd`. */
function request(id: string, cwd: string, ms: number, text: string, uuid: string): string {
  return JSON.stringify({
    type: "user", isSidechain: false, uuid, parentUuid: null, message: { role: "user", content: text }, timestamp: at(ms),
    promptId: `p-${uuid}`, origin: { kind: "human" }, userType: "external", entrypoint: "cli", cwd, sessionId: id, gitBranch: BRANCH,
  });
}

function answer(id: string, cwd: string, ms: number, text: string, uuid: string, parent: string): string {
  return JSON.stringify({ type: "assistant", isSidechain: false, uuid, parentUuid: parent, message: { role: "assistant", content: [{ type: "text", text }] }, timestamp: at(ms), cwd, sessionId: id, gitBranch: BRANCH });
}

/** The three sessions, as files under the fixture's Claude projects folder. Returns each file by session id. */
function writeSessions(projects: string, cwd: string): Record<string, string> {
  const folder = path.join(projects, "-repo-link");
  fs.mkdirSync(folder, { recursive: true });
  const file = (id: string) => path.join(folder, `${id}.jsonl`);
  const write = (id: string, lines: string[]) => fs.writeFileSync(file(id), lines.join("\n") + "\n");
  write("s-maker", [
    request("s-maker", cwd, CREATED - 30 * 60_000, "연결 기록 패널 만들어 줘", "m1"),
    answer("s-maker", cwd, CREATED - 20 * 60_000, "패널을 만들었습니다.", "m2", "m1"),
    request("s-maker", cwd, CREATED - 60_000, "검증 끝났으면 PR 올려 줘", "m3"),
    JSON.stringify({ type: "pr-link", sessionId: "s-maker", prNumber: 31, prRepository: "acme/repo", prUrl: "https://github.com/acme/repo/pull/31", timestamp: at(CREATED + 1_000) }),
  ]);
  write("s-review", [request("s-review", cwd, CREATED + 30 * 60_000, "리뷰 코멘트 두 개 반영해 줘", "r1"), answer("s-review", cwd, CREATED + 40 * 60_000, "반영했습니다.", "r2", "r1")]);
  write("s-gone", [request("s-gone", cwd, CREATED + 50 * 60_000, "e2e 스냅샷 다시 찍어 줘", "g1")]);
  return { "s-maker": file("s-maker"), "s-review": file("s-review"), "s-gone": file("s-gone") };
}

/**
 * The pull request's card, opened from the sidebar's PR list into the lane's 보기
 * slot (PRD sidebar-task-lens B27), its folded `연결` line opened so the link
 * record shows; the card keeps that line open while the app runs.
 */
async function openCard(page: Page, number: number, fresh = true): Promise<Locator> {
  if (fresh) await (await showProjectRow(page, page.locator("[data-project]", { hasText: "repo" }).locator("[data-checkout]").first())).click();
  await showSidebarPrs(page);
  // Waiting starts folded (B7); a group opens from its header.
  await page.locator('[data-pr-group] [data-pr-focus="group"]').first().waitFor({ timeout: 30_000 });
  for (const header of await page.locator('[data-pr-group][data-folded="true"] [data-pr-focus="group"]').all()) await header.click();
  await page.locator(`[data-pr-row="${number}"]`).click({ timeout: 30_000 });
  const card = page.locator(`aside[data-lane="detail"] [data-pr-card="${number}"]`);
  // The timeline is never folded (PRD pr-card-and-session-record-redesign D-03).
  await card.locator("[data-pr-card-timeline]").waitFor({ timeout: 30_000 });
  return card;
}

async function chooseTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.keyboard.press(chord("settings"));
  await page.locator('[data-settings-tab="general"]').click();
  await page.locator(`[data-theme-option="${theme}"]`).click();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
  await animationsFinished(page);
}

test("a pull request's panel shows the sessions that made it and worked on it after their panes closed", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "home", ".zshenv"), `export PATH="${path.join(herdr.root, "bin")}:$PATH"\n`);
    const repo = path.join(fs.realpathSync(herdr.root), "repo");
    const tree = path.join(fs.realpathSync(herdr.root), "repo-link");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["worktree", "add", "-b", BRANCH, tree]);
    const files = writeSessions(claudeProjects(herdr), tree);
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    const gh = fakeGh(herdr.root);
    daemon = await startHided(herdr, "link-graph", undefined, { PATH: `${gh}${path.delimiter}${herdr.fixturePath}` });
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });
    await chooseTheme(page, "light");

    // The card's timeline: the milestones the list knows first (`PR 열림`,
    // then the timeless check row right after it), then the sessions oldest
    // first by their start, each keeping the request it was given in its
    // tooltip; the maker's row carries `Created PR` (B6, B7; PRD
    // pr-card-and-session-record-redesign B4, B7). The maker session started before the PR was opened, so it stands first.
    const panel = await openCard(page, 31);
    await expect(panel).toBeVisible();
    const lines = panel.locator("[data-link-session]");
    await expect(lines).toHaveCount(3, { timeout: 60_000 });
    expect(await lines.evaluateAll((items) => items.map((item) => item.getAttribute("data-link-session")))).toEqual(["s-maker", "s-review", "s-gone"]);
    await expect(panel.locator("[data-link-sessions]")).toHaveAttribute("data-link-sessions", "3");
    const rows = panel.locator("[data-link-session], [data-timeline-milestone]");
    expect((await rows.evaluateAll((items) => items.map((item) => item.getAttribute("data-link-session") ?? item.getAttribute("data-timeline-milestone")))).slice(0, 3)).toEqual(["s-maker", "opened", "checks_passed"]);
    const maker = panel.locator('[data-link-session="s-maker"]');
    await expect(maker).toHaveAttribute("data-link-role", "created");
    await expect(maker).toHaveAttribute("data-link-request", "검증 끝났으면 PR 올려 줘");
    await expect(maker.locator('[data-link-chip="created"]')).toBeVisible();
    await expect(panel.locator(`[data-pr-panel-worktree="${tree}"]`)).not.toHaveAttribute("data-removed", "true");
    await expect(panel.locator("[data-link-loading]")).toHaveCount(0, { timeout: 60_000 });
    await page.mouse.move(2, 998);
    await screenshot(page, "link-graph-panel-light");

    // An ended row's buttons take the time's place on the pointer; View
    // conversation opens that session's record in the 보기 slot (B9, B10,
    // B11; PRD sidebar-task-lens B47), and the slot's ← comes back to the card.
    await maker.hover();
    await expect(maker.locator('[data-link-button="view"]')).toBeVisible();
    await expect(maker.locator('[data-link-button="resume"]')).toBeVisible();
    await maker.locator('[data-link-button="view"]').click();
    await expect(page.locator('aside[data-lane="detail"] [data-session-record="lane"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator("[data-overview-page]")).toHaveCount(0);
    await page.locator("[data-detail-back]").click();
    await expect(panel).toBeVisible();

    // A session whose file is gone stays as a dim line, and Resume says why (B15);
    // the card reads the record again when it opens. The close chord, pressed
    // with the keyboard in the lane (← took it), closes the slot as its ✕ does.
    fs.rmSync(files["s-gone"]);
    await page.keyboard.press(chord("close_tab"));
    await expect(panel).toHaveCount(0);
    await openCard(page, 31, false);
    const gone = panel.locator('[data-link-session="s-gone"]');
    await expect(gone).toHaveAttribute("data-link-file", "missing", { timeout: 30_000 });
    await expect(gone.locator('[data-link-missing="s-gone"]')).toBeVisible();
    await gone.hover();
    await expect(gone.locator('[data-link-button="resume"]')).toHaveAttribute("data-link-blocked", "links.why.file");
    await expect(gone.locator('[data-link-button="view"]')).toHaveAttribute("aria-disabled", "true");

    await chooseTheme(page, "dark");
    await page.mouse.move(2, 998);
    await screenshot(page, "link-graph-panel-dark");

    // Resume starts that session in the branch's worktree, once (B12).
    await maker.hover();
    await maker.locator('[data-link-button="resume"]').click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible({ timeout: 60_000 });
    expect(sent.get("agent_start_in_checkout")).toBe(1);
    expect(last.get("agent_start_in_checkout")).toMatchObject({ checkout_path: tree, provider: "claude", resume_session_id: "s-maker" });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});

// B5/B8: the selected native file reaches the actual queued resume worker.
for (const [kind, id, resumeFlag] of [["pi", PI_ID, "--session"], ["omp", OMP_ID, "--resume"]] as const) {
test(`a ${kind} archive resume keeps its selected source and uses the current control`, async ({ page }) => {
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    fs.writeFileSync(path.join(herdr.root, "home", ".zshenv"), `export PATH="${path.join(herdr.root, "bin")}:$PATH"\n`);
    const repo = path.join(fs.realpathSync(herdr.root), "repo");
    const tree = path.join(fs.realpathSync(herdr.root), "repo-link");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["worktree", "add", "-b", BRANCH, tree]);
    const session = prepareNativeWriter(herdr, kind, tree);
    const source = fs.readFileSync(path.join(herdr.root, `${kind}-session-seed.jsonl`), "utf8") + `${JSON.stringify({
      type: "message", id: "pr-tool-result", timestamp: at(CREATED + 1_000),
      message: { role: "toolResult", toolCallId: "pr-tool", toolName: "bash", isError: false,
        content: [{ type: "text", text: "https://github.com/acme/repo/pull/31" }], timestamp: CREATED + 1_000 },
    })}\n`;
    fs.writeFileSync(session, source);
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);
    const gh = fakeGh(herdr.root);
    daemon = await startHided(herdr, `${kind}-archive`, herdr.env.HOME, { PATH: `${gh}${path.delimiter}${herdr.fixturePath}` });
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    const panel = await openCard(page, 31);
    const archived = panel.locator(`[data-link-session="${id}"]`);
    await expect(archived).toBeVisible({ timeout: 60_000 });
    // The line can show before the panel has its worktree, and a line's
    // buttons show only under the pointer, so it is hovered once the panel
    // has settled and Resume has a worktree to start in.
    await expect(panel.locator(`[data-pr-panel-worktree="${tree}"]`)).not.toHaveAttribute("data-removed", "true");
    await expect(panel.locator("[data-link-loading]")).toHaveCount(0, { timeout: 60_000 });
    // The card's own parts settle first: a part that lands after the hover draws a line that moves the row under the pointer.
    await expect(panel.locator('[data-pr-card-part-state="pending"]')).toHaveCount(0, { timeout: 60_000 });
    const resume = archived.locator('[data-link-button="resume"]');
    await expect(resume).toBeEnabled();
    await archived.hover();
    await expect(archived.locator('[data-link-button="view"]')).toBeVisible();
    await archived.locator('[data-link-button="view"]').click();
    await expect(page.locator('aside[data-lane="detail"] [data-session-record="lane"]')).toBeVisible({ timeout: 30_000 });
    await screenshot(page, `${kind}-archive-record`);
    // The record took the 보기 slot; its ← brings the card back.
    await page.locator("[data-detail-back]").click();
    await expect(panel.locator(`[data-pr-panel-worktree="${tree}"]`)).not.toHaveAttribute("data-removed", "true");
    await expect(panel.locator("[data-link-loading]")).toHaveCount(0, { timeout: 60_000 });
    // The card's own parts settle first: a failed part draws a line that moves the row under the pointer.
    await expect(panel.locator('[data-pr-card-part-state="pending"]')).toHaveCount(0, { timeout: 60_000 });
    await expect(resume).toBeEnabled();
    await archived.hover();
    await resume.click();
    await expect.poll(() => agentsIn(herdr, tree), { timeout: 60_000 }).toContain(kind);
    await expect.poll(() => fs.existsSync(path.join(herdr.root, `${kind}-launches.jsonl`))).toBe(true);
    const launches = fs.readFileSync(path.join(herdr.root, `${kind}-launches.jsonl`), "utf8").trim().split("\n").map(line => JSON.parse(line) as string[]);
    expect(launches).toEqual([[resumeFlag, id]]);
    expect(fs.readFileSync(session, "utf8")).toBe(source);
    await screenshot(page, `${kind}-archive-resume`);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
}
