import { beforeAll, describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { createInterfaceI18n } from "./i18n/instance";
import {
  decorationFor,
  explorerGitMark,
  explorerRows,
  filteredRows,
  firstChildSelection,
  gitDecorations,
  helperNeedsSettings,
  moveSelection,
  parentPath,
  parentSelection,
  pasteRefusal,
  pasteTarget,
  relativeTo,
  rowForPath,
  rowTitle,
  selectionAfterRemoval,
  treeCommandFor,
} from "./explorer";
import { effectiveRegistry, systemRegistry } from "./shortcuts";
import type { ChangedFileStatus, ChangesSnapshot, DeviceHost } from "./snapshot";
import type { DirectoryList } from "./store";

const ROOT = "/checkout/hide";

function listing(folder: string, entries: [string, boolean][]): DirectoryList {
  return {
    kind: "file_list",
    root_path: folder,
    entries: entries.map(([name, isDirectory]) => ({
      name,
      path: `${folder}/${name}`,
      is_directory: isDirectory,
    })),
    truncated: false,
  };
}

function changes(entries: [string, ChangedFileStatus][]): ChangesSnapshot {
  return {
    root_path: ROOT,
    entries: entries.map(([relative, status]) => ({
      path: `${ROOT}/${relative}`,
      relative_path: relative,
      previous_relative_path: null,
      status,
      added_lines: null,
      removed_lines: null,
    })),
    committed: [],
    base_branch: null,
    selected_path: null,
    selected_committed: false,
    diff: null,
    unavailable_reason: null,
  };
}

const TOP = listing(ROOT, [
  ["src", true],
  ["README.md", false],
  [".gitignore", false],
]);
const SRC = listing(`${ROOT}/src`, [
  ["a.ts", false],
  ["nested", true],
]);

let t: TFunction<"translation">;
let ko: TFunction<"translation">;
beforeAll(async () => {
  t = (await createInterfaceI18n("en")).t;
  ko = (await createInterfaceI18n("ko")).t;
});

describe("explorerRows", () => {
  it("shows a folder's children in the order hided sent them, at depth zero", () => {
    const rows = explorerRows({ rootPath: ROOT, listings: { [ROOT]: TOP }, expandedPaths: [], changes: null });
    expect(rows.map((row) => row.name)).toEqual(["src", "README.md", ".gitignore"]);
    expect(rows.every((row) => row.depth === 0)).toBe(true);
    expect(rows.map((row) => row.isDirectory)).toEqual([true, false, false]);
  });

  it("walks into a folder the core says is expanded and indents its children", () => {
    const rows = explorerRows({
      rootPath: ROOT,
      listings: { [ROOT]: TOP, [`${ROOT}/src`]: SRC },
      expandedPaths: [`${ROOT}/src`],
      changes: null,
    });
    expect(rows.map((row) => `${row.depth}:${row.name}`)).toEqual([
      "0:src",
      "1:a.ts",
      "1:nested",
      "0:README.md",
      "0:.gitignore",
    ]);
    expect(rows.map((row) => row.expanded)).toEqual([true, false, false, false, false]);
    expect(rows.map((row) => row.listing?.root_path ?? null)).toEqual([
      `${ROOT}/src`,
      null,
      null,
      null,
      null,
    ]);
  });

  it("keeps a folder closed when the core has not expanded it, listing or not", () => {
    const rows = explorerRows({
      rootPath: ROOT,
      listings: { [ROOT]: TOP, [`${ROOT}/src`]: SRC },
      expandedPaths: [],
      changes: null,
    });
    expect(rows.map((row) => row.name)).toEqual(["src", "README.md", ".gitignore"]);
    expect(rows.map((row) => row.expanded)).toEqual([false, false, false]);
  });

  it("shows no child under an expanded folder whose listing has not arrived", () => {
    const rows = explorerRows({
      rootPath: ROOT,
      listings: { [ROOT]: TOP },
      expandedPaths: [`${ROOT}/src`],
      changes: null,
    });
    expect(rows.map((row) => row.name)).toEqual(["src", "README.md", ".gitignore"]);
  });

  it("shows nothing at all before the root's own listing arrives", () => {
    expect(explorerRows({ rootPath: ROOT, listings: {}, expandedPaths: [], changes: null })).toEqual([]);
  });
});

describe("filteredRows", () => {
  it("keeps each matched file under its ancestor folders, folders first, every folder open", () => {
    const rows = filteredRows({
      rootPath: ROOT,
      files: [{ relative_path: "README.md" }, { relative_path: "src/nested/deep.ts" }, { relative_path: "src/a.ts" }],
      changes: null,
    });
    expect(rows.map((row) => `${row.depth}:${row.name}`)).toEqual(["0:src", "1:nested", "2:deep.ts", "1:a.ts", "0:README.md"]);
    expect(rows.map((row) => row.path)).toEqual([
      `${ROOT}/src`,
      `${ROOT}/src/nested`,
      `${ROOT}/src/nested/deep.ts`,
      `${ROOT}/src/a.ts`,
      `${ROOT}/README.md`,
    ]);
    expect(rows.filter((row) => row.isDirectory).every((row) => row.expanded)).toBe(true);
  });
});

describe("git decorations", () => {
  it("places a registered subfolder's changed file under its actual Explorer path", () => {
    const scoped = changes([["inside.txt", "modified"]]);
    scoped.root_path = `${ROOT}/registered`;
    scoped.entries[0]!.path = `${ROOT}/registered/inside.txt`;
    const decorations = gitDecorations(scoped, ROOT);
    expect(decorationFor(decorations, `${ROOT}/registered`, ROOT, true)?.status).toBe("modified");
    expect(decorationFor(decorations, `${ROOT}/registered/inside.txt`, ROOT, false)?.status).toBe("modified");
    expect(decorationFor(decorations, `${ROOT}/inside.txt`, ROOT, false)).toBeNull();
    expect(gitDecorations(scoped, "/another-checkout").files.size).toBe(0);
  });

  it("gives a file the letter Git names its status with", () => {
    const decorations = gitDecorations(changes([["src/a.ts", "modified"]]), ROOT);
    const decoration = decorationFor(decorations, `${ROOT}/src/a.ts`, ROOT, false);
    expect(decoration).toMatchObject({ badge: "M", status: "modified" });
    expect(t(decoration!.title)).toBe("Modified");
  });

  it("gives a folder the dot and its riskiest descendant's status", () => {
    const decorations = gitDecorations(
      changes([
        ["src/a.ts", "modified"],
        ["src/nested/b.ts", "conflict"],
        ["src/nested/c.ts", "untracked"],
      ]),
      ROOT,
    );
    const folder = decorationFor(decorations, `${ROOT}/src`, ROOT, true);
    expect(folder).toMatchObject({ badge: "●", status: "conflict" });
    expect(t(folder!.title)).toBe("Contains changed files; top status: conflict");
    expect(decorationFor(decorations, `${ROOT}/src/nested`, ROOT, true)?.status).toBe("conflict");
  });

  it("marks nothing under a folder with no changed descendant", () => {
    const decorations = gitDecorations(changes([["src/a.ts", "modified"]]), ROOT);
    expect(decorationFor(decorations, `${ROOT}/src/nested`, ROOT, true)).toBeNull();
    expect(decorationFor(decorations, `${ROOT}/README.md`, ROOT, false)).toBeNull();
    expect(decorationFor(gitDecorations(null, ROOT), `${ROOT}/src/a.ts`, ROOT, false)).toBeNull();
  });

  it("words a folder's tooltip in the chosen language", () => {
    const rows = explorerRows({ rootPath: ROOT, listings: { [ROOT]: TOP }, expandedPaths: [], changes: changes([["src/a.ts", "deleted"]]) });
    expect(rowTitle(rows[0]!, ROOT, ko)).toBe("src · 변경된 파일이 있어요. 대표 상태: 삭제");
  });

  it("puts the relative path and the status title in the row's tooltip", () => {
    const rows = explorerRows({
      rootPath: ROOT,
      listings: { [ROOT]: TOP },
      expandedPaths: [],
      changes: changes([["src/a.ts", "deleted"]]),
    });
    expect(rows.map((row) => rowTitle(row, ROOT, t))).toEqual([
      "src · Contains changed files; top status: deleted",
      "README.md",
      ".gitignore",
    ]);
  });

  it("presents the root itself as a dot", () => {
    expect(relativeTo(ROOT, ROOT)).toBe(".");
    expect(relativeTo(`${ROOT}/src/a.ts`, ROOT)).toBe("src/a.ts");
    expect(relativeTo("/elsewhere/a.ts", ROOT)).toBe("/elsewhere/a.ts");
  });
});

describe("tree navigation", () => {
  const rows = explorerRows({
    rootPath: ROOT,
    listings: { [ROOT]: TOP, [`${ROOT}/src`]: SRC },
    expandedPaths: [`${ROOT}/src`],
    changes: null,
  });

  it("moves the cursor one row and clamps at both ends", () => {
    expect(moveSelection(rows, null, 1)).toBe(`${ROOT}/src`);
    expect(moveSelection(rows, `${ROOT}/src`, 1)).toBe(`${ROOT}/src/a.ts`);
    expect(moveSelection(rows, `${ROOT}/src/a.ts`, 1)).toBe(`${ROOT}/src/nested`);
    expect(moveSelection(rows, `${ROOT}/src/a.ts`, -1)).toBe(`${ROOT}/src`);
    expect(moveSelection(rows, `${ROOT}/src`, -1)).toBe(`${ROOT}/src`);
    expect(moveSelection(rows, `${ROOT}/.gitignore`, 1)).toBe(`${ROOT}/.gitignore`);
    expect(moveSelection([], `${ROOT}/src`, 1)).toBeNull();
  });

  it("moves the cursor to the enclosing folder, skipping a deeper sibling", () => {
    expect(parentSelection(rows, `${ROOT}/src/a.ts`)).toBe(`${ROOT}/src`);
    expect(parentSelection(rows, `${ROOT}/src/nested`)).toBe(`${ROOT}/src`);
    expect(parentSelection(rows, `${ROOT}/src`)).toBeNull();
    expect(parentSelection(rows, null)).toBeNull();
  });

  it("names the first child only while the folder is expanded with rows under it", () => {
    expect(firstChildSelection(rows, `${ROOT}/src`)).toBe(`${ROOT}/src/a.ts`);
    expect(firstChildSelection(rows, `${ROOT}/README.md`)).toBeNull();
    expect(firstChildSelection(rows, `${ROOT}/.gitignore`)).toBeNull();
  });

  it("finds the row a path names and reports a path the tree does not show", () => {
    expect(rowForPath(rows, `${ROOT}/src/a.ts`)?.name).toBe("a.ts");
    expect(rowForPath(rows, `${ROOT}/gone`)).toBeNull();
    expect(rowForPath(rows, null)).toBeNull();
  });
});

describe("removal selection", () => {
  const rows = explorerRows({
    rootPath: ROOT,
    listings: { [ROOT]: TOP, [`${ROOT}/src`]: SRC },
    expandedPaths: [`${ROOT}/src`],
    changes: null,
  });

  it("names the folder a path sits in", () => {
    expect(parentPath(`${ROOT}/src/a.ts`)).toBe(`${ROOT}/src`);
    expect(parentPath(`${ROOT}/README.md`)).toBe(ROOT);
  });

  it("selects the next sibling, else the previous sibling, else the parent", () => {
    // `src` is followed by README.md, so removing it selects README.md.
    expect(selectionAfterRemoval(rows, `${ROOT}/src`, ROOT)).toBe(`${ROOT}/README.md`);
    // `.gitignore` is last, so removing it selects its previous sibling.
    expect(selectionAfterRemoval(rows, `${ROOT}/.gitignore`, ROOT)).toBe(`${ROOT}/README.md`);
    // The folder's first child is followed by its sibling.
    expect(selectionAfterRemoval(rows, `${ROOT}/src/a.ts`, ROOT)).toBe(`${ROOT}/src/nested`);
    expect(selectionAfterRemoval(rows, `${ROOT}/missing`, ROOT)).toBe(ROOT);
  });
});

describe("the Explorer's Git status mark (S5.5 B20, B22)", () => {
  const base = { root_path: "/repo", entries: [], committed: [], selected_path: null, diff: null } as unknown as ChangesSnapshot;
  it("says a failed read is unavailable or out of date and never draws it as current", () => {
    expect(explorerGitMark(null, t)).toEqual({ state: "loading", text: "Loading Git status" });
    expect(explorerGitMark({ ...base, unavailable_reason: "git is not installed" }, t)?.text).toBe("Git status unavailable: git is not installed");
    expect(explorerGitMark({ ...base, stale_reason: "git status timed out" }, t)?.state).toBe("stale");
    expect(explorerGitMark(base, t)).toBeNull();
  });

  it("draws nothing for a folder that is not a repository", () => {
    expect(explorerGitMark({ ...base, unavailable_reason: "/plain is not inside a Git repository", not_a_repository: true }, t)).toBeNull();
  });

  it("words the mark's sentence in the chosen language", () => {
    expect(explorerGitMark({ ...base, unavailable_reason: "git is not installed" }, ko)?.text).toBe("Git 상태를 읽지 못했어요: git is not installed");
  });
});

describe("a device listing that waits on Settings", () => {
  const host = (state: DeviceHost["state"]) => ({ state }) as DeviceHost;
  it("sends the operator to Settings only when a consent decision is what is missing", () => {
    expect(helperNeedsSettings(host("not_allowed"))).toBe(true);
    expect(helperNeedsSettings(host("identity_changed"))).toBe(true);
    expect(helperNeedsSettings(host("connecting"))).toBe(false);
    expect(helperNeedsSettings(host("unavailable"))).toBe(false);
    expect(helperNeedsSettings(undefined)).toBe(false);
  });
});

describe("the chords the tree answers itself", () => {
  const press = (code: string, mods: { meta?: boolean; alt?: boolean; shift?: boolean; ctrl?: boolean } = {}) => ({ code, ...mods });

  it("reads the path copies on a Mac as VS Code's chords and Move to Trash as ⌘⌫", () => {
    const mac = systemRegistry("mac");
    expect(treeCommandFor(press("KeyC", { meta: true, alt: true }), mac, "electron")).toBe("copy_path");
    expect(treeCommandFor(press("KeyC", { meta: true, alt: true, shift: true }), mac, "electron")).toBe("copy_relative_path");
    expect(treeCommandFor(press("Backspace", { meta: true }), mac, "electron")).toBe("move_to_trash");
    expect(treeCommandFor(press("KeyC", { meta: true }), mac, "electron")).toBe("explorer_copy");
  });

  it("reads the clipboard as the system's own chords: ⌘X, ⌘C, ⌘V on a Mac, Ctrl+X, Ctrl+C, Ctrl+V on Windows and Linux, in a browser tab too", () => {
    for (const host of ["electron", "browser"] as const) {
      const mac = systemRegistry("mac");
      expect(treeCommandFor(press("KeyX", { meta: true }), mac, host)).toBe("explorer_cut");
      expect(treeCommandFor(press("KeyC", { meta: true }), mac, host)).toBe("explorer_copy");
      expect(treeCommandFor(press("KeyV", { meta: true }), mac, host)).toBe("explorer_paste");
      const pc = systemRegistry("pc");
      expect(treeCommandFor(press("KeyX", { ctrl: true }), pc, host)).toBe("explorer_cut");
      expect(treeCommandFor(press("KeyC", { ctrl: true }), pc, host)).toBe("explorer_copy");
      expect(treeCommandFor(press("KeyV", { ctrl: true }), pc, host)).toBe("explorer_paste");
    }
    // A terminal's Ctrl+Shift+C on Windows and Linux is not the tree's.
    expect(treeCommandFor(press("KeyC", { ctrl: true, shift: true }), systemRegistry("pc"), "electron")).toBeNull();
  });

  it("reads them on Windows and Linux as Alt+Shift+C, Alt+Shift+H and Delete, leaving Alt+Shift+R to the PRs tool", () => {
    const pc = systemRegistry("pc");
    expect(treeCommandFor(press("KeyC", { alt: true, shift: true }), pc, "electron")).toBe("copy_path");
    expect(treeCommandFor(press("KeyH", { alt: true, shift: true }), pc, "electron")).toBe("copy_relative_path");
    expect(treeCommandFor(press("KeyR", { alt: true, shift: true }), pc, "electron")).toBeNull();
    expect(treeCommandFor(press("Delete"), pc, "electron")).toBe("move_to_trash");
  });

  it("follows the chord the operator bound, and has none in a browser tab until one is bound", () => {
    const bound = effectiveRegistry({ copy_path: "cmd+shift+y" }, "electron", "mac").registry;
    expect(treeCommandFor(press("KeyY", { meta: true, shift: true }), bound, "electron")).toBe("copy_path");
    expect(treeCommandFor(press("KeyC", { meta: true, alt: true }), bound, "electron")).toBeNull();
    expect(treeCommandFor(press("KeyC", { meta: true, alt: true }), systemRegistry("mac"), "browser")).toBeNull();
  });
});

describe("where a paste lands and what it refuses", () => {
  it("pastes into the selected folder, the folder of a selected file, else the root", () => {
    expect(pasteTarget({ path: `${ROOT}/src`, isDirectory: true }, ROOT)).toBe(`${ROOT}/src`);
    expect(pasteTarget({ path: `${ROOT}/src/a.ts`, isDirectory: false }, ROOT)).toBe(`${ROOT}/src`);
    expect(pasteTarget({ path: `${ROOT}/README.md`, isDirectory: false }, ROOT)).toBe(ROOT);
    expect(pasteTarget(null, ROOT)).toBe(ROOT);
  });

  const held = (over: Partial<{ path: string; isDirectory: boolean; root: string; device: string }> = {}) => ({
    path: `${ROOT}/src`,
    isDirectory: true,
    root: ROOT,
    device: "local",
    ...over,
  });

  it("refuses another checkout or device, and a folder into itself or below itself", () => {
    expect(pasteRefusal(held(), `${ROOT}/docs`, ROOT, "local")).toBeNull();
    expect(pasteRefusal(held({ isDirectory: false, path: `${ROOT}/a.ts` }), `${ROOT}/src`, ROOT, "local")).toBeNull();
    expect(pasteRefusal(held({ root: "/checkout/other" }), `${ROOT}/docs`, ROOT, "local")).toBe("explorer.pasteOtherCheckout");
    expect(pasteRefusal(held({ device: "studio" }), `${ROOT}/docs`, ROOT, "local")).toBe("explorer.pasteOtherCheckout");
    expect(pasteRefusal(held(), `${ROOT}/src`, ROOT, "local")).toBe("explorer.pasteIntoItself");
    expect(pasteRefusal(held(), `${ROOT}/src/nested`, ROOT, "local")).toBe("explorer.pasteIntoItself");
    expect(pasteRefusal(held(), `${ROOT}/src-extra`, ROOT, "local")).toBeNull();
  });
});
