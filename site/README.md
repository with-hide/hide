# hide introduction site

This directory contains the static introduction-site source prepared for issue #338 and the intended `withhide.dev` domain.
It has no build step, packages, framework, webfont service, analytics, cookies, or application environment variables.
Bricolage Grotesque (headings and body) and Geist Mono (labels and drawn panels) are served from `assets/fonts/` under the SIL Open Font License (`OFL-BricolageGrotesque.txt`, `OFL-Geist.txt`), copied unchanged from the `@fontsource-variable/bricolage-grotesque` 5.3.0 and `geist` 1.4.2 npm packages.
Serving this directory locally does not publish it or configure the domain.
`llms.txt` lists the guides an agent reads to use or build hide, as links to their Markdown on the main branch; it names guides, never a release or a figure, so it changes only when a guide is added, renamed or retired.

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
The page is the multi-agent IDE for people: "Run more agents. Hold less." It is kept short, one claim and one picture per section, in this order: the hero (one window and the supported agents), Hide AI (a switch that turns the same Tasks list from off to on, then three cards: the PR list, a session record and Memory), the multi-agent view (a recording of the Overview's work graph, the lead's pane with its workers, and a Conductor beside its mission map), built on Herdr (real terminal panes underneath, nothing to migrate, and the three layers), beside the agent (the browser display, where the agent's pointer runs the command over it, and a pull request card under the question it answers), every machine (a map of a laptop, a Linux server, the always-on core and a phone, with a letter running between agents on different machines), pricing (one free plan and a line for the Software Factory), the FAQ, and a closing that carries the download chooser.
Each section's claims follow the owning guide: the supported agents and their tiers [agent-hooks.md](../docs/agent-hooks.md), the four groups and status marks [status-model.md](../docs/status-model.md), agents handing work to each other [delivery.md](../docs/delivery.md), the core, devices and the phone [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [INSTALL.md](../docs/INSTALL.md), Hide AI [AI_PROVIDERS.md](../docs/AI_PROVIDERS.md), Project Memory, the backlog, the sidebar's lists, the Project tool, the Overview and the session record [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), the browser display and its commands [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md), and adopting an existing Herdr server [INSTALL.md](../docs/INSTALL.md), which the FAQ answers.
Every picture of a hide screen is the app itself, never a drawing of it: `assets/shots/` holds captures of the shell's own window and crops of it, over invented example work (a Claude Code lead with Codex, Antigravity and OpenCode agents), each in Dark and Light at 2x, and the page picks one with `prefers-color-scheme`.
They are produced by the gallery's `landing` scene (`web/src/gallery/LandingScene.tsx`, with its text in `landingSceneData.ts`, the frozen sidebar and Overview fixtures reworded, and the project's tasks, counts and cost written for the story), so they match the shipped UI and carry no personal data; `/gallery?scene=landing&shot=hero|tasks|quiet|prs|map|projects|attention|plain|team|pr|record|conductor|memory|browser|core&theme=dark|light` opens one; the lane shots' card, record and mission are in `landingLaneData.ts`.
Maintainer capture tooling is maintained separately from this public repository.
It starts this checkout's web dev server and a headless Chromium, fails on any Korean text or page error, and writes each shot as `<name>-dark.webp` and `<name>-light.webp`.
Set each image's `width` and `height` in `index.html` to the captured size, so the page does not shift while it loads.
The hero is the one whole window (1456x900); every other picture is a crop of a gallery shot, so it shows the part the section talks about at a size that stays legible on a phone.
The hero is the `hero` shot: the whole window over the same story, the sidebar's Projects list, the lead's terminal and the asking agent's side by side, and the Project tool's Status tab for the billing-app: its counts, last seven days' cost and to-do list.
`prs-list` is the sidebar's PR list cropped from the `prs` shot, and `map` is the Overview's work graph over the reference Overview world reworded into the story.
`rows-on` and `rows-off` are the sidebar's Tasks list cropped from the `tasks` and `quiet` shots, with Hide AI on and off, following [status-model.md](../docs/status-model.md), With and without Hide AI: off, a task is called by its provider's name unless it is a Claude Code session with its own title. The story keeps one or two pull requests per task and stars one task, so the list shows every group a person sorts by: Needs You, Unread, Revisit, Working and Waiting. Off follows the same table where it differs beyond titles: the release notes' question was only in a turn's plain text, so it waits under Revisit only when Hide AI is on; pull requests #438 and #439 were linked by the label (`created_prs`), not by the link reader, so they show only when it is on; and an unread stop keeps the plain dot, since only the label judges a turn finished.
`team` is the lead's pane head cropped from the hero window (345x400), with its child list opened by the capture; `conductor` is the lead conducting the story's mission with its map in the lane, cropped from a wider window (1284x540) so the whole map shows.
`pr-lane`, `record-lane` and `memory-lane` crop the lane from the window with pull request #431's card, the lead's session record or the Project tool's Memory tab in it, and `browser-lane` crops the browser display from the `browser` shot. Over it the script draws the pointer hide draws on a driven page (`hided/assets/browser/overlay.js`): the same accent arrow glides to a place on the page, ripples where it clicks and flashes the element, while the bubble above names the `hide browser` command; it leaves two seconds after the last action, as the overlay does.
The machine map is drawn by the page, not captured, because Settings does not show agents talking across machines: each step names the sender, the recipient and the letter, every letter passes through the core as delivery does ([delivery.md](../docs/delivery.md)), and the laptop closes while the core keeps its agents going.
`browser` is a Codex pane beside a browser display and the file it opened; a gallery tab has no desktop host to draw a page, so the scene stands in a host that answers nothing and gives the display an invented invoices page as its still.
The team section plays `assets/video/map-dark` or `map-light` over the `map` still: a recording of the same scene, cut together from presses: the whole map, then the lead, a payments agent, the agent asking to merge and a docs agent, each opened with its links lit, then the whole map again.
The script picks H.264 (`.mp4`) where the browser plays it and VP9 (`.webm`) where it does not, plays it only while it is on screen and the tab is visible, and never loads it under `prefers-reduced-motion` or without script, where the still stays.
The Hide AI switch flips between off and on every 2.6 seconds while it is on screen and stops for good at the first press; it rests on "on" when it leaves the screen, never flips by itself under `prefers-reduced-motion`, and shows "on" without script. The browser pointer and the machine map's letters likewise run only while on screen and in a visible tab, and under reduced motion or without script the pointer stays away and the map holds its first letter.
The terminals hold what Claude Code would draw there (`HERO_TERMINALS`), written into each pane once its terminal has fitted; headless Chromium draws xterm's WebGL canvas blank at 2x, so the maintainer capture turns WebGL off and the terminals use xterm's DOM renderer, the one the app falls back to.
Every section owl is an `<img class="owl" data-owl="...">` whose `src` is the pose's still frame and whose `data-motion` is its loop, so a pose is replaced by changing those two files.
Supported agents are named in text; the machine map beside the names shows the marks of the three agents it uses, copied unchanged from the app's own bundle (`assets/agents/`: Claude Code's and Codex's artwork the app ships, OpenCode's MIT mark), whose terms `web/src/assets/agents/manifest.json` records. Antigravity's mark stays out of the page, because Google's brand terms ask for a request before a Google logo is used.
The section points follow the same guides as the copy: the parent's minute, the five-minute letter and the causes that reach the person [delivery.md](../docs/delivery.md) and [status-model.md](../docs/status-model.md), a card's check, promotion and pointer [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [agent-hooks.md](../docs/agent-hooks.md), and the `hide browser` and `hide file open` commands the browser shot's terminal shows [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md).
`site.js` runs the one-time scroll reveals, opens the FAQ answer a link points at, and swaps each owl, the hero peek and the section owls, between its still frame and its loop: the loop plays only while that owl is on screen and the tab is visible, and under `prefers-reduced-motion` the reveals show at once and no loop is ever loaded.
The hero peek stands on the `hero` shot's edges.
An owl's loop is CSS keyframes inside its own SVG, so it plays as a plain `<img>` with no player script; the in-file reduced-motion rule is a second guard, because an SVG inside `<img>` does not see the page's media queries in every browser.
The header icon's blink is the page's other repeating image: it is SMIL inside an `<img>`, which a hidden tab does not paint, and under `prefers-reduced-motion` a `<picture>` source shows `hide-icon.svg` instead.
The Software Factory is one line under the free plan; its link is an email to the site's contact address, because the site has no backend to receive a sign-up.
Pro carries no price, because none is decided; free covers everything a person runs themselves.
The workflow and feature descriptions follow [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), with current main's agent launcher, project overview and Workspace components as implementation references.
Supported platforms, signing limitations, prerequisites and installation links follow [INSTALL.md](../docs/INSTALL.md).
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
The shots and the map recording show invented example work drawn by the shipped components; a recorded demo of a real session is pending the related product fixes and capture review.
DNS ownership, hosting, HTTPS and live `withhide.dev` behavior have not been verified or changed by this source work.
A complete public stable release must exist before real archive downloads can be verified end to end.
Before publication, refresh feature and platform copy against current main, independently review the site and any media, test the live release links and checksums, and authorize deployment separately.
The page copy describes current main, which is ahead of the latest public release (v0.4.1 of 2026-10-16 predates Antigravity support and the four status marks); publish it with or after the release that ships those features.
