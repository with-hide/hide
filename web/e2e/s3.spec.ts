// S3 Explorer and editor flows on an isolated pinned Herdr (PRD
// web-shell-pivot-s3 B1, B3, B4, B5): a real git checkout listed lazily in the
// Explorer, a folder expanded through the core's ui state, a file opened into
// the checkout's preview tab, then edited, saved, and saved again after the
// disk changed underneath it. Every command runs against a private server; the
// operator's Herdr is never touched.

import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { toPage } from "../../desktop/src/main/wirePath";
import { startHerdr, type HerdrFixture } from "./herdr-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { countSent, enterWorkspace, screenshot, showDocuments, showExplorer, showProjectRow } from "./wire";
import { chord, mod } from "./chords";
import { dumpOnFailure } from "./failure-dump";
import { onDisk, quietFor } from "./wait";

const SOURCE = "export const answer = 41;\n";

/** A 1x1 PNG, so an image viewer has real bytes to decode. */
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  "base64",
);

/**
 * A wide RGB PNG of 64-pixel squares, larger than any viewer, so it opens
 * fitted to the viewer's width and every zoom past the fit scrolls across.
 */
function widePng(width: number, height: number): Buffer {
  const rows = Array.from({ length: height }, (_, y) => {
    const row = Buffer.alloc(1 + width * 3);
    for (let x = 0; x < width; x += 1) row.fill(((x >> 6) + (y >> 6)) % 2 ? 60 : 210, 1 + x * 3, 4 + x * 3);
    return row;
  });
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const frame = Buffer.alloc(8 + data.length + 4);
    frame.writeUInt32BE(data.length, 0);
    body.copy(frame, 4);
    frame.writeUInt32BE(zlib.crc32(body), 4 + body.length);
    return frame;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // bits per sample
  header[9] = 2; // RGB
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", zlib.deflateSync(Buffer.concat(rows))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** A minimal one-page PDF with a correct cross-reference table. */
function minimalPdf(): Buffer {
  const objects = [
    "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
    "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
    "4 0 obj\n<< /Length 44 >>\nstream\nBT /F1 24 Tf 40 100 Td (Hello PDF) Tj ET\nendstream\nendobj\n",
    "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
  ];
  let pdf = "%PDF-1.4\n";
  const offsets: number[] = [];
  for (const object of objects) {
    offsets.push(pdf.length);
    pdf += object;
  }
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf, "latin1");
}

/** A committed file the working tree then changes, so the core reports it. */
function gitFixture(dir: string): void {
  execFileSync("git", ["init", "-q"], { cwd: dir });
  fs.writeFileSync(path.join(dir, "tracked.txt"), "one\n");
  execFileSync("git", ["add", "-A"], { cwd: dir });
  execFileSync("git", ["-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-c", "commit.gpgsign=false", "commit", "-qm", "init"], { cwd: dir });
  fs.writeFileSync(path.join(dir, "tracked.txt"), "two\n");
}

type Fixture = {
  herdr: HerdrFixture;
  daemon: Daemon;
  /** The resolved checkout root in the core's wire spelling. */
  repo: string;
  file: string;
  sent: Map<string, number>;
  lastSent: Map<string, Record<string, unknown>>;
};

/** Starts the isolated stack and leaves the Explorer showing `src/main.ts`. */
async function openCheckout(
  page: Page,
  beforeLoad?: (page: Page) => Promise<void>,
  extraFiles: Record<string, string> = {},
): Promise<Fixture> {
  const herdr = await startHerdr();
  let daemon: Daemon | null = null;
  try {
    const repoDir = path.join(herdr.root, "repo");
    fs.mkdirSync(path.join(repoDir, "src"), { recursive: true });
    fs.writeFileSync(path.join(repoDir, "src", "main.ts"), SOURCE);
    fs.writeFileSync(path.join(repoDir, "README.md"), "# repo\n");
    for (const [name, contents] of Object.entries(extraFiles)) {
      fs.writeFileSync(path.join(repoDir, name), contents);
    }
    fs.writeFileSync(
      path.join(repoDir, "notes.md"),
      "# Title\n\n- one\n- two\n\n- [ ] open\n\nsee [text](https://example.com) now\n\n```rust\nfn main() {}\n```\n",
    );
    // A block longer than the pane's eight lines, so the pane scrolls inside
    // itself rather than pushing the body down (D-11).
    fs.writeFileSync(
      path.join(repoDir, "post.md"),
      `---\ntitle: Post\n${Array.from({ length: 12 }, (_, index) => `key${index}: value`).join("\n")}\n---\n\n# Body\n`,
    );
    fs.writeFileSync(path.join(repoDir, ".gitignore"), "node_modules\n");
    fs.writeFileSync(path.join(repoDir, "shot.png"), PNG);
    fs.writeFileSync(path.join(repoDir, "wide.png"), widePng(2400, 600));
    fs.writeFileSync(path.join(repoDir, "doc.pdf"), minimalPdf());
    fs.copyFileSync(path.resolve("e2e/fixtures/tiny.mp4"), path.join(repoDir, "clip.mp4"));
    gitFixture(repoDir);
    // Sparse and untracked: past the editable cap, so the browser offers a
    // download instead of a buffer (D-12).
    fs.writeFileSync(path.join(repoDir, "huge.txt"), "");
    fs.truncateSync(path.join(repoDir, "huge.txt"), 17 * 1024 * 1024);
    const repo = toPage(fs.realpathSync(repoDir));
    herdr.run([
      "workspace", "create", "--cwd", repoDir, "--label", "repo",
      "--env", `PATH=${herdr.fixturePath}`, "--no-focus",
    ]);

    daemon = await startHided(herdr, "s3");
    const lastSent = new Map<string, Record<string, unknown>>();
    const sent = countSent(page, lastSent);
    await beforeLoad?.(page);
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);

    // Focus the repository checkout and show the files tool with its tree
    // (PRD right-lane B9).
    const row = await showProjectRow(page, page.locator("[data-project]", { hasText: "repo" }).locator("[data-checkout]").first());
    await row.click();
    await expect(row).toHaveAttribute("aria-current", "true");
    await showExplorer(page);
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/src"]`)).toBeVisible();

    // Expand src and open the file into the preview tab.
    await page.locator(`[data-explorer-row="${repo}/src"]`).click();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toBeVisible();
    await page.locator(`[data-explorer-row="${repo}/src/main.ts"]`).click();
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("export const answer = 41;");
    return { herdr, daemon, repo, file: `${repo}/src/main.ts`, sent, lastSent };
  } catch (error) {
    daemon?.stop();
    herdr.stop();
    throw error;
  }
}

/** The id a draft is stored under (S5.5 B9): this host, the device, the checkout root and the path. */
/**
 * Leaves the app for a page of the same origin that runs none of it, so a
 * test can shape the IndexedDB the app will find without the app opening it
 * at the same time. Returns the app's URL to go back to.
 */
async function leaveApp(page: Page): Promise<string> {
  const app = page.url();
  await page.goto(new URL("/health", app).toString());
  return app;
}

function draftId(hostId: string, root: string, file: string, device: string): string {
  return [hostId, device, root, file].join("\u0000");
}

type DraftRow = { id: string; host: string | null; device: string | null; root: string; path: string; contents: string; updated_at: number };

/** Writes one row into the shell's v4 draft store, as a tab or an older build left it. */
async function seedDraft(page: Page, row: DraftRow): Promise<void> {
  await page.evaluate(async (row) => {
    await new Promise<void>((resolve, reject) => {
      const opened = indexedDB.open("hide-shell", 4);
      opened.onupgradeneeded = () => {
        if (!opened.result.objectStoreNames.contains("drafts_v4")) opened.result.createObjectStore("drafts_v4", { keyPath: "id" });
      };
      opened.onerror = () => reject(opened.error);
      opened.onsuccess = () => {
        const database = opened.result;
        const transaction = database.transaction("drafts_v4", "readwrite");
        transaction.objectStore("drafts_v4").put(row);
        transaction.oncomplete = () => { database.close(); resolve(); };
        transaction.onerror = () => { database.close(); reject(transaction.error); };
      };
    });
  }, row);
}

/** Every row of the shell's v4 draft store. */
async function storedDrafts(page: Page): Promise<DraftRow[]> {
  return page.evaluate(async () => new Promise<DraftRow[]>((resolve, reject) => {
    const opened = indexedDB.open("hide-shell", 4);
    opened.onerror = () => reject(opened.error);
    opened.onsuccess = () => {
      const database = opened.result;
      if (!database.objectStoreNames.contains("drafts_v4")) { database.close(); resolve([]); return; }
      const read = database.transaction("drafts_v4", "readonly").objectStore("drafts_v4").getAll();
      read.onsuccess = () => { database.close(); resolve(read.result as DraftRow[]); };
      read.onerror = () => { database.close(); reject(read.error); };
    };
  }));
}

function close(fixture: Fixture): void {
  fixture.daemon.stop();
  fixture.herdr.stop();
}

test.describe.configure({ timeout: 90_000 });

test("Explorer opens HTML as a browser page and keeps source editing explicit", async ({ page }) => {
  const names = ["리포트 1.html", "report.HTM", "report.xhtml"];
  const fixture = await openCheckout(page, undefined, Object.fromEntries(names.map((name) => [
    name, '<!doctype html><meta charset="utf-8"><title>Report</title><h1>리포트</h1>',
  ])));
  const { repo, sent, lastSent } = fixture;
  try {
    const filesBefore = sent.get("file_open") ?? 0;
    for (const [index, name] of names.entries()) {
      const row = page.locator(`[data-explorer-row="${repo}/${name}"]`);
      if (index === 0) await row.click();
      else if (index === 1) await row.dblclick();
      else {
        await page.locator("[data-explorer-tree]").focus();
        await page.locator('[data-explorer-tree]').press("ArrowDown");
        // Move selection to the named file without opening it, then Enter.
        await expect(page.locator('[data-explorer-tree] [aria-selected="true"]')).toHaveAttribute("data-explorer-row", `${repo}/${name}`);
        await page.locator("[data-explorer-tree]").press("Enter");
      }
      await expect.poll(() => lastSent.get("browser_open")?.url).toBe(new URL(`file://${repo}/${name}`).href);
      await expect(page.locator('[data-browser-display]').last()).toBeVisible();
      expect(sent.get("file_open") ?? 0).toBe(filesBefore);
    }
    const html = page.locator(`[data-explorer-row="${repo}/${names[0]}"]`);
    await html.click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="open-source"]').click();
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("<h1>리포트</h1>");
    expect(lastSent.get("file_open")).toMatchObject({ path: `${repo}/${names[0]}`, preview: false });
    await screenshot(page, "explorer-html-source");
  } finally {
    close(fixture);
  }
});

