// The PRs tool and the pull request decision card on an isolated pinned
// Herdr and hided (PRD work-tab-pr-lens; PRD sidebar-task-lens B26-B37): a
// Git project whose pull requests a fake `gh` answers and records every write
// of, so nothing reaches GitHub. The list groups by whose move it is with a
// reason line per row (B6, B8, B24); a row opens the card in the lane's 보기
// slot, never as a Files tab (B27, B50); Merge asks once and goes out
// conditioned on the head commit the card showed (B37); a pull request that
// moved after the card was drawn is refused and the card says so (B37);
// Update branch reaches a behind pull request. Light and Dark captures land in
// HIDE_E2E_SCREENSHOT_DIR.

import { expect, test, type Locator, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { initGithubRepository } from "./github-remote";
import { startHerdr } from "./herdr-fixture";
import { startHided } from "./hided-fixture";
import { fakeGh, git, head, moveHead, skewCardHead, writes, type ExtraQuery } from "./work-prs-fixture";
import { chord } from "./chords";
import { chooseTheme, countSent, registerFolder, screenshot, showProjectRow, showTool, setLanePinned } from "./wire";

test.describe.configure({ timeout: 180_000 });

/** A project on a private Herdr and hided with the fake `gh`, the PRs tool open. */
async function openWork(page: Page, fn: (stack: { gh: { bin: string; state: string }; work: Locator; sent: Map<string, number>; last: Map<string, Record<string, unknown>>; daemon: { origin: string; token: string; home: string } }) => Promise<void>, options: { failing?: ExtraQuery[]; teamAsk?: boolean } = {}): Promise<void> {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    const repo = path.join(herdr.root, "repo");
    const origin = path.join(herdr.root, "origin.git");
    fs.mkdirSync(repo);
    git(herdr.root, ["init", "--bare", origin]);
    initGithubRepository(repo);
    fs.writeFileSync(path.join(repo, "README.md"), "# repo\n");
    git(repo, ["add", "README.md"]);
    git(repo, ["commit", "-m", "initial"]);
    git(repo, ["remote", "add", "origin", origin]);
    git(repo, ["push", "-q", "origin", "main"]);
    git(repo, ["remote", "set-head", "origin", "main"]);
    herdr.run(["workspace", "create", "--cwd", repo, "--label", "repo", "--env", `PATH=${herdr.fixturePath}`, "--no-focus"]);

    const gh = fakeGh(herdr.root, options);
    daemon = await startHided(herdr, "work-prs", undefined, { PATH: `${gh.bin}${path.delimiter}${herdr.fixturePath}` });
    const last = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, last);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "repo" }).locator("[data-checkout]").first())).click();
    await chooseTheme(page, "light");
    await showTool(page, "prs");
    const work = page.locator('aside[data-lane="prs"]');
    await expect(work.locator("[data-pr-group]").first()).toBeVisible({ timeout: 30_000 });
    await fn({ gh, work, sent, last, daemon });
  } finally {
    daemon?.stop();
    herdr.stop();
  }
}

/** The card in the lane's 보기 slot (PRD sidebar-task-lens D-50). */
const cardOf = (page: Page, number: number) => page.locator(`aside[data-lane="detail"] [data-pr-card="${number}"]`);

/** Opens the card's folded files line (B36); the card keeps it open while the app runs. */
async function unfold(card: Locator, section: "files"): Promise<void> {
  const toggle = card.locator(`[data-pr-card-toggle="${section}"]`);
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
}

