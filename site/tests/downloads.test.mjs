import test from "node:test";
import assert from "node:assert/strict";
import { heroOffer, validateRelease } from "../downloads.js";

// The installers a release carries (PRD auto-update D-35), each with its checksum.
const base = "https://github.com/with-hide/hide/releases";
const filenames = [
  "hide-v1.2.3-macos-arm64.dmg",
  "hide-v1.2.3-windows-x64-setup.exe",
  "hide-v1.2.3-linux-x64.deb",
  "hide-v1.2.3-linux-x64.rpm",
];
function publicRelease() {
  return {
    draft: false,
    prerelease: false,
    tag_name: "v1.2.3",
    html_url: `${base}/tag/v1.2.3`,
    assets: filenames.flatMap(name => [name, `${name}.sha256`]).map(name => ({
      name, state: "uploaded", size: 123,
      browser_download_url: `${base}/download/v1.2.3/${name}`,
    })),
  };
}
const unavailable = release => assert.equal(validateRelease(release).status, "unavailable");

test("one complete stable release returns every installer and checksum at its own tag, both Linux packages included", () => {
  const result = validateRelease(publicRelease());
  assert.equal(result.status, "ready");
  assert.equal(result.tag, "v1.2.3");
  assert.equal(result.releaseUrl, `${base}/tag/v1.2.3`);
  const offered = (names, formats) => names.map((name, index) => ({
    format: formats[index], url: `${base}/download/v1.2.3/${name}`, checksumUrl: `${base}/download/v1.2.3/${name}.sha256`,
  }));
  assert.deepEqual(result.downloads["macos-arm64"].files, offered([filenames[0]], ["dmg"]));
  assert.deepEqual(result.downloads["windows-x64"].files, offered([filenames[1]], ["installer"]));
  assert.deepEqual(result.downloads["linux-x64"].files, offered(filenames.slice(2), ["deb", "rpm"]));
  assert.equal(result.downloads["macos-x64"], undefined);
  assert.equal(result.downloads["windows-arm64"], undefined);
});

test("draft, prerelease and missing publication flags never offer downloads", () => {
  for (const flags of [{ draft: true }, { prerelease: true }, { draft: undefined }, { prerelease: undefined }, { draft: "false" }]) {
    unavailable({ ...publicRelease(), ...flags });
  }
});

test("only an explicit stable version tag is accepted", () => {
  for (const tag_name of ["", "latest", "1.2.3", "v1.2", "v1.2.3-beta.1", "v01.2.3", "v1.2.3/other", "v1.2.3\n"]) {
    const release = publicRelease();
    release.tag_name = tag_name;
    release.html_url = `${base}/tag/${tag_name}`;
    for (const asset of release.assets) {
      asset.name = asset.name.replace("v1.2.3", tag_name);
      asset.browser_download_url = `${base}/download/${tag_name}/${asset.name}`;
    }
    unavailable(release);
  }
  const release = publicRelease();
  release.tag_name = "v0.0.0";
  release.html_url = `${base}/tag/v0.0.0`;
  for (const asset of release.assets) {
    asset.name = asset.name.replace("v1.2.3", "v0.0.0");
    asset.browser_download_url = `${base}/download/v0.0.0/${asset.name}`;
  }
  assert.equal(validateRelease(release).status, "ready");
});

test("any missing installer or checksum withholds every platform", () => {
  for (let missing = 0; missing < filenames.length * 2; missing++) {
    const release = publicRelease();
    release.assets.splice(missing, 1);
    unavailable(release);
  }
});

test("empty, pending, duplicate or differently named assets are incomplete", () => {
  for (const size of [0, -1, 1.5, "123", null, Infinity]) {
    const release = publicRelease();
    release.assets[0].size = size;
    unavailable(release);
  }
  for (const state of ["new", "starter", undefined]) {
    const release = publicRelease();
    release.assets[5].state = state;
    unavailable(release);
  }
  const duplicate = publicRelease();
  duplicate.assets.push({ ...duplicate.assets[0] });
  unavailable(duplicate);
  const different = publicRelease();
  different.assets[0].name = "hide-v1.2.3-macos-arm64.zip";
  unavailable(different);
});

test("release and asset URLs must name this repository, exact tag and filename", () => {
  for (const html_url of [`${base}/latest`, `${base}/tag/v1.2.2`, "https://example.com/releases/tag/v1.2.3"]) {
    unavailable({ ...publicRelease(), html_url });
  }
  for (const url of [
    `${base}/download/v1.2.2/${filenames[0]}`, `${base}/download/v1.2.3/wrong.zip`,
    `http://github.com/with-hide/hide/releases/download/v1.2.3/${filenames[0]}`,
    `https://github.com.example.com/with-hide/hide/releases/download/v1.2.3/${filenames[0]}`,
    `https://github.com@evil.example/with-hide/hide/releases/download/v1.2.3/${filenames[0]}`,
    `javascript:alert(1)`, `${base}/download/v1.2.3/${filenames[0]}?token=other`,
  ]) {
    const release = publicRelease();
    release.assets[0].browser_download_url = url;
    unavailable(release);
  }
});

test("malformed API data fails closed; unrelated assets do not replace required ones", () => {
  for (const release of [null, {}, [], { ...publicRelease(), assets: null }, { ...publicRelease(), assets: [null] }]) unavailable(release);
  const release = publicRelease();
  release.assets.push({ name: "source-code.zip", size: 0 });
  assert.equal(validateRelease(release).status, "ready");
});

test("the hero offers the one macOS or Windows installer with its checksum and notes, unless the processor rules it out", () => {
  const ready = validateRelease(publicRelease());
  const mac = heroOffer(ready, "macos", "");
  assert.equal(mac.label, "Download for macOS");
  assert.equal(mac.detail, "Apple Silicon · macOS 14 or later");
  assert.equal(mac.url, `${base}/download/v1.2.3/${filenames[0]}`);
  assert.equal(mac.checksumUrl, `${base}/download/v1.2.3/${filenames[0]}.sha256`);
  assert.equal(mac.releaseUrl, `${base}/tag/v1.2.3`);
  assert.equal(heroOffer(ready, "macos", "arm").url, mac.url);
  assert.equal(heroOffer(ready, "windows", "").url, `${base}/download/v1.2.3/${filenames[1]}`);
  assert.equal(heroOffer(ready, "macos", "x86"), null);
  assert.equal(heroOffer(ready, "windows", "arm"), null);
  assert.equal(heroOffer(ready, "linux", "x86"), null);
  assert.equal(heroOffer(ready, "", ""), null);
  for (const state of [{ status: "loading" }, { status: "error" }, validateRelease({ ...publicRelease(), prerelease: true })]) {
    assert.equal(heroOffer(state, "macos", ""), null);
  }
});
