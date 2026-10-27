import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import { createGitHubClient, prepareDraft, publishDraft } from "../release-draft.mjs";

const tag = "v1.2.3";
const rc = "v0.4.0-rc.1";
const sha = "1".repeat(40);
const marker = `<!-- hide-release-source:${sha} -->`;
/** The installers each tag ships (PRD auto-update D-35), each with a checksum. */
const namesOf = (tag) => ["macos-arm64.dmg", "macos-arm64.zip", "windows-x64-setup.exe", "linux-x64.deb", "linux-x64.rpm"].map((s) => `hide-${tag}-${s}`);
/** Which installers each update file names. */
const updatesOf = (tag) => ({
  "latest-mac.yml": [`hide-${tag}-macos-arm64.zip`],
  "latest.yml": [`hide-${tag}-windows-x64-setup.exe`],
  "latest-linux.yml": [`hide-${tag}-linux-x64.deb`, `hide-${tag}-linux-x64.rpm`],
});
const blockmapsOf = (tag) => [`hide-${tag}-macos-arm64.zip.blockmap`, `hide-${tag}-windows-x64-setup.exe.blockmap`];
const names = namesOf(tag);
/** Five installers and checksums, three update files, two blockmaps, two Linux kits and their manifest. */
const ASSET_COUNT = 18;
const digest = (data) => `sha256:${createHash("sha256").update(data).digest("hex")}`;

