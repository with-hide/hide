// Public, unsigned candidate provenance. The workflow checks main ancestry;
// this boundary refuses unverified sources before any build or journey.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { releaseKind } from "./check-release-assets.mjs";
import { deviceHelperDigests, HELPER_PROGRAMS } from "../desktop/scripts/device-helpers.mjs";
import { checkReleaseBinaries } from "../desktop/scripts/release-binaries.mjs";

const REPOSITORY = "with-hide/hide";
const SHA = /^[a-f0-9]{40}$/;
const DECIMAL = /^[1-9][0-9]*$/;
// A verify run that has started, or has passed. A `pending` run waits behind
// another main run, and the next push to main replaces it (pr.yml's
// concurrency), so a candidate naming it could name a run that never runs.
const STARTED = new Set(["queued", "in_progress"]);

export function candidateReceipt(inputs, context, verification) {
  if (context.repository !== REPOSITORY || context.event !== "workflow_dispatch"
    || context.ref !== "refs/heads/main" || !SHA.test(context.workflowSha)
    || context.sha !== context.workflowSha) {
    throw new Error("dispatch the candidate workflow from with-hide/hide main");
  }
  releaseKind(inputs.tag);
  if (!SHA.test(inputs.public_sha)) throw new Error("public_sha must be a full lowercase commit SHA");
  for (const [key, value] of Object.entries({
    verification_run_id: inputs.verification_run_id, build_order: inputs.build_order,
    run_id: context.runId, run_attempt: context.runAttempt,
  })) {
    if (typeof value !== "string" || !DECIMAL.test(value) || !Number.isSafeInteger(Number(value))) {
      throw new Error(`${key} must be a positive safe decimal integer`);
    }
  }
  if (String(verification.id) !== inputs.verification_run_id
    || verification.repository?.full_name !== REPOSITORY
    || verification.path !== ".github/workflows/pr.yml" || verification.name !== "verify"
    || verification.event !== "push" || verification.head_branch !== "main"
    || verification.head_sha !== inputs.public_sha || !Array.isArray(verification.pull_requests)
    || verification.pull_requests.length !== 0) {
    throw new Error("verification_run_id must be the verify push/main run of public_sha");
  }
  if (!STARTED.has(verification.status) && !(verification.status === "completed" && verification.conclusion === "success")) {
    throw new Error(`verify run ${verification.id} is ${verification.status}${verification.conclusion ? `/${verification.conclusion}` : ""}: `
      + "dispatch while it runs or after it passed, never while it waits behind another main run or after it failed");
  }
  return {
    schema: 4, repository: REPOSITORY, public_sha: inputs.public_sha, tag: inputs.tag,
    build_order: inputs.build_order, verification_run_id: inputs.verification_run_id,
    workflow_sha: context.workflowSha, run_id: context.runId, run_attempt: Number(context.runAttempt),
    // No candidate calls a signed-in provider since 2026-10-27 (docs/BUILD.md,
    // Unsigned release candidates); the field stays because its consumer reads it.
    live_agents: false,
  };
}

const identityOf = (receipt) => ({ commit: receipt.public_sha, version: receipt.tag.replace(/^v/, ""), order: receipt.build_order });

/** The receipt with the digests of both device helpers this run built, checked against its identity. */
export function withDeviceHelpers(receipt, directory) {
  if (receipt.schema !== 4 || "device_helpers" in receipt) throw new Error("device helpers complete a schema 4 receipt once");
  return { ...receipt, device_helpers: deviceHelperDigests(directory, identityOf(receipt)) };
}

/**
 * The receipt with the digests of the Mac release binaries this run compiled,
 * which the private publisher assembles, signs and notarizes without building
 * them again. `directory` holds the compile artifact as uploaded: the binaries
 * under target/release and their record under desktop/out.
 */
export function withMacosBinaries(receipt, directory) {
  if (receipt.schema !== 4 || "macos_binaries" in receipt) throw new Error("the Mac binaries complete a schema 4 receipt once");
  const record = path.join(directory, "desktop", "out", "release-binaries.json");
  checkReleaseBinaries(record, path.join(directory, "target", "release"), HELPER_PROGRAMS, identityOf(receipt));
  return { ...receipt, macos_binaries: JSON.parse(fs.readFileSync(record, "utf8")).files };
}

// Only GitHub's runtime-injected variables are read here. Candidate inputs
// come from its event file, not a second application environment contract.
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const [action, ...files] = process.argv.slice(2);
    let receipt;
    if (action === "source" && files.length === 1) {
      const event = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
      const verification = JSON.parse(fs.readFileSync(files[0], "utf8"));
      receipt = candidateReceipt(event.inputs ?? {}, {
        repository: process.env.GITHUB_REPOSITORY, event: process.env.GITHUB_EVENT_NAME,
        ref: process.env.GITHUB_REF, sha: process.env.GITHUB_SHA,
        workflowSha: process.env.GITHUB_WORKFLOW_SHA, runId: process.env.GITHUB_RUN_ID,
        runAttempt: process.env.GITHUB_RUN_ATTEMPT,
      }, verification);
    } else if (action === "builds" && files.length === 3) {
      receipt = withMacosBinaries(withDeviceHelpers(JSON.parse(fs.readFileSync(files[0], "utf8")), files[1]), files[2]);
    } else {
      throw new Error("usage: node scripts/release-candidate.mjs source verification-run.json | builds receipt.json device-helpers-folder macos-binaries-folder");
    }
    console.log(JSON.stringify(receipt));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