test("the Explorer lists a checkout, expands a folder and opens a preview tab", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    // The tree lists the root: directories first, hidden names shown, `.git`
    // and the escaping names left out (B1).
    await expect(page.locator("[data-explorer]")).toHaveAttribute("data-explorer", repo);
    await expect(page.locator(`[data-explorer-row="${repo}/.gitignore"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/README.md"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/tracked.txt"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/.git"]`)).toHaveCount(0);
    await screenshot(page, "s3-explorer-root");

    // The core's changed-file set colors the modified row (B1, D-06).
    await expect(page.locator(`[data-explorer-row="${repo}/tracked.txt"]`)).toHaveAttribute("data-decoration", "modified", { timeout: 20_000 });

    // Expanding a folder is one ui_state_update and one file_list; its
    // children appear without a reload (B1, B3).
    expect(lastSent.get("file_list")).toMatchObject({ root: repo, path: `${repo}/src` });
    await expect.poll(() => sent.get("ui_state_update")).toBeGreaterThanOrEqual(2);
    await screenshot(page, "s3-explorer-expanded");

    // A single click opened the checkout's preview tab (B3), and the
    // keyboard went with it out of the tree.
    const fileTab = page.locator('[data-tab-kind="file"]');
    await expect(fileTab).toHaveCount(1);
    await expect(fileTab).toHaveAttribute("data-preview", "true");
    expect(lastSent.get("file_open")).toMatchObject({ path: `${repo}/src/main.ts`, preview: true });
    await expect(page.locator("[data-editor-codemirror] .cm-content")).toBeFocused();
    await screenshot(page, "s3-preview-tab");

    // ⌘⇧K promotes the preview display to an ordinary one (B3; S7 B2).
    await page.keyboard.press(chord("keep_open"));
    await expect.poll(() => sent.get("view_layout.keep_open")).toBe(1);
    await expect(fileTab).toHaveAttribute("data-preview", "false");

    // ⌘W closes the file the click opened: one view_layout close, and the
    // terminal canvas returns.
    await page.keyboard.press(chord("close_tab"));
    await expect.poll(() => sent.get("view_layout.close")).toBe(1);
    await expect(page.locator('[data-tab-kind="file"]')).toHaveCount(0);
    await expect(page.locator("[data-canvas]")).toBeVisible();
  } finally {
    close(fixture);
  }
});

test("the Explorer's root row creates, folds and copies paths, and a long tree keeps blank room to right-click", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(page.url()).origin });
    const rootRow = page.locator("[data-explorer-root-row]");
    const actionIds = ["new-file", "new-folder", "collapse-all"];

    // The creation and fold buttons show when the pointer is on the row.
    for (const id of actionIds) await expect(rootRow.locator(`[data-explorer-root-action="${id}"]`)).toHaveCSS("opacity", "0");
    await rootRow.hover();
    for (const id of actionIds.slice(0, 2)) await expect(rootRow.locator(`[data-explorer-root-action="${id}"]`)).toHaveCSS("opacity", "1");
    await screenshot(page, "s3-explorer-root-hover");

    // The fixture left `src` open, so Collapse All folds it and is then off
    // until a folder is open again.
    const collapse = rootRow.locator('[data-explorer-root-action="collapse-all"]');
    await expect(collapse).toBeEnabled();
    await rootRow.hover();
    await collapse.click();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveCount(0);
    await expect(page.locator(`[data-explorer-row="${repo}/src"]`)).toBeVisible();
    await expect(collapse).toBeDisabled();

    // New File on the root row opens the name field at the top of the root.
    await rootRow.hover();
    await rootRow.locator('[data-explorer-root-action="new-file"]').click();
    await expect(page.locator('[data-explorer-draft="create"] input')).toBeVisible();
    await page.keyboard.press("Escape");

    // A right-click on the root row opens the root's menu.
    await rootRow.click({ button: "right" });
    const menu = page.locator("[data-explorer-menu]");
    await expect(menu).toHaveAttribute("data-explorer-menu", repo);
    // A browser tab has no file manager to reveal in, and no chord for the path copies.
    await expect(menu.locator("[data-menu-item]")).toHaveText(["New File", "New Folder", "Copy Path", "Copy Relative Path"]);
    await expect(menu.locator('[data-menu-item="rename"]')).toHaveCount(0);
    await expect(menu.locator('[data-menu-item="trash"]')).toHaveCount(0);
    await screenshot(page, "s3-explorer-root-menu");
    await menu.locator('[data-menu-item="copy_path"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(repo);

    // A row's menu copies the path whole or below the checkout.
    await page.locator(`[data-explorer-row="${repo}/README.md"]`).click({ button: "right" });
    await expect(menu.locator('[data-menu-item="copy_path"]')).toBeVisible();
    await screenshot(page, "s3-explorer-row-menu");
    await menu.locator('[data-menu-item="copy_relative_path"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("README.md");
    expect(sent.get("path_rename") ?? 0).toBe(0);
    expect(lastSent.get("path_trash")).toBeUndefined();

    // Blank room under the last row is part of the tree, so the root's menu
    // can always be reached; a pointer there is not on a row.
    const tail = page.locator("[data-explorer-tail]");
    const lastRow = page.locator("[data-explorer-row]").last();
    const tailBox = (await tail.boundingBox())!;
    const lastBox = (await lastRow.boundingBox())!;
    expect(tailBox.y + tailBox.height).toBeGreaterThan(lastBox.y + lastBox.height);
    await tail.click({ button: "right", position: { x: 20, y: tailBox.height - 4 } });
    await expect(menu).toHaveAttribute("data-explorer-menu", repo);
  } finally {
    close(fixture);
  }
});

test("the Explorer copies, pastes, duplicates and cuts items inside one checkout", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    const row = (name: string) => page.locator(`[data-explorer-row="${repo}/${name}"]`);
    // Opening a file hands the keyboard to its View, so the tree takes it back as a Tab press from the row would.
    const select = async (name: string, opens = true) => {
      await row(name).click();
      if (opens && !name.endsWith("/src") && !name.endsWith("src")) await expect(page.locator('[data-tab-kind="file"]').filter({ hasText: name.split("/").pop()! })).toBeVisible();
      await page.locator("[data-explorer-tree]").focus();
    };

    // Copy README.md and paste with the tree focused and the file selected: the
    // root is its folder, so the host names the copy `README copy.md`.
    await select("README.md");
    await page.keyboard.press("ControlOrMeta+c");
    await page.keyboard.press("ControlOrMeta+v");
    await expect.poll(() => sent.get("path_copy")).toBe(1);
    expect(lastSent.get("path_copy")).toMatchObject({ root: repo, path: `${repo}/README.md`, destination: repo });
    await expect(row("README copy.md")).toBeVisible();
    expect(fs.readFileSync(`${repo}/README copy.md`, "utf8")).toBe("# repo\n");
    // The copy is selected once it has landed; the clipboard still holds README.md, so the next paste is a second copy of it.
    await expect(row("README copy.md")).toHaveAttribute("data-selected", "true");
    await page.keyboard.press("ControlOrMeta+v");
    await expect(row("README copy 2.md")).toBeVisible();
    expect(fs.existsSync(`${repo}/README copy 2.md`)).toBe(true);
    await screenshot(page, "s3-explorer-pasted-copy");

    // Duplicate in a row's menu is a copy into the item's own folder.
    await row("src/main.ts").click({ button: "right" });
    const menu = page.locator("[data-explorer-menu]");
    await expect(menu.locator('[data-menu-item="paste"]')).toBeVisible();
    await screenshot(page, "s3-explorer-row-menu-clipboard");
    await menu.locator('[data-menu-item="duplicate"]').click();
    await expect(row("src/main copy.ts")).toBeVisible();
    expect(fs.existsSync(`${repo}/src/main copy.ts`)).toBe(true);

    // Cut moves on paste: one path_move, and the clipboard is empty afterwards.
    await select("README copy.md");
    await page.keyboard.press("ControlOrMeta+x");
    await expect(row("README copy.md")).toHaveAttribute("data-cut", "true");
    await select("src");
    await page.keyboard.press("ControlOrMeta+v");
    await expect.poll(() => sent.get("path_move")).toBe(1);
    expect(lastSent.get("path_move")).toMatchObject({ root: repo, path: `${repo}/README copy.md`, destination: `${repo}/src` });
    await expect(row("README copy.md")).toHaveCount(0);
    expect(fs.existsSync(`${repo}/README copy.md`)).toBe(false);
    expect(fs.existsSync(`${repo}/src/README copy.md`)).toBe(true);
    await page.locator("[data-explorer-root-row]").click({ button: "right" });
    await expect(menu.locator('[data-menu-item="paste"]')).toHaveCount(0);
    await page.keyboard.press("Escape");

    // A folder pasted into itself is refused where it was pasted, with nothing sent.
    await select("src");
    await page.keyboard.press("ControlOrMeta+c");
    await page.keyboard.press("ControlOrMeta+v");
    await expect(page.locator(`[data-explorer-failure="${repo}/src"]`)).toBeVisible();
    expect(sent.get("path_copy")).toBe(1 + 1 + 1);
  } finally {
    close(fixture);
  }
});

test("the Explorer takes files dropped from the file manager into a folder, and refuses a drop past a cap whole", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    /** A drag from the file manager: files in a DataTransfer, dropped on the element at `selector`. */
    const dropFiles = (selector: string, files: { name: string; text?: string; bytes?: number }[]) =>
      page.evaluate(
        ([target, list]) => {
          const element = document.querySelector(target);
          if (!element) throw new Error(`drop target ${target} not found`);
          const data = new DataTransfer();
          for (const file of list) data.items.add(new File([file.bytes ? new Uint8Array(file.bytes) : (file.text ?? "")], file.name));
          element.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: data }));
          element.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
          element.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
        },
        [selector, files] as const,
      );
    const folder = `[data-explorer-row="${repo}/src"]`;

    // The folder under the pointer lights up while a file is over it.
    await page.evaluate((target) => {
      const data = new DataTransfer();
      data.items.add(new File(["x"], "over.txt"));
      document.querySelector(target)!.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
    }, folder);
    await expect(page.locator(folder)).toHaveAttribute("data-drop", "true");
    await screenshot(page, "s3-explorer-drop-over");

    await dropFiles(folder, [{ name: "dropped.txt", text: "from the file manager" }]);
    await expect.poll(() => sent.get("path_import")).toBe(1);
    expect(lastSent.get("path_import")).toMatchObject({ root: repo, destination: `${repo}/src` });
    await expect(page.locator(`[data-explorer-row="${repo}/src/dropped.txt"]`)).toBeVisible();
    expect(fs.readFileSync(`${repo}/src/dropped.txt`, "utf8")).toBe("from the file manager");
    await screenshot(page, "s3-explorer-dropped");

    // The same name again is a new item, never a replacement.
    await dropFiles(folder, [{ name: "dropped.txt", text: "second" }]);
    await expect(page.locator(`[data-explorer-row="${repo}/src/dropped copy.txt"]`)).toBeVisible();
    expect(fs.readFileSync(`${repo}/src/dropped.txt`, "utf8")).toBe("from the file manager");

    // The empty room under the rows is the root.
    await dropFiles("[data-explorer-tail]", [{ name: "at-root.txt", text: "root" }]);
    await expect(page.locator(`[data-explorer-row="${repo}/at-root.txt"]`)).toBeVisible();

    // A file past the cap adds nothing and says so under the folder dropped on.
    await dropFiles(folder, [{ name: "ok.txt", text: "ok" }, { name: "big.bin", bytes: 21 * 1024 * 1024 }]);
    await expect(page.locator(`[data-explorer-failure="${repo}/src"]`)).toContainText("20 MiB");
    expect(fs.existsSync(`${repo}/src/ok.txt`)).toBe(false);
    expect(fs.existsSync(`${repo}/src/big.bin`)).toBe(false);
    expect(sent.get("path_import")).toBe(3);
    await screenshot(page, "s3-explorer-drop-refused");
  } finally {
    close(fixture);
  }
});