test("the PRs tool groups by whose move it is and says why on each row", async ({ page }) => {
  await openWork(page, async ({ work }) => {
    // The rail is the documents button, then Memory · PRs | Explorer · Changes, PRs pressed (B23; PRD lane-two-columns B2).
    await expect(page.locator("[data-lane-tool]")).toHaveCount(5);
    await expect(page.locator('[data-lane-tool="prs"]')).toHaveAttribute("aria-pressed", "true");
    // The PRs icon carries the number of its Needs You rows: the three Needs You PRs (B29).
    await expect(page.locator('[data-lane-tool="prs"] [data-lane-prs-count]')).toHaveText("3");

    // The groups by whose move it is; each row says why in its second line (B6, B8, B24).
    const groups = work.locator("[data-pr-group]");
    expect(await groups.evaluateAll((sections) => sections.map((section) => section.getAttribute("data-pr-group")))).toEqual(["needs_you", "blocked", "drafts", "bots", "merged"]);
    const rowsOf = (group: string) => work.locator(`[data-pr-group="${group}"] [data-pr]`).evaluateAll((rows) => rows.map((row) => row.getAttribute("data-pr")));
    await expect(work.locator('[data-pr-group="needs_you"] [data-pr]')).toHaveCount(3);
    expect((await rowsOf("needs_you")).sort()).toEqual(["31", "32", "33"]);
    expect(await rowsOf("blocked")).toEqual(["34"]);
    // Drafts, Bots and merged start folded (B7); a group opens from its header.
    for (const group of ["drafts", "bots", "merged"]) await expect(work.locator(`[data-pr-group="${group}"]`)).toHaveAttribute("data-folded", "true");
    await work.locator('[data-pr-group="drafts"] [data-pr-focus="group"]').click();
    expect(await rowsOf("drafts")).toEqual(["35"]);
    const reason = (number: number) => work.locator(`[data-pr="${number}"] [data-pr-reason]`);
    await expect(reason(31)).toHaveAttribute("data-pr-reason", "merge");
    await expect(reason(32)).toHaveAttribute("data-pr-reason", "update");
    await expect(reason(33)).toHaveAttribute("data-pr-reason", "review");
    await expect(reason(34)).toHaveAttribute("data-pr-reason", "conflict");
    await screenshot(page, "work-prs-light");
  });
});

/** The groups the PRs tool draws and the pull requests in each, in the order drawn. */
async function groupsOf(work: Locator): Promise<Record<string, string[]>> {
  // Everything after Blocked starts folded (B7); a group opens from its header.
  for (const group of ["waiting", "drafts", "bots", "merged"]) {
    const section = work.locator(`[data-pr-group="${group}"]`);
    if ((await section.count()) > 0 && (await section.getAttribute("data-folded")) === "true") await section.locator('[data-pr-focus="group"]').click();
  }
  return work.locator("[data-pr-group]").evaluateAll((sections) =>
    Object.fromEntries(sections.map((section) => [section.getAttribute("data-pr-group")!, [...section.querySelectorAll("[data-pr]")].map((row) => row.getAttribute("data-pr")!).sort()])),
  );
}

// Issue #1189: the pull request list alone decides whether a project's read
// succeeds. A question beside it that fails leaves the judgments derived from
// it unknown, the list stays, and the ⚠ in the PRs tool's head says which question it was.
const UNREAD = '[data-prs-header] [data-prs-stale="true"]';
const warning = (work: Locator) => work.locator(UNREAD);
/** The ⚠ says `text` in its tooltip, and its accessible name; the lane is captured with the tooltip open, cropped so no pane or terminal prompt is in it. */
async function warnsWith(page: Page, work: Locator, text: string, name: string): Promise<void> {
  await expect(warning(work)).toHaveAttribute("aria-label", text);
  await warning(work).hover();
  await expect(page.locator('[data-slot="tooltip-content"]')).toContainText(text);
  const dir = process.env.HIDE_E2E_SCREENSHOT_DIR;
  if (!dir) return;
  // The tooltip slides in: capture after it has settled.
  for (let frame = 0; frame < 20; frame += 1) await page.evaluate(() => new Promise((done) => requestAnimationFrame(done)));
  const box = (await work.boundingBox())!;
  const tip = (await page.locator('[data-slot="tooltip-content"]').boundingBox())!;
  const right = Math.max(box.x + box.width, tip.x + tip.width);
  await page.screenshot({ path: path.join(dir, `${name}.png`), clip: { x: box.x, y: Math.min(box.y, tip.y), width: right - box.x, height: box.height } });
}

/** The reason a row says, by its kind and words. */
const reasonOf = (work: Locator, number: number) => work.locator(`[data-pr="${number}"] [data-pr-reason]`).first();

