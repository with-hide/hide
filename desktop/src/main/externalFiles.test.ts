import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, it, vi } from "vitest";
import { ExternalFiles, MAX_EXTERNAL_GRANTS, MAX_EXTERNAL_TEXT_BYTES, externalRoute } from "./externalFiles";

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
    expect(files.keep(new Set())).toEqual([granted.id]);
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
    files.keep(new Set());
    expect(files.inside(id, style)).toBe(false);
    expect(files.pageUrl(id)).toBeNull();
  });
});