async function fixture(t, options = {}) {
  const tag = options.tag ?? "v1.2.3";
  const runs = path.resolve("agents/runs/release-draft-tests");
  await fs.mkdir(runs, { recursive: true });
  const directory = await fs.mkdtemp(path.join(runs, "fixture-"));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const files = new Map();
  for (const name of namesOf(tag)) {
    // Fixture bytes; no installer is run.
    const data = gzipSync(Buffer.from(name));
    files.set(name, data);
    files.set(`${name}.sha256`, Buffer.from(`${digest(data).slice(7)}  ${name}\n`));
  }
  for (const name of blockmapsOf(tag)) files.set(name, gzipSync(Buffer.from(name)));
  for (const [info, updates] of Object.entries(updatesOf(tag))) {
    const entries = updates.map((name) => ({ name, sha512: createHash("sha512").update(files.get(name)).digest("base64"), size: files.get(name).length }));
    const lines = [`version: ${tag.slice(1)}`, "files:", ...entries.flatMap((e) => [`  - url: ${e.name}`, `    sha512: ${e.sha512}`, `    size: ${e.size}`]), `path: ${entries[0].name}`, `sha512: ${entries[0].sha512}`, "releaseDate: '2026-10-16T00:00:00.000Z'"];
    files.set(info, Buffer.from(`${lines.join("\n")}\n`));
  }
  const platforms = {};
  for (const platform of ["linux-x86_64", "linux-aarch64"]) {
    const asset = `hide-device-${platform}-${tag.slice(1)}.tar.gz`;
    const data = gzipSync(Buffer.from(asset));
    files.set(asset, data);
    const programs = Object.fromEntries(["hided", "hide", "hide-agent-hooks", "herdr"].map((name) => [name, { size: 64, sha256: "a".repeat(64) }]));
    platforms[platform] = { asset, size: data.length, sha256: digest(data).slice(7), files: programs };
  }
  files.set("device-bundles.json", Buffer.from(JSON.stringify({ format: 1, version: tag.slice(1), commit: sha, order: "42", platforms })));
  for (const [name, data] of files) await fs.writeFile(path.join(directory, name), data);
  const state = {
    release: options.release ? structuredClone(options.release) : null,
    assets: options.assets ? structuredClone(options.assets) : [],
    requests: [], writes: [], assetGets: 0, created: [], publications: [],
  };
  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, "http://fixture.invalid");
    const route = url.pathname.replace("/repos/example/project", "");
    const data = [];
    for await (const part of req) data.push(part);
    const bytes = Buffer.concat(data);
    state.requests.push({ method: req.method, route });
    if (req.method !== "GET") state.writes.push({ method: req.method, route });
    const answer = (status, body) => { res.writeHead(status, { "Content-Type": "application/json" }); res.end(JSON.stringify(body)); };
    if (options.fail === route) return answer(options.status ?? 500, { message: "fixture failure" });
    if (route === `/git/ref/tags/${tag}`) return answer(200, { object: { type: options.annotated ? "tag" : "commit", sha: options.tagSha ?? sha } });
    if (route === `/git/tags/${sha}`) return answer(200, { object: { type: "commit", sha } });
    if (route === "/releases" && req.method === "GET") {
      const page = Number(url.searchParams.get("page"));
      if (options.pages) return answer(200, options.pages[page - 1] ?? []);
      return answer(200, state.release ? [{ ...state.release, assets: state.assets }] : []);
    }
    if (route === "/releases" && req.method === "POST") {
      const payload = JSON.parse(bytes);
      // GitHub answers a target commit for an existing tag under immutable releases this way.
      if ("target_commitish" in payload) return answer(403, { message: "Resource not accessible by integration" });
      state.created.push(payload);
      state.release = { ...payload, id: 7 };
      return answer(201, state.release);
    }
    if (route === "/releases/7" && req.method === "PATCH") {
      state.publications.push(JSON.parse(bytes));
      state.release = { ...state.release, draft: false };
      return answer(200, { ...state.release, ...options.publishAnswer });
    }
    if (route === "/releases/7" && req.method === "GET") {
      if (options.publishBeforeRead) state.release.draft = false;
      if (options.wrongId) return answer(200, { ...state.release, id: 8 });
      return answer(200, state.release);
    }
    if (route === "/releases/7/assets" && req.method === "GET") {
      state.assetGets++;
      return answer(200, state.assets);
    }
    if (route === "/releases/7/assets" && req.method === "POST") {
      if (Number(req.headers["content-length"]) !== bytes.length) return answer(411, { message: "asset byte length is required" });
      // A publication after the client's last GET is blocked by the immutable releases setting.
      if (options.publishDuringUpload) state.release.draft = false;
      if (!state.release.draft) return answer(422, { message: "immutable published release" });
      const name = url.searchParams.get("name");
      if (state.assets.some((a) => a.name === name)) return answer(422, { message: "duplicate asset" });
      const asset = { name, size: bytes.length, state: "uploaded", digest: digest(bytes) };
      state.assets.push(asset);
      return answer(201, asset);
    }
    answer(404, { message: "unhandled fixture route" });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => { server.closeAllConnections(); server.close(resolve); }));
  const base = `http://127.0.0.1:${server.address().port}`;
  const client = createGitHubClient({ repository: "example/project", writeToken: "fixture-write", fetchImpl: (url, opts) => fetch(`${base}${new URL(url).pathname}${new URL(url).search}`, opts) });
  return { state, directory, files, run: () => prepareDraft({ tag, sha, directory, client }), publish: () => publishDraft({ tag, sha, directory, client }) };
}

const draft = () => ({ id: 7, tag_name: tag, body: marker, draft: true, prerelease: false });
const rcDraft = () => ({ ...draft(), tag_name: rc, prerelease: true });

test("complete fresh draft posts every matching asset without PATCH or DELETE; retry writes nothing", async (t) => {
  const f = await fixture(t);
  assert.deepEqual(await f.run(), { id: 7, tag, assets: ASSET_COUNT });
  // The draft, then one upload per asset.
  assert.equal(f.state.writes.length, ASSET_COUNT + 1);
  assert.ok(f.state.writes.every((r) => r.method === "POST"));
  assert.equal(f.state.assets.length, ASSET_COUNT);
  for (const a of f.state.assets) assert.equal(a.digest, digest(f.files.get(a.name)));
  f.state.writes.length = 0;
  await f.run();
  assert.deepEqual(f.state.writes, []);
});

test("published, legacy and ambiguous drafts stop without mutation", async (t) => {
  for (const options of [{ release: { ...draft(), draft: false } }, { release: { ...draft(), body: "old draft" } }, { pages: [[{ ...draft(), assets: [] }, { ...draft(), assets: [] }]] }]) {
    const f = await fixture(t, options);
    await assert.rejects(f.run());
    assert.deepEqual(f.state.writes, []);
  }
});