test("an unreadable account keeps the list and leaves every pull request's owner unjudged", async ({ page }) => {
  await openWork(
    page,
    async ({ work }) => {
      // Nobody can be said to be the author: the four open ones wait, plain.
      expect(Object.keys(await groupsOf(work))).toEqual(["waiting", "drafts", "bots", "merged"]);
      expect((await groupsOf(work)).waiting).toEqual(["31", "32", "33", "34"]);
      for (const number of [31, 32, 33, 34]) {
        await expect(reasonOf(work, number)).toHaveAttribute("data-pr-reason", "unjudged");
        await expect(reasonOf(work, number)).toContainText("Not judged · can't tell if it is yours");
      }
      // What the list alone decides is still decided: a draft, a bot's, a merged one.
      expect((await groupsOf(work)).drafts).toEqual(["35"]);
      expect((await groupsOf(work)).bots).toEqual(["36"]);
      expect((await groupsOf(work)).merged).toEqual(["37"]);
      // Nothing is asked of the operator, so the rail's PRs icon carries no number.
      await expect(page.locator('[data-lane-tool="prs"] [data-lane-prs-count]')).toHaveCount(0);
      await warnsWith(page, work, "GitHub read incomplete · could not read the signed-in account", "work-prs-unread-account-light");
    },
    { failing: ["account"] },
  );
});

test("unreadable review requests keep the list and judge a team's ask only as unknown", async ({ page }) => {
  await openWork(
    page,
    async ({ work }) => {
      // The request list still names the account on 33, and nobody on the others, so only the team's ask is unknown.
      expect(Object.keys(await groupsOf(work))).toEqual(["needs_you", "blocked", "waiting", "drafts", "bots", "merged"]);
      expect((await groupsOf(work)).needs_you).toEqual(["31", "32", "33"]);
      await expect(reasonOf(work, 33)).toHaveAttribute("data-pr-reason", "review");
      expect((await groupsOf(work)).waiting).toEqual(["38"]);
      await expect(reasonOf(work, 38)).toHaveAttribute("data-pr-reason", "unjudged");
      await expect(reasonOf(work, 38)).toContainText("Not judged · can't tell if a review was asked of you");
      await warnsWith(page, work, "GitHub read incomplete · could not read review requests", "work-prs-unread-review-requests-light");
    },
    { failing: ["review_requests"], teamAsk: true },
  );
});

test("an unreadable merge queue keeps the list and holds back only the pull requests a queue could hold", async ({ page }) => {
  await openWork(
    page,
    async ({ work }) => {
      const groups = await groupsOf(work);
      // 31 (clean) and 32 (behind) could be queued; 33 is a review, 34 a conflict.
      expect(groups.needs_you).toEqual(["33"]);
      expect(groups.blocked).toEqual(["34"]);
      expect(groups.waiting).toEqual(["31", "32"]);
      for (const number of [31, 32]) {
        await expect(reasonOf(work, number)).toHaveAttribute("data-pr-reason", "unjudged");
        await expect(reasonOf(work, number)).toContainText("Not judged · can't tell if it is in the merge queue");
      }
      await expect(page.locator('[data-lane-tool="prs"] [data-lane-prs-count]')).toHaveText("1");
      await warnsWith(page, work, "GitHub read incomplete · could not read the merge queue", "work-prs-unread-merge-queue-light");
    },
    { failing: ["merge_queue"] },
  );
});

test("all three extra questions failing still lists every pull request and names all three", async ({ page }) => {
  await openWork(
    page,
    async ({ work }) => {
      const groups = await groupsOf(work);
      expect(Object.keys(groups)).toEqual(["waiting", "drafts", "bots", "merged"]);
      expect(groups.waiting).toEqual(["31", "32", "33", "34", "38"]);
      await expect(reasonOf(work, 31)).toContainText("Not judged · can't tell if it is yours");
      // Plain Waiting says nothing of whose move it is, so the rail's PRs icon carries no number.
      await expect(page.locator('[data-lane-tool="prs"] [data-lane-prs-count]')).toHaveCount(0);
      await warnsWith(page, work, "GitHub read incomplete · could not read the signed-in account, review requests, the merge queue", "work-prs-unread-all-light");
    },
    { failing: ["account", "review_requests", "merge_queue"], teamAsk: true },
  );
});

