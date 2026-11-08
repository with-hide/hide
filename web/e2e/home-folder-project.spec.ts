// The account's home folder as a project (PRD home-folder-project B1-B9,
// B17), against a private engine and hided on a temporary HOME: hided asks
// about it instead of refusing it, registers it only once its own path is
// confirmed, lists its top level without hidden names and app data, finds
// files under it in ⌘P without those names or another project's folder, and
// removing it changes nothing under home.
import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { startFixture } from "./engine-fixture";
import { asFirstRun, startHided } from "./hided-fixture";
import { registerFolder, screenshot, sendEvent } from "./wire";
import { toPage } from "../../desktop/src/main/wirePath";

test.describe.configure({ timeout: 180_000 });

/** The system's app-data folder at the top of a home, where it names one (`hide_host::list::APP_DATA_DIR`). */
const APP_DATA = process.platform === "darwin" ? "Library" : process.platform === "win32" ? "AppData" : null;

type Listed = { entries: { name: string }[] };
type Indexed = { files: { relative_path: string }[]; indexing: boolean };

test("the home folder is asked about once, added when confirmed, and listed and searched without its clutter", async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const engine = await startFixture();
  let daemon: Awaited<ReturnType<typeof startHided>> | null = null;
  try {
    daemon = await asFirstRun(await startHided(engine, "home-folder-project"));
    const home = daemon.home;
    const write = (relative: string) => {
      fs.mkdirSync(path.dirname(path.join(home, relative)), { recursive: true });
      fs.writeFileSync(path.join(home, relative), "x");
    };
    write(".dotfile");
    write(".config/tool/settings.json");
    write("Documents/note.md");
    write("work/todo.txt");
    write("work/app/main.rs");
    if (APP_DATA) write(`${APP_DATA}/cache.db`);
    await page.goto(`${daemon.origin}/#token=${daemon.token}`);
    await expect(page.locator("[data-main-screen]")).toBeVisible({ timeout: 20_000 });
    // A plain folder is a folder row: one button named for it (the dialog names it for its folder).
    const project = page.locator("[data-project]").filter({ has: page.getByRole("button", { name: "home", exact: true }) });
    // A project inside home, which the home project's search leaves to its own.
    await registerFolder(page, daemon, path.join(home, "work/app"));

    // B1: home is asked about, in its own words, and nothing is registered.
    const add = (confirmed: boolean) => ({ path: toPage(home), label: "account", initialize_git: false, ...(confirmed ? { confirmed_outside_home: toPage(home) } : {}) });
    const asked = await sendEvent(page, daemon, "create_workspace", add(false), "path_refused");
    expect(asked?.payload).toMatchObject({ reason: "needs_confirmation", home: true });
    // B7: a folder that holds home is still refused.
    const holder = await sendEvent(page, daemon, "create_workspace", { path: toPage(path.dirname(home)), label: "up", initialize_git: false }, "path_refused");
    expect((holder?.payload as { reason?: string } | undefined)?.reason).toBe("home_root");

    // B1, B4: typing `~` in the dialog leads to the same question, with its button holding the keyboard (B3).
    await page.locator("[data-sidebar-new-workspace]").click();
    const dialog = page.locator("[data-add-project]");
    await dialog.locator("[data-add-project-browse]").click();
    await dialog.locator("[data-folder-browser-field]").fill("~");
    await dialog.locator("[data-folder-browser-field]").press("Enter");
    await expect(dialog.locator("[data-folder-browser-target]")).toHaveAttribute("data-folder-browser-target", home);
    await dialog.getByRole("button", { name: "Add", exact: true }).click();
    const question = dialog.locator("[data-registration-confirm]");
    await expect(question).toContainText("home folder");
    await expect(dialog.locator("[data-registration-confirm-add]")).toBeFocused();
    await screenshot(page, "home-folder-project-question");
    await expect(project).toHaveCount(0);

    // B2: confirmed, it is a project of its own.
    await dialog.locator("[data-registration-confirm-add]").click();
    await expect(project).toBeVisible({ timeout: 30_000 });

    // B8: the root lists no hidden name and no app data, and every other folder.
    const listing = await sendEvent(page, daemon, "file_list", { root: toPage(home), path: toPage(home) }, "directory_list");
    const names = (listing?.payload as Listed).entries.map((entry) => entry.name);
    expect(names).toEqual(expect.arrayContaining(["Documents", "work"]));
    expect(names.filter((name) => name.startsWith("."))).toEqual([]);
    if (APP_DATA) expect(names).not.toContain(APP_DATA);
    // A hidden path named directly still lists.
    const typed = await sendEvent(page, daemon, "file_list", { root: toPage(home), path: toPage(path.join(home, ".config")) }, "directory_list");
    expect((typed?.payload as Listed).entries.map((entry) => entry.name)).toContain("tool");

    // B18: a folder under home opens as in any project, a hidden one too.
    const documents = await sendEvent(page, daemon, "file_list", { root: toPage(home), path: toPage(path.join(home, "Documents")) }, "directory_list");
    expect((documents?.payload as Listed).entries.map((entry) => entry.name)).toContain("note.md");

    // B9: ⌘P finds files under home, none under hidden or app-data folders or the other project's.
    let files: string[] = [];
    await expect
      .poll(
        async () => {
          const found = (await sendEvent(page, daemon!, "file_index", { root: toPage(home), query: "" }, "file_index_result"))?.payload as Indexed;
          files = found.files.map((file) => file.relative_path);
          return found.indexing;
        },
        { timeout: 60_000 },
      )
      .toBe(false);
    expect(files).toEqual(expect.arrayContaining(["Documents/note.md", "work/todo.txt"]));
    expect(files.filter((file) => file.startsWith(".") || file.startsWith("work/app/"))).toEqual([]);
    if (APP_DATA) expect(files.filter((file) => file.startsWith(`${APP_DATA}/`))).toEqual([]);

    // B17: removing the project removes only its registration.
    const id = await project.first().getAttribute("data-project");
    await sendEvent(page, daemon, "remove_workspace", { workspace_id: id });
    await expect(project).toHaveCount(0, { timeout: 30_000 });
    expect(fs.existsSync(path.join(home, ".dotfile"))).toBe(true);
    expect(fs.existsSync(path.join(home, "Documents/note.md"))).toBe(true);
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
