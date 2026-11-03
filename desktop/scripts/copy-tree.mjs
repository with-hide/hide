// The one folder copy the desktop scripts use, in place of fs.cpSync.
// On Windows, fs.cpSync crashed node outright (exit 3221225477, a segfault)
// on a path with non-ASCII characters, such as a user folder named in Korean
// (with-hide/hide#2042), and it left the installed package's
// resources/app.asar behind on the package lane. This copy goes entry by
// entry through the calls that take any path, keeps links as links, and
// throws at the first step that fails.
import fs from "node:fs";
import path from "node:path";

/** Copies `source` (a folder or a file) to `target`, links as links, modes kept. */
export function copyTree(source, target) {
  const stat = fs.lstatSync(source);
  if (stat.isSymbolicLink()) {
    fs.symlinkSync(fs.readlinkSync(source), target);
  } else if (stat.isDirectory()) {
    fs.mkdirSync(target, { recursive: true });
    for (const entry of fs.readdirSync(source)) copyTree(path.join(source, entry), path.join(target, entry));
  } else if (stat.isFile()) {
    fs.copyFileSync(source, target);
    fs.chmodSync(target, stat.mode & 0o7777);
  } else {
    throw new Error(`${source} is neither a file, a folder nor a link`);
  }
}
