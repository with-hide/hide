import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, it, vi } from "vitest";
import { ExternalFiles, MAX_EXTERNAL_GRANTS, MAX_EXTERNAL_TEXT_BYTES, UNNAMED_GRANT_GRACE_MS, agentOpen, clickedRoute, externalPartition, externalRoute } from "./externalFiles";

// The physical spelling `fs.realpath` answers, as the shell's probe sends it.
const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "hide-external-files-")));
afterAll(() => fs.rmSync(root, { recursive: true, force: true }));

function file(name: string, contents: string | Uint8Array, mode = 0o644): string {
  const target = path.join(root, name);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, contents);
  fs.chmodSync(target, mode);
  return target;
}

const MACH_O = new Uint8Array([0xcf, 0xfa, 0xed, 0xfe, 0, 0, 0, 0]);

describe("externalRoute", () => {
  it("opens a viewer's files in Hide, a script as text, and an HTML file as a page or its source", async () => {
    expect(await externalRoute(file("shot.png", "png"), false)).toEqual({ action: "grant", kind: "image" });
    expect(await externalRoute(file("notes.txt", "hello\n"), false)).toEqual({ action: "grant", kind: "text" });
    expect(await externalRoute(file("run.sh", "#!/bin/sh\necho hi\n", 0o755), false)).toEqual({ action: "grant", kind: "text" });
    expect(await externalRoute(file("tool.py", "print('hi')\n"), false)).toEqual({ action: "grant", kind: "text" });
    expect(await externalRoute(file("page.html", "<p>hi</p>"), false)).toEqual({ action: "grant", kind: "html" });
    expect(await externalRoute(file("page.html", "<p>hi</p>"), true)).toEqual({ action: "grant", kind: "text" });
    // A PDF is known by its first bytes, as a checkout file is.
    expect(await externalRoute(file("report", "%PDF-1.7\n"), false)).toEqual({ action: "grant", kind: "pdf" });
    expect(await externalRoute(file("clip.mp4", new Uint8Array([0, 0, 0, 24, 0x66, 0x74, 0x79, 0x70])), false)).toEqual({ action: "grant", kind: "video" });
  });

  it("reveals a program or an installer, and hands audio, a folder and other binaries to the system (D-08, D-10)", async () => {
    expect(await externalRoute(file("tool", MACH_O, 0o755), false)).toEqual({ action: "reveal", reason: "program" });
    expect(await externalRoute(file("setup.pkg", "x"), false)).toEqual({ action: "reveal", reason: "installer" });
    expect(await externalRoute(file("song.mp3", "x"), false)).toEqual({ action: "system", reason: "audio" });
    fs.mkdirSync(path.join(root, "folder"), { recursive: true });
    expect(await externalRoute(path.join(root, "folder"), false)).toEqual({ action: "system", reason: "folder" });
    expect(await externalRoute(file("blob.bin", new Uint8Array([1, 0, 2, 0])), false)).toEqual({ action: "system", reason: "binary" });
  });

  it("still grants a file it cannot read, so its tab says it cannot be opened (B16)", async () => {
    expect(await externalRoute(path.join(root, "gone.txt"), false)).toEqual({ action: "grant", kind: "text" });
  });
});

describe("clickedRoute", () => {
  it("grants a file deleted since the hover by its name, so its tab says it cannot be opened (B16)", async () => {
    const gone = path.join(root, "deleted-since.png");
    expect(await clickedRoute(gone, false)).toEqual({ real: gone, route: { action: "grant", kind: "image" } });
    expect(await clickedRoute(path.join(root, "deleted-song.mp3"), false)).toMatchObject({ route: { action: "system", reason: "audio" } });
  });

  it.skipIf(process.platform === "win32")("refuses a path that now resolves through a link swapped in since the hover", async () => {
    const link = path.join(root, "swapped-link.txt");
    fs.symlinkSync(file("elsewhere/target.txt", "not this"), link);
    expect(await clickedRoute(link, false)).toEqual({ refused: "not_physical" });
  });

  it("brings an open file's one grant back when it is clicked again after being deleted (B10, B16)", async () => {
    const files = new ExternalFiles(vi.fn());
    const notes = file("open-then-deleted.txt", "here");
    const first = files.grant(notes, "text")!;
    fs.rmSync(notes);
    const clicked = await clickedRoute(notes, false);
    expect(clicked).toMatchObject({ route: { action: "grant", kind: "text" } });
    expect(files.grant(notes, "text")).toEqual(first);
    expect(await files.read(first.id)).toEqual({ failure: "ENOENT" });
  });
});