test("the Explorer filter keeps matches under their folders and Escape restores the tree", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo } = fixture;
  try {
    // Collapse src, so its file is a match inside a folder the tree never listed open.
    await page.locator(`[data-explorer-row="${repo}/src"]`).click();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveCount(0);

    const filter = page.locator("[data-explorer-filter]");
    await filter.fill("main");
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toBeVisible({ timeout: 20_000 });
    await expect(page.locator(`[data-explorer-row="${repo}/src"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/README.md"]`)).toHaveCount(0);
    await screenshot(page, "explorer-filter");

    // Escape brings back the tree as it was: src collapsed, the rest listed.
    await filter.press("Escape");
    await expect(filter).toHaveValue("");
    await expect(page.locator(`[data-explorer-row="${repo}/README.md"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveCount(0);
  } finally {
    close(fixture);
  }
});

// @platform: A save and a change on disk, on the platform's filesystem.
test("editing a document marks it dirty, saves it, and a disk change asks how to resolve", { tag: "@platform" }, async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, sent, lastSent } = fixture;
  try {
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 42;\n");

    // Each keystroke is a file_draft, so wait for the one that carries the
    // whole typed text; the tab shows the core's dirty badge (B4).
    await expect.poll(() => lastSent.get("file_draft")?.contents_utf8).toBe("export const answer = 42;\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();

    // Autosave: no chord, and the draft lands on disk after the idle delay,
    // with the tab clean again (D-10).
    await expect.poll(() => sent.get("file_save"), { timeout: 5_000 }).toBeGreaterThanOrEqual(1);
    expect(lastSent.get("file_save")).toMatchObject({ path: file, contents_utf8: "export const answer = 42;\n" });
    await expect.poll(() => onDisk(file)).toBe("export const answer = 42;\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toHaveCount(0);
    await screenshot(page, "s3-editor-saved");

    // ⌘S still saves at once.
    const savesBeforeChord = sent.get("file_save") ?? 0;
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => sent.get("file_save")).toBe(savesBeforeChord + 1);

    // A change on disk after the open makes the next save a conflict, and the
    // tab offers the two choices (B5).
    fs.writeFileSync(file, "export const answer = 0;\n");
    const ahead = new Date(Date.now() + 5_000);
    fs.utimesSync(file, ahead, ahead);
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 43;\n");
    await page.keyboard.press(chord("save_file"));
    await expect(page.locator("[data-editor-conflict]")).toBeVisible();
    await expect.poll(() => onDisk(file)).toBe("export const answer = 0;\n");
    await screenshot(page, "s3-editor-conflict");

    // Reload takes the disk contents into the editor.
    await page.locator('[data-conflict-action="reload"]').click();
    await expect(page.locator("[data-editor-conflict]")).toHaveCount(0);
    await expect(content).toContainText("export const answer = 0;");
    await expect.poll(() => sent.get("file_conflict")).toBe(1);
    expect(lastSent.get("file_conflict")).toMatchObject({ action: "reload" });
  } finally {
    close(fixture);
  }
});

test("a refused save keeps the tab dirty and takes the saving mark off", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, sent } = fixture;
  try {
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 42;\n");
    await expect.poll(() => sent.get("file_save"), { timeout: 5_000 }).toBeGreaterThanOrEqual(1);
    await expect.poll(() => onDisk(file)).toBe("export const answer = 42;\n");

    // A file the process can no longer write: the next autosave is refused, so
    // the tab stops saying it is saving and stays dirty, with the detail in
    // the diagnostic log (B5, D-10).
    fs.chmodSync(file, 0o444);
    const savesBefore = sent.get("file_save") ?? 0;
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 43;\n");
    await expect.poll(() => sent.get("file_save"), { timeout: 5_000 }).toBeGreaterThan(savesBefore);
    const tab = page.locator('[data-tab-kind="file"]');
    await expect(tab).toHaveAttribute("data-saving", "false", { timeout: 10_000 });
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    await expect.poll(() => onDisk(file)).toBe("export const answer = 42;\n");
  } finally {
    fs.chmodSync(file, 0o644);
    close(fixture);
  }
});

test("a draft that cannot be stored stays editable and holds other documents read-only until it is saved (S5.5 B44)", async ({ page }) => {
  // No IndexedDB behaves as a full draft store: every write is refused.
  const fixture = await openCheckout(page, (page) =>
    page.addInitScript(() => Object.defineProperty(window, "indexedDB", { value: undefined, configurable: true })),
  );
  const { repo, file, sent, lastSent } = fixture;
  try {
    // A refused save keeps the edit unsaved, so its only copy is this tab.
    fs.chmodSync(file, 0o444);
    const content = page.locator("[data-editor-body] .cm-content");
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 44;\n");
    await expect(page.locator('[data-editor-draft-hold="unstored"]')).toBeVisible();
    await expect(page.locator('[data-tab-kind="file"][data-tab-only="true"]')).toHaveCount(1);
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();

    // Export hands the unstored draft to the browser's download.
    const download = page.waitForEvent("download");
    await page.locator('[data-editor-draft-hold="unstored"] [data-export-draft]').click();
    const exported = await download;
    expect(fs.readFileSync(await exported.path(), "utf8")).toBe("export const answer = 44;\n");
    await screenshot(page, "s55-draft-unstored");

    // Another document opens held read-only: typing changes nothing.
    await page.locator(`[data-explorer-row="${repo}/README.md"]`).dblclick();
    await expect(page.locator("[data-editor-body] .cm-content")).toContainText("# repo");
    await expect(page.locator('[data-editor-draft-hold="held"]')).toBeVisible();
    const draftsBefore = sent.get("file_draft") ?? 0;
    await page.locator("[data-editor-body] .cm-content").click();
    await page.keyboard.type("typed while held");
    await expect(page.locator("[data-editor-body] .cm-content")).not.toContainText("typed while held");
    expect(sent.get("file_draft") ?? 0).toBe(draftsBefore);
    await screenshot(page, "s55-draft-held");

    // Saving the unstored draft ends the hold.
    fs.chmodSync(file, 0o644);
    await page.locator('[data-tab-kind="file"][data-tab-only="true"]').click();
    await expect(page.locator('[data-editor-draft-hold="unstored"]')).toBeVisible();
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => onDisk(file), { timeout: 10_000 }).toBe("export const answer = 44;\n");
    await expect(page.locator("[data-editor-draft-hold]")).toHaveCount(0);
    await page.locator('[data-tab-kind="file"]', { hasText: "README.md" }).click();
    await expect(page.locator("[data-editor-draft-hold]")).toHaveCount(0);
    expect(lastSent.get("file_save")).toMatchObject({ path: file });
  } finally {
    fs.chmodSync(file, 0o644);
    close(fixture);
  }
});

test("a deleted open file reports a failed save without losing its draft", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, sent } = fixture;
  try {
    fs.unlinkSync(file);
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 44;\n");
    await expect.poll(() => sent.get("file_save"), { timeout: 5_000 }).toBeGreaterThanOrEqual(1);
    await expect(page.locator('[data-tab-kind="file"]')).toHaveAttribute("data-saving", "false", { timeout: 10_000 });
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    await expect(content).toContainText("export const answer = 44;");
    expect(fs.existsSync(file)).toBe(false);
    // The refusal is shown where the save is, with its reason, Export and
    // Retry, and a retry is one more save that still recreates nothing
    // (S5.5 B15, B45).
    const bar = page.locator('[data-editor-save-state="refused"]');
    await expect(bar).toBeVisible({ timeout: 10_000 });
    await expect(bar).toContainText("Not saved:");
    await expect(bar.locator("[data-export-draft]")).toBeVisible();
    const before = sent.get("file_save") ?? 0;
    await bar.locator("[data-save-retry]").click();
    await expect.poll(() => sent.get("file_save"), { timeout: 5_000 }).toBe(before + 1);
    await expect(bar).toBeVisible({ timeout: 10_000 });
    expect(fs.existsSync(file)).toBe(false);
  } finally {
    close(fixture);
  }
});

test("leaving a dirty tab saves it and never writes it into the next file", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, file } = fixture;
  const readme = path.join(repo, "README.md");
  const readmeBefore = fs.readFileSync(readme, "utf8");
  try {
    const content = page.locator('[data-editor-body] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 99;\n");

    // Leave within the idle window: the draft still reaches its own file, and
    // the tab that takes the screen never receives it (D-10, D-14).
    await page.locator(`[data-explorer-row="${repo}/README.md"]`).click();
    await expect.poll(() => onDisk(file), { timeout: 10_000 }).toBe("export const answer = 99;\n");
    expect(fs.readFileSync(readme, "utf8")).toBe(readmeBefore);
    await expect(page.locator('[data-editor-body] .cm-content')).not.toContainText("answer = 99");
  } finally {
    close(fixture);
  }
});

test("an undone edit is what the next save writes", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, sent } = fixture;
  try {
    const content = page.locator('[data-editor-body] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("X");
    await expect.poll(() => onDisk(file), { timeout: 10_000 }).toBe(`${SOURCE}X`);

    // Undo that keystroke: the core hears it, so the save that follows writes
    // the text the editor shows rather than the text that was undone (B4).
    await page.keyboard.press("ControlOrMeta+KeyZ");
    await expect(content).not.toContainText("X");
    const savesBefore = sent.get("file_save") ?? 0;
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => sent.get("file_save")).toBeGreaterThan(savesBefore);
    await expect.poll(() => onDisk(file)).toBe(SOURCE);
    await expect(page.locator('[data-editor-dirty="true"]')).toHaveCount(0);
  } finally {
    close(fixture);
  }
});

test("a find request does not follow into the next document", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo } = fixture;
  try {
    await page.locator('[data-editor-body] .cm-content').click();
    await page.keyboard.press(chord("find_in_pane"));
    await expect(page.locator(".cm-search")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator(".cm-search")).toHaveCount(0);

    // The next document opens with the document, not with the last find bar.
    await page.locator(`[data-explorer-row="${repo}/README.md"]`).click();
    await expect(page.locator('[data-editor-body] .cm-content')).toContainText("repo");
    await expect(page.locator(".cm-search")).toHaveCount(0);
  } finally {
    close(fixture);
  }
});

test("closing a dirty tab saves the draft instead of dropping it", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, lastSent } = fixture;
  try {
    // One keystroke, then the close: the idle timer cannot have saved it yet,
    // so the draft can only reach the disk through the close itself.
    const content = page.locator('[data-editor-body] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("X");

    const tab = page.locator('[data-tab-kind="file"]');
    await tab.hover();
    await tab.locator('button[aria-label^="Close view"]').click();
    await expect
      .poll(() => (lastSent.get("view_layout.close")?.pending_save as { contents_utf8?: string } | null)?.contents_utf8)
      .toBe(`${SOURCE}X`);
    await expect(page.locator('[data-tab-kind="file"]')).toHaveCount(0);
    await expect.poll(() => onDisk(file), { timeout: 10_000 }).toBe(`${SOURCE}X`);
  } finally {
    close(fixture);
  }
});

test("a closed tab's draft is not written back when the file is reopened", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, file, sent } = fixture;
  try {
    const content = page.locator('[data-editor-body] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 5;\n");
    const tab = page.locator('[data-tab-kind="file"]');
    await tab.hover();
    await tab.locator('button[aria-label^="Close view"]').click();
    await expect(page.locator('[data-tab-kind="file"]')).toHaveCount(0);
    await expect.poll(() => onDisk(file), { timeout: 10_000 }).toBe("export const answer = 5;\n");

    // An agent rewrites the file, the operator reopens it, and saves with
    // nothing typed: the closed tab's draft must not come back with it (B5).
    fs.writeFileSync(file, "export const answer = 0;\n");
    await page.locator(`[data-explorer-row="${repo}/src/main.ts"]`).click();
    await expect(content).toContainText("answer = 0");
    const savesBefore = sent.get("file_save") ?? 0;
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => sent.get("file_save")).toBeGreaterThan(savesBefore);
    await expect.poll(() => onDisk(file)).toBe("export const answer = 0;\n");
  } finally {
    close(fixture);
  }
});

test("a conflicted background tab is not closed away with its draft", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, file, sent } = fixture;
  const readme = path.join(repo, "README.md");
  try {
    // Make the first tab conflicted, then move to a second tab so the
    // conflicted one is no longer the showing document.
    const content = page.locator('[data-editor-body] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 8;\n");
    fs.writeFileSync(file, "export const answer = 0;\n");
    const ahead = new Date(Date.now() + 5_000);
    fs.utimesSync(file, ahead, ahead);
    await page.keyboard.press(chord("save_file"));
    await expect(page.locator("[data-editor-conflict]")).toBeVisible();

    await page.locator(`[data-explorer-row="${repo}/README.md"]`).click();
    await page.locator('[data-editor-body] .cm-content').click();
    await page.keyboard.type("edit");

    // Closing the conflicted tab from the strip must not discard its draft:
    // the close-save is refused, so the tab stays with the conflict showing.
    const conflicted = page.locator(`[data-tab-kind="file"]`, { hasText: "main.ts" });
    await conflicted.hover();
    await conflicted.locator('button[aria-label^="Close view"]').click();
    await expect.poll(() => sent.get("view_layout.close")).toBe(1);
    await expect(page.locator(`[data-tab-kind="file"]`, { hasText: "main.ts" })).toHaveCount(1);
    await expect.poll(() => onDisk(file)).toBe("export const answer = 0;\n");

    // The refused close kept the tab, the draft and the choice to resolve it.
    await page.locator(`[data-tab-kind="file"]`, { hasText: "main.ts" }).click();
    await expect(page.locator("[data-editor-conflict]")).toBeVisible();
    await expect(page.locator('[data-editor-body] .cm-content')).toContainText("answer = 8");
    // The other tab kept its own edit and never received this one.
    expect(fs.readFileSync(readme, "utf8")).not.toContain("answer = 8");
  } finally {
    close(fixture);
  }
});

test("images, PDFs and videos render from hided file bytes", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    // An image decodes from the bytes hided streamed.
    await page.locator(`[data-explorer-row="${repo}/shot.png"]`).click();
    const image = page.locator('[data-viewer="image"] img');
    await expect(image).toBeVisible();
    await expect.poll(async () => image.evaluate((node) => (node as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
    await expect.poll(() => sent.get("file_bytes")).toBe(1);
    await screenshot(page, "s3-viewer-image");

    // A PDF renders one canvas page through pdf.js.
    await page.locator(`[data-explorer-row="${repo}/doc.pdf"]`).click();
    await expect(page.locator('[data-viewer="pdf"]')).toBeVisible();
    await expect(page.locator('[data-pdf-page="1"]')).toBeVisible({ timeout: 20_000 });
    await screenshot(page, "s3-viewer-pdf");

    // A video plays from its bytes, and seeking moves the playhead (B7).
    await page.locator(`[data-explorer-row="${repo}/clip.mp4"]`).click();
    const video = page.locator('[data-viewer="video"] video');
    await expect(video).toBeVisible();
    await expect.poll(async () => video.evaluate((node) => (node as HTMLVideoElement).duration), { timeout: 20_000 }).toBeGreaterThan(0);
    await video.evaluate((node) => {
      (node as HTMLVideoElement).currentTime = 0.5;
    });
    await expect.poll(async () => video.evaluate((node) => (node as HTMLVideoElement).currentTime)).toBeGreaterThan(0.4);
    await screenshot(page, "s3-viewer-video");
  } finally {
    close(fixture);
  }
});

test("an image and a PDF zoom like a browser page: a pinch around the pointer, the text-size chords in Chrome's steps", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    // The image opens at its fit: whole across the viewer.
    await page.locator(`[data-explorer-row="${repo}/wide.png"]`).click();
    const viewer = page.locator('[data-viewer="image"]');
    const image = viewer.locator("img");
    await expect(image).toHaveCSS("visibility", "visible");
    await expect(viewer).toHaveAttribute("data-viewer-zoom", "1");
    const room = (await viewer.boundingBox())!;
    const fit = (await image.boundingBox())!;
    expect(fit.width).toBeLessThan(2400);
    expect(fit.width).toBeLessThanOrEqual(room.width);

    // A pinch arrives as a Ctrl wheel: it grows the image around the
    // pointer, so the point 70% across stays under it.
    const pointer = { x: fit.x + fit.width * 0.7, y: fit.y + fit.height * 0.5 };
    await page.mouse.move(pointer.x, pointer.y);
    await page.keyboard.down("Control");
    for (let notch = 0; notch < 3; notch += 1) await page.mouse.wheel(0, -100);
    await page.keyboard.up("Control");
    const width = async () => (await image.boundingBox())?.width ?? 0;
    await expect.poll(width).toBeGreaterThan(fit.width * 2);
    const zoomed = (await image.boundingBox())!;
    expect((pointer.x - zoomed.x) / zoomed.width).toBeCloseTo(0.7, 2);
    await screenshot(page, "s3-viewer-image-zoomed");

    // With the image holding the keyboard, ⌘0 returns to the fit and ⌘= takes
    // Chrome's next step; neither sizes the editor's text.
    await viewer.click();
    await page.keyboard.press(chord("text_reset"));
    await expect(viewer).toHaveAttribute("data-viewer-zoom", "1");
    await expect.poll(width).toBeCloseTo(fit.width, 0);
    await page.keyboard.press(chord("text_larger"));
    await expect(viewer).toHaveAttribute("data-viewer-zoom", "1.1");
    await expect.poll(width).toBeCloseTo(fit.width * 1.1, 0);
    expect(sent.get("editor_text_scale") ?? 0).toBe(0);

    // A PDF page is drawn again at the scale it is shown at, so it stays sharp.
    await page.locator(`[data-explorer-row="${repo}/doc.pdf"]`).click();
    const pdf = page.locator('[data-viewer="pdf"]');
    const sheet = pdf.locator('[data-pdf-page="1"]');
    const canvas = sheet.locator("canvas");
    await expect(canvas).toBeVisible({ timeout: 20_000 });
    const pixels = () => canvas.evaluate((node) => (node as HTMLCanvasElement).width);
    const shown = (await sheet.boundingBox())!;
    const ratio = await page.evaluate(() => window.devicePixelRatio);
    expect(await pixels()).toBeCloseTo(shown.width * ratio, -1);
    await page.mouse.move(shown.x + shown.width / 2, shown.y + shown.height / 2);
    await page.keyboard.down("Control");
    for (let notch = 0; notch < 3; notch += 1) await page.mouse.wheel(0, -100);
    await page.keyboard.up("Control");
    await expect.poll(async () => (await sheet.boundingBox())?.width ?? 0).toBeGreaterThan(shown.width * 2);
    const grown = (await sheet.boundingBox())!;
    await expect.poll(pixels, { timeout: 10_000 }).toBeCloseTo(grown.width * ratio, -1);
    await screenshot(page, "s3-viewer-pdf-zoomed");

    // The text-size chords still size a text document's text.
    await page.locator(`[data-explorer-row="${repo}/src/main.ts"]`).click();
    const code = page.locator("[data-editor-codemirror] .cm-content");
    await expect(code).toContainText("export const answer = 41;");
    await code.click();
    await page.keyboard.press(chord("text_larger"));
    await expect.poll(() => sent.get("editor_text_scale")).toBe(1);
  } finally {
    close(fixture);
  }
});

test("Markdown opens in Live and toggles to source", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    await page.locator(`[data-explorer-row="${repo}/notes.md"]`).click();
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await expect(page.locator('[data-markdown-mode="live"]')).toBeVisible();
    // The caret opens at the top; move it to the end so the first block is
    // not the touched one and its markup is hidden (B6).
    await content.click();
    await page.keyboard.press("ControlOrMeta+End");

    // Live draws the heading and hides its hashes, draws the list bullets and
    // the task checkbox, styles the link text and the fenced code (B6).
    await expect(page.locator(".cm-md-heading-1")).toHaveCount(1, { timeout: 20_000 });
    await expect(page.locator(".cm-md-heading-1")).toHaveText("Title");
    await expect(page.locator(".cm-md-bullet")).toHaveCount(3);
    await expect(page.locator(".cm-md-checkbox")).toHaveCount(1);
    await expect(page.locator(".cm-md-link")).toHaveText("text");
    await expect(page.locator(".cm-md-code")).toHaveCount(1);
    await expect(page.locator(".cm-md-hidden-line")).toHaveCount(2);
    await screenshot(page, "s3-markdown-live");

    // Live hides markup from the DOM, not from the document: Source mode
    // shows the source again, unchanged.
    await page.locator('[data-markdown-mode="live"]').click();
    await expect.poll(() => sent.get("file_view")).toBe(1);
    await expect(page.locator('[data-markdown-mode="source"]')).toBeVisible();
    await expect(page.locator(".cm-md-heading-1")).toHaveCount(0);
    await expect(page.locator(".cm-md-bullet")).toHaveCount(0);
    await expect(content).toContainText("# Title");
    await expect(content).toContainText("[text](https://example.com)");

    // Back to Live is one more file_view.
    await page.locator('[data-markdown-mode="source"]').click();
    await expect.poll(() => sent.get("file_view")).toBe(2);
    await expect(page.locator(".cm-md-heading-1")).toHaveCount(1);
  } finally {
    close(fixture);
  }
});

test("Markdown Live draws frontmatter in its own scrolling pane", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, lastSent } = fixture;
  try {
    await page.locator(`[data-explorer-row="${repo}/post.md"]`).click();
    await expect(page.locator('[data-markdown-mode="live"]')).toBeVisible();
    const pane = page.locator("[data-editor-frontmatter]");
    const body = page.locator("[data-editor-body] .cm-content");
    await expect(pane.locator(".cm-content")).toContainText("title: Post");
    // The block left the body: it is drawn once, in the pane (D-11).
    await expect(body).not.toContainText("title: Post");
    // Live hides the heading's hash when the caret is elsewhere, so the body
    // reads as its text.
    await expect(body).toContainText("Body");

    // The pane is about eight lines tall and scrolls inside itself, so the
    // body below it does not move with the block (D-11).
    const measured = await pane.evaluate((element) => ({
      client: element.clientHeight,
      scroll: element.querySelector(".cm-scroller")?.scrollHeight ?? 0,
    }));
    const lineHeight = await pane.evaluate(() => Number.parseFloat(getComputedStyle(document.querySelector("[data-editor-frontmatter] .cm-content")!).fontSize) * 1.5);
    expect(measured.scroll).toBeGreaterThan(measured.client);
    expect(measured.client).toBeGreaterThan(lineHeight * 6);
    expect(measured.client).toBeLessThan(lineHeight * 9);

    // Editing the pane edits the document: the draft carries the whole file,
    // block first (D-11, B4).
    await pane.locator(".cm-content").click();
    await page.keyboard.type("x");
    await expect.poll(() => lastSent.get("file_draft")?.contents_utf8 ?? "").toContain("x");
    const draft = String(lastSent.get("file_draft")?.contents_utf8 ?? "");
    expect(draft.startsWith("---\n")).toBe(true);
    expect(draft).toContain("# Body");

    // Source mode is unchanged: one buffer, the block inline (D-11).
    await page.locator('[data-markdown-mode="live"]').click();
    await expect(page.locator("[data-editor-frontmatter]")).toHaveCount(0);
    await expect(page.locator("[data-editor-codemirror] .cm-content")).toContainText("title: Post");

    // And Live brings the pane back.
    await page.locator('[data-markdown-mode="source"]').click();
    await expect(page.locator("[data-editor-frontmatter]")).toBeVisible();
    await expect(page.locator("[data-editor-body] .cm-content")).not.toContainText("title: Post");
  } finally {
    close(fixture);
  }
});

/** A drag of HTML5 `draggable` rows, which Playwright's mouse drag does not
 * drive: the source's dragstart carries a DataTransfer to the folder's drop. */
async function dragRow(page: Page, from: string, to: string): Promise<void> {
  await page.evaluate(
    ([fromPath, toPath]) => {
      const source = document.querySelector(`[data-explorer-row="${fromPath}"]`);
      const target = document.querySelector(`[data-explorer-row="${toPath}"]`);
      if (!source || !target) throw new Error(`drag ${fromPath} -> ${toPath}: rows not found`);
      const data = new DataTransfer();
      source.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: data }));
      target.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
      target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
      source.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: data }));
    },
    [from, to],
  );
}

// @platform: Trash is a system call on macOS.
test("the Explorer creates, renames, moves and trashes entries", { tag: "@platform" }, async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    // New File in `src`: one file_create, and the core opens the created file.
    await page.locator(`[data-explorer-row="${repo}/src"]`).click({ button: "right" });
    await expect(page.locator("[data-explorer-menu]")).toBeVisible();
    await page.locator('[data-menu-item="new-file"]').click();
    const create = page.locator('[data-explorer-draft="create"] input');
    await expect(create).toBeVisible();
    await create.fill("added.ts");
    await create.press("Enter");
    await expect.poll(() => sent.get("file_create")).toBe(1);
    expect(lastSent.get("file_create")).toMatchObject({ root: repo, parent: `${repo}/src`, name: "added.ts" });
    await expect(page.locator(`[data-explorer-row="${repo}/src/added.ts"]`)).toBeVisible();
    // The created file opened as a tab beside the one already open.
    await expect(page.locator('[data-tab-kind="file"]').filter({ hasText: "added.ts" })).toHaveCount(1);
    await screenshot(page, "s3-explorer-created");

    // A name that already exists fails and says so under the row it started
    // from; nothing is created.
    await page.locator(`[data-explorer-row="${repo}/src"]`).click({ button: "right" });
    await page.locator('[data-menu-item="new-file"]').click();
    const clash = page.locator('[data-explorer-draft="create"] input');
    await clash.fill("main.ts");
    await clash.press("Enter");
    await expect(page.locator(`[data-explorer-failure="${repo}/src/main.ts"]`)).toBeVisible({ timeout: 20_000 });
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveCount(1);
    await screenshot(page, "s3-explorer-failure");
    await expect(clash).toHaveValue("main.ts");
    await clash.fill("corrected.ts");
    await clash.press("Enter");
    await expect(page.locator(`[data-explorer-row="${repo}/src/corrected.ts"]`)).toBeVisible();

    // Rename: one path_rename, and the row takes the new name; the created
    // file's stored draft follows it to the new path.
    const addedPath = `${repo}/src/added.ts`;
    await seedDraft(page, {
      id: draftId(fixture.daemon.hostId, repo, addedPath, fixture.daemon.node), host: fixture.daemon.hostId, device: fixture.daemon.node,
      root: repo, path: addedPath, contents: "unsaved recovery copy", updated_at: Date.now(),
    });
    await page.locator(`[data-explorer-row="${repo}/src/added.ts"]`).click({ button: "right" });
    await page.locator('[data-menu-item="rename"]').click();
    const rename = page.locator('[data-explorer-draft="rename"] input');
    // The closing menu hands focus back a tick later; the field it opened
    // keeps the keyboard and nothing is sent until the operator commits.
    await expect(rename).toBeFocused();
    await quietFor(page, 250, "nothing is sent until the operator commits the rename");
    await expect(rename).toBeFocused();
    expect(sent.get("path_rename") ?? 0).toBe(0);
    await rename.fill("renamed.ts");
    await rename.press("Enter");
    await expect.poll(() => sent.get("path_rename")).toBe(1);
    expect(lastSent.get("path_rename")).toMatchObject({ root: repo, path: `${repo}/src/added.ts`, name: "renamed.ts" });
    await expect(page.locator(`[data-explorer-row="${repo}/src/renamed.ts"]`)).toBeVisible();
    await expect.poll(async () => (await storedDrafts(page)).some((row) => row.path === addedPath)).toBe(false);

    // New Folder in the root, then drag the file onto it: one path_move.
    await page.locator("[data-explorer-tree]").click({ button: "right", position: { x: 20, y: 400 } });
    await page.locator('[data-menu-item="new-folder"]').click();
    const folder = page.locator('[data-explorer-draft="create"] input');
    await folder.fill("dest");
    await folder.press("Enter");
    await expect.poll(() => sent.get("dir_create")).toBe(1);
    await expect(page.locator(`[data-explorer-row="${repo}/dest"]`)).toBeVisible();
    await dragRow(page, `${repo}/src/renamed.ts`, `${repo}/dest`);
    await expect.poll(() => sent.get("path_move")).toBe(1);
    expect(lastSent.get("path_move")).toMatchObject({ root: repo, path: `${repo}/src/renamed.ts`, destination: `${repo}/dest` });

    // Trash behind the confirmation: nothing goes out until it is confirmed.
    const destInode = fs.statSync(`${repo}/dest`, { bigint: true }).ino.toString();
    await page.locator(`[data-explorer-row="${repo}/dest"]`).click({ button: "right" });
    await page.locator('[data-menu-item="trash"]').click();
    const dialog = page.locator("[data-confirm-trash]");
    await expect(dialog).toBeVisible();
    expect(sent.get("path_trash") ?? 0).toBe(0);
    await screenshot(page, "s3-explorer-trash");
    await page.locator("[data-trash-confirm]").click();
    await expect.poll(() => sent.get("path_trash")).toBe(1);
    // The tree selects the removed folder's next sibling, which is `src`.
    // It names the folder the operator confirmed by its inode, so the host
    // refuses a folder that replaced it while the prompt was open (S5.5 B17).
    expect(lastSent.get("path_trash")).toMatchObject({ root: repo, path: `${repo}/dest`, select_after: `${repo}/src`, inode: destInode });
    await expect(page.locator(`[data-explorer-row="${repo}/dest"]`)).toHaveCount(0);
  } finally {
    close(fixture);
  }
});

test("a loopback browser downloads a file past the editing cap", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    await page.locator(`[data-explorer-row="${repo}/huge.txt"]`).click();
    const body = page.locator("[data-editor-preview-only]");
    await expect(body).toBeVisible();
    await expect(body).toContainText("preview-only");
    const download = page.locator("[data-editor-download]");
    await expect(download).toHaveText("Download");
    await screenshot(page, "s3-preview-only");

    // The URL is 127.0.0.1 in this fixture, just as an SSH tunnel can be.
    // Fake only the browser's file picker; the real daemon supplies all bytes.
    await page.evaluate(() => {
      const browser = window as Window & { downloadedBytes?: number; downloadClosed?: boolean };
      browser.downloadedBytes = 0;
      browser.downloadClosed = false;
      Object.defineProperty(window, "showSaveFilePicker", {
        configurable: true,
        value: async () => ({
          createWritable: async () => ({
            write: async (bytes: Uint8Array) => { browser.downloadedBytes = (browser.downloadedBytes ?? 0) + bytes.byteLength; },
            close: async () => { browser.downloadClosed = true; },
            abort: async () => {},
          }),
        }),
      });
    });
    await download.click();
    await expect.poll(() => page.evaluate(() => (window as Window & { downloadClosed?: boolean }).downloadClosed)).toBe(true);
    expect(await page.evaluate(() => (window as Window & { downloadedBytes?: number }).downloadedBytes)).toBe(17 * 1024 * 1024);
    expect(sent.get("open_external") ?? 0).toBe(0);
    expect(sent.get("file_bytes") ?? 0).toBeGreaterThan(1);
    await expect(page.locator("[data-editor-download-failed]")).toHaveCount(0);
  } finally {
    close(fixture);
  }
});

test("a browser without a save picker explains the 256 MiB download limit", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    fs.truncateSync(path.join(repo, "huge.txt"), 257 * 1024 * 1024);
    await page.locator(`[data-explorer-row="${repo}/huge.txt"]`).click();
    await expect(page.locator("[data-editor-preview-only]")).toBeVisible();
    await page.evaluate(() => {
      Object.defineProperty(window, "showSaveFilePicker", { configurable: true, value: undefined });
    });
    await page.locator("[data-editor-download]").click();
    await expect(page.locator("[data-editor-download-failed]")).toContainText(
      "Use a browser with a file save picker for files over 256 MiB",
    );
    const evidenceDir = process.env.HIDE_E2E_SCREENSHOT_DIR;
    if (evidenceDir) {
      await page.locator("[data-editor-preview-only]").screenshot({ path: path.join(evidenceDir, "s3-no-picker-limit.png") });
    }
    expect(sent.get("file_bytes")).toBe(1);
    expect(sent.get("open_external") ?? 0).toBe(0);
  } finally {
    close(fixture);
  }
});

test("a preview-only document closes without a save", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent, lastSent } = fixture;
  try {
    // A stale recovery buffer is exactly the trap: a preview-only document has
    // no draft the core would accept, so the restore must decline it rather
    // than hand it back as a close-save the core refuses (D-14).
    const stale = draftId(fixture.daemon.hostId, repo, `${repo}/huge.txt`, fixture.daemon.node);
    await seedDraft(page, {
      id: stale, host: fixture.daemon.hostId, device: fixture.daemon.node, root: repo, path: `${repo}/huge.txt`,
      contents: "stale draft\n", updated_at: Date.now(),
    });
    await page.locator(`[data-explorer-row="${repo}/huge.txt"]`).click();
    await expect(page.locator("[data-editor-preview-only]")).toBeVisible();
    // The stale buffer was declined rather than planted as a draft.
    await expect.poll(() => sent.get("file_draft") ?? 0, { timeout: 2_000 }).toBe(0);

    // No draft the core would accept exists here, so the close is one step.
    await page.locator('[data-tab-kind="file"]').hover();
    await page.locator('[data-tab-kind="file"] button[aria-label^="Close view"]').click();
    await expect(page.locator('[data-tab-kind="file"]')).toHaveCount(0);
    await expect.poll(() => sent.get("view_layout.close")).toBe(1);
    expect(lastSent.get("view_layout.close")?.pending_save ?? null).toBeNull();
  } finally {
    close(fixture);
  }
});

// @platform: File watching is FSEvents on macOS and inotify on Linux.
test("a change in an expanded folder refreshes the tree without a reload", { tag: "@platform" }, async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo } = fixture;
  try {
    // `src` is expanded and watched; a file written into it appears on its own.
    // The watcher arms a moment after the ui state reaches the daemon, so a
    // missed first write is retried with a fresh name rather than assumed.
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toBeVisible();
    let created = "";
    await expect(async () => {
      created = `watched-${Date.now()}.ts`;
      fs.writeFileSync(path.join(repo, "src", created), "export const watched = true;\n");
      await expect(page.locator(`[data-explorer-row="${repo}/src/${created}"]`)).toBeVisible({ timeout: 1_500 });
    }).toPass({ timeout: 30_000 });
    await screenshot(page, "s3-watch-refresh");

    // Deleting it disappears the same way.
    fs.rmSync(path.join(repo, "src", created));
    await expect(page.locator(`[data-explorer-row="${repo}/src/${created}"]`)).toHaveCount(0, { timeout: 15_000 });
  } finally {
    close(fixture);
  }
});

// @platform: ⌘ chords, the command key of the system hide ships on.
test("⌘P opens a file by name and ⌘K switches checkout", { tag: "@platform" }, async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo, sent } = fixture;
  try {
    // Collapse the folder first: the palette open must reveal the document's
    // row again through the core's expanded set (B3).
    await page.locator(`[data-explorer-row="${repo}/src"]`).click();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveCount(0);
    await page.keyboard.press(chord("toggle_lane"));
    await expect(page.locator("[data-workspace-screen]")).toHaveAttribute("data-lane-mode", "closed");

    // ⌘P: hided indexes the checkout, ranks the typed name and opens it in the
    // preview tab (B12). The open brings the lane back with the Explorer the
    // close left in its column, and it shows the row revealed (PRD
    // lane-two-columns B7, B15).
    await page.keyboard.press(chord("open_file"));
    const input = page.locator('[data-palette="Open file"] [data-palette-input]');
    await expect(input).toBeVisible();
    await input.fill("main");
    const row = page.locator(`[data-palette-row="${repo}/src/main.ts"]`);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await screenshot(page, "s3-palette-files");
    await row.click();
    await expect(page.locator("[data-palette-input]")).toHaveCount(0);
    await expect(page.locator('[data-tab-kind="file"]').filter({ hasText: "main.ts" })).toHaveCount(1);
    await expect(page.locator('[data-lane="explorer"] [data-explorer]')).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${repo}/src/main.ts"]`)).toHaveAttribute("data-selected", "true", { timeout: 10_000 });
    expect(sent.get("file_index") ?? 0).toBeGreaterThanOrEqual(1);

    // ⌘K: the snapshot's projects and checkouts are searched here, and picking
    // a checkout focuses its Workspace (B13; a Project opens its Overview, S6 B1).
    const focusBefore = sent.get("focus_checkout") ?? 0;
    await page.keyboard.press(chord("search"));
    const search = page.locator('[data-palette="Search"] [data-palette-input]');
    await expect(search).toBeVisible();
    await search.fill("fixture");
    const projectRow = page.locator('[data-palette-group="checkouts"] [data-palette-row]').filter({ hasText: "fixture" }).first();
    await expect(projectRow).toBeVisible();
    await screenshot(page, "s3-palette-search");
    await projectRow.click();
    await expect(page.locator("[data-palette-input]")).toHaveCount(0);
    await expect.poll(() => sent.get("focus_checkout")).toBeGreaterThan(focusBefore);
  } finally {
    close(fixture);
  }
});

