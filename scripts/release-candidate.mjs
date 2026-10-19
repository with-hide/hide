// Public, unsigned candidate provenance. The workflow checks main ancestry;
// this boundary refuses unverified sources before any build or keyed journey.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { releaseKind } from "./check-release-assets.mjs";

const REPOSITORY = "with-hide/hide";
const SHA = /^[a-f0-9]{40}$/;
const DECIMAL = /^[1-9][0-9]*$/;

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
    || verification.head_sha !== inputs.public_sha || verification.status !== "completed"
    || verification.conclusion !== "success" || !Array.isArray(verification.pull_requests)
    || verification.pull_requests.length !== 0) {
    throw new Error("verification_run_id must be the successful verify push/main run of public_sha");
  }
  return {
    schema: 1, repository: REPOSITORY, public_sha: inputs.public_sha, tag: inputs.tag,
    build_order: inputs.build_order, verification_run_id: inputs.verification_run_id,
    workflow_sha: context.workflowSha, run_id: context.runId, run_attempt: Number(context.runAttempt),
  };
}

// Only GitHub's runtime-injected variables are read here. Candidate inputs
// come from its event file, not a second application environment contract.
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    if (process.argv.length !== 3) throw new Error("usage: node scripts/release-candidate.mjs verification-run.json");
    const event = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
    const verification = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
    const receipt = candidateReceipt(event.inputs ?? {}, {
      repository: process.env.GITHUB_REPOSITORY, event: process.env.GITHUB_EVENT_NAME,
      ref: process.env.GITHUB_REF, sha: process.env.GITHUB_SHA,
      workflowSha: process.env.GITHUB_WORKFLOW_SHA, runId: process.env.GITHUB_RUN_ID,
      runAttempt: process.env.GITHUB_RUN_ATTEMPT,
    }, verification);
    console.log(JSON.stringify(receipt));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
