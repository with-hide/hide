// A checkout row's pull request (PRD checkout-pr-glyph-card): on an isolated
// pinned Herdr and hided, with a fake `gh` that knows one approved, passing
// pull request on the worktree's branch. The row's glyph is a button that
// opens the pull request in a browser display of the Workspace in front, and
// in the default browser with ⌘ or while no Workspace is in front; hovering
// the row opens the card with the badge, number, title and the Review,
// Checks, Branch, Agents and Commit rows, which stays while the pointer
// is on it and closes on Escape; the row's menu offers Open pull request #n;
// a checkout with no pull request keeps a plain glyph and a plain card. Light
// and Dark captures land in HIDE_E2E_SCREENSHOT_DIR.

import { expect, test } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { labelAgent, startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { fixtureProgram } from "./platform-fixture";
import { chooseTheme, countSent, rest, rowGeometry, screenshot } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";
import { quietFor } from "./wait";
import { initGithubRepository } from "./github-remote";

test.describe.configure({ timeout: 180_000 });

const BRANCH = "feature/pr-card";
const TITLE = "사이드바 가독성: 오른쪽 조작, 흔들리지 않는 행, Projects 계보 접기";

function git(cwd: string, args: string[]): void {
  execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, stdio: "ignore" });
}

/** A `gh` that is logged in and lists one open, approved pull request whose checks pass, on `BRANCH`. */
function fakeGh(dir: string, url: string): string {
  const bin = path.join(dir, "gh-bin");
  fs.mkdirSync(bin, { recursive: true });
  const pulls = JSON.stringify([
    {
      number: 180,
      title: TITLE,
      statusCheckRollup: [{ __typename: "CheckRun", status: "COMPLETED", conclusion: "SUCCESS", name: "verify" }],
      headRefName: BRANCH,
      baseRefName: "main",
      state: "OPEN",
      reviewDecision: "APPROVED",
      isDraft: false,
      url,
      mergedAt: null,
      updatedAt: "2026-09-27T00:00:00Z",
      closingIssuesReferences: [],
    },
  ]);
  fixtureProgram(
    bin,
    "gh",
    `const key = process.argv.slice(2, 4).join(" ");
const answers = { "pr list": ${JSON.stringify(pulls)}, "repo view": '{"nameWithOwner":"acme/repo"}', "issue list": "[]" };
if (key === "auth status") process.exit(0);
if (key === "api user") { console.log("example-user"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { console.log("[]"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { console.log('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}'); process.exit(0); }
if (key in answers) { console.log(answers[key]); process.exit(0); }
console.error("unsupported: " + process.argv.slice(2).join(" "));
process.exit(1);
`,
  );
  return bin;
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

/** A Herdr workspace at `cwd` with a fake `claude` agent titled `task`. */
async function workspaceAt(herdr: HerdrFixture, cwd: string, task: string): Promise<void> {
  const created = herdr.run(["workspace", "create", "--cwd", cwd, "--label", path.basename(cwd), "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]) as {
    result: { root_pane: { pane_id: string } };
  };
  const pane = created.result.root_pane.pane_id;
  await prompt(herdr, pane);
  herdr.run(["agent", "start", `agent-${path.basename(cwd)}`, "--kind", "claude", "--pane", pane]);
  labelAgent(herdr, pane, { task });
}

test("a checkout's pull request: the glyph opens it, the row's card describes it, the menu names it", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  // Where the pull request lives: a page of this test's own, so an open lands somewhere real.
  const site = http.createServer((_request, response) => response.end("<title>Pull request 180</title>"));
  await new Promise<void>((resolve) => site.listen(0, "127.0.0.1", resolve));
  const port = (site.address() as { port: number }).port;
  const url = `http://127.0.0.1:${port}/acme/repo/pull/180`;
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    const worktree = path.join(herdr.root, "repo-pr");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    git(worktree, ["commit", "--allow-empty", "-m", "card"]);
    await workspaceAt(herdr, repo, "메인 체크아웃 정리");
    await workspaceAt(herdr, worktree, "PR 카드 구현");

    daemon = await startHided(herdr, "checkout-pr", undefined, { PATH: `${fakeGh(herdr.root, url)}${path.delimiter}${herdr.fixturePath}` });
    const sent = countSent(page);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const primary = project.locator("[data-checkout-row]", { hasText: /^main/ });
    const feature = project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) });
    const glyph = feature.locator('[data-checkout-pr-glyph="180"]');
    const card = page.locator('[data-checkout-card="pull_request"]');

    // B1, B3: once GitHub has answered, the worktree's glyph is the pull
    // request's mark in its state, approved with passing checks mergeable,
    // and a button named after it; main's glyph is not.
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "pull_request", { timeout: 30_000 });
    await expect(glyph.locator("[data-pr-mark]")).toHaveAttribute("data-pr-mark", "mergeable");
    await expect(glyph).toHaveAccessibleName("Open pull request #180");
    await expect(glyph).toHaveCSS("cursor", "pointer");
    await expect(primary.locator("[data-checkout-pr-glyph]")).toHaveCount(0);
    await expect(primary.locator("[data-checkout]")).toHaveAttribute("data-checkout-kind", "primary");

    // B4: on All projects, where a first run starts, no Workspace is in front, so the glyph opens
    // the default browser, which a browser tab shows as a new page; nothing opens in hide.
    await expect(page.locator("[data-overview-page]")).toBeVisible();
    const focusBeforeExternal = sent.get("focus_checkout") ?? 0;
    const popup = page.context().waitForEvent("page");
    await glyph.click();
    const opened = await popup;
    expect(opened.url()).toBe(url);
    await opened.close();
    await expect(page.locator("[data-main-screen]")).toBeVisible();
    expect(sent.get("focus_checkout") ?? 0).toBe(focusBeforeExternal);
    expect(sent.get("browser_open") ?? 0).toBe(0);

    // B5, B6, B7: the row's card, after the tooltip's delay: the badge in the
    // review decision, the number, Open PR, the title, and every row with a
    // value; the row itself and the row after it do not move under it.
    const featureRow = feature.locator("[data-checkout]").locator("xpath=..");
    const parts = [feature.getByText(BRANCH, { exact: true }), feature.locator("[data-checkout-age]")];
    await feature.locator("[data-checkout-toggle]").click(); // Folded, line two is its purpose (the agent's title) and age (agent-hierarchy-screens B35).
    await expect(feature.locator("[data-purpose]")).toHaveText("PR 카드 구현");
    await expect(feature.locator("[data-checkout-age]")).toHaveText("now");
    await rest(page);
    const atRest = await rowGeometry(featureRow, project.locator("[data-inactive-checkouts]").or(project.locator("[data-checkout-row]").last()), parts);
    await feature.locator("[data-checkout]").hover();
    await expect(card).toBeVisible();
    await expect(card.locator("[data-checkout-card-badge]")).toHaveText("Approved");
    await expect(card.locator("[data-checkout-card-title]")).toHaveText(TITLE);
    await expect(card.locator('[data-checkout-card-row="review"]')).toHaveText("Approved");
    await expect(card.locator('[data-checkout-card-row="checks"]')).toHaveText("Passing");
    await expect(card.locator('[data-checkout-card-row="branch"]')).toHaveText(BRANCH);
    // Its one agent is quiet, which no mark counts (status-marks B19).
    await expect(card.locator('[data-checkout-card-row="agents"]')).toHaveCount(0);
    await expect(card.locator('[data-checkout-card-row="commit"]')).toHaveText("now");
    await expect(card.locator("[data-checkout-card-open]")).toHaveText("Open PR");
    expect(await rowGeometry(featureRow, project.locator("[data-inactive-checkouts]").or(project.locator("[data-checkout-row]").last()), parts)).toEqual(atRest);
    await screenshot(page, "checkout-pr-card-dark");

    // B9: the card stays while the pointer crosses onto it, and Escape closes it.
    await card.locator("[data-checkout-card-title]").hover();
    await quietFor(page, 700, "the card stays while the pointer crosses onto it");
    await expect(card).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(card).toHaveCount(0);
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();

    // B1: with the worktree's Workspace in front, the glyph opens the pull
    // request as a browser display of that Workspace, and the row neither
    // opens nor unfolds from the press.
    await feature.locator("[data-checkout]").click();
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(feature.locator("[data-checkout]")).toHaveAttribute("aria-current", "true");
    await expect(feature.locator("[data-checkout-toggle]")).toHaveAttribute("aria-expanded", "true");
    const opens = sent.get("focus_checkout") ?? 0;
    await glyph.click();
    // A browser tab cannot draw the page; its toolbar still names the address.
    const address = page.locator("[data-browser-address]");
    await expect(address).toHaveText(url.replace(/^https?:\/\//, ""));
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveAccessibleName(`Page: ${url}`);
    await expect(page.locator("[data-browser-display]")).toHaveCount(1);
    expect(sent.get("focus_checkout") ?? 0).toBe(opens);
    await expect(feature.locator("[data-checkout-toggle]")).toHaveAttribute("aria-expanded", "true");
    await screenshot(page, "checkout-pr-display");

    // B2: ⌘-click opens the default browser and adds nothing in hide.
    const popupAgain = page.context().waitForEvent("page");
    await glyph.click({ modifiers: ["ControlOrMeta"] });
    const openedAgain = await popupAgain;
    expect(openedAgain.url()).toBe(url);
    await openedAgain.close();
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(1);

    // B10: the row's menu names the pull request; choosing it opens the same
    // address, which the Workspace already shows, so no second display appears,
    // and the keyboard goes to that page (B20) rather than back to the row.
    // The menu is opened on the focused row with the pointer away from the
    // sidebar and the item chosen by keyboard, so the card that shows on
    // focus, closed by the press, has only the menu's returned focus to
    // reopen it, and must not.
    await rest(page);
    await feature.locator("[data-checkout]").focus();
    await expect(card).toBeVisible();
    const rowBox = await feature.locator("[data-checkout]").boundingBox();
    if (!rowBox) throw new Error("the checkout row has no box");
    await feature.locator("[data-checkout]").dispatchEvent("contextmenu", { clientX: rowBox.x + rowBox.width / 2, clientY: rowBox.y + rowBox.height / 2 });
    const menu = page.getByRole("menu", { name: `${BRANCH} actions` });
    await expect(menu).toBeVisible();
    await expect(card).toHaveCount(0);
    await expect(menu.locator("[data-menu-label]")).toHaveText([
      "Open",
      "New tab here",
      "New group…",
      "Open pull request #180",
      "Set purpose…",
      "Set as default checkout",
      "Copy branch name",
      "Copy path",
      "Delete worktree…",
    ]);
    // The pull request follows Open, New tab here and New group; arrow down to it.
    // The menu moves focus to the next item on a timer after each key, so
    // each press waits for the move before the next.
    const focusedItem = () => page.evaluate(() => document.activeElement?.getAttribute("data-menu-item") ?? null);
    let focused = await focusedItem();
    for (let press = 0; press < 8 && focused !== "open_pull_request"; press += 1) {
      const before = focused;
      await page.keyboard.press("ArrowDown");
      await expect.poll(focusedItem).not.toBe(before);
      focused = await focusedItem();
    }
    await expect(menu.getByRole("menuitem", { name: /^Open pull request #180/ })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(menu).toHaveCount(0);
    await expect(page.locator('[data-view-tab-bar] [role="tab"]')).toHaveCount(1);
    await expect(address).toHaveText(url.replace(/^https?:\/\//, ""));
    // A browser tab has no page to hand the keyboard to, so the page's tab takes it.
    await expect(page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]')).toBeFocused();
    await quietFor(page, 300, "the page keeps the keyboard after the menu gives it back");
    await expect(page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]')).toBeFocused();
    await expect(page.locator("[data-checkout-card]")).toHaveCount(0);
    await primary.locator("[data-checkout]").click({ button: "right" });
    const primaryMenu = page.getByRole("menu", { name: "main actions" });
    await expect(primaryMenu).toBeVisible();
    await expect(primaryMenu.getByRole("menuitem", { name: /pull request/ })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(primaryMenu).toHaveCount(0);

    // B8: a checkout without a pull request has a card with no header: its
    // branch, agents and commit. B9: the card also opens on keyboard focus.
    await rest(page);
    await page.keyboard.press("Shift");
    await primary.locator("[data-checkout]").focus();
    const plain = page.locator('[data-checkout-card="plain"]');
    await expect(plain).toBeVisible();
    await expect(plain.locator("[data-checkout-card-badge]")).toHaveCount(0);
    await expect(plain.locator('[data-checkout-card-row="branch"]')).toHaveText("main");
    await expect(plain.locator('[data-checkout-card-row="review"]')).toHaveCount(0);
    await rest(page);
    await expect(plain).toHaveCount(0);

    await chooseTheme(page, "light");
    await feature.locator("[data-checkout]").hover();
    await expect(card).toBeVisible();
    await screenshot(page, "checkout-pr-card-light");
  } finally {
    daemon?.stop();
    herdr.stop();
    site.close();
  }
});

// @platform: The card names a checkout by the path the core reads it at, which the system spells (`C:\...` on Windows, /private/var on macOS) and the page shows in the wire spelling.
test("a checkout's card names the checkout by its real path", { tag: "@platform" }, async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    fs.mkdirSync(repo);
    git(repo, ["init"]);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    const worktree = path.join(herdr.root, "repo-pr");
    git(repo, ["worktree", "add", "-b", BRANCH, worktree]);
    await workspaceAt(herdr, repo, "메인 체크아웃 정리");
    await workspaceAt(herdr, worktree, "PR 카드 구현");

    daemon = await startHided(herdr, "checkout-path");
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });

    const project = page.locator("[data-project]").filter({ has: page.locator("[data-project-row]", { hasText: /^repo/ }) });
    const plain = page.locator('[data-checkout-card="plain"]');
    for (const [row, folder] of [
      [project.locator("[data-checkout-row]", { hasText: /^main/ }), repo],
      [project.locator("[data-checkout-row]").filter({ has: page.locator(`[data-checkout][aria-label^="${BRANCH}"]`) }), worktree],
    ] as const) {
      // The card opens on keyboard focus (B9), once the row's purpose and age are on line two.
      // The age is drawn once the core's background Git read has answered (a lookup alone has a
      // ten-second deadline, `REPOSITORY_TIMEOUT`), so this waits for that signal as
      // `worktree-delete.spec.ts` does, not for the default five seconds.
      await expect(row.locator("[data-checkout-age]")).toHaveText(/.+/, { timeout: 30_000 });
      await rest(page);
      await expect(plain).toHaveCount(0);
      await page.keyboard.press("Shift");
      await row.locator("[data-checkout]").focus();
      await expect(plain).toBeVisible();
      await expect(plain.locator('[data-checkout-card-row="path"]')).toHaveText(toPage(fs.realpathSync(folder)));
    }
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
