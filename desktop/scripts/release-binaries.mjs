// The boundary between package.mjs's two halves (`compile`, then
// `assemble`): the record of the release binaries one half built, which the
// other packages only after checking. Between them the binaries may cross a
// workflow artifact, which keeps their bytes but not their mode.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const digest = (file) => createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** Records each binary's digest with the commit, version and order it was built for. */
export function recordReleaseBinaries(record, folder, names, identity) {
  const files = Object.fromEntries(names.map((name) => [name, digest(path.join(folder, name))]));
  fs.writeFileSync(record, `${JSON.stringify({ ...identity, files }, null, 2)}\n`);
}

/** Refuses binaries built for another package, or missing, changed or linked ones, and makes the checked ones executable. */
export function checkReleaseBinaries(record, folder, names, identity) {
  const recorded = JSON.parse(fs.readFileSync(record, "utf8"));
  if (recorded.commit !== identity.commit || recorded.version !== identity.version || recorded.order !== identity.order) {
    throw new Error(`the release binaries were built for ${recorded.version} at ${recorded.commit} order ${recorded.order}, not this package: compile them again`);
  }
  if (Object.keys(recorded.files ?? {}).sort().join() !== [...names].sort().join()) throw new Error(`the release binaries record names other programs than ${names.join(", ")}`);
  for (const name of names) {
    const file = path.join(folder, name);
    if (!fs.lstatSync(file).isFile() || digest(file) !== recorded.files[name]) throw new Error(`the release binary ${name} differs from what compile recorded`);
    fs.chmodSync(file, 0o755);
  }
}
