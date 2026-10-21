import { describe, expect, it } from "vitest";
import { copiedExplorerFile, isImageDrag, terminalPathText } from "./explorerDrag";

describe("Explorer terminal input", () => {
  it("keeps Unicode, spaces and apostrophes while escaping shell expansion, without submitting", () => {
    expect(terminalPathText("/project/한글 ' $HOME `cmd` \\\".txt")).toBe('"/project/한글 \' \\$HOME \\`cmd\\` \\\\\\".txt" ');
    expect(terminalPathText("C:/work/한글 파일.txt")).toBe('"C:/work/한글 파일.txt" ');
    expect(terminalPathText("//server/share/a b.txt")).toBe('"//server/share/a b.txt" ');
  });

  it("refuses control characters and non-absolute paths", () => {
    for (const path of ["relative.txt", "/project/a\ncommand", "/project/a\rcommand", "/project/\u001b[200~"]) {
      expect(() => terminalPathText(path)).toThrow("invalid_path");
    }
  });

  it("routes image formats to binary ingress, while folders and ordinary files remain distinct", () => {
    const file = { path: "/project/shot.PNG", directory: false, device: "local", root: "/project" };
    expect(isImageDrag(file)).toBe(true);
    expect(isImageDrag({ ...file, directory: true })).toBe(false);
    expect(isImageDrag({ ...file, path: "/project/image.png.txt" })).toBe(false);
  });

  it("only treats this window's copied file path as file input", () => {
    const held = { mode: "copy" as const, path: "/project/image.png", isDirectory: false, root: "/project", device: "local", terminalCopy: true };
    expect(copiedExplorerFile(held, held.path)?.path).toBe(held.path);
    expect(copiedExplorerFile(held, "ordinary copied text")).toBeNull();
    expect(copiedExplorerFile({ ...held, terminalCopy: false }, held.path)).toBeNull();
    expect(copiedExplorerFile({ ...held, mode: "cut" }, held.path)).toBeNull();
    expect(copiedExplorerFile({ ...held, isDirectory: true }, held.path)).toBeNull();
  });
});
