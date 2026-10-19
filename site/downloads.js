const RELEASES_URL = "https://github.com/with-hide/hide/releases";
const LATEST_API = "https://api.github.com/repos/with-hide/hide/releases/latest";
// The installers a person downloads (PRD auto-update D-35); Linux offers both packages.
const PLATFORMS = Object.freeze({
  "macos-arm64": { label: "macOS · Apple Silicon (ARM64)", files: [{ suffix: "macos-arm64.dmg", format: "dmg" }] },
  "windows-x64": { label: "Windows · Intel or AMD (x64)", files: [{ suffix: "windows-x64-setup.exe", format: "installer" }] },
  "linux-x64": { label: "Linux · Intel or AMD (x64)", files: [{ suffix: "linux-x64.deb", format: "deb" }, { suffix: "linux-x64.rpm", format: "rpm" }] },
});

/** A public, stable, complete release is the only source of installer links. */
export function validateRelease(release) {
  if (!release || release.draft !== false || release.prerelease !== false
    || typeof release.tag_name !== "string"
    || release.tag_name.trim() !== release.tag_name
    || !/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(release.tag_name)) {
    return { status: "unavailable", reason: "not-stable" };
  }
  const tag = release.tag_name;
  const releaseUrl = `${RELEASES_URL}/tag/${tag}`;
  if (release.html_url !== releaseUrl || !Array.isArray(release.assets)) {
    return { status: "unavailable", reason: "invalid-release" };
  }
  const downloads = {};
  for (const [platform, { label, files }] of Object.entries(PLATFORMS)) {
    const offered = [];
    for (const { suffix, format } of files) {
      const name = `hide-${tag}-${suffix}`;
      const urls = [];
      for (const assetName of [name, `${name}.sha256`]) {
        const matches = release.assets.filter(asset => asset?.name === assetName);
        const asset = matches[0];
        const expectedUrl = `${RELEASES_URL}/download/${tag}/${assetName}`;
        if (matches.length !== 1 || asset.state !== "uploaded"
          || !Number.isSafeInteger(asset.size) || asset.size <= 0
          || asset.browser_download_url !== expectedUrl) {
          return { status: "unavailable", reason: "incomplete-assets" };
        }
        urls.push(expectedUrl);
      }
      offered.push({ format, url: urls[0], checksumUrl: urls[1] });
    }
    downloads[platform] = { label, files: offered };
  }
  return { status: "ready", tag, releaseUrl, downloads };
}

/**
 * The hero's one-click download: a validated release's installer for an OS that has exactly one, offered only while the
 * processor the browser reports (Chromium's architecture hint; `""` when it gives none) is not one that installer
 * excludes. Linux has two packages and every other setup goes to the chooser.
 */
export function heroOffer(state, os, arch) {
  if (state.status !== "ready") return null;
  const setup = os === "macos" && arch !== "x86" ? "macos-arm64"
    : os === "windows" && arch !== "arm" ? "windows-x64" : null;
  if (!setup) return null;
  const [file] = state.downloads[setup].files;
  return {
    label: os === "macos" ? "Download for macOS" : "Download for Windows",
    detail: os === "macos" ? "Apple Silicon · macOS 14 or later" : "x64",
    url: file.url,
    checksumUrl: file.checksumUrl,
    releaseUrl: state.releaseUrl,
    tag: state.tag,
  };
}

