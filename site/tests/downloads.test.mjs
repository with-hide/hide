import test from "node:test";
import assert from "node:assert/strict";
import { APP_STORE_URL, heroOffer, phoneOf, platformCards, validateRelease } from "../public/downloads.js";

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
    format: formats[index], name, url: `${base}/download/v1.2.3/${name}`, checksumUrl: `${base}/download/v1.2.3/${name}.sha256`,
  }));
  assert.deepEqual(result.downloads["macos-arm64"].files, offered([filenames[0]], ["dmg"]));
  assert.deepEqual(result.downloads["windows-x64"].files, offered([filenames[1]], ["exe"]));
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

test("the hero offers the one macOS or Windows installer with what opening it shows, unless the processor rules it out", () => {
  const ready = validateRelease(publicRelease());
  const mac = heroOffer(ready, "macos", "");
  assert.equal(mac.label, "Download for macOS");
  assert.equal(mac.detail, "Apple Silicon · macOS 14 or later");
  assert.equal(mac.firstLaunch, "Opens with no warning: drag hide into Applications");
  assert.deepEqual(mac.others, ["Windows", "Linux"]);
  assert.equal(mac.url, `${base}/download/v1.2.3/${filenames[0]}`);
  assert.equal(mac.checksumUrl, `${base}/download/v1.2.3/${filenames[0]}.sha256`);
  assert.equal(mac.releaseUrl, `${base}/tag/v1.2.3`);
  assert.equal(heroOffer(ready, "macos", "arm").url, mac.url);
  const windows = heroOffer(ready, "windows", "");
  assert.equal(windows.url, `${base}/download/v1.2.3/${filenames[1]}`);
  assert.match(windows.firstLaunch, /More info, then Run anyway/);
  assert.deepEqual(windows.others, ["macOS", "Linux"]);
  assert.equal(heroOffer(ready, "macos", "x86"), null);
  assert.equal(heroOffer(ready, "windows", "arm"), null);
  assert.equal(heroOffer(ready, "linux", "x86"), null);
  assert.equal(heroOffer(ready, "", ""), null);
  for (const state of [{ status: "loading" }, { status: "error" }, validateRelease({ ...publicRelease(), prerelease: true })]) {
    assert.equal(heroOffer(state, "macos", ""), null);
  }
});

const card = (view, id) => view.groups.flatMap(group => group.cards).find(item => item.id === id);
const primaries = view => view.groups.flatMap(group => group.cards).flatMap(item => item.actions).filter(action => action.primary);

test("every platform is a card, computers then phones, and the visitor's own carries the one primary button", () => {
  const ready = validateRelease(publicRelease());
  const mac = platformCards(ready, { os: "macos" });
  assert.equal(mac.tag, "v1.2.3");
  assert.deepEqual(mac.groups.map(group => [group.id, group.cards.map(item => item.id)]),
    [["desktop", ["macos", "windows", "linux"]], ["phone", ["ios", "android"]]]);
  assert.equal(card(mac, "macos").mine, true);
  assert.deepEqual(primaries(mac), [{ label: "Download .dmg", href: `${base}/download/v1.2.3/${filenames[0]}`, primary: true }]);
  assert.deepEqual(card(mac, "macos").checksums, [{ label: "SHA-256", href: `${base}/download/v1.2.3/${filenames[0]}.sha256` }]);
  assert.equal(card(mac, "macos").guide, "/docs/start/install-macos/");
  assert.equal(card(mac, "windows").mine, false);
  // Linux has one processor and two packages: both files at once, each with the command that installs it.
  const linux = card(platformCards(ready, { os: "linux" }), "linux");
  assert.deepEqual(linux.actions.map(action => action.label), ["Download .deb", "Download .rpm"]);
  assert.deepEqual(linux.commands, [`sudo apt install ./${filenames[2]}`, `sudo dnf install ./${filenames[3]}`]);
  // With no system suggested nothing is the visitor's, so no button is primary.
  assert.deepEqual(primaries(platformCards(ready)), []);
});

test("when GitHub does not answer, every computer card still opens the latest release and the phone cards are unchanged", () => {
  for (const state of [{ status: "loading" }, { status: "error" }, { status: "unavailable" }, validateRelease({ ...publicRelease(), draft: true })]) {
    const view = platformCards(state, { os: "windows" });
    assert.equal(view.tag, null);
    for (const id of ["macos", "windows", "linux"]) {
      assert.deepEqual(card(view, id).actions.map(action => action.href), [`${base}/latest`]);
      assert.deepEqual(card(view, id).checksums, []);
    }
    assert.equal(card(view, "windows").actions[0].primary, true);
    assert.deepEqual(card(view, "ios").actions.map(action => action.href), [APP_STORE_URL]);
  }
});

test("a processor the one build excludes reads one line and is not the visitor's", () => {
  const ready = validateRelease(publicRelease());
  const intel = card(platformCards(ready, { os: "macos", arch: "x86" }), "macos");
  assert.match(intel.excluded, /Intel processor\. hide runs on Apple Silicon Macs only/);
  assert.equal(intel.mine, false);
  assert.equal(intel.sees, null);
  assert.match(card(platformCards(ready, { os: "windows", arch: "arm" }), "windows").excluded, /ARM processor/);
  assert.equal(card(platformCards(ready, { os: "macos", arch: "arm" }), "macos").excluded, null);
});

test("on a phone the phones come first; the iPhone leads with the App Store and Android has no store to link yet", () => {
  const ready = validateRelease(publicRelease());
  const ios = platformCards(ready, { os: "macos", phone: "ios" });
  assert.deepEqual(ios.groups.map(group => group.id), ["phone", "desktop"]);
  assert.deepEqual(primaries(ios), [{ label: "App Store", href: APP_STORE_URL, primary: true }]);
  assert.equal(card(ios, "macos").mine, false, "a phone suggests no computer");
  const android = card(platformCards(ready, { phone: "android" }), "android");
  assert.equal(android.mine, true);
  assert.equal(android.soon, true);
  assert.deepEqual(android.actions, []);
  assert.equal(android.requirement, "Coming to Google Play");
});

test("only an iPhone or an Android phone counts as a phone", () => {
  const iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
  const android = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Mobile Safari/537.36";
  const tablet = "Mozilla/5.0 (Linux; Android 15; SM-X910) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";
  const ipad = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
  assert.equal(phoneOf(iphone), "ios");
  assert.equal(phoneOf(android), "android");
  assert.equal(phoneOf(tablet), "");
  assert.equal(phoneOf(tablet, true), "android");
  assert.equal(phoneOf(ipad), "");
});
