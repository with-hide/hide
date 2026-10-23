// The Explorer's Git status is a mark in its header row, so the answer landing
// never moves a row the operator is aiming at (issue 570). The answer is held at the
// socket while the rows are drawn and measured, then released.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, showExplorer } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

const git = (herdr: HerdrFixture, root: string, ...args: string[]) => execFileSync("git", ["-C", root, ...args], { env: herdr.env, stdio: "ignore" });

/** Each folder the Explorer can show, set up before hided starts, and the Git mark its answer leaves in the header. */
const FOLDERS: Array<{ name: string; setup: (herdr: HerdrFixture, root: string) => void; mark: { state: string; says: RegExp } | null }> = [
  // Nothing for the operator to do, so nothing is drawn.
  { name: "a plain folder", setup: () => {}, mark: null },
  { name: "a repository", setup: (herdr, root) => git(herdr, root, "init", "-q"), mark: null },
  // `rev-parse` finds the repository without the index and `status` then fails
  // on it: a failure the operator can repair.
  {
    name: "a repository whose index is corrupt",
    setup: (herdr, root) => {
      git(herdr, root, "init", "-q");
      fs.writeFileSync(path.join(root, ".git", "index"), "not an index");
    },
    mark: { state: "unavailable", says: /^Git status unavailable: Git could not read this repository/ },
  },
];

for (const folder of FOLDERS) {
  test(`an Explorer row stays where it was drawn when the Git status of ${folder.name} lands`, async ({ page }) => {
    const herdr = await startHerdr({ agents: false });
    let daemon: Daemon | null = null;
    try {
      const root = fs.realpathSync(path.join(herdr.root, "fixture"));
      for (const name of ["first.md", "second.md"]) fs.writeFileSync(path.join(root, name), `# ${name}\n`);
      folder.setup(herdr, root);
      daemon = await startHided(herdr, "explorer-git-status");
      const release = await holdGitAnswer(page, root);
      await page.goto(`${daemon.origin}/#token=${daemon.token}`);
      await enterWorkspace(page, "fixture");
      await showExplorer(page);
      const second = page.locator(`[data-explorer-row="${toPage(root)}/second.md"]`);
      const mark = page.locator("[data-explorer-git]");
      await expect(mark).toHaveAttribute("data-explorer-git", "loading");
      await expect(second).toBeVisible();
      const drawn = await second.boundingBox();
      await release();
      await expect(page.locator('[data-explorer-git="loading"]')).toHaveCount(0);
      expect(await second.boundingBox()).toEqual(drawn);
      if (folder.mark) {
        await expect(mark).toHaveAttribute("data-explorer-git", folder.mark.state);
        await expect(mark).toHaveAttribute("aria-label", folder.mark.says);
      } else await expect(mark).toHaveCount(0);
    } finally {
      daemon?.stop();
      herdr.stop();
    }
  });
}

/**
 * Holds the frame carrying `root`'s Git answer, and every frame after it in
 * order, until the returned release. A folder listing answers its own request
 * and draws the rows being measured, so it passes.
 */
async function holdGitAnswer(page: Page, root: string): Promise<() => Promise<void>> {
  let holding = true;
  const held: Array<() => void> = [];
  await page.routeWebSocket(/\/ws$/, (socket) => {
    const server = socket.connectToServer();
    socket.onMessage((message) => server.send(message));
    server.onMessage((message) => {
      const frame = typeof message === "string" && message.startsWith("{") ? JSON.parse(message) : null;
      const listing = frame?.type === "directory_list";
      const gitAnswer = frame?.payload?.changes?.root_path === toPage(root);
      if (holding && !listing && (held.length > 0 || gitAnswer)) {
        held.push(() => socket.send(message));
        return;
      }
      socket.send(message);
    });
  });
  return async () => {
    await expect.poll(() => held.length).toBeGreaterThan(0);
    holding = false;
    for (const send of held.splice(0)) send();
  };
}