describe("agentOpen", () => {
  const files = () => new ExternalFiles(() => undefined);

  it("grants what a click would, a page or its text as the command asks, and the physical file behind a link", async () => {
    const held = files();
    const page = await agentOpen(held, file("agent/report.html", "<p>hi</p>"), true);
    expect(page).toMatchObject({ grant: { name: "report.html", kind: "html" } });
    expect(await agentOpen(held, path.join(root, "agent/report.html"), false)).toMatchObject({ grant: { kind: "text" } });
    expect(await agentOpen(held, file("agent/shot.png", "png"), false)).toMatchObject({ grant: { kind: "image" } });
    // The same page again keeps its one grant, so its tab comes forward (B10).
    expect(await agentOpen(held, path.join(root, "agent/report.html"), true)).toEqual(page);
    if (process.platform !== "win32") {
      // An agent writes the path as it has it: `/tmp` is a link on macOS.
      const link = path.join(root, "agent-link");
      fs.symlinkSync(path.join(root, "agent"), link);
      const through = await agentOpen(held, path.join(link, "report.html"), true);
      expect(through).toEqual(page);
      if ("grant" in through) expect(held.get(through.grant.id)?.real).toBe(path.join(root, "agent/report.html"));
    }
  });

  it("refuses what it cannot show with a reason the agent acts on, and opens nothing outside Hide", async () => {
    const held = files();
    expect(await agentOpen(held, path.join(root, "agent/missing.png"), false)).toEqual({ refused: "file_missing" });
    fs.mkdirSync(path.join(root, "agent/folder"), { recursive: true });
    expect(await agentOpen(held, path.join(root, "agent/folder"), false)).toEqual({ refused: "unsupported_kind" });
    expect(await agentOpen(held, file("agent/song.mp3", "x"), false)).toEqual({ refused: "unsupported_kind" });
    expect(await agentOpen(held, file("agent/tool", MACH_O, 0o755), false)).toEqual({ refused: "unsupported_kind" });
    expect(await agentOpen(held, file("agent/notes.md", "# hi\n"), true)).toEqual({ refused: "not_html" });
    if (process.platform !== "win32" && process.getuid?.() !== 0) {
      expect(await agentOpen(held, file("agent/locked.txt", "secret", 0o000), false)).toEqual({ refused: "file_unavailable" });
    }
    expect(held.live()).toEqual([]);
  });

  it("is told no past the grants' cap (B17)", async () => {
    const held = files();
    for (let index = 0; index < MAX_EXTERNAL_GRANTS; index += 1) held.grant(path.join(root, `cap-${index}.txt`), "text");
    expect(await agentOpen(held, file("agent/one-more.txt", "x"), false)).toEqual({ refused: "outside_files_full" });
  });
});