test("Explorer files paste paths and images attach to the receiving terminal", { tag: "@platform" }, async ({ page }) => {
  const fixture = await openCheckout(page);
  const { herdr, sent } = fixture;
  try {
    const folder = path.join(herdr.root, "fixture");
    const name = "한글 ' $HOME 파일.txt";
    const file = path.join(folder, name);
    fs.writeFileSync(file, "original file");
    fs.writeFileSync(path.join(folder, "drag-image.png"), PNG);
    const checkout = page.locator("[data-project]", { hasText: "fixture" }).locator("[data-checkout]").first();
    await checkout.click();
    await expect(checkout).toHaveAttribute("aria-current", "true");
    await showExplorer(page);
    const source = toPage(fs.realpathSync(file));
    const image = toPage(fs.realpathSync(path.join(folder, "drag-image.png")));
    await expect(page.locator(`[data-explorer-row="${source}"]`)).toBeVisible();
    await expect(page.locator(`[data-explorer-row="${image}"]`)).toBeVisible();
    const [target] = herdr.panes;
    await expect(page.locator(`[data-pane-view="${target}"]`)).toBeVisible();
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(page.url()).origin });
    const [log] = herdr.inputLogs;
    const read = () => fs.existsSync(log) ? fs.readFileSync(log, "utf8") : "";
    const before = read().length;
    const dragToTerminal = (from: string) => page.evaluate(({ from, target }) => {
      const row = document.querySelector(`[data-explorer-row="${from}"]`)!;
      const pane = document.querySelector(`[data-pane-view="${target}"]`)!;
      const data = new DataTransfer();
      row.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: data }));
      const over = new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data });
      pane.dispatchEvent(over);
      if (!over.defaultPrevented) throw new Error("terminal refused the Explorer drag");
      pane.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
      row.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: data }));
    }, { from, target });
    await dragToTerminal(source);
    await expect.poll(read).toContain(name.replace("$", "\\$"));
    expect(read().slice(before)).not.toMatch(/[\r\n]/);
    expect(sent.get("attachment_commit") ?? 0).toBe(0);
    expect(fs.readFileSync(file, "utf8")).toBe("original file");
    await dragToTerminal(image);
    await expect.poll(read).toContain("drag-image.png");
    await expect.poll(() => sent.get("attachment_commit")).toBe(1);
    expect(sent.get("path_move") ?? 0).toBe(0);
    await expect(page.locator(`[data-pane-view="${target}"]`)).toHaveAttribute("data-focused", "true");
    await expect(page.locator("[data-pane-attachment-refusal]")).toHaveCount(0);
    expect(fs.readFileSync(path.join(folder, "drag-image.png"))).toEqual(PNG);
    // Right-click Copy writes a public path; terminal Paste preserves file intent.
    await page.locator(`[data-explorer-row="${source}"]`).click({ button: "right" });
    await page.locator('[data-explorer-menu] [data-menu-item="copy"]').click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(source);
    // Terminal Paste is ⌘V on macOS, Ctrl+Shift+V on Windows and Linux
    // (UI_BEHAVIOR.md, Keyboard): Ctrl+V remains input to the program there.
    await page.locator(`[data-pane-view="${target}"] .xterm-helper-textarea`).focus();
    await page.keyboard.press(mod("KeyV"));
    await expect.poll(() => read().split(name.replace("$", "\\$")).length - 1).toBe(2);
    // Keyboard Copy followed by native Paste uses the same image attachment route.
    await page.locator("[data-explorer-tree]").focus();
    // Selection starts empty; the image sorts before the Korean text file.
    await page.keyboard.press("ArrowDown");
    await expect(page.locator(`[data-explorer-row="${image}"]`)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ControlOrMeta+c");
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(image);
    await page.locator(`[data-pane-view="${target}"] .xterm-helper-textarea`).focus();
    await page.keyboard.press(mod("KeyV"));
    await expect.poll(() => read().split("drag-image.png").length - 1).toBe(2);
    await screenshot(page, "explorer-terminal-drop");
  } finally {
    close(fixture);
  }
});

