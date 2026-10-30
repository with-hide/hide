import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { awaitStartedVerification, candidateReceipt, VERIFY_START_LIMIT_MS, withDeviceHelpers, withMacosBinaries } from "../release-candidate.mjs";
import { recordReleaseBinaries } from "../../desktop/scripts/release-binaries.mjs";

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
  schema: 4, repository: "with-hide/hide", public_sha: publicSha, tag: "v1.2.3-rc.1",
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
    assert.throws(() => candidateReceipt(inputs, context, { ...verification, ...patch }), /dispatch while it is pending, runs or after it passed/);
  }
});

// GitHub's answers for the named verify run, one per read, on a clock the waits move.
function verifyQueue(states) {
  const clock = { ms: 0, waits: [] };
  const reads = states.map((state) => ({ ...verification, ...state }));
  return {
    clock,
    read: async () => reads.length > 1 ? reads.shift() : reads[0],
    options: { now: () => clock.ms, wait: async (ms) => { clock.waits.push(ms); clock.ms += ms; } },
  };
}
const pending = { status: "pending", conclusion: null };

test("a candidate waits for a verify run pending behind another main run to start", async () => {
  const queue = verifyQueue([pending, pending, { status: "in_progress", conclusion: null }]);
  const { verification: started, waitedMs } = await awaitStartedVerification(inputs, queue.read, queue.options);
  assert.equal(started.status, "in_progress");
  assert.equal(waitedMs, 60_000);
  assert.deepEqual(queue.clock.waits, [30_000, 30_000]);
  assert.deepEqual(candidateReceipt(inputs, context, started), receipt);
  // A started or passed run is taken at once.
  const ready = verifyQueue([{}]);
  assert.equal((await awaitStartedVerification(inputs, ready.read, ready.options)).waitedMs, 0);
  assert.deepEqual(ready.clock.waits, []);
});

test("a pending verify run that a later merge replaces, or that never starts, refuses the candidate", async () => {
  // pr.yml's concurrency cancels the waiting run when the next main run queues.
  const replaced = verifyQueue([pending, { status: "completed", conclusion: "cancelled" }]);
  const { verification: cancelled } = await awaitStartedVerification(inputs, replaced.read, replaced.options);
  assert.throws(() => candidateReceipt(inputs, context, cancelled), /later main run replaced it/);
  const stuck = verifyQueue([pending]);
  await assert.rejects(awaitStartedVerification(inputs, stuck.read, stuck.options), /still waits behind another main run after 45 minutes/);
  assert.ok(stuck.clock.ms <= VERIFY_START_LIMIT_MS);
  // A wrong run is refused at the first read, not after the wait.
  const wrong = verifyQueue([{ ...pending, head_sha: workflowSha }]);
  await assert.rejects(awaitStartedVerification(inputs, wrong.read, wrong.options), /verify push\/main run of public_sha/);
  assert.deepEqual(wrong.clock.waits, []);
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

test("device helpers complete only a fresh schema 4 receipt", (t) => {
  const { root } = helperArtifacts(t, helperIdentity);
  const complete = withDeviceHelpers(receipt, root);
  assert.throws(() => withDeviceHelpers(complete, root), /once/);
  assert.throws(() => withDeviceHelpers({ ...receipt, schema: 3 }, root), /once/);
});

// The compile artifact as release-candidate.yml uploads it, recorded by the
// packager's own `recordReleaseBinaries`.
function macosArtifact(t, identity) {
  fs.mkdirSync(runs, { recursive: true });
  const root = fs.mkdtempSync(path.join(runs, "macos-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const binaries = path.join(root, "target", "release");
  fs.mkdirSync(binaries, { recursive: true });
  fs.mkdirSync(path.join(root, "desktop", "out"), { recursive: true });
  const names = ["hided", "hide", "hide-agent-hooks"];
  for (const name of names) fs.writeFileSync(path.join(binaries, name), `mach-o ${name}`);
  recordReleaseBinaries(path.join(root, "desktop", "out", "release-binaries.json"), binaries, names, identity);
  const digests = Object.fromEntries(names.map((name) => [name, createHash("sha256").update(`mach-o ${name}`).digest("hex")]));
  return { root, binaries, digests };
}

test("the receipt records the digests of the Mac binaries compiled for its source, version and order", (t) => {
  const { root, digests } = macosArtifact(t, helperIdentity);
  assert.deepEqual(withMacosBinaries(receipt, root), { ...receipt, macos_binaries: digests });
});

test("Mac binaries of another source, version or order, or a changed, missing or linked one, complete no receipt", (t) => {
  for (const patch of [{ commit: workflowSha }, { version: "1.2.3" }, { order: "500" }]) {
    const { root } = macosArtifact(t, { ...helperIdentity, ...patch });
    assert.throws(() => withMacosBinaries(receipt, root), /built for/);
  }
  const changed = macosArtifact(t, helperIdentity);
  fs.appendFileSync(path.join(changed.binaries, "hided"), "changed");
  assert.throws(() => withMacosBinaries(receipt, changed.root), /differs/);
  const missing = macosArtifact(t, helperIdentity);
  fs.rmSync(path.join(missing.binaries, "hide-agent-hooks"));
  assert.throws(() => withMacosBinaries(receipt, missing.root));
  const linked = macosArtifact(t, helperIdentity);
  fs.renameSync(path.join(linked.binaries, "hide"), path.join(linked.root, "hide"));
  fs.symlinkSync(path.join(linked.root, "hide"), path.join(linked.binaries, "hide"));
  assert.throws(() => withMacosBinaries(receipt, linked.root), /differs/);
  assert.throws(() => withMacosBinaries(receipt, path.join(linked.root, "nothing")));
});

test("Mac binaries complete only a fresh schema 4 receipt, beside its device helpers", (t) => {
  const { root, digests } = macosArtifact(t, helperIdentity);
  const helpers = helperArtifacts(t, helperIdentity);
  const complete = withMacosBinaries(withDeviceHelpers(receipt, helpers.root), root);
  assert.deepEqual(complete, { ...receipt, device_helpers: helpers.digests, macos_binaries: digests });
  assert.throws(() => withMacosBinaries(complete, root), /once/);
  assert.throws(() => withMacosBinaries({ ...receipt, schema: 3 }, root), /once/);
});
