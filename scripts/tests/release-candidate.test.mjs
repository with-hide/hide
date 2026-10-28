import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { candidateReceipt, withDeviceHelpers } from "../release-candidate.mjs";

// The producer/consumer contract: public dispatch inputs, one receipt,
// the exact push/main verification it names, never private revision metadata.
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
const receipt = {
  schema: 3, repository: "with-hide/hide", public_sha: publicSha, tag: "v1.2.3-rc.1",
  build_order: "501", verification_run_id: "71", workflow_sha: workflowSha, run_id: "81", run_attempt: 2,
  live_agents: false,
};

test("a verified candidate records public source, workflow, version and exact run attempt", () => {
  assert.deepEqual(candidateReceipt(inputs, context, verification), receipt);
  assert.equal(candidateReceipt({ ...inputs, tag: "v1.2.3" }, context, verification).tag, "v1.2.3");
});

test("a candidate may start while the named verify run runs, and the receipt names that run", () => {
  for (const status of ["queued", "in_progress"]) {
    assert.deepEqual(candidateReceipt(inputs, context, { ...verification, status, conclusion: null }), receipt);
  }
});

test("a wrong, waiting, failed or PR verification cannot authorize a candidate", () => {
  for (const patch of [
    { id: 72 }, { repository: { full_name: "other/hide" } }, { path: ".github/workflows/nightly.yml" },
    { name: "another check" }, { event: "pull_request" }, { head_branch: "feature" },
    { head_sha: workflowSha }, { pull_requests: [{ number: 1 }] }, { pull_requests: undefined },
  ]) {
    assert.throws(() => candidateReceipt(inputs, context, { ...verification, ...patch }), /verify push\/main run of public_sha/);
  }
  for (const patch of [
    { status: "pending", conclusion: null }, { status: "waiting", conclusion: null }, { status: "requested", conclusion: null },
    { conclusion: "failure" }, { conclusion: "cancelled" }, { conclusion: "skipped" }, { conclusion: "timed_out" },
  ]) {
    assert.throws(() => candidateReceipt(inputs, context, { ...verification, ...patch }), /dispatch while it runs or after it passed/);
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

// Both helper artifacts as device-helpers.yml uploads them; a minimal
// executable header stands in for each Linux program.
const runs = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../agents/runs/release-candidate-tests");
function helperArtifacts(t, identity) {
  fs.mkdirSync(runs, { recursive: true });
  const root = fs.mkdtempSync(path.join(runs, "helpers-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const digests = {};
  for (const [platform, machine] of [["linux-x86_64", 62], ["linux-aarch64", 183]]) {
    fs.mkdirSync(path.join(root, platform));
    const files = {};
    for (const name of ["hided", "hide", "hide-agent-hooks"]) {
      const bytes = Buffer.alloc(64);
      bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1]);
      bytes.writeUInt16LE(3, 16);
      bytes.writeUInt16LE(machine, 18);
      bytes.write(name, 32);
      fs.writeFileSync(path.join(root, platform, name), bytes);
      files[name] = createHash("sha256").update(bytes).digest("hex");
    }
    fs.writeFileSync(path.join(root, platform, "manifest.json"), JSON.stringify({ platform, ...identity, files }));
    digests[platform] = files;
  }
  return { root, digests };
}
const helperIdentity = { commit: publicSha, version: "1.2.3-rc.1", order: "501" };

test("the receipt records the digests of both device helpers built for its source, version and order", (t) => {
  const { root, digests } = helperArtifacts(t, helperIdentity);
  assert.deepEqual(withDeviceHelpers(receipt, root), { ...receipt, device_helpers: digests });
});

test("helpers of another source, version or order, or a changed helper, complete no receipt", (t) => {
  for (const patch of [{ commit: workflowSha }, { version: "1.2.3" }, { order: "500" }]) {
    const { root } = helperArtifacts(t, { ...helperIdentity, ...patch });
    assert.throws(() => withDeviceHelpers(receipt, root), /identity mismatch/);
  }
  const { root } = helperArtifacts(t, helperIdentity);
  fs.appendFileSync(path.join(root, "linux-aarch64", "hided"), "changed");
  assert.throws(() => withDeviceHelpers(receipt, root), /digest mismatch/);
  fs.rmSync(path.join(root, "linux-aarch64"), { recursive: true });
  assert.throws(() => withDeviceHelpers(receipt, root));
});

test("device helpers complete only a fresh schema 3 receipt", (t) => {
  const { root } = helperArtifacts(t, helperIdentity);
  const complete = withDeviceHelpers(receipt, root);
  assert.throws(() => withDeviceHelpers(complete, root), /once/);
  assert.throws(() => withDeviceHelpers({ ...receipt, schema: 2 }, root), /once/);
});