test("a dropped file reaches the terminal as an attachment", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { herdr, sent } = fixture;
  try {
    // The fixture checkout's panes run the shim that logs every PTY byte, so
    // the pasted token is observable there; openCheckout focused the repository.
    // The repository's pane stays on screen until the core answers the checkout
    // click, so the drop waits for the fixture's own first pane, by its id.
    const project = page.locator("[data-project]", { hasText: "fixture" });
    await project.locator("[data-checkout]").first().click();
    const [target] = herdr.panes;
    await expect(page.locator(`[data-pane-view="${target}"]`)).toBeVisible();

    // A file drop is bytes the browser can read but cannot name; the shell
    // uploads them, and the core pastes the staged path into the PTY (B14).
    await page.evaluate(async ({ base64, pane }) => {
      const binary = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const file = new File([binary], "dropped.png", { type: "image/png" });
      const data = new DataTransfer();
      data.items.add(file);
      const view = document.querySelector(`[data-pane-view="${pane}"]`) as HTMLElement;
      view.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
      view.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    }, { base64: PNG.toString("base64"), pane: target });

    await expect.poll(() => sent.get("attachment_stage")).toBe(1);
    await expect.poll(() => sent.get("attachment_commit")).toBe(1);

    // The shim logs every byte its PTY received, so the pasted token shows up
    // in the log of the pane it was dropped on (`inputLogs` follows `panes`).
    const [log] = herdr.inputLogs;
    const read = () => (fs.existsSync(log) ? fs.readFileSync(log, "utf8") : "");
    // The drop's paste reaches the shim's PTY only through the core: if it never shows, what the shim
    // log holds, what the pane runs and what the page sent say which hop dropped it.
    await dumpOnFailure(
      "s3 dropped file",
      () => ({
        log: { exists: fs.existsSync(log), bytes: read().length, tail: read().slice(-200) },
        sent: Object.fromEntries([...sent]),
        process: herdr.run(["pane", "process-info", "--pane", target]),
        screen: herdr.run(["pane", "read", target, "--source", "recent-unwrapped", "--lines", "20"]),
      }),
      () => expect.poll(read, { timeout: 15_000 }).toContain("dropped.png"),
    );

    // ⌘V of an image: the same flow, with the image staged at the path the core
    // reads a clipboard attachment from (B14).
    await page.evaluate(({ base64, pane }) => {
      const binary = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const file = new File([binary], "clipboard.png", { type: "image/png" });
      const data = new DataTransfer();
      data.items.add(file);
      const host = document.querySelector(`[data-pane-view="${pane}"] [data-terminal-host]`) as HTMLElement;
      host.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data }));
    }, { base64: PNG.toString("base64"), pane: target });
    await expect.poll(read, { timeout: 15_000 }).toContain("TerminalClipboard");
    await expect(page.locator("[data-pane-attachment-refusal]")).toHaveCount(0);
    await screenshot(page, "s3-attachment-drop");
  } finally {
    close(fixture);
  }
});

