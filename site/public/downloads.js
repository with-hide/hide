import { markElement } from "./platform-marks.js";

const RELEASES_URL = "https://github.com/with-hide/hide/releases";
const LATEST_API = "https://api.github.com/repos/with-hide/hide/releases/latest";
// The installers' names carry the version, so without the API the one stable link is the latest release's page.
const LATEST_RELEASE_PAGE = `${RELEASES_URL}/latest`;
export const APP_STORE_URL = "https://apps.apple.com/app/hide-remote-coding-agents/id6832780181";
const PHONE_GUIDE = "/docs/reference/faq/#how-do-i-connect-my-phone";
// The installers a person downloads (PRD auto-update D-35); Linux offers both packages.
const PLATFORMS = Object.freeze({
  "macos-arm64": { label: "macOS · Apple Silicon (ARM64)", files: [{ suffix: "macos-arm64.dmg", format: "dmg" }] },
  "windows-x64": { label: "Windows · Intel or AMD (x64)", files: [{ suffix: "windows-x64-setup.exe", format: "exe" }] },
  "linux-x64": { label: "Linux · Intel or AMD (x64)", files: [{ suffix: "linux-x64.deb", format: "deb" }, { suffix: "linux-x64.rpm", format: "rpm" }] },
});

// Each computer system has exactly one supported processor, so the page never asks for one: the card names it in the
// requirement and says what the person sees when they open the file (docs/INSTALL.md owns both facts).
const SYSTEMS = Object.freeze({
  macos: {
    name: "macOS", mark: "apple", setup: "macos-arm64", requirement: "Apple Silicon · macOS 14 or later",
    sees: "Opens with no warning: drag hide into Applications",
    excluded: "This Mac has an Intel processor. hide runs on Apple Silicon Macs only.",
    guide: "/docs/start/install-macos/",
  },
  windows: {
    name: "Windows", mark: "windows", setup: "windows-x64", requirement: "x64 PC · not Windows on ARM",
    sees: "Unsigned: at SmartScreen choose More info, then Run anyway",
    excluded: "This PC has an ARM processor. hide runs on x64 Windows only.",
    guide: "/docs/start/install-windows/",
  },
  linux: {
    name: "Linux", mark: "linux", setup: "linux-x64", requirement: "x64 · Debian, Ubuntu or Fedora",
    sees: "Install the .deb with apt or the .rpm with dnf",
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

function excludes(os, arch) {
  return (os === "macos" && arch === "x86") || (os === "windows" && arch === "arm");
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
    firstLaunch: system.sees,
    others: Object.keys(SYSTEMS).filter(other => other !== os).map(other => SYSTEMS[other].name),
    url: file.url,
    checksumUrl: file.checksumUrl,
    releaseUrl: state.releaseUrl,
    tag: state.tag,
  };
}

/**
 * Every platform as a card, in two rows: the computers, then the phones (the phones first on a phone). Every card has a
 * working way in whatever GitHub answers: a validated release gives each computer its files and checksums, and anything
 * else (loading, a spent rate limit, an incomplete release) gives the latest release's page. The visitor's own platform
 * is marked `mine` and carries the page's one primary button; a processor its one build excludes reads one line instead.
 */
export function platformCards(state, { os = "", arch = "", phone = "" } = {}) {
  const ready = state.status === "ready";
  const computer = key => {
    const system = SYSTEMS[key];
    const blocked = !phone && key === os && excludes(os, arch);
    const mine = !phone && key === os && !blocked;
    const files = ready ? state.downloads[system.setup].files : [];
    return {
      id: key, name: system.name, mark: system.mark, requirement: system.requirement, guide: system.guide, mine,
      soon: false,
      sees: blocked ? null : system.sees,
      excluded: blocked ? system.excluded : null,
      actions: ready
        ? files.map((file, index) => ({ label: `Download .${file.format}`, href: file.url, primary: mine && index === 0 }))
        : [{ label: "Download from GitHub", href: LATEST_RELEASE_PAGE, primary: mine }],
      commands: files.filter(file => INSTALL_COMMAND[file.format]).map(file => `${INSTALL_COMMAND[file.format]}${file.name}`),
      checksums: files.map(file => ({ label: files.length > 1 ? `.${file.format}` : "SHA-256", href: file.checksumUrl })),
    };
  };
  const phones = [
    {
      id: "ios", name: "iPhone", mark: "apple", requirement: "iOS 15 or later", guide: PHONE_GUIDE, mine: phone === "ios",
      soon: false, sees: "Pair it with one QR scan from Settings > Mobile in hide", excluded: null,
      actions: [{ label: "App Store", href: APP_STORE_URL, primary: phone === "ios" }], commands: [], checksums: [],
    },
    {
      id: "android", name: "Android", mark: "android", requirement: "Coming to Google Play", guide: PHONE_GUIDE,
      mine: phone === "android", soon: true,
      sees: "Today, scan the QR with the camera and pair in the phone's browser", excluded: null,
      actions: [], commands: [], checksums: [],
    },
  ];
  const groups = [
    { id: "desktop", title: "Desktop", cards: Object.keys(SYSTEMS).map(computer) },
    { id: "phone", title: "Phone", cards: phones },
  ];
  if (phone) groups.reverse();
  return { status: state.status, tag: ready ? state.tag : null, releaseUrl: ready ? state.releaseUrl : LATEST_RELEASE_PAGE, groups };
}

const el = (tag, props = {}, ...children) => {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.filter(child => child !== null && child !== undefined && child !== false));
  return node;
};
const arrow = glyph => el("span", { ariaHidden: "true", textContent: glyph });

function cardElement(card) {
  const head = el("div", { className: "card-head" }, markElement(card.mark),
    el("h4", { className: "card-name", textContent: card.name }),
    card.mine ? el("span", { className: "card-mine", textContent: "Your system" }) : null);
  const actions = card.actions.length
    ? el("div", { className: "card-actions" }, ...card.actions.map(action =>
      el("a", { className: `button ${action.primary ? "primary" : "secondary"}`, href: action.href }, `${action.label} `, arrow("↗"))))
    : null;
  const commands = card.commands.length
    ? el("div", { className: "card-commands" }, ...card.commands.map(command => el("code", { textContent: command })))
    : null;
  const phoneCard = card.id === "ios" || card.id === "android";
  const links = el("p", { className: "card-links" },
    ...(card.checksums.length
      ? [card.checksums.length > 1 ? "Optional SHA-256: " : "Optional: ",
        ...card.checksums.flatMap((sum, index) => [index ? " · " : "", el("a", { href: sum.href, textContent: sum.label })]), " · "]
      : []),
    el("a", { href: card.guide, textContent: phoneCard ? "How to connect" : "Install guide" }));
  const article = el("article", { className: `card${card.mine ? " mine" : ""}${card.soon ? " soon" : ""}` },
    head,
    el("p", { className: "card-req", textContent: card.requirement }),
    actions,
    card.excluded ? el("p", { className: "card-excluded", textContent: card.excluded }) : null,
    card.sees ? el("p", { className: "card-sees", textContent: `${card.sees}.` }) : null,
    commands,
    links);
  article.setAttribute("aria-label", card.name);
  return article;
}

function renderCards(container, view) {
  container.replaceChildren(...view.groups.map(group => {
    const section = el("section", { className: `card-group ${group.id}` },
      el("p", { className: "card-group-title", textContent: group.title }),
      el("div", { className: "card-grid" }, ...group.cards.map(cardElement)));
    section.setAttribute("aria-label", group.title);
    return section;
  }));
}

function mountDownloads() {
  const cards = document.getElementById("downloads");
  if (!cards) return;
  const title = document.getElementById("release-title");
  const retry = document.getElementById("retry-release");
  const heroButton = document.getElementById("hero-download");
  const heroSecond = document.getElementById("hero-second");
  const heroReq = document.getElementById("hero-req");
  const heroNote = document.getElementById("hero-note");
  const heroReqDefault = heroReq ? [...heroReq.childNodes].map(node => node.cloneNode(true)) : [];
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

  // The status line only refines the cards: the version when GitHub answered, otherwise why each button opens the
  // release page, with a way to ask again.
  function render() {
    renderHero();
    const view = platformCards(state, { os: osHint, arch, phone });
    renderCards(cards, view);
    retry.hidden = state.status === "loading" || state.status === "ready";
    retry.disabled = state.status === "loading";
    if (state.status === "ready") {
      title.replaceChildren(`${view.tag} · `, link(view.releaseUrl, "Release notes ", arrow("↗")));
    } else if (state.status === "loading") {
      title.textContent = "Checking the latest release…";
    } else {
      title.textContent = "Couldn't read the latest version from GitHub, so each button opens the latest release there.";
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

  retry.addEventListener("click", load);
  window.addEventListener("pagehide", () => request?.abort());
  load();
}

if (typeof document !== "undefined") mountDownloads();
