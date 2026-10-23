# hide introduction site

This directory contains the static introduction-site source prepared for issue #338 and the intended `withhide.dev` domain.
It has no build step, packages, framework, webfont service, analytics, cookies, or application environment variables.
Bricolage Grotesque (headings and body) and Geist Mono (labels and drawn panels) are served from `assets/fonts/` under the SIL Open Font License (`OFL-BricolageGrotesque.txt`, `OFL-Geist.txt`), copied unchanged from the `@fontsource-variable/bricolage-grotesque` 5.3.0 and `geist` 1.4.2 npm packages.
Serving this directory locally does not publish it or configure the domain.

## Preview and checks

From the repository root, run the focused download-policy tests:

```sh
node --test site/tests/downloads.test.mjs
```

Use Node.js 22.7 or later, which recognizes the browser ES module in `downloads.js` without a package manifest.
For a local preview, choose a free loopback port and serve only this directory:

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory site
```

Open `http://127.0.0.1:8080/` in the browser and stop the server with Ctrl+C when finished.
Browser modules require HTTP; opening `index.html` through `file:` is not the preview procedure.
Check desktop at 1440 pixels and mobile at 390 pixels, including keyboard navigation, text wrapping, manual OS and processor selection, loading, unavailable and retry states.
Run evidence belongs under ignored `agents/runs/public-readiness/site-338/`, never in this directory or a commit.

## Download policy

The browser reads the unauthenticated GitHub `releases/latest` API for `with-hide/hide`.
It requests no credentials, permits one request at a time, and cancels a request after ten seconds or when leaving the page.
There is no polling or persistent release cache.
Only a release with explicit `draft: false`, `prerelease: false`, a stable `v<major>.<minor>.<patch>` tag and that tag's exact repository release URL is eligible.
All four installers below and their individual `.sha256` sidecars must exist exactly once, be uploaded, have a positive integer size, and have exact repository download URLs for the same release tag.

| Setup | Required installers |
| --- | --- |
| macOS, Apple Silicon | `hide-v<version>-macos-arm64.dmg` |
| Windows, x64 | `hide-v<version>-windows-x64-setup.exe` |
| Linux, x64 | `hide-v<version>-linux-x64.deb` and `hide-v<version>-linux-x64.rpm` |

Any incomplete pair withholds downloads for every platform.
Linux shows both packages, the `.deb` first, each with its own checksum.
The app updates itself from the release's update files afterwards, so the page offers only the installers ([INSTALL.md](../docs/INSTALL.md)).
The browser's OS suggestion is only a hint; the visitor can change it and must choose a processor.
Unsupported combinations offer no installer.
The hero's button is the one shortcut past the chooser (`heroOffer`): for a macOS or Windows suggestion, where one installer exists, it links that validated installer and shows its processor and system requirement, the release notes, the checksum and a link to the chooser on the line under it.
It stands back to `Get hide ↓` for Linux, for no suggestion, before validation, after any failure, and when Chromium's architecture hint names a processor the installer does not cover (x86 on macOS, ARM on Windows); other browsers give no hint, so the line names the processor instead.
The installer, checksum and version-specific release links appear together only after validation and a supported selection.
The site links the published checksum for the visitor to verify; it does not download and cryptographically verify the installer itself.
API errors, timeouts, invalid JSON, no public release and ineligible releases leave useful GitHub Releases and source-install links available.
The same links remain usable without JavaScript.
Browser QA should simulate GitHub responses at that external boundary for ready, incomplete, draft, loading, rate-limit and network-error cases, without changing the shipped source or following test download URLs.

## Content and artwork authority