test("a stale draft beyond two full pages is found, and listing failure is not absence", async (t) => {
  const unrelated = Array.from({ length: 100 }, (_, i) => ({ tag_name: `v0.0.${i}` }));
  const f = await fixture(t, { pages: [unrelated, unrelated, [{ ...draft(), assets: [{ name: "hide-v1.2.2-macos-arm64.zip" }] }]] });
  await assert.rejects(f.run());
  assert.deepEqual(f.state.writes, []);
  assert.equal(f.state.requests.filter((r) => r.route === "/releases").length, 3);
  const failed = await fixture(t, { fail: "/releases", status: 429 });
  await assert.rejects(failed.run());
  assert.deepEqual(failed.state.writes, []);
});

test("matching partial uploaded draft resumes; mismatched, starter or duplicate assets are preserved", async (t) => {
  const f = await fixture(t, { release: draft() });
  const name = names[0], data = f.files.get(name);
  f.state.assets.push({ name, size: data.length, state: "uploaded", digest: digest(data) });
  await f.run();
  assert.equal(f.state.writes.length, ASSET_COUNT - 1);
  assert.ok(f.state.writes.every((r) => r.route === "/releases/7/assets"));
  for (const change of [{ digest: "sha256:" + "0".repeat(64) }, { size: 1 }, { state: "starter" }]) {
    const bad = await fixture(t, { release: draft(), assets: [{ name, size: data.length, state: "uploaded", digest: digest(data), ...change }] });
    await assert.rejects(bad.run());
    assert.deepEqual(bad.state.writes, []);
    assert.equal(bad.state.assets.length, 1);
  }
});

test("publication before GET and between GET and upload cannot replace public assets", async (t) => {
  for (const options of [{ publishBeforeRead: true }, { publishDuringUpload: true }]) {
    const f = await fixture(t, { release: draft(), ...options });
    await assert.rejects(f.run());
    assert.equal(f.state.assets.length, 0);
    assert.ok(f.state.writes.every((r) => r.method === "POST" && r.route === "/releases/7/assets"));
    if (options.publishBeforeRead) assert.deepEqual(f.state.writes, []);
  }
});

test("moved tag, wrong release ID and incomplete local package fail; annotated tag works", async (t) => {
  const moved = await fixture(t, { tagSha: "2".repeat(40) });
  await assert.rejects(moved.run());
  assert.deepEqual(moved.state.writes, []);
  const wrong = await fixture(t, { release: draft(), wrongId: true });
  await assert.rejects(wrong.run());
  assert.deepEqual(wrong.state.writes, []);
  const local = await fixture(t);
  await fs.unlink(path.join(local.directory, names[0]));
  await assert.rejects(local.run());
  assert.deepEqual(local.state.requests, []);
  const annotated = await fixture(t, { annotated: true });
  assert.equal((await annotated.run()).assets, ASSET_COUNT);
});

test("a complete verified draft is published once, as the latest, after its last read", async (t) => {
  const f = await fixture(t);
  assert.deepEqual(await f.publish(), { id: 7, tag, assets: ASSET_COUNT });
  assert.deepEqual(f.state.created, [{ tag_name: tag, name: tag, body: marker, draft: true, prerelease: false }]);
  assert.deepEqual(f.state.publications, [{ draft: false, make_latest: "true" }]);
  assert.deepEqual(f.state.writes.at(-1), { method: "PATCH", route: "/releases/7" });
  assert.equal(f.state.writes.filter((r) => r.method === "PATCH").length, 1);
  assert.equal(f.state.release.draft, false);
});

test("a moved tag, an incomplete package or differing remote bytes publish nothing", async (t) => {
  const moved = await fixture(t, { tagSha: "2".repeat(40) });
  await assert.rejects(moved.publish());
  assert.deepEqual(moved.state.publications, []);
  const local = await fixture(t);
  await fs.unlink(path.join(local.directory, names[1]));
  await assert.rejects(local.publish());
  assert.deepEqual(local.state.requests, []);
  const differing = await fixture(t, { release: draft(), assets: [{ name: names[0], size: 1, state: "uploaded", digest: digest(Buffer.from("x")) }] });
  await assert.rejects(differing.publish());
  assert.deepEqual(differing.state.publications, []);
});

