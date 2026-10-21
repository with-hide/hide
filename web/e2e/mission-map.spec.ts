// A Conductor's mission as the operator meets it, on an isolated pinned
// Herdr and hided (PRD software-factory-conductor B14, B15, B19, B21, B48):
// the real `hide mission` commands from the pane's own shell change the
// core's record, the first plan opens the map in the lane's 보기 slot once,
// the map follows each change of the record, and Settings › Conductor keeps
// the cap the next `hide mission settings` reads.

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
    { key: "cause", title: "원인 조사", kind: "research" },
    { key: "api", title: "API 추가", kind: "change", delivery: "pr", worker: { agent: "claude" } },
    { key: "screen", title: "API 쓰는 화면", kind: "change", delivery: "pr", depends_on: ["api"] },
  ],
};

test("the Conductor's commands draw the mission map once, the map follows the record, and the Conductor settings keep the cap", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const herdr = await startHerdr({ agents: false });
  let daemon: Daemon | null = null;
  try {
    // A mission belongs to a repository Project, so the Conductor runs in one.
    const repoDir = path.join(herdr.root, "missions-repo");
    fs.mkdirSync(repoDir);
    const repo = fs.realpathSync(repoDir);
    for (const args of [["init", "-q", "-b", "main"], ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-q", "--allow-empty", "-m", "baseline"]]) {
      const git = spawnSync("git", ["-C", repo, ...args], { encoding: "utf8" });
      expect(git.status, git.stderr).toBe(0);
    }
    const created = herdr.run(["workspace", "create", "--cwd", repo, "--label", "missions-repo", "--env", `PATH=${herdr.fixturePath}`, "--focus"]) as { result: { root_pane: { pane_id: string } } };
    daemon = await startHided(herdr, "mission-map");
    const pane = created.result.root_pane.pane_id;
    const conductor = await agentPane(herdr, daemon.stateDir, pane);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await enterWorkspace(page, "missions-repo");
    const map = page.locator('aside[data-detail="mission"]');

    // The core reads this machine's projects just after it starts; until then it answers that it is still reading.
    await expect.poll(async () => (await conductor(["mission", "create", "--title", "이슈 정리"])).out, { timeout: 20_000 }).toContain('"ok":true');
    // A mission without a plan opens nothing (B14: the first plan does).
    await expect(page.locator("[data-workspace-screen]")).toBeVisible();
    await expect(map).toHaveCount(0);

    await mission(conductor, ["mission", "plan", "--json", JSON.stringify(PLAN)]);
    await expect(map).toBeVisible();
    await expect(map.locator("[data-detail-title]")).toHaveAttribute("data-detail-title", /이슈 정리/);
    await expect(map.locator("[data-mission-status]")).toHaveAttribute("data-mission-status", "planning");
    await expect(map.locator("[data-mission-node]")).toHaveCount(3);
    // The node that waits on another is drawn below it (down flow).
    const api = (await map.locator('[data-mission-node="api"]').boundingBox())!;
    const screen = (await map.locator('[data-mission-node="screen"]').boundingBox())!;
    expect(screen.y).toBeGreaterThan(api.y + api.height);

    await mission(conductor, ["mission", "start"]);
    // The node's worker is a live agent row (here the Conductor's own registration), as a spawned worker's is.
    const listed = (herdr.run(["api", "snapshot"]) as { result: { snapshot: { agents: { pane_id: string; terminal_id: string; agent_session?: { value: string } }[] } } }).result.snapshot.agents.find((agent) => agent.pane_id === pane)!;
    const self = JSON.parse((await conductor(["agent", "register", "--host-scope", herdr.socket, "--session", listed.agent_session!.value, "--instance", listed.terminal_id, "--name", "conductor", "--pane", pane])).out.trim().split("\n").at(-1)!) as { ok: boolean; value: { id: string } };
    expect(self.ok, JSON.stringify(self)).toBe(true);
    await mission(conductor, ["mission", "node", "api", "--status", "working", "--agent-id", self.value.id, "--line", "엔드포인트 작성 중"]);
    await expect(map.locator("[data-mission-status]")).toHaveAttribute("data-mission-status", "running");
    const node = map.locator('[data-mission-node="api"]');
    await expect(node).toHaveAttribute("data-node-status", "working");
    await expect(node.locator("[data-node-line]")).toHaveText("엔드포인트 작성 중");
    // Its worker row names the agent the plan chose, with the time it has worked.
    await expect(node.locator("[data-node-worker]")).toHaveAttribute("data-node-worker", "claude");
    await expect(node.locator("[data-mission-duration]")).toBeVisible();
    await mission(conductor, ["mission", "decide", "--line", "화면 PR 방식 선택"]);
    await expect(map.locator('[data-mission-banner="decision"]')).toContainText("화면 PR 방식 선택");
    // A card opens its detail beside it in the app too, where the terminal keeps the keyboard.
    await node.click();
    await expect(page.locator('[data-mission-detail="api"]')).toBeVisible();
    await node.click();
    await expect(page.locator('[data-mission-detail="api"]')).toHaveCount(0);
    await screenshot(page, "mission-map-running");

    // Closed by the operator, a later plan does not open it again.
    await map.locator("[data-detail-close]").click();
    await expect(map).toHaveCount(0);
    await mission(conductor, ["mission", "plan", "--json", JSON.stringify({ ...PLAN, nodes: [...PLAN.nodes, { key: "together", title: "함께 확인", kind: "verify", depends_on: ["screen"] }] })]);
    const shown = await mission(conductor, ["mission", "show"]);
    expect(JSON.stringify(shown.result)).toContain("together");
    await expect(map).toHaveCount(0);
    // The Conductor's row carries the mission mark, and pressing it opens the map again (B14).
    const chip = page.locator("nav[data-sidebar] [data-mission-chip]");
    await expect(chip).toHaveAttribute("data-mission-chip-status", "running");
    await expect(chip).toContainText("0/4");
    await chip.click();
    await expect(map).toBeVisible();
    await expect(map.locator("[data-mission-node]")).toHaveCount(4);

    // Settings › Conductor writes the cap the Conductor reads.
    await page.locator("[data-open-settings]").click();
    const sheet = page.locator('[data-settings="true"]');
    await sheet.locator('[data-settings-tab="conductor"]').click();
    await sheet.locator("[data-conductor-cap]").click();
    await page.getByRole("option", { name: "6", exact: true }).click();
    await expect(sheet.locator("[data-conductor-cap]")).toHaveAttribute("data-conductor-cap", "6");
    await expect.poll(async () => JSON.stringify((await mission(conductor, ["mission", "settings"])).result)).toContain('"concurrency":6');
  } finally {
    daemon?.stop();
    herdr.stop();
  }
});