test("the card opens in the 보기 slot and Merge goes out on the head it showed", async ({ page }) => {
  await openWork(page, async ({ gh, work, sent, last }) => {
    // A row opens the card in the 보기 slot and no Files tab (B27, B50), and
    // merge asks once, then goes out conditioned on the head commit the card
    // showed (B37).
    await work.locator('[data-pr-row="31"]').click();
    const card = cardOf(page, 31);
    await expect(card).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('[data-view-tab-bar] [role="tab"]', { hasText: "#31" })).toHaveCount(0);
    // The rail's slot button draws the card's PR mark in its state and stands
    // apart from the tools by one rule, so it is not read as a second PRs (B46).
    const rail = await page.locator("[data-tool-rail] > [data-lane-tool], [data-tool-rail] > [data-tool-rail-divider]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-lane-tool") ?? "divider"));
    expect(rail).toEqual(["detail", "documents", "divider", "memory", "prs", "divider", "explorer", "changes"]);
    await expect(page.locator('[data-lane-tool="detail"] [data-detail-mark]')).toHaveAttribute("data-detail-mark", "mergeable");
    await expect(card.locator('[data-pr-card-reason="merge"]')).toBeVisible();
    // Issue linking lives in the menu and hands focus to its picker.
    // The other actions still assign, create an issue or copy the branch.
    await card.locator('[data-pr-card-menu="31"]').click();
    await expect(page.getByRole("menuitem")).toHaveText(["Link issue", "Assign", "Create new issue", "Copy branch name"]);
    await page.getByRole("menuitem", { name: "Link issue", exact: true }).click();
    await expect(page.locator('[data-pr-link-picker="31"]').getByRole("combobox")).toBeFocused();
    await page.keyboard.press("Escape");
    // The timeline holds the pull request's milestones oldest first, with no
    // session yet (PRD pr-card-and-session-record-redesign B4, B5); the body
    // is open under its heading chips (B12, B13).
    await expect(card.locator("[data-timeline-milestone]").first()).toHaveAttribute("data-timeline-milestone", "opened");
    await expect(card.locator("[data-timeline-now]")).toBeVisible();
    await expect(card.locator("[data-pr-card-files]")).toHaveAttribute("data-pr-card-files", "3", { timeout: 30_000 });
    await expect(card.locator("[data-pr-card-outline]")).toBeVisible();
    await expect(card.locator("[data-pr-card-markdown]")).toBeVisible();
    await unfold(card, "files");
    await expect(card.locator("[data-pr-card-generated]")).toBeVisible();
    await screenshot(page, "work-pr-card-light");
    await chooseTheme(page, "dark");
    await screenshot(page, "work-pr-card-dark");
    await chooseTheme(page, "light");
    const merge = card.locator("[data-pr-card-merge]");
    await expect(merge).toBeEnabled();
    expect(writes(gh.state)).toEqual([]);
    await merge.click();
    await expect(card.locator("[data-pr-card-confirm]")).toBeVisible();
    expect(writes(gh.state)).toEqual([]);
    await card.locator("[data-pr-card-confirm-yes]").click();
    // The merged pull request's buttons, and with them the write line, are
    // gone as soon as the merge is seen, so the write is read from gh's log.
    await expect.poll(() => writes(gh.state), { timeout: 30_000 }).toHaveLength(1);
    expect(sent.get("pr_merge")).toBe(1);
    expect(last.get("pr_merge")).toMatchObject({ pr_number: 31, head: head(gh.state, 31) });
    expect(writes(gh.state)).toEqual([["pr", "merge", "31", "--merge", "--match-head-commit", head(gh.state, 31)]]);
    // The merged pull request's card draws no Merge, and the list, still in
    // the tool column beside the card (PRD lane-two-columns B9), has it out
    // of Needs You.
    await expect(card.locator("[data-pr-card-merge]")).toHaveCount(0);
    await expect(card).toBeVisible();
    await expect(work.locator('[data-pr-group="needs_you"] [data-pr]').first()).toBeVisible();
    await expect(work.locator('[data-pr-group="needs_you"] [data-pr="31"]')).toHaveCount(0);
    await work.locator('[data-pr-group="merged"] [data-pr-focus="group"]').click();
    await expect(work.locator('[data-pr-group="merged"] [data-pr="31"]')).toHaveCount(1);
  });
});

