const RELEASES_URL = "https://github.com/with-hide/hide/releases";
const LATEST_API = "https://api.github.com/repos/with-hide/hide/releases/latest";
export const APP_STORE_URL = "https://apps.apple.com/app/hide-remote-coding-agents/id6832780181";
// The installers a person downloads (PRD auto-update D-35); Linux offers both packages.
const PLATFORMS = Object.freeze({
  "macos-arm64": { label: "macOS · Apple Silicon (ARM64)", files: [{ suffix: "macos-arm64.dmg", format: "dmg" }] },
  "windows-x64": { label: "Windows · Intel or AMD (x64)", files: [{ suffix: "windows-x64-setup.exe", format: "installer" }] },
  "linux-x64": { label: "Linux · Intel or AMD (x64)", files: [{ suffix: "linux-x64.deb", format: "deb" }, { suffix: "linux-x64.rpm", format: "rpm" }] },
});

// Each system has exactly one supported processor, so the page never asks for one: it names the processor in the
// system's requirement and says what the person sees when they open the file (docs/INSTALL.md owns both facts).
const SYSTEMS = Object.freeze({
  macos: {
    name: "macOS", setup: "macos-arm64", requirement: "Apple Silicon · macOS 14 or later",
    firstLaunch: "Opens with no warning: drag hide into Applications",
    excluded: "This Mac has an Intel processor. hide runs on Apple Silicon Macs only.",
    guide: "/docs/start/install-macos/",
  },
  windows: {
    name: "Windows", setup: "windows-x64", requirement: "x64 PC · not Windows on ARM",
    firstLaunch: "Unsigned: at SmartScreen choose More info, then Run anyway",
    excluded: "This PC has an ARM processor. hide runs on x64 Windows only.",
    guide: "/docs/start/install-windows/",
  },
  linux: {
    name: "Linux", setup: "linux-x64", requirement: "x64 · Debian, Ubuntu or Fedora",
    firstLaunch: "Install it with your package manager",
    guide: "/docs/start/install-linux/",
  },
});
const INSTALL_COMMAND = Object.freeze({ deb: "sudo apt install ./", rpm: "sudo dnf install ./" });

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
      offered.push({ format, name, url: urls[0], checksumUrl: urls[1] });
    }
    downloads[platform] = { label, files: offered };
  }
  return { status: "ready", tag, releaseUrl, downloads };
}

/** Which phone the browser runs on: `ios` for an iPhone, `android` for an Android phone, `""` for anything else. */
export function phoneOf(userAgent, mobileHint = false) {
  if (/iPhone|iPod/.test(userAgent)) return "ios";
  if (/Android/.test(userAgent) && (mobileHint || /Mobile/.test(userAgent))) return "android";
  return "";
}

/**
 * The hero's one-click download: a validated release's installer for an OS that has exactly one, offered only while the
 * processor the browser reports (Chromium's architecture hint; `""` when it gives none) is not one that installer
 * excludes. Linux has two packages and every other setup goes to the download section.
 */
export function heroOffer(state, os, arch) {
  if (state.status !== "ready") return null;
  const system = SYSTEMS[os];
  if (!system || os === "linux" || excludes(os, arch)) return null;
  const [file] = state.downloads[system.setup].files;
  return {
    label: `Download for ${system.name}`,
    detail: system.requirement,
    firstLaunch: system.firstLaunch,
    others: Object.keys(SYSTEMS).filter(other => other !== os).map(other => SYSTEMS[other].name),
    url: file.url,
    checksumUrl: file.checksumUrl,
    releaseUrl: state.releaseUrl,
    tag: state.tag,
  };
}

function excludes(os, arch) {
  return (os === "macos" && arch === "x86") || (os === "windows" && arch === "arm");
}

/**
 * The download section for a validated release: the visitor's system first, its file and checksum at once, and the
 * other systems after it. With no system suggested every system is listed alike (`lead` is null). A processor the
 * browser reports that the system's one build excludes replaces that system's files with one line saying so.
 */
