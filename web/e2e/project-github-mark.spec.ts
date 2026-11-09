// A project row's GitHub mark: on an isolated engine and hided, a Git
// project whose remote names a GitHub repository (a fake `gh` answers for
// acme/repo) draws the mark at its row's end, named after the repository, and
// a ⌘ press opens the repository in the default browser without selecting the
// project; a plain folder draws no mark, and no project row draws agent counts.
// Light and Dark captures land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { openProject, withControl } from "./daemon-actions";
import { startAgentInProject } from "./fixture-agents";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { chooseTheme, countSent, rest, screenshot } from "./wire";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 180_000 });

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

/** A `gh` that is logged in and knows acme/repo with no pull requests or issues. */
function fakeGh(dir: string): string {
  const bin = path.join(dir, "gh-bin");
  fs.mkdirSync(bin, { recursive: true });
  fixtureProgram(
    bin,
    "gh",
    `const key = process.argv.slice(2, 4).join(" ");
const answers = { "pr list": "[]", "repo view": '{"nameWithOwner":"acme/repo"}', "issue list": "[]" };
if (key === "auth status") process.exit(0);
if (key === "api user") { console.log("example-user"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { console.log("[]"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HideHeadRuns")) { const repository = {}; for (let i = 0; i < process.argv.slice(2).join(" ").split("object(oid:").length - 1; i++) repository["c" + i] = { checkSuites: { totalCount: 0, nodes: [] } }; console.log(JSON.stringify({ data: { repository } })); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { console.log('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}'); process.exit(0); }
if (key in answers) { console.log(answers[key]); process.exit(0); }
console.error("unsupported: " + process.argv.slice(2).join(" "));
process.exit(1);
`,
  );
  return bin;
}

test("a GitHub project's row ends in the repository's mark, a folder's in none, and no row counts agents", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.context().route("https://github.com/**", (route) => route.fulfill({ contentType: "text/html", body: "<title>acme/repo</title>" }));
  const fixture = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(fixture.root, "repo");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    const notes = path.join(fixture.root, "notes");
    fs.mkdirSync(notes);

    daemon = await startHided(fixture, "project-github-mark", undefined, { PATH: `${fakeGh(fixture.root)}${path.delimiter}${fixture.fixturePath}` });
    // One agent in each folder, as the start panel starts one.
    await withControl(daemon, async (control) => {
      for (const [dir, task] of [[repo, "저장소 정리"], [notes, "메모 정리"]] as const) {
        await startAgentInProject(fixture, daemon!, await openProject(control, dir, path.basename(dir)), { task });
      }
    });
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]").or(page.locator("[data-workspace-screen]"))).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const folder = page.locator("[data-project]", { hasText: /^notes/ });
    const mark = project.locator("[data-project-github]");

    // The mark is named after the repository GitHub answered for, once it has; the folder never has one.
    // It waits for the pointer, as the row's + and ⋯ do, so the pointer goes onto the row first.
    await project.locator("[data-project-row]").hover({ position: { x: 12, y: 8 } });
    await expect(mark).toBeVisible({ timeout: 30_000 });
    await expect(mark).toHaveAccessibleName("Open acme/repo on GitHub");
    await expect(folder.locator("[data-checkout]")).toBeVisible();
    await expect(folder.locator("[data-project-github]")).toHaveCount(0);
    // No project or folder row counts its agents; their own rows say it.
    await expect(project.locator("[data-project-row]").locator("xpath=..").locator("[data-badge-part]")).toHaveCount(0);
    await expect(folder.locator("[data-checkout]").locator("xpath=..").locator("[data-badge-part]")).toHaveCount(0);
    await expect(project.locator("[data-project-row]")).toHaveAccessibleName("repo");

    // The mark stays while the pointer is on the row, where the row's + and ⋯ show.
    await project.locator("[data-project-row]").hover({ position: { x: 12, y: 8 } });
    await expect(project.locator("[data-row-actions]")).toBeVisible();
    await expect(mark).toHaveCSS("opacity", "1");
    await rest(page);
    await expect(mark).toBeHidden();

    // ⌘ asks for the default browser: the repository opens there and the project is neither opened nor selected.
    const before = new Map(sent);
    await project.locator("[data-project-row]").hover({ position: { x: 12, y: 8 } });
    const popup = page.context().waitForEvent("page");
    await mark.click({ modifiers: ["ControlOrMeta"] });
    const opened = await popup;
    expect(opened.url()).toBe("https://github.com/acme/repo");
    await opened.close();
    await expect(project.locator("[data-project-row]")).not.toHaveAttribute("aria-current", "true");
    for (const event of ["focus_checkout", "browser_open"]) expect(sent.get(event) ?? 0).toBe(before.get(event) ?? 0);

    for (const theme of ["light", "dark"] as const) {
      await chooseTheme(page, theme);
      await rest(page);
      await screenshot(page, `project-github-mark-${theme}`);
    }
  } finally {
    await daemon?.stop();
    await fixture.stop();
  }
});