test("an unsaved edit survives a socket drop and reconnect", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo } = fixture;
  try {
    await page.locator(`[data-explorer-row="${repo}/notes.md"]`).click();
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await expect(content).toContainText("# Title");
    await content.click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("edited across a reconnect\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();

    // A server-side drop makes the shell reconnect on its own; the core keeps
    // the draft, and the buffer reconciles against it (B8). A live connection
    // draws no badge at all.
    await page.evaluate(() => window.__hideProbe?.dropSocket());
    await page.context().setOffline(true);
    await expect(page.locator("[data-connection]")).toHaveText(/reconnecting/, { timeout: 15_000 });
    // Keep the socket down past the 600 ms autosave window: no save reaches the
    // file meanwhile. A dropped save must be retried after reconnection, then
    // reach the actual file (D-10).
    await quietFor(page, 800, "no save reaches the file while the socket is down, past the 600 ms autosave window");
    expect(fs.readFileSync(path.join(repo, "notes.md"), "utf8")).not.toContain("edited across a reconnect");
    await page.context().setOffline(false);
    await expect(page.locator("[data-connection]")).toHaveCount(0, { timeout: 20_000 });
    await expect(content).toContainText("edited across a reconnect");
    await expect.poll(() => onDisk(path.join(repo, "notes.md")), { timeout: 20_000 }).toContain("edited across a reconnect");
    await expect(page.locator('[data-editor-dirty="true"]')).toHaveCount(0);
    await screenshot(page, "s3-buffer-reconnect");
  } finally {
    await page.context().setOffline(false);
    close(fixture);
  }
});