function mountDownloads() {
  const os = document.getElementById("os");
  const cpu = document.getElementById("cpu");
  const hint = document.getElementById("platform-hint");
  const title = document.getElementById("release-title");
  const detail = document.getElementById("release-detail");
  const region = document.getElementById("release-status");
  const validated = document.getElementById("validated-download");
  const version = document.getElementById("release-version");
  const archive = document.getElementById("archive-link");
  const checksum = document.getElementById("checksum-link");
  const second = document.getElementById("second-link");
  const secondChecksum = document.getElementById("second-checksum-link");
  const notes = document.getElementById("version-link");
  const retry = document.getElementById("retry-release");
  const heroButton = document.getElementById("hero-download");
  const heroNote = document.getElementById("hero-note");
  let state = { status: "loading" };
  let arch = "";
  let request = null;

  // OS is only a hint. Browser identity cannot establish the processor.
  const reported = navigator.userAgentData?.platform || navigator.platform || "";
  const osHint = /mac/i.test(reported) ? "macos"
    : /win/i.test(reported) ? "windows"
      : /linux/i.test(reported) ? "linux" : "";
  if (osHint) {
    os.value = osHint;
    hint.textContent = "Your browser suggested an OS. Confirm it and choose your processor; either can be changed.";
  }
  navigator.userAgentData?.getHighEntropyValues?.(["architecture"])
    .then(values => { arch = values.architecture ?? ""; renderHero(); })
    .catch(() => {});

  // The hero offers its one installer with the release's notes and checksum beside it, or keeps pointing to the chooser.
  function renderHero() {
    const offer = heroOffer(state, osHint, arch);
    heroNote.replaceChildren();
    heroNote.hidden = !offer;
    if (!offer) {
      heroButton.href = "#download";
      heroButton.firstChild.textContent = "Get hide ";
      heroButton.lastChild.textContent = "↓";
      return;
    }
    heroButton.href = offer.url;
    heroButton.firstChild.textContent = `${offer.label} `;
    heroButton.lastChild.textContent = "↗";
    const link = (href, text) => Object.assign(document.createElement("a"), { href, textContent: text });
    heroNote.append(
      `${offer.detail} · `, link(offer.releaseUrl, offer.tag), " · ", link(offer.checksumUrl, "SHA-256"), " · ",
      link("#download", "Other systems"),
    );
  }

  function render() {
    renderHero();
    validated.hidden = true;
    for (const link of [archive, checksum, second, secondChecksum, notes]) link.removeAttribute("href");
    second.hidden = true;
    secondChecksum.hidden = true;
    retry.hidden = state.status === "loading";
    retry.disabled = state.status === "loading";
    region.setAttribute("aria-busy", String(state.status === "loading"));
    if (state.status === "loading") {
      title.textContent = "Checking the latest release…";
      detail.textContent = "Looking for a complete stable release on GitHub. You can also use Releases or build from source.";
    } else if (state.status === "error") {
      title.textContent = "Couldn't check downloads.";
      detail.textContent = "Try again, browse GitHub Releases, or follow the source-install guide.";
    } else if (state.status === "unavailable") {
      title.textContent = "No complete stable download is available.";
      detail.textContent = "Check GitHub Releases for updates, or build from source with the installation guide.";
    } else {
      const selected = state.downloads[`${os.value}-${cpu.value}`];
      if (!os.value || !cpu.value) {
        title.textContent = `Stable release ${state.tag} is available.`;
        detail.textContent = "Choose your operating system and processor to see its installer and checksum.";
      } else if (!selected) {
        title.textContent = "No package for this setup.";
        detail.textContent = "Packages cover macOS Apple Silicon, Windows x64 and Linux x64. Check the installation guide for requirements.";
      } else {
        const [first, other] = selected.files;
        title.textContent = "Your download is ready.";
        detail.textContent = other
          ? "Choose the .deb for Debian or Ubuntu, or the .rpm for Fedora. hide updates itself from then on."
          : "Download the installer and open it. hide updates itself from then on.";
        version.textContent = `${state.tag} · ${selected.label}`;
        archive.href = first.url;
        archive.firstChild.textContent = `Download for ${os.options[os.selectedIndex].text}${other ? ` (.${first.format})` : ""} `;
        checksum.href = first.checksumUrl;
        checksum.textContent = other ? `.${first.format} SHA-256 checksum` : "SHA-256 checksum";
        if (other) {
          second.href = other.url;
          second.firstChild.textContent = `Download .${other.format} `;
          secondChecksum.href = other.checksumUrl;
          secondChecksum.textContent = `.${other.format} SHA-256 checksum`;
          second.hidden = false;
          secondChecksum.hidden = false;
        }
        notes.href = state.releaseUrl;
        notes.firstChild.textContent = `Release notes for ${state.tag} `;
        validated.hidden = false;
      }
    }
  }

  async function load() {
    if (request) return;
    request = new AbortController();
    const active = request;
    const timer = setTimeout(() => active.abort(), 10_000);
    state = { status: "loading" };
    render();
    try {
      const response = await fetch(LATEST_API, {
        signal: active.signal,
        credentials: "omit",
        referrerPolicy: "no-referrer",
        cache: "no-store",
        headers: { Accept: "application/vnd.github+json" },
      });
      if (response.status === 404) {
        state = { status: "unavailable", reason: "not-found" };
      } else if (!response.ok) {
        state = { status: "error" };
      } else {
        state = validateRelease(await response.json());
      }
    } catch {
      state = { status: "error" };
    } finally {
      clearTimeout(timer);
      request = null;
      render();
    }
  }

  os.addEventListener("change", render);
  cpu.addEventListener("change", render);
  retry.addEventListener("click", load);
  window.addEventListener("pagehide", () => request?.abort());
  load();
}

if (typeof document !== "undefined") mountDownloads();
