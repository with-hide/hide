import fs from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { copyTree } from "./copy-tree.mjs";

const folders = [];
afterEach(() => folders.splice(0).forEach((folder) => fs.rmSync(folder, { recursive: true, force: true })));

function scratch() {
  const parent = path.resolve("out/copy-tree-tests");
  fs.mkdirSync(parent, { recursive: true });
  const folder = fs.mkdtempSync(path.join(parent, "copy-"));
  folders.push(folder);
  return folder;
}

describe("copyTree", () => {
  // with-hide/hide#2042: a checkout under a user folder named in Korean
  // crashed node at the desktop build's folder copy on Windows.
  it("copies a nested folder whose paths are named in Korean", () => {
    const root = scratch();
    const source = path.join(root, "이호연", "static");
    fs.mkdirSync(path.join(source, "그림"), { recursive: true });
    fs.writeFileSync(path.join(source, "status.html"), "<p>상태</p>");
    fs.writeFileSync(path.join(source, "그림", "부엉이.svg"), "<svg/>");
    const target = path.join(root, "배포", "static");

    copyTree(source, target);

    expect(fs.readFileSync(path.join(target, "status.html"), "utf8")).toBe("<p>상태</p>");
    expect(fs.readFileSync(path.join(target, "그림", "부엉이.svg"), "utf8")).toBe("<svg/>");
  });

  it("refuses a source that does not exist", () => {
    const root = scratch();
    expect(() => copyTree(path.join(root, "없음"), path.join(root, "target"))).toThrow(/ENOENT/);
  });
});