The brand line, approved owl mark, palette and voice come from [BRAND.md](../docs/BRAND.md).
`assets/hide-mark.png` is an unchanged copy of `design/brand/hide-mark.png`; keep its full square and proportions.
`assets/hide-icon-blinking.svg` is an unchanged copy of `design/brand/hide-icon-blinking.svg`, the animated app icon, and `assets/hide-icon.svg` is the same file with its `<animateTransform>` elements removed, which leaves the resting pose.
The page is the multi-agent IDE for people: "Run more agents. Hold less." Its sections, in order, are the hero (a window with four tabs, the supported agents and a line for Herdr users), the team (one tree whose agents write to each other, with only decisions crossing over to the person), Hide AI (an agent's raw work beside the sidebar line it becomes), Project Memory (six sessions of one project), beside the agent (an agent driving the browser display it opened and opening the file it changed), every machine (the core, the devices and the phone), the setup (the agent, harness and terminal a visitor already runs, with hide as one layer and the Herdr answer), the essentials, the Software Factory (Soon), pricing with where the work runs, the FAQ, and a closing that carries the download chooser.
Each section's claims follow the owning guide: the supported agents and their tiers [agent-hooks.md](../docs/agent-hooks.md), the four groups and status marks [status-model.md](../docs/status-model.md), agents handing work to each other [delivery.md](../docs/delivery.md), the core, devices and the phone [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [INSTALL.md](../docs/INSTALL.md), Hide AI [AI_PROVIDERS.md](../docs/AI_PROVIDERS.md), Project Memory, the backlog, the sidebar's lists, the Project tool, the Overview and the session record [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), the browser display and its commands [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md), and adopting an existing Herdr server [INSTALL.md](../docs/INSTALL.md), which the FAQ answers.
The hero's tabs, the Hide AI section and the machines section show the app itself: `assets/shots/` holds captures of the shell's own window with the sidebar on each of its three lists beside the Project tool, of the Overview's work graph, of Settings > Devices and Mobile, and of its Sidebar, over invented example work (a Claude Code lead, Codex workers and OpenCode helpers), each in Dark and Light at 2x, and the page picks one with `prefers-color-scheme`.
They are produced by the gallery's `landing` scene (`web/src/gallery/LandingScene.tsx`, with its text in `landingSceneData.ts`, the frozen sidebar and Overview fixtures reworded, and the project's tasks, counts and cost written for the story), so they match the shipped UI and carry no personal data; `/gallery?scene=landing&shot=hero|tasks|prs|map|attention|core&theme=dark|light` opens one.
Maintainer capture tooling is maintained separately from this public repository.
It starts this checkout's web dev server and a headless Chromium, fails on any Korean text or page error, and writes `hero`, `tasks`, `prs`, `map`, `core` and `attention` as `<name>-dark.webp` and `<name>-light.webp`.
Set each image's `width` and `height` in `index.html` to the captured size, so the page does not shift while it loads.
The four tab shots and `core` are one window size (1456x900), so a tab change never moves the page and a phone shows each window scaled down whole.
The hero is the `hero` shot: the whole window over the same story, the sidebar's Projects list, the lead's terminal and the asking agent's side by side, and the Project tool's Status tab for the billing-app: its counts, last seven days' cost and to-do list.
`tasks` and `prs` are the same window with the sidebar on its Tasks and PR lists, `map` is the Overview's work graph over the reference Overview world reworded into the story, and `core` is Settings > Devices and Mobile beside the sidebar.
The tabs turn every seven seconds while the window is on screen and the tab is visible, show a fill under the selected one, and stop for good at the first press; under `prefers-reduced-motion` they never turn by themselves, and without script the first window shows alone.
The terminals hold what Claude Code would draw there (`HERO_TERMINALS`), written into each pane once its terminal has fitted; headless Chromium draws xterm's WebGL canvas blank at 2x, so the maintainer capture turns WebGL off and the terminals use xterm's DOM renderer, the one the app falls back to.
Every section owl is an `<img class="owl" data-owl="...">` whose `src` is the pose's still frame and whose `data-motion` is its loop, so a pose is replaced by changing those two files.
Supported agents are named in text; third-party agent marks stay inside the app, where `web/src/assets/agents/manifest.json` records their terms, so the page's drawings name each agent with a two-letter monogram and its name.
The team, setup, terminal, Memory and beside-the-agent drawings are the page's own (`scenes.js` over markup in `index.html`): the markup holds each one's resting frame, the one reduced motion and a page without script show, and the script plays one only while it is on screen and the tab is visible, stopping every timer it started when it leaves.
Their claims follow the same guides as the copy: who wakes on a letter and who has Memory [agent-hooks.md](../docs/agent-hooks.md), the parent's minute, the five-minute letter and the causes that reach the person [delivery.md](../docs/delivery.md) and [status-model.md](../docs/status-model.md), a card's check, promotion and pointer [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [agent-hooks.md](../docs/agent-hooks.md), and the `hide browser` and `hide file open` commands the beside-the-agent terminal types [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md).
`site.js` runs the one-time scroll reveals and the hero's tabs, and swaps each owl, the hero peek and the section owls, between its still frame and its loop: the loop plays only while that owl is on screen and the tab is visible, and under `prefers-reduced-motion` the reveals show at once and no loop is ever loaded.
The hero peek stands on the `hero` shot's edges, so it shows only while that tab is selected.
An owl's loop is CSS keyframes inside its own SVG, so it plays as a plain `<img>` with no player script; the in-file reduced-motion rule is a second guard, because an SVG inside `<img>` does not see the page's media queries in every browser.
The header icon's blink is the page's other repeating image: it is SMIL inside an `<img>`, which a hidden tab does not paint, and under `prefers-reduced-motion` a `<picture>` source shows `hide-icon.svg` instead.
The Factory is marked Soon and has no sign-up form, because the site has no backend to receive one: its action is to watch the repository's releases on GitHub.
Pro and Team carry no price, because none is decided; free covers everything a person runs themselves.
The workflow and feature descriptions follow [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), with current main's agent launcher, project overview and Workspace components as implementation references.
Supported platforms, signing limitations, prerequisites and installation links follow [INSTALL.md](../docs/INSTALL.md).
The FAQ's speed answer quotes no figure and links the dated candidate measurements in [PERFORMANCE_RESULTS.md](../docs/PERFORMANCE_RESULTS.md); if a figure is ever quoted on the page again, give its sample count and boundary beside it, and never describe internal buffer/frame endpoints as physical display latency.
The recorded comparison attempt did not complete a matching Orca workload, so the site makes no relative performance claim.
The site has its own small CSS scale and brand palette; it does not consume or replace the product's generated UI tokens.
`assets/owls/` holds the owl character's poses, generated with the approved mark as the style reference, keyed to transparency and traced to SVG; [BRAND.md](../docs/BRAND.md#the-owl-character) owns what the character may and may not do, the header shows the blinking app icon and the favicon stays the unchanged `hide-mark.png`.
`owl-peek.svg` is the `owl-agents` owl traced into vector parts (ears, body, wings, face) and animated by CSS keyframes inside the file, so it plays as a plain `<img>` with no player script.
Its field spans the window's width with the screenshot's top edge 200 units down, and the owl hides behind straight edges of the hero shot: it pops up over the window's top edge above the lead's pane, leans out from behind the Project tool's left edge, rises from behind the sidebar's footer and pops up again above the Project tool, looks at one place each time, and ducks back.
The maintainer capture measures those edges on the scene it photographs and writes them into `owl-peek.svg` and `owl-peek-still.svg`, so a new hero moves the owl with it; a hero of another shape needs the owl's field redrawn, and the capture refuses it.
`owl-peek-still.svg` is its first stop held still, which the page shows while the hero is off screen, in a hidden tab and under `prefers-reduced-motion`, because an SVG inside `<img>` does not see the page's media queries in every browser.
The page is dark by default, on the Owl Field palette: the mark's blue-gray field taken down to night, lime as the one primary accent and the field color as the secondary on kickers, numbers and markers.
It follows `prefers-color-scheme` for a light variant that keeps the same primary and secondary.

## Remaining publication work

Source preparation does not complete issue #338's publication acceptance criteria.
The shots show invented example work drawn by the shipped components; a recorded demo is pending the related product fixes and capture review.
No video or placeholder demo link is included here.
DNS ownership, hosting, HTTPS and live `withhide.dev` behavior have not been verified or changed by this source work.
A complete public stable release must exist before real archive downloads can be verified end to end.
Before publication, refresh feature and platform copy against current main, independently review the site and any media, test the live release links and checksums, and authorize deployment separately.
The page copy describes current main, which is ahead of the latest public release (v0.4.1 of 2026-10-16 predates Antigravity support and the four status marks); publish it with or after the release that ships those features.