export function downloadView(state, os, arch) {
  if (state.status !== "ready") return null;
  const view = key => {
    const system = SYSTEMS[key];
    const blocked = key === os && excludes(os, arch);
    return {
      os: key,
      name: system.name,
      requirement: system.requirement,
      firstLaunch: system.firstLaunch,
      guide: system.guide,
      excluded: blocked ? system.excluded : null,
      files: blocked ? [] : state.downloads[system.setup].files.map(file => ({
        ...file, command: INSTALL_COMMAND[file.format] ? `${INSTALL_COMMAND[file.format]}${file.name}` : null,
      })),
    };
  };
  const lead = SYSTEMS[os] ? view(os) : null;
  const others = Object.keys(SYSTEMS).filter(key => key !== os).map(view);
  return { tag: state.tag, releaseUrl: state.releaseUrl, lead, others };
}

const el = (tag, props = {}, ...children) => {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.filter(child => child !== null && child !== undefined && child !== false));
  return node;
};
const arrow = glyph => el("span", { ariaHidden: "true", textContent: glyph });

// One system: its name and requirement, its files, what opening one shows, then the optional checksum and the guide.
function systemBlock(system, lead) {
  const head = el("div", { className: "system-head" },
    el(lead ? "h4" : "h5", { className: "system-name", textContent: system.name }),
    el("span", { className: "system-req", textContent: system.requirement }));
  if (system.excluded) {
    return el("div", { className: "system" }, head, el("p", { className: "system-excluded", textContent: system.excluded }),
      el("p", { className: "system-links" }, el("a", { href: system.guide, textContent: "Install guide" })));
  }
  const multiple = system.files.length > 1;
  const buttons = el("div", { className: "actions system-files" }, ...system.files.map((file, index) => {
    const text = multiple ? `Download .${file.format} ` : `Download for ${system.name} `;
    const kind = lead && index === 0 ? "primary" : "secondary";
    return el("a", { className: `button ${kind}`, href: file.url }, text, arrow("↗"));
  }));
  const commands = system.files.filter(file => file.command);
  const sees = commands.length
    ? el("div", { className: "system-sees" }, el("p", { textContent: `${system.firstLaunch}:` }),
      ...commands.map(file => el("code", { className: "system-command", textContent: file.command })))
    : el("p", { className: "system-sees", textContent: `${system.firstLaunch}.` });
  const checksums = system.files.flatMap((file, index) => [
    index ? " · " : "", el("a", { href: file.checksumUrl, textContent: multiple ? `.${file.format}` : "SHA-256" }),
  ]);
  const links = el("p", { className: "system-links" },
    multiple ? "Optional SHA-256 check: " : "Optional: ", ...checksums, " · ",
    el("a", { href: system.guide, textContent: "Install guide" }));
  return el("div", { className: "system" }, head, buttons, sees, links);
}

function renderSystems(container, view, { fold }) {
  const version = el("p", { className: "release-version" }, `${view.tag} · `,
    el("a", { href: view.releaseUrl }, "Release notes ", arrow("↗")));
  if (!view.lead || !fold) {
    const all = view.lead ? [view.lead, ...view.others] : view.others;
    container.replaceChildren(version, ...all.map(system => systemBlock(system, system === view.lead)));
    return;
  }
  const also = el("details", { className: "also", id: "also-for" },
    el("summary", { textContent: `Also for ${view.others.map(system => system.name).join(" · ")}` }),
    ...view.others.map(system => systemBlock(system, false)));
  container.replaceChildren(version, systemBlock(view.lead, true), also);
}