describe("ExternalFiles", () => {
  it("grants a file once, lets go of what no tab names, and logs no path or name (B10, B14, B22)", async () => {
    const log = vi.fn();
    const files = new ExternalFiles(log);
    const shot = file("secret-name.png", "png bytes");
    const granted = files.grant(shot, "image")!;
    expect(granted).toEqual({ id: expect.stringMatching(/^[0-9a-f]{32}$/), name: "secret-name.png", kind: "image" });
    expect(files.grant(shot, "image")).toEqual(granted);
    expect(files.live()).toEqual([granted.id]);
    expect(await files.read(granted.id)).toEqual({ bytes: new Uint8Array(Buffer.from("png bytes")) });

    expect(files.keep(new Set([granted.id]))).toEqual([]);
    expect(files.keep(new Set())).toEqual([{ id: granted.id, slot: null }]);
    expect(files.live()).toEqual([]);
    expect(await files.read(granted.id)).toEqual({ failure: "grant_gone" });
    for (const [, fields] of log.mock.calls) {
      expect(JSON.stringify(fields)).not.toContain("secret-name");
      expect(JSON.stringify(fields)).not.toContain(root);
    }
  });

  it("holds at most the cap, and a click past it is told no (B17)", () => {
    const log = vi.fn();
    const files = new ExternalFiles(log);
    for (let n = 0; n < MAX_EXTERNAL_GRANTS; n += 1) expect(files.grant(path.join(root, `f${n}.txt`), "text")).not.toBeNull();
    expect(files.grant(path.join(root, "one-more.txt"), "text")).toBeNull();
    expect(log).toHaveBeenLastCalledWith("external.cap_reached", { kind: "text", cap: MAX_EXTERNAL_GRANTS });
  });

  it("reads what is on the disk now, and a text past the tab's size as preview-only, as a checkout file", async () => {
    const files = new ExternalFiles(vi.fn());
    const notes = file("notes.md", "one\n");
    const { id } = files.grant(notes, "text")!;
    fs.writeFileSync(notes, "two\n");
    expect(await files.read(id)).toEqual({ bytes: new Uint8Array(Buffer.from("two\n")) });
    const big = file("big.log", new Uint8Array(MAX_EXTERNAL_TEXT_BYTES + 1).fill(0x61));
    expect(await files.read(files.grant(big, "text")!.id)).toEqual({ preview_only: true });
  });

  it.skipIf(process.platform === "win32")("refuses a read through a link swapped in after the click, and says why in the log alone", async () => {
    const log = vi.fn();
    const files = new ExternalFiles(log);
    const target = file("swapped.txt", "mine");
    const { id } = files.grant(target, "text")!;
    fs.rmSync(target);
    fs.symlinkSync(file("elsewhere/other.txt", "not mine"), target);
    expect(await files.read(id)).toEqual({ failure: "not_physical" });
    expect(log).toHaveBeenLastCalledWith("external.read_failed", { grant: id, kind: "text", reason: "not_physical" });
  });

  it("lets an HTML page's files come from its folder alone, links resolved, while the grant lasts (D-03)", () => {
    const files = new ExternalFiles(vi.fn());
    const page = file("site/index.html", "<p>hi</p>");
    const style = file("site/css/app.css", "p{}");
    const outside = file("private.txt", "secret");
    const { id } = files.grant(page, "html")!;
    expect(files.pageUrl(id)).toMatch(/^file:\/\/.*index\.html$/);
    expect(files.inside(id, style)).toBe(true);
    expect(files.inside(id, path.join(root, "site", "missing.png"))).toBe(true);
    expect(files.inside(id, outside)).toBe(false);
    expect(files.inside(id, path.join(root, "site", "..", "private.txt"))).toBe(false);
    if (process.platform !== "win32") {
      fs.symlinkSync(outside, path.join(root, "site", "link.txt"));
      expect(files.inside(id, path.join(root, "site", "link.txt"))).toBe(false);
    }
    files.keep(new Set([id]));
    files.keep(new Set());
    expect(files.inside(id, style)).toBe(false);
    expect(files.pageUrl(id)).toBeNull();
  });

  it("keeps a grant no sync has named yet, so a sync the shell sent before the click's answer lets go of nothing (B14)", () => {
    let now = 1_000;
    const files = new ExternalFiles(vi.fn(), () => now);
    const fresh = files.grant(file("fresh.txt", "fresh"), "text")!;
    expect(files.keep(new Set())).toEqual([]);
    expect(files.live()).toEqual([fresh.id]);
    // One whose open never reached the core goes once the grace is over.
    now += UNNAMED_GRANT_GRACE_MS;
    expect(files.keep(new Set())).toEqual([{ id: fresh.id, slot: null }]);
    // A closed tab's file clicked again comes back as fresh, so the sync that closed it does not take it.
    const again = files.grant(file("again.txt", "again"), "text")!;
    files.keep(new Set([again.id]));
    expect(files.grant(file("again.txt", "again"), "text")).toEqual(again);
    expect(files.keep(new Set())).toEqual([]);
  });

  it("opens a page and its source as two grants, so the source never replaces the page (B9)", () => {
    const files = new ExternalFiles(vi.fn());
    const page = file("both/index.html", "<p>hi</p>");
    const shown = files.grant(page, "html")!;
    const source = files.grant(page, "text")!;
    expect(source.id).not.toBe(shown.id);
    expect(files.partition(shown.id)).toBe(externalPartition(0));
    expect(files.partition(source.id)).toBeNull();
  });

  it("gives HTML pages at most the cap's sessions, and a session again only once its storage is cleared (B4)", () => {
    let now = 0;
    const files = new ExternalFiles(vi.fn(), () => now);
    const partitions = new Set<string>();
    for (let n = 0; n < MAX_EXTERNAL_GRANTS * 2; n += 1) {
      const { id } = files.grant(file(`pages/p${n}.html`, "<p>"), "html")!;
      partitions.add(files.partition(id)!);
      now += UNNAMED_GRANT_GRACE_MS;
      const [released] = files.keep(new Set());
      expect(released).toEqual({ id, slot: expect.any(Number) });
      // Until its storage is cleared the slot is not handed out again.
      if (n === 0) {
        expect(files.partition(files.grant(file("pages/next.html", "<p>"), "html")!.id)).not.toBe(externalPartition(released!.slot!));
        now += UNNAMED_GRANT_GRACE_MS;
        files.slotCleared(files.keep(new Set())[0]!.slot!);
      }
      files.slotCleared(released!.slot!);
    }
    expect(partitions.size).toBeLessThanOrEqual(MAX_EXTERNAL_GRANTS);
  });
});