test("the lane chord brings the card back with its tool and alone, and the slot button opens another checkout's closed lane (lane-two-columns B15, B22)", async ({ page }) => {
  await openWork(page, async ({ work, daemon }) => {
    const screen = page.locator("[data-workspace-screen]");
    const slot = page.locator('[data-lane-tool="detail"]');
    await work.locator('[data-pr-row="31"]').click();
    const card = cardOf(page, 31);
    await expect(card).toBeVisible({ timeout: 30_000 });
    await expect(screen).toHaveAttribute("data-lane-shows", "prs");

    // Closed whole beside the PRs tool, the chord brings both back (B15).
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(card).toBeVisible();
    await expect(screen).toHaveAttribute("data-lane-shows", "prs");

    // The card alone, its tool column folded by the PRs icon, comes back alone.
    await page.locator('[data-lane-tool="prs"]').click();
    await expect(screen).toHaveAttribute("data-lane-shows", "none");
    await expect(card).toBeVisible();
    await page.keyboard.press(chord("toggle_lane"));
    await expect(screen).toHaveAttribute("data-lane-mode", "closed");
    await page.keyboard.press(chord("toggle_lane"));
    await expect(card).toBeVisible();
    await expect(screen).toHaveAttribute("data-lane-shows", "none");

    // A checkout whose lane was never opened: the window's slot still holds
    // the card, and its button opens that checkout's lane on it (B22).
    const other = path.join(daemon.home, "projects", "other");
    fs.mkdirSync(other, { recursive: true });
    fs.writeFileSync(path.join(other, "README.md"), "# other\n");
    await registerFolder(page, daemon, other);
    await expect(page.locator("[data-project]")).toHaveCount(2, { timeout: 20_000 });
    await (await showProjectRow(page, page.locator("[data-project]", { hasText: "other" }).locator("[data-checkout]").first())).click();
    await expect(screen).toHaveAttribute("data-lane-mode", "closed");
    await expect(slot).toHaveAttribute("aria-pressed", "false");
    await slot.click();
    await expect(card).toBeVisible();
    await expect(slot).toHaveAttribute("aria-pressed", "true");
  });
});

test("a branch update on a pull request that moved is refused, looked at again, then goes out on the new head", async ({ page }) => {
  await openWork(page, async ({ gh, work }) => {
    // GitHub refuses, the card says it changed, Look again reads it and the
    // update then goes out on the new head (B32, B33).
    await work.locator('[data-pr-row="32"]').click();
    const behind = cardOf(page, 32);
    await expect(behind).toBeVisible({ timeout: 30_000 });
    await expect(behind.locator('[data-pr-card-reason="update"]')).toBeVisible();
    await expect(behind.locator("[data-pr-card-files]")).toBeVisible({ timeout: 30_000 });
    const shown = head(gh.state, 32);
    moveHead(gh.state, 32, shown.slice(0, 39) + "e");
    await behind.locator("[data-pr-card-update]").click();
    await behind.locator('[data-pr-card-confirm-yes="update"]').click();
    await expect(behind.locator('[data-pr-card-write="failed"]')).toHaveAttribute("data-pr-card-write-reason", "head_moved", { timeout: 30_000 });
    await screenshot(page, "work-pr-card-moved-light");
    // Until Look again has read the card the refused write cannot be repeated.
    await expect(behind.locator("[data-pr-card-update]")).toBeDisabled();
    // Look again reads the list and the card; the update goes out once both
    // show the new head, on the head the card showed (B33).
    await behind.locator("[data-pr-card-look-again]").click();
    await expect(behind.locator("[data-pr-card-title]")).toContainText("(moved)", { timeout: 30_000 });
    await expect(behind).toHaveAttribute("data-pr-card-head", `${shown.slice(0, 39)}e`, { timeout: 30_000 });
    await behind.locator("[data-pr-card-update]").click();
    await behind.locator('[data-pr-card-confirm-yes="update"]').click();
    await expect(behind.locator('[data-pr-card-write="done"]')).toBeVisible({ timeout: 30_000 });
    const updates = writes(gh.state).filter((args) => args[0] === "api");
    expect(updates).toHaveLength(2);
    expect(updates[1]).toEqual(["api", "--method", "PUT", "repos/acme/repo/pulls/32/update-branch", "-f", `expected_head_sha=${shown.slice(0, 39)}e`]);
  });
});