function mountDownloads() {
  const systems = document.getElementById("downloads");
  if (!systems) return;
  const title = document.getElementById("release-title");
  const detail = document.getElementById("release-detail");
  const region = document.getElementById("release-status");
  const retry = document.getElementById("retry-release");
  const heroButton = document.getElementById("hero-download");
  const heroSecond = document.getElementById("hero-second");
  const heroReq = document.getElementById("hero-req");
  const heroNote = document.getElementById("hero-note");
  const heroReqDefault = heroReq ? [...heroReq.childNodes].map(node => node.cloneNode(true)) : [];
  const fold = systems.dataset.fold !== "none";
  let state = { status: "loading" };
  let arch = "";
  let request = null;

  // OS is only a hint, and a phone suggests no computer at all. Browser identity cannot establish the processor.
  const phone = phoneOf(navigator.userAgent, navigator.userAgentData?.mobile === true);
  const reported = navigator.userAgentData?.platform || navigator.platform || "";
  const osHint = phone ? ""
    : /mac/i.test(reported) ? "macos"
      : /win/i.test(reported) ? "windows"
        : /linux/i.test(reported) ? "linux" : "";
  navigator.userAgentData?.getHighEntropyValues?.(["architecture"])
    .then(values => { arch = values.architecture ?? ""; render(); })
    .catch(() => {});

  const link = (href, text, ...rest) => el("a", { href }, text, ...rest);
  const setButton = (button, href, text, glyph) => {
    button.href = href;
    button.replaceChildren(`${text} `, arrow(glyph));
  };

  // The hero: on a phone the phone app leads and the computer download follows; on a computer the one installer for
  // its system, with what opening it shows on the line under it, or the way down to the download section.
  function renderHero() {
    if (!heroButton) return;
    heroReq.replaceChildren(...heroReqDefault.map(node => node.cloneNode(true)));
    heroNote.replaceChildren();
    heroNote.hidden = true;
    if (phone) {
      heroButton.hidden = phone !== "ios";
      if (phone === "ios") setButton(heroButton, APP_STORE_URL, "Get the iPhone app", "↗");
      setButton(heroSecond, "#download", "Get hide for your computer", "↓");
      heroSecond.className = `button ${phone === "ios" ? "secondary" : "primary"}`;
      heroReq.replaceChildren(phone === "ios"
        ? "Free · pairs with hide on your computer by one QR scan"
        : "Android app coming to Google Play · today, pair from your phone's browser");
      return;
    }
    const offer = heroOffer(state, osHint, arch);
    if (!offer) {
      setButton(heroButton, "#download", "Get hide", "↓");
      return;
    }
    setButton(heroButton, offer.url, offer.label, "↗");
    heroReq.replaceChildren(`${offer.detail} · `, link(offer.releaseUrl, offer.tag), ` · ${offer.firstLaunch} · `,
      link(APP_STORE_URL, "iPhone app ", arrow("↗")));
    heroNote.append(link("#download", `Also for ${offer.others.join(" · ")}`, " ", arrow("↓")), " · ",
      link(offer.checksumUrl, "SHA-256"), " · Free · Fair source (FSL)");
    heroNote.hidden = false;
  }

  function render() {
    renderHero();
    retry.hidden = state.status === "loading";
    retry.disabled = state.status === "loading";
    region.setAttribute("aria-busy", String(state.status === "loading"));
    const view = downloadView(state, osHint, arch);
    region.hidden = Boolean(view);
    systems.hidden = !view;
    if (view) {
      renderSystems(systems, view, { fold });
      retry.hidden = true;
      return;
    }
    systems.replaceChildren();
    if (state.status === "loading") {
      title.textContent = "Checking the latest release…";
      detail.textContent = "Looking for a complete stable release on GitHub. You can also use Releases or build from source.";
    } else if (state.status === "error") {
      title.textContent = "Couldn't check downloads.";
      detail.textContent = "Try again, browse GitHub Releases, or follow the source-install guide.";
    } else {
      title.textContent = "No complete stable download is available.";
      detail.textContent = "Check GitHub Releases for updates, or build from source with the installation guide.";
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

  // The hero's "Also for" opens the other systems where it lands.
  heroNote?.addEventListener("click", event => {
    if (event.target.closest('a[href="#download"]')) {
      const also = document.getElementById("also-for");
      if (also) also.open = true;
    }
  });
  retry.addEventListener("click", load);
  window.addEventListener("pagehide", () => request?.abort());
  load();
}

if (typeof document !== "undefined") mountDownloads();
