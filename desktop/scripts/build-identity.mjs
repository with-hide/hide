// Build identity shared by the native Mac app and every foreign helper artifact.
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const run = (file, args) => execFileSync(file, args, { cwd: repo, encoding: "utf8" }).trim();

/** The first-parent count of the last commit HEAD shares with main. */
export function orderOnMain() {
  if (run("git", ["rev-parse", "--is-shallow-repository"]) === "true") {
    throw new Error("a shallow checkout cannot count the package's place in main's history: fetch it, or set HIDE_BUILD_ORDER");
  }
  const base = run("git", ["merge-base", "HEAD", "origin/main"]);
  return run("git", ["rev-list", "--first-parent", "--count", base]);
}

// The app version comes from the release tag (release.yml passes
// HIDE_VERSION). A local build is named after the newest release tag behind
// it and its own commit, `0.4.0-main.<commit>`: it never claims a release it
// is not, even on a tagged commit, and orders with the releases it follows
// (hided's build_order reads MAJOR.MINOR.PATCH). That shape is no release
// number, so the app never updates itself from it (src/main/updater.ts).
export function resolveVersion() {
  const requested = process.env.HIDE_VERSION?.replace(/^v/, "");
  if (requested) {
    if (!/^[0-9]/.test(requested)) throw new Error(`HIDE_VERSION is not a version: '${requested}'`);
    return requested;
  }
  let tag;
  try {
    tag = run("git", ["describe", "--tags", "--match", "v[0-9]*", "--abbrev=0"]);
  } catch {
    tag = "";
  }
  const release = /^v([0-9]+\.[0-9]+\.[0-9]+)/.exec(tag)?.[1];
  if (!release) throw new Error(`HIDE_VERSION is unset and no v<version> tag is behind this tree (got '${tag || "nothing"}')`);
  // A semver identifier of digits alone may not start with 0, so a commit
  // that happens to be all digits keeps git's own `g` mark.
  const short = run("git", ["rev-parse", "--short=8", "HEAD"]);
  const commit = /^[0-9]+$/.test(short) ? `g${short}` : short;
  const dirty = run("git", ["status", "--porcelain", "--untracked-files=no"]) !== "";
  return `${release}-main.${commit}${dirty ? ".dirty" : ""}`;
}


if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const commit = run("git", ["rev-parse", "HEAD"]);
  const version = resolveVersion();
  const order = process.env.HIDE_BUILD_ORDER || orderOnMain();
  if (!/^[0-9]+$/.test(order)) throw new Error("invalid HIDE_BUILD_ORDER");
  console.log(JSON.stringify({ commit, version, order }));
}