test("an existing v1 recovery draft survives the IndexedDB upgrade", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file } = fixture;
  try {
    const app = await leaveApp(page);
    await page.evaluate(async ({ path }) => {
      await new Promise<void>((resolve, reject) => {
        const removed = indexedDB.deleteDatabase("hide-shell");
        removed.onsuccess = () => resolve();
        removed.onerror = () => reject(removed.error);
      });
      await new Promise<void>((resolve, reject) => {
        const opened = indexedDB.open("hide-shell", 1);
        opened.onupgradeneeded = () => opened.result.createObjectStore("buffers", { keyPath: "path" });
        opened.onerror = () => reject(opened.error);
        opened.onsuccess = () => {
          const database = opened.result;
          const transaction = database.transaction("buffers", "readwrite");
          transaction.objectStore("buffers").put({ path, contents: "export const answer = 99;\n", updated_at: Date.now() });
          transaction.oncomplete = () => { database.close(); resolve(); };
          transaction.onerror = () => { database.close(); reject(transaction.error); };
        };
      });
    }, { path: file });
    await page.goto(app);
    await enterWorkspace(page, "repo");
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 99");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 99;\n");
  } finally {
    close(fixture);
  }
});

test("a root-keyed v2 recovery draft survives the IndexedDB upgrade", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  try {
    const app = await leaveApp(page);
    await page.evaluate(async ({ path, root }) => {
      await new Promise<void>((resolve, reject) => {
        const removed = indexedDB.deleteDatabase("hide-shell");
        removed.onsuccess = () => resolve();
        removed.onerror = () => reject(removed.error);
      });
      await new Promise<void>((resolve, reject) => {
        const opened = indexedDB.open("hide-shell", 2);
        opened.onupgradeneeded = () => opened.result.createObjectStore("buffers", { keyPath: "id" });
        opened.onerror = () => reject(opened.error);
        opened.onsuccess = () => {
          const database = opened.result;
          const transaction = database.transaction("buffers", "readwrite");
          transaction.objectStore("buffers").put({ id: `${root}\u0000${path}`, root, path, contents: "export const answer = 98;\n", updated_at: Date.now() });
          transaction.oncomplete = () => { database.close(); resolve(); };
          transaction.onerror = () => { database.close(); reject(transaction.error); };
        };
      });
    }, { path: file, root: repo });
    await page.goto(app);
    await enterWorkspace(page, "repo");
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 98");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 98;\n");
  } finally {
    close(fixture);
  }
});

test("of two legacy drafts for one file the newer is restored, and the older is kept for recovery (S5.5 B12)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  try {
    const app = await leaveApp(page);
    await page.evaluate(async ({ path, root }) => {
      await new Promise<void>((resolve, reject) => {
        const removed = indexedDB.deleteDatabase("hide-shell");
        removed.onsuccess = () => resolve();
        removed.onerror = () => reject(removed.error);
      });
      await new Promise<void>((resolve, reject) => {
        const opened = indexedDB.open("hide-shell", 2);
        opened.onupgradeneeded = () => opened.result.createObjectStore("buffers", { keyPath: "id" });
        opened.onerror = () => reject(opened.error);
        opened.onsuccess = () => {
          const database = opened.result;
          const transaction = database.transaction("buffers", "readwrite");
          const store = transaction.objectStore("buffers");
          const now = Date.now();
          store.put({ id: `${root}\u0000${path}`, root, path, contents: "export const answer = 96;\n", updated_at: now });
          store.put({ id: `\u0000${path}`, root: "", path, contents: "export const answer = 97;\n", updated_at: now - 60_000 });
          transaction.oncomplete = () => { database.close(); resolve(); };
          transaction.onerror = () => { database.close(); reject(transaction.error); };
        };
      });
    }, { path: file, root: repo });
    await page.goto(app);
    await enterWorkspace(page, "repo");
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 96");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 96;\n");
    const kept = await page.evaluate(() => new Promise<string[]>((resolve, reject) => {
      const opened = indexedDB.open("hide-shell");
      opened.onerror = () => reject(opened.error);
      opened.onsuccess = () => {
        const database = opened.result;
        const all = database.transaction("drafts_v4").objectStore("drafts_v4").getAll();
        all.onsuccess = () => { database.close(); resolve((all.result as Array<{ contents: string }>).map((row) => row.contents)); };
        all.onerror = () => { database.close(); reject(all.error); };
      };
    }));
    expect(kept).toContain("export const answer = 97;\n");
  } finally {
    close(fixture);
  }
});

test("one draft id in both legacy stores migrates as its newer row (S5.5 B12)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  try {
    const app = await leaveApp(page);
    await page.evaluate(async ({ path, root }) => {
      await new Promise<void>((resolve, reject) => {
        const removed = indexedDB.deleteDatabase("hide-shell");
        removed.onsuccess = () => resolve();
        removed.onerror = () => reject(removed.error);
      });
      await new Promise<void>((resolve, reject) => {
        const opened = indexedDB.open("hide-shell", 3);
        opened.onupgradeneeded = () => {
          opened.result.createObjectStore("buffers", { keyPath: "id" });
          opened.result.createObjectStore("buffers_v2", { keyPath: "id" });
        };
        opened.onerror = () => reject(opened.error);
        opened.onsuccess = () => {
          const database = opened.result;
          const transaction = database.transaction(["buffers", "buffers_v2"], "readwrite");
          const now = Date.now();
          // The store read first holds the newer row, so a row-by-row merge
          // would let the older one from the second store overwrite it.
          transaction.objectStore("buffers").put({ id: `${root}\u0000${path}`, root, path, contents: "export const answer = 96;\n", updated_at: now });
          transaction.objectStore("buffers_v2").put({ id: `${root}\u0000${path}`, root, path, contents: "export const answer = 97;\n", updated_at: now - 60_000 });
          transaction.oncomplete = () => { database.close(); resolve(); };
          transaction.onerror = () => { database.close(); reject(transaction.error); };
        };
      });
    }, { path: file, root: repo });
    await page.goto(app);
    await enterWorkspace(page, "repo");
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 96");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 96;\n");
  } finally {
    close(fixture);
  }
});

test("an old draft is restored, never discarded for its age (S5.5 B12)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  try {
    await seedDraft(page, {
      id: draftId(fixture.daemon.hostId, repo, file, fixture.daemon.node), host: fixture.daemon.hostId, device: fixture.daemon.node,
      root: repo, path: file, contents: "export const answer = 97;\n", updated_at: Date.now() - 400 * 24 * 60 * 60 * 1000,
    });
    await page.reload();
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 97");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 97;\n");
  } finally {
    close(fixture);
  }
});

test("a draft this host stored for its own machine under `local` moves to the node id and is restored (core-host-node D-21)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  const host = fixture.daemon.hostId;
  try {
    await seedDraft(page, {
      id: draftId(host, repo, file, "local"), host, device: "local",
      root: repo, path: file, contents: "export const answer = 98;\n", updated_at: Date.now(),
    });
    await page.reload();
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 98");
    const stored = await storedDrafts(page);
    expect(stored.some((row) => row.device === "local")).toBe(false);
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 98;\n");
  } finally {
    close(fixture);
  }
});

