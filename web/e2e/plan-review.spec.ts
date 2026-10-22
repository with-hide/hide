// A session's own plan as the operator meets it, on an isolated pinned Herdr
// and hided (PRD hide-plan B4-B7, B11, B13, B14): the real `hide plan`
// commands from the agent's own pane change the core's record, a plan put up
// for review asks the operator on the pane's band and the chip says Review,
// the agent's approval clears both, a working step takes the pane's second
// line, and the chip's map opens large in the 보기 slot.

import { expect, test } from "@playwright/test";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { agentPane, type HideAnswer } from "./agent-pane";
import { startHerdr } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

test.describe.configure({ timeout: 120_000 });

type Answer = { ok: boolean; reason?: string; result?: Record<string, unknown> };

/** One `hide plan` command run as the agent; every one here is expected to succeed. */
async function plan(hide: (args: string[]) => Promise<HideAnswer>, args: string[]): Promise<Answer> {
  const ran = await hide(["plan", ...args]);
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  const answer = JSON.parse(line) as Answer;
  expect(answer.ok, `plan ${args.join(" ")}: ${line}`).toBe(true);
  return answer;
}

const PLAN = {
  nodes: [
    { key: "spec", title: "Spec", depends_on: [], status: "done" },
    { key: "core", title: "Core events", depends_on: ["spec"], status: "planned" },
    { key: "web", title: "Web map", depends_on: ["core"], status: "planned", output: "pr" },
    { key: "docs", title: "Docs", depends_on: ["spec"], status: "planned" },
  ],
};

test("a plan put up for review asks on the pane, the agent's approval clears it, and its steps draw on the chip and in the 보기 slot", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const repoDir = path.join(herdr.root, "plan-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    for (const args of [["init", "-q", "-b", "main"], ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "baseline"]]) {
      const git = spawnSync("git", ["-C", repo, ...args], { encoding: "utf8" });
      expect(git.status, git.stderr).toBe(0);
    }
    const created = herdr.run(["workspace", "create", "--cwd", repo, "--label", "plan-repo", "--env", `PATH=${herdr.fixturePath}`, "--focus"]) as { result: { root_pane: { pane_id: string } } };
    daemon = await startHided(herdr, "plan-review");
    const pane = created.result.root_pane.pane_id;
    const agent = await agentPane(herdr, daemon.stateDir, pane);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "plan-repo");

    // The saved plans are read once the link worker is up; until then the core says so.
    await expect.poll(async () => (await agent(["plan", "set", "--json", JSON.stringify(PLAN), "--ask", "범위 확인"])).out, { timeout: 20_000 }).toContain('"ok":true');
    const chip = page.locator("[data-pane-plan-chip]");
    await expect(chip).toHaveAttribute("data-plan-chip-state", "review");
    await expect(chip).toHaveText("Review");
    // The review is the agent's question: the pane's ask band says it (B14).
    await expect(page.locator('[data-pane-header-band="answer"]')).toContainText("계획 검토 · 4단계");
    // The sidebar's second line asks the same.
    const sidebar = page.locator("[data-sidebar-content]").first();
    await expect(sidebar).toContainText("계획 검토 · 4단계");
    // Putting a plan up opens no 보기 slot by itself (D-29).
    await expect(page.locator('aside[data-detail="plan"]')).toHaveCount(0);

    await plan(agent, ["approve"]);
    await expect(chip).toHaveAttribute("data-plan-chip-state", "running");
    await expect(chip).toHaveText("1/4");
    await expect(page.locator('[data-pane-header-band="answer"]')).toHaveCount(0);
    await expect(sidebar).not.toContainText("계획 검토");

    await plan(agent, ["node", "core", "--status", "working", "--line", "이벤트 쓰는 중"]);
    // The working step and its line take the pane's second line (B7).
    await expect(page.locator("[data-pane-own-progress]")).toHaveText("Core events · 이벤트 쓰는 중");
    // A step whose step above is not done cannot be done (B2).
    const refused = JSON.parse((await agent(["plan", "node", "web", "--status", "done"])).out.trim().split("\n").at(-1)!) as Answer;
    expect([refused.ok, refused.reason]).toEqual([false, "plan_order_refused"]);
    await plan(agent, ["node", "core", "--status", "done"]);
    await expect(chip).toHaveText("2/4");

    // The chip's popover is the small map; Larger draws it in the 보기 slot.
    await chip.hover();
    const popover = page.locator("[data-pane-plan]");
    await expect(popover.locator("[data-plan-step]")).toHaveCount(4);
    await expect(popover.locator('[data-plan-step="core"]')).toHaveAttribute("data-step-status", "done");
    await screenshot(page, "plan-chip-popover");
    await popover.locator("[data-pane-plan-large]").click();
    const map = page.locator('aside[data-detail="plan"]');
    await expect(map).toBeVisible();
    await expect(map.locator('[data-plan-graph="large"] [data-plan-step]')).toHaveCount(4);
    await expect(map.locator("[data-plan-banner]")).toHaveCount(0);
    // The step that waits on another is drawn below it (down flow).
    const core = (await map.locator('[data-plan-step="core"]').boundingBox())!;
    const web = (await map.locator('[data-plan-step="web"]').boundingBox())!;
    expect(web.y).toBeGreaterThan(core.y + core.height);
    await screenshot(page, "plan-map-slot");

    // Dropping the plan takes the chip away.
    await plan(agent, ["drop"]);
    await expect(chip).toHaveCount(0);
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