test("an answer that is not the verified release, published, fails the run", async (t) => {
  const f = await fixture(t, { publishAnswer: { draft: true } });
  await assert.rejects(f.publish(), /not the verified release, published/);
  assert.equal(f.state.publications.length, 1);
});

test("an -rc tag drafts a pre-release with its own assets and publishes it, never as the latest", async (t) => {
  const f = await fixture(t, { tag: rc });
  assert.deepEqual(await f.publish(), { id: 7, tag: rc, assets: ASSET_COUNT });
  assert.deepEqual(f.state.created, [{ tag_name: rc, name: rc, body: marker, draft: true, prerelease: true }]);
  assert.deepEqual(f.state.assets.map((a) => a.name).sort(), [
    "hide-v0.4.0-rc.1-linux-x64.deb", "hide-v0.4.0-rc.1-linux-x64.deb.sha256",
    "hide-v0.4.0-rc.1-linux-x64.rpm", "hide-v0.4.0-rc.1-linux-x64.rpm.sha256",
    "hide-v0.4.0-rc.1-macos-arm64.dmg", "hide-v0.4.0-rc.1-macos-arm64.dmg.sha256",
    "hide-v0.4.0-rc.1-macos-arm64.zip", "hide-v0.4.0-rc.1-macos-arm64.zip.blockmap", "hide-v0.4.0-rc.1-macos-arm64.zip.sha256",
    "hide-v0.4.0-rc.1-windows-x64-setup.exe", "hide-v0.4.0-rc.1-windows-x64-setup.exe.blockmap", "hide-v0.4.0-rc.1-windows-x64-setup.exe.sha256",
    "latest-linux.yml", "latest-mac.yml", "latest.yml",
    "device-bundles.json", "hide-device-linux-aarch64-0.4.0-rc.1.tar.gz", "hide-device-linux-x86_64-0.4.0-rc.1.tar.gz",
  ].sort());
  assert.deepEqual(f.state.publications, [{ draft: false, make_latest: "false" }]);
  assert.equal(f.state.writes.filter((r) => r.method === "PATCH").length, 1);
});

test("a draft or a published answer of the other release kind stops the run", async (t) => {
  for (const [kind, release] of [[tag, { ...draft(), prerelease: true }], [rc, { ...rcDraft(), prerelease: false }]]) {
    const f = await fixture(t, { tag: kind, release });
    await assert.rejects(f.publish());
    assert.deepEqual(f.state.writes, []);
  }
  const resumed = await fixture(t, { tag: rc, release: rcDraft() });
  assert.equal((await resumed.run()).assets, ASSET_COUNT);
  const answered = await fixture(t, { tag: rc, publishAnswer: { prerelease: false } });
  await assert.rejects(answered.publish(), /not the verified release, published/);
});


test("device manifest from another source blocks draft creation before any network request", async (t) => {
  const f = await fixture(t);
  const file = path.join(f.directory, "device-bundles.json");
  const manifest = JSON.parse(await fs.readFile(file, "utf8"));
  manifest.commit = "2".repeat(40);
  await fs.writeFile(file, JSON.stringify(manifest));
  await assert.rejects(f.run(), /this release, source/);
  assert.deepEqual(f.state.requests, []);
});

test("a partial draft containing a matching Linux bundle resumes without replacing it", async (t) => {
  const f = await fixture(t, { release: draft() });
  const name = "hide-device-linux-aarch64-1.2.3.tar.gz", data = f.files.get(name);
  f.state.assets.push({ name, size: data.length, state: "uploaded", digest: digest(data) });
  assert.equal((await f.run()).assets, 18);
  assert.equal(f.state.assets.filter((asset) => asset.name === name).length, 1);
  assert.equal(f.state.writes.length, 17);
});
