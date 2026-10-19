import test from "node:test";
import assert from "node:assert/strict";
import { candidateReceipt } from "../release-candidate.mjs";

// The producer/consumer contract: four public dispatch inputs, one receipt,
// exact successful push/main verification, never private revision metadata.
const publicSha = "a".repeat(40);
const workflowSha = "b".repeat(40);
const inputs = { public_sha: publicSha, tag: "v1.2.3-rc.1", verification_run_id: "71", build_order: "501" };
const context = {
  repository: "with-hide/hide", event: "workflow_dispatch", ref: "refs/heads/main",
  sha: workflowSha, workflowSha, runId: "81", runAttempt: "2",
};
const verification = {
  id: 71, repository: { full_name: "with-hide/hide" }, path: ".github/workflows/pr.yml", name: "verify",
  event: "push", head_branch: "main", head_sha: publicSha,
  status: "completed", conclusion: "success", pull_requests: [],
};

test("a verified candidate records public source, workflow, version and exact run attempt", () => {
  assert.deepEqual(candidateReceipt(inputs, context, verification), {
    schema: 1, repository: "with-hide/hide", public_sha: publicSha, tag: "v1.2.3-rc.1",
    build_order: "501", verification_run_id: "71", workflow_sha: workflowSha, run_id: "81", run_attempt: 2,
  });
  assert.equal(candidateReceipt({ ...inputs, tag: "v1.2.3" }, context, verification).tag, "v1.2.3");
});

test("a wrong, partial, failed or PR verification cannot authorize a candidate", () => {
  for (const patch of [
    { id: 72 }, { repository: { full_name: "other/hide" } }, { path: ".github/workflows/nightly.yml" },
    { name: "another check" }, { event: "pull_request" }, { head_branch: "feature" },
    { head_sha: workflowSha }, { status: "in_progress" }, { conclusion: "failure" },
    { conclusion: "cancelled" }, { conclusion: "skipped" }, { pull_requests: [{ number: 1 }] },
    { pull_requests: undefined },
  ]) {
    assert.throws(() => candidateReceipt(inputs, context, { ...verification, ...patch }), /successful verify push\/main/);
  }
});

test("candidate provenance cannot be issued from a branch, fork or different workflow revision", () => {
  for (const patch of [
    { repository: "fork/hide" }, { event: "pull_request" }, { ref: "refs/heads/feature" },
    { workflowSha: publicSha }, { workflowSha: "main" }, { runId: "0" }, { runAttempt: "0" },
  ]) assert.throws(() => candidateReceipt(inputs, { ...context, ...patch }, verification));
});

test("ambiguous sources, versions and build orders fail before packaging", () => {
  for (const patch of [
    { public_sha: "main" }, { public_sha: publicSha.toUpperCase() }, { public_sha: publicSha.slice(0, 12) },
    { tag: "v1.2.3-beta.1" }, { tag: "v1.2.3-rc.0" }, { tag: "v01.2.3" },
    { build_order: "0" }, { build_order: "01" }, { build_order: "1\n2" }, { build_order: "9007199254740992" },
    { build_order: 1 }, { verification_run_id: "71/attempts/1" }, { verification_run_id: "" },
  ]) assert.throws(() => candidateReceipt({ ...inputs, ...patch }, context, verification));
  for (const build_order of ["1", "9007199254740991"]) {
    assert.equal(candidateReceipt({ ...inputs, build_order }, context, verification).build_order, build_order);
  }
});
