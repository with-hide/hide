import { describe, expect, it } from "vitest";
import { explorerMenuItems } from "./explorer";
import { translate } from "./i18n/client";
import { revealExternalEntry, revealLabel } from "./revealExternal";

const REVEAL_HERE_ONLY = "Only for files and folders on this computer.";

describe("reveal_external (issue 324)", () => {
  it("names the act the way the host's OS does", () => {
    expect(translate(revealLabel("darwin"))).toBe("Reveal in Finder");
    expect(translate(revealLabel("win32"))).toBe("Reveal in File Explorer");
    expect(translate(revealLabel("linux"))).toBe("Open Containing Folder");
    expect(translate(revealLabel("freebsd"))).toBe("Show in File Manager");
    expect(translate(revealLabel(undefined))).toBe("Show in File Manager");
  });

  it("is absent in a browser tab, disabled with the reason on another device, and blocked by a known reason here", () => {
    const host = { label: revealLabel("darwin") };
    expect(revealExternalEntry(null, "local", "local", translate)).toEqual([]);
    expect(revealExternalEntry(host, "local", "local", translate)).toEqual([{ id: "reveal_external", label: "Reveal in Finder", unavailable: null }]);
    expect(revealExternalEntry(host, "studio", "local", translate, "The file was deleted.")[0]?.unavailable).toBe(REVEAL_HERE_ONLY);
    expect(revealExternalEntry(host, "local", "local", translate, "The file was deleted.", true)).toEqual([
      { id: "reveal_external", label: "Reveal in Finder", unavailable: "The file was deleted.", separated: true },
    ]);
  });
});

describe("the Explorer's context menu", () => {
  /** Each item as the menu draws it: a separator line before it, then its label. */
  const drawn = (items: { label: string; separated?: boolean }[]) => items.flatMap((item) => [...(item.separated ? ["─"] : []), item.label]);
  const base = { html: false, besideReason: null, reveal: { label: revealLabel("darwin") }, device: "local", node: "local", canPaste: false, chord: () => "" };
  const file = { path: "/Users/example/repo/a.html" };
  const folder = { path: "/Users/example/repo/src" };

  it("puts the reveal between a file's opens and Rename, and after a folder's creations", () => {
    expect(drawn(explorerMenuItems({ ...base, isDirectory: false, row: file, html: true }))).toEqual([
      "Open to the side",
      "Open in Browser",
      "Open source",
      "─",
      "Reveal in Finder",
      "─",
      "Cut",
      "Copy",
      "Duplicate",
      "─",
      "Copy Path",
      "Copy Relative Path",
      "─",
      "Rename",
      "─",
      "Move to Trash",
    ]);
    expect(drawn(explorerMenuItems({ ...base, isDirectory: true, row: folder }))).toEqual([
      "New File",
      "New Folder",
      "─",
      "Reveal in Finder",
      "─",
      "Cut",
      "Copy",
      "Duplicate",
      "─",
      "Copy Path",
      "Copy Relative Path",
      "─",
      "Rename",
      "─",
      "Move to Trash",
    ]);
  });

  it("offers the creations, the reveal and the path copies on the root, never a rename or a trash, and no reveal in a browser tab", () => {
    expect(drawn(explorerMenuItems({ ...base, isDirectory: true, row: null }))).toEqual(["New File", "New Folder", "─", "Reveal in Finder", "─", "Copy Path", "Copy Relative Path"]);
    expect(drawn(explorerMenuItems({ ...base, reveal: null, isDirectory: false, row: file }))).toEqual(["Open to the side", "─", "Cut", "Copy", "Duplicate", "─", "Copy Path", "Copy Relative Path", "─", "Rename", "─", "Move to Trash"]);
  });

  it("offers Paste, after Copy, only while the clipboard holds an item, and on the root too", () => {
    expect(drawn(explorerMenuItems({ ...base, canPaste: true, isDirectory: false, row: file }))).toEqual([
      "Open to the side",
      "─",
      "Reveal in Finder",
      "─",
      "Cut",
      "Copy",
      "Paste",
      "Duplicate",
      "─",
      "Copy Path",
      "Copy Relative Path",
      "─",
      "Rename",
      "─",
      "Move to Trash",
    ]);
    expect(drawn(explorerMenuItems({ ...base, canPaste: true, isDirectory: true, row: null }))).toEqual(["New File", "New Folder", "─", "Reveal in Finder", "─", "Paste", "─", "Copy Path", "Copy Relative Path"]);
  });

  it("shows each clipboard item's chord and none for Duplicate", () => {
    const chords: Record<string, string> = { explorer_cut: "⌘X", explorer_copy: "⌘C", explorer_paste: "⌘V" };
    const items = explorerMenuItems({ ...base, canPaste: true, isDirectory: false, row: file, chord: (id) => chords[id] ?? "" });
    expect(items.filter((item) => ["cut", "copy", "paste", "duplicate"].includes(item.id)).map((item) => [item.id, item.shortcut])).toEqual([
      ["cut", "⌘X"],
      ["copy", "⌘C"],
      ["paste", "⌘V"],
      ["duplicate", undefined],
    ]);
  });

  it("lists the reveal disabled with the reason on another device's tree", () => {
    const remote = explorerMenuItems({ ...base, device: "studio", isDirectory: false, row: file });
    expect(remote.find((item) => item.id === "reveal_external")?.unavailable).toBe(REVEAL_HERE_ONLY);
  });
});