test("the list and the card read without sideways scrolling in each language and both themes", async ({ page }) => {
  await openWork(page, async ({ work }) => {
    // The lane opens unpinned, over the agents; a pinned one would dock and redraw under the keys.
    await setLanePinned(page, false);
    await work.locator('[data-pr-row="33"]').click();
    const review = cardOf(page, 33);
    await expect(review).toBeVisible({ timeout: 30_000 });
    const overflow = (target: Locator) => target.evaluate((node) => node.scrollWidth - node.clientWidth);
    // The narrowest lane (B48): the divider's keys narrow it to its minimum.
    const divider = page.locator('[data-lane-divider="lane"]');
    await divider.focus();
    const reading = (name: string) => divider.evaluate((node, attribute) => Number(node.getAttribute(attribute)), name);
    const narrowest = await reading("aria-valuemin");
    for (let before = await reading("aria-valuenow"); before > narrowest; before = await reading("aria-valuenow")) {
      await page.keyboard.press("ArrowRight");
      await expect(divider).not.toHaveAttribute("aria-valuenow", String(before), { timeout: 10_000 });
    }
    await expect(divider).toHaveAttribute("aria-valuenow", String(narrowest));
    for (const [language, theme] of [["ko", "light"], ["ja", "dark"], ["zh-CN", "light"], ["en", "dark"]] as const) {
      await page.keyboard.press(chord("settings"));
      await page.locator('[data-settings-tab="general"]').click();
      await page.locator("[data-interface-language]").click();
      await page.locator(`[data-language-option="${language}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await page.locator(`[data-theme-option="${theme}"]`).click();
      await page.keyboard.press("Escape");
      await expect(page.locator('[data-settings="true"]')).toHaveCount(0);
      // One thing shows at a time: the card in the 보기 slot, then the list in PRs,
      // and the same row brings the card back in front.
      await expect(review).toBeVisible();
      await expect.poll(() => overflow(review)).toBeLessThanOrEqual(0);
      await showTool(page, "prs");
      await expect(work.locator('[data-prs-view="board"]')).toBeVisible();
      await expect.poll(() => overflow(work.locator('[data-prs-view="board"]'))).toBeLessThanOrEqual(0);
      await screenshot(page, `work-prs-${language}-${theme}`);
      await work.locator('[data-pr-row="33"]').click();
    }
  });
});

test("a merged pull request opens its card and draws its parts, with no Merge", async ({ page }) => {
  await openWork(page, async ({ work }) => {
    await work.locator('[data-pr-group="merged"] [data-pr-focus="group"]').click();
    await work.locator('[data-pr-row="37"]').click();
    const card = cardOf(page, 37);
    await expect(card).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('[data-lane-tool="detail"] [data-detail-mark]')).toHaveAttribute("data-detail-mark", "merged");
    await expect(card.locator("[data-pr-card-files]")).toBeVisible({ timeout: 30_000 });
    // A merged pull request's timeline ends on `머지됨` with no now row (B6).
    await expect(card.locator("[data-timeline-milestone]").last()).toHaveAttribute("data-timeline-milestone", "merged");
    await expect(card.locator("[data-timeline-now]")).toHaveCount(0);
    await expect(card.locator("[data-pr-card-markdown]")).toBeVisible();
    await expect(card.locator("[data-pr-card-merge]")).toHaveCount(0);
    // A finished pull request's ⋯ keeps only the branch name to copy (D-28).
    await card.locator('[data-pr-card-menu="37"]').click();
    await expect(page.getByRole("menuitem")).toHaveText(["Copy branch name"]);
    await page.keyboard.press("Escape");
  });
});

test("a card read at another head than the list's cannot merge: the write is refused before gh is asked", async ({ page }) => {
  await openWork(page, async ({ gh, work }) => {
    skewCardHead(gh.state, 31, head(gh.state, 31).slice(0, 39) + "a");
    await work.locator('[data-pr-row="31"]').click();
    const card = cardOf(page, 31);
    await expect(card.locator("[data-pr-card-files]")).toBeVisible({ timeout: 30_000 });
    await card.locator("[data-pr-card-merge]").click();
    await card.locator('[data-pr-card-confirm-yes="merge"]').click();
    await expect(card.locator('[data-pr-card-write="failed"]')).toHaveAttribute("data-pr-card-write-reason", "head_moved", { timeout: 30_000 });
    await expect(card.locator("[data-pr-card-merge]")).toBeDisabled();
    expect(writes(gh.state)).toEqual([]);
  });
});