test("a draft whose tab a daemon restart lost is kept for recovery and opens where it belongs (S5.5 B10)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo, sent } = fixture;
  const original = fs.readFileSync(file, "utf8");
  try {
    // Keep the edit from being saved: the file cannot be written, so the
    // draft exists only in the core's memory and in the browser.
    fs.chmodSync(file, 0o444);
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 77;\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    await expect.poll(async () => (await storedDrafts(page)).find((row) => row.path === file)?.contents, { timeout: 5_000 }).toBe("export const answer = 77;\n");
    await expect.poll(() => sent.get("file_save") ?? 0, { timeout: 5_000 }).toBeGreaterThanOrEqual(1);

    // The daemon restarts on the same port and state: its core forgot the
    // tab and the draft, the host kept its id. Without its Workspace views
    // file the tab is not restored, as after a build that kept none.
    const hostBefore = fixture.daemon.hostId;
    const restarted = await fixture.daemon.restart((state) => fs.rmSync(path.join(state, "workspace-views.json"), { force: true }));
    fixture.daemon = restarted;
    expect(restarted.hostId).toBe(hostBefore);
    // The page under test survives the restart and reconnects: it must not be reloaded.
    // eslint-disable-next-line hide-e2e/reopen-after-restart-through-blank -- the reconnect of the same page is the subject
    await page.goto(`${restarted.origin}/?probe=1#token=${restarted.token}`);
    const line = page.locator("[data-draft-recovery]");
    await expect(line).toBeVisible({ timeout: 20_000 });
    expect((await storedDrafts(page)).find((row) => row.path === file)?.contents).toBe("export const answer = 77;\n");
    await screenshot(page, "s55-draft-recovery-line");

    // Nothing reached the disk, and a file that can be written again is where
    // the draft goes: Review offers its checkout, Open restores it into the
    // editor on its own document, and only the save that lands takes it out
    // of the store.
    expect(fs.readFileSync(file, "utf8")).toBe(original);
    fs.chmodSync(file, 0o644);
    await page.locator("[data-draft-recovery-review]").click();
    const id = encodeURIComponent(draftId(restarted.hostId, repo, file, restarted.node));
    const showCheckout = page.locator(`[data-draft-show-checkout="${id}"]`);
    if (await showCheckout.count()) await showCheckout.click();
    await expect(page.locator(`[data-draft-open="${id}"]`)).toBeVisible({ timeout: 10_000 });
    await screenshot(page, "s55-draft-recovery-sheet");
    await page.locator(`[data-draft-open="${id}"]`).click();
    await expect(content).toContainText("answer = 77", { timeout: 20_000 });
    await expect(line).toHaveCount(0);
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 77;\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toHaveCount(0, { timeout: 10_000 });
    await expect.poll(async () => (await storedDrafts(page)).some((row) => row.path === file), { timeout: 10_000 }).toBe(false);
  } finally {
    fs.chmodSync(file, 0o644);
    close(fixture);
  }
});

test("a View tab a daemon restart restores takes its unsaved draft back (S6 B19)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file } = fixture;
  const original = fs.readFileSync(file, "utf8");
  try {
    fs.chmodSync(file, 0o444);
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 78;\n");
    await expect.poll(async () => (await storedDrafts(page)).find((row) => row.path === file)?.contents, { timeout: 5_000 }).toBe("export const answer = 78;\n");

    fixture.daemon = await fixture.daemon.restart();
    // The page under test survives the restart and reconnects: it must not be reloaded.
    // eslint-disable-next-line hide-e2e/reopen-after-restart-through-blank -- the reconnect of the same page is the subject
    await page.goto(`${fixture.daemon.origin}/?probe=1#token=${fixture.daemon.token}`);
    // The Workspace brings the tab back and the draft returns into it,
    // unsaved; nothing reached the disk and nothing asks for recovery.
    await expect(page.locator('[data-view-area] [data-tab-kind="file"]')).toHaveCount(1, { timeout: 20_000 });
    await expect(content).toContainText("answer = 78", { timeout: 20_000 });
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    await expect(page.locator("[data-draft-recovery]")).toHaveCount(0);
    expect(fs.readFileSync(file, "utf8")).toBe(original);
  } finally {
    fs.chmodSync(file, 0o644);
    close(fixture);
  }
});

test("a draft is discarded only when the operator confirms it (S5.5 B11)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { repo } = fixture;
  try {
    const elsewhere = { id: draftId("host-another", repo, `${repo}/src/main.ts`, "local"), host: "host-another", device: "local", root: repo, path: `${repo}/src/main.ts`, contents: "other host\n", updated_at: 1 };
    const unverified = { id: ["", "", "", "/old/path.ts"].join("\u0000"), host: null, device: null, root: "", path: "/old/path.ts", contents: "v1\n", updated_at: 2 };
    await seedDraft(page, elsewhere);
    await seedDraft(page, unverified);
    await page.reload();
    await expect(page.locator("[data-draft-recovery]")).toHaveAttribute("data-draft-recovery", "2", { timeout: 20_000 });
    await page.locator("[data-draft-recovery-review]").click();
    // Neither can be opened here: one belongs to another host, the other
    // does not name its checkout. Both can be exported or discarded.
    await expect(page.locator(`[data-draft-open="${encodeURIComponent(elsewhere.id)}"]`)).toHaveCount(0);
    await expect(page.locator(`[data-draft-open="${encodeURIComponent(unverified.id)}"]`)).toHaveCount(0);
    const download = page.waitForEvent("download");
    await page.locator(`[data-draft-export="${encodeURIComponent(unverified.id)}"]`).click();
    expect((await download).suggestedFilename()).toBe("path.ts.draft");
    await page.locator(`[data-draft-discard="${encodeURIComponent(unverified.id)}"]`).click();
    expect((await storedDrafts(page)).some((row) => row.id === unverified.id)).toBe(true);
    await page.locator(`[data-draft-discard-confirm="${encodeURIComponent(unverified.id)}"]`).click();
    await expect.poll(async () => (await storedDrafts(page)).map((row) => row.id)).toEqual([elsewhere.id]);
  } finally {
    close(fixture);
  }
});

test("an interrupted draft store upgrade loses nothing and completes on the next start (S5.5 B12)", async ({ page }) => {
  const fixture = await openCheckout(page);
  const { file, repo } = fixture;
  try {
    const app = await leaveApp(page);
    await page.evaluate(async ({ path, root }) => {
      await new Promise<void>((resolve, reject) => {
        const removed = indexedDB.deleteDatabase("hide-shell");
        removed.onsuccess = () => resolve();
        removed.onerror = () => reject(removed.error);
      });
      await new Promise<void>((resolve, reject) => {
        const opened = indexedDB.open("hide-shell", 3);
        opened.onupgradeneeded = () => opened.result.createObjectStore("buffers_v2", { keyPath: "id" });
        opened.onerror = () => reject(opened.error);
        opened.onsuccess = () => {
          const database = opened.result;
          const transaction = database.transaction("buffers_v2", "readwrite");
          transaction.objectStore("buffers_v2").put({ id: `${root}\u0000${path}`, root, path, contents: "export const answer = 96;\n", updated_at: Date.now() });
          transaction.oncomplete = () => { database.close(); resolve(); };
          transaction.onerror = () => { database.close(); reject(transaction.error); };
        };
      });
      // An upgrade that dies halfway: it created the new store and then
      // aborted, as a closed tab or a crash would.
      await new Promise<void>((resolve) => {
        const opened = indexedDB.open("hide-shell", 4);
        opened.onupgradeneeded = () => {
          opened.result.createObjectStore("drafts_v4", { keyPath: "id" });
          opened.transaction!.abort();
        };
        opened.onerror = () => resolve();
        opened.onsuccess = () => { opened.result.close(); resolve(); };
      });
    }, { path: file, root: repo });
    const survived = await page.evaluate(async () => new Promise<number>((resolve, reject) => {
      const opened = indexedDB.open("hide-shell");
      opened.onerror = () => reject(opened.error);
      opened.onsuccess = () => {
        const database = opened.result;
        const version = database.version;
        const rows = database.objectStoreNames.contains("buffers_v2") ? database.transaction("buffers_v2").objectStore("buffers_v2").count() : null;
        if (!rows) { database.close(); resolve(-version); return; }
        rows.onsuccess = () => { database.close(); resolve(rows.result); };
      };
    }));
    expect(survived).toBe(1);
    await page.goto(app);
    await enterWorkspace(page, "repo");
    await expect(page.locator('[data-editor-codemirror] .cm-content')).toContainText("answer = 96");
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 96;\n");
  } finally {
    close(fixture);
  }
});

test("a save refused at the daemon's boundary keeps the draft unsaved, across a reload (S5.5 B13-B15)", async ({ page }) => {
  let refusing = true;
  let refused = 0;
  const fixture = await openCheckout(page, async (page) => {
    // Stands in for the daemon's boundary: a save is answered with the
    // refusal frame hided sends and never reaches the core.
    await page.routeWebSocket(/\/ws$/, (socket) => {
      const server = socket.connectToServer();
      socket.onMessage((message) => {
        if (refusing && typeof message === "string" && message.includes('"kind":"file_save"')) {
          refused += 1;
          const event = JSON.parse(message) as { payload: { path: string } };
          socket.send(JSON.stringify({ type: "path_refused", payload: { kind: "file_save", path: event.payload.path, reason: "outside_checkout" } }));
          return;
        }
        server.send(message);
      });
      server.onMessage((message) => socket.send(message));
    });
  });
  const { file } = fixture;
  const original = fs.readFileSync(file, "utf8");
  try {
    const content = page.locator('[data-editor-codemirror] .cm-content');
    await content.click();
    await page.keyboard.press("ControlOrMeta+KeyA");
    await page.keyboard.type("export const answer = 55;\n");
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => refused, { timeout: 10_000 }).toBeGreaterThanOrEqual(1);
    // Never shown as saved: the tab stays unsaved, the file is untouched,
    // and the draft is in the store.
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    await expect(page.locator('[data-tab-kind="file"]')).toHaveAttribute("data-saving", "false", { timeout: 10_000 });
    expect(fs.readFileSync(file, "utf8")).toBe(original);
    await expect.poll(async () => (await storedDrafts(page)).find((row) => row.path === file)?.contents, { timeout: 5_000 }).toBe("export const answer = 55;\n");

    await page.reload();
    await expect(content).toContainText("answer = 55", { timeout: 20_000 });
    await showDocuments(page);
    await expect(page.locator('[data-editor-dirty="true"]')).toBeVisible();
    expect(fs.readFileSync(file, "utf8")).toBe(original);

    refusing = false;
    await content.click();
    await page.keyboard.press(chord("save_file"));
    await expect.poll(() => onDisk(file), { timeout: 20_000 }).toBe("export const answer = 55;\n");
    await expect(page.locator('[data-editor-dirty="true"]')).toHaveCount(0);
  } finally {
    close(fixture);
  }
});
