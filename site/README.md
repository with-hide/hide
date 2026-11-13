# hide site: the introduction page and the user docs

This directory is one Astro project for `withhide.dev`: the introduction page, prepared for issue #338, and the user docs at `/docs`.
The page itself has no framework, webfont service, cookies, or application environment variables: it is plain HTML, CSS and scripts in `public/`, which the build copies to the site's root unchanged, so `index.html` is served at `/` byte for byte and every relative path in it (`./assets/...`) still resolves.
The docs are Starlight (`src/content/docs/docs/`), which serves `/docs/*`; no build step touches the landing.
Vercel Web Analytics records visitors and page views through the deferred `/_vercel/insights/script.js` script in `index.html`, using Vercel's plain-HTML integration without an npm package.
Web Analytics must be enabled for the hosting project and the site redeployed before Vercel serves the insights routes; no API key belongs in the page.
Local static previews do not serve these Vercel routes, so an analytics-script 404 there is expected and does not prevent the site from working.
After deployment, verify a successful insights script load and page-view POST in the browser's Network panel, then confirm the visit in the project's Analytics dashboard.
Bricolage Grotesque (headings and body) and Geist Mono (labels and drawn panels) are served from `assets/fonts/` under the SIL Open Font License (`OFL-BricolageGrotesque.txt`, `OFL-Geist.txt`), copied unchanged from the `@fontsource-variable/bricolage-grotesque` 5.3.0 and `geist` 1.4.2 npm packages.
Building or serving the site locally does not publish it or configure the domain.
`llms.txt` lists the guides an agent reads to use or build hide, as links to their Markdown on the main branch; it names guides, never a release or a figure, so it changes only when a guide is added, renamed or retired.

## Layout

| Path | What it is |
| --- | --- |
| `public/` | The introduction page exactly as before: `index.html`, `styles.css`, `scenes.js`, `site.js`, `one-terminal.js`, `downloads.js`, `llms.txt`, `vercel.json` and `assets/`. Every other path in this README that names `index.html` or `assets/` means this folder. |
| `src/content/docs/docs/{start,guides,features,reference}/` | The docs, in the four sidebar groups. The folder carries the `docs/` prefix, so a page's address is its path under it. |
| `src/data/cli-notes.json` | One human line and an example per `hide` command, which the generated CLI page needs. |
| `src/components/`, `src/styles/owl-field.css` | The header, the "Used in guides" list and the theme: the landing's Owl Field variables (`public/styles.css`) mapped onto Starlight's, in dark and light, with the landing's two fonts from `public/assets/fonts/`. |
| `scripts/add-embed.mjs` | Copies the live parts to `dist/embed/` after the site builds. |

The pictures in the docs are the product's own components, not screenshots: `web/docs-embed.html?part=<id>`, built by `web/docs-embed.config.ts`, framed at `/embed/docs-embed.html?part=<id>&theme=dark|light&lang=en`; a small script in the page head keeps each frame on the page's theme.
A guide names the features it uses in a `features: [id, ...]` header, and a feature page's "Used in guides" list is generated from those headers, never typed.
`site/` is a pnpm root of its own (`pnpm-workspace.yaml`, `pnpm-lock.yaml`), not a member of the repository's web, desktop and mobile workspace, so Astro is installed by no other lane and their frozen installs do not change.

## Build and checks

From the repository root:

```sh
bash scripts/verify-web.sh install docs          # the web shell's packages and the site's own
node scripts/docs/check-all.mjs --hide target/debug/hide
```

The site is pinned to pnpm 10.34.9 (`packageManager` in `package.json`, and the docs lane's `pnpm/action-setup`): run pnpm from inside `site/` (`cd site && pnpm install --frozen-lockfile`) so it switches to that version; a machine whose default is pnpm 12 fails with `ERR_PNPM_BAD_PM_VERSION` when it is pointed at `site/` from outside with `--dir`.

`check-all` is the docs lane's command (`docs/TESTING.md`, The user docs): it checks that guides and features name each other, generates the CLI reference from the `--help` of one `hide` binary and `contracts/hide-cli.json` (failing on a command with no note, and on a note for a gone command), builds the live parts and builds the site into `site/dist/`.
`--hide <path>` (or `HIDE_BIN`) names the one `hide` whose `--help` owns which commands exist; the default is `target/debug/hide`, a build of this checkout (`bash scripts/verify-cargo.sh build -p hided --bin hide`). An installed app's `hide`, such as `/Applications/hide.app/Contents/Resources/hide`, is accepted but may be older than this commit, so `check-all` says so: a local pass with it is a limited check, and CI, which builds `hide` from the commit and passes it with `--ci`, is the gate.
`--checks-only` runs the checks before the builds (the guides, the site's tests and the generated pages) for a quick loop.
For writing pages with a reload, run `check-all` once (it writes the generated files and builds the live parts), then `cd site && pnpm dev`; the live frames load only from the built site, so check them with the preview below.
The download-policy tests and the checks on the privacy and support pages need no install, and the docs lane runs them inside `check-all`:

```sh
bash scripts/verify-web.sh site test
```

`privacy.html` and `support.html` are the two pages the App Store listing points at
as its privacy policy URL and support URL. Each carries an English and a Korean body
in one file, linked from the footer and from each other's header, and each states
only what the shipped code does: [`public/privacy.html`](public/privacy.html) is the authority for
what the desktop app, this site, the relay, the phone app and Hide AI send, and a
change to any of those changes this page in the same release.

Use Node.js 22.7 or later, which recognizes the browser ES module in `public/downloads.js` without a package manifest of its own.
To preview the built site, run `cd site && pnpm preview --host 127.0.0.1 --port 4399` and open `http://127.0.0.1:4399/` and `/docs/`.
Browser modules require HTTP; opening `index.html` through `file:` is not the preview procedure.
Check desktop at 1440 pixels and mobile at 390 pixels, in dark and light, including keyboard navigation, text wrapping, each system leading the download section, a phone's hero, loading, unavailable and retry states on the landing and `/download`, and search and the theme switch in the docs.
Run evidence belongs under ignored `agents/runs/`, never in this directory or a commit.

## Deploying

Vercel is not linked to GitHub; the operator alone says when production changes, and it is deployed by hand from a scratch copy of the build, never from this checkout.
`vercel.json` ships inside the build, because it is in `public/`.

1. Build from the commit to publish: the two commands under Build and checks. The result is `site/dist/`.
2. Copy it to a scratch folder outside the repository: `scratch=$(mktemp -d) && cp -R site/dist/. "$scratch"`.
3. In the scratch folder, link it to the `withhide` project under team `yansfils-projects` (`vercel link --yes --project withhide --scope yansfils-projects`).
4. A preview is `vercel deploy`, which gives a unique URL and moves no domain; production is `vercel deploy --prod`, run only when the operator says.
5. Delete the `.vercel/` and `.env.local` the link wrote, and the scratch folder.
6. Open `/`, `/docs/`, `/docs/reference/cli/` and `/embed/docs-embed.html?part=task-row` on the deployment.

Vercel's own build does not run: the project has no framework preset and serves the files as uploaded.
Web Analytics records the landing as before; the docs pages load no analytics script.


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
Each system has one supported processor, so the page never asks for one: the browser's OS suggestion only orders the systems, the visitor's first with its files at once and the others folded under "Also for" (`downloadView`); with no suggestion, or on a phone, every system is listed alike.
Each system names its requirement, says in one line what opening the file shows (macOS opens with no warning and is dragged into Applications, Windows SmartScreen asks for More info and Run anyway, Linux names the `apt` or `dnf` command), and links its checksum as an optional check and its install guide under `/docs/start/`.
When Chromium's architecture hint names a processor the one build excludes (x86 on macOS, ARM on Windows), that system shows one line saying so instead of its files; other browsers give no hint, so the requirement names the processor instead.
`/download` (`public/download.html`) lists every system with the same code and no fold, for the docs and the app to link.
The hero's button is the one shortcut past the download section (`heroOffer`): for a macOS or Windows suggestion it links that validated installer, and the line under it carries the requirement, the release notes, what opening the file shows and the iPhone app, then a line to the other systems, the checksum and the license.
It stands back to `Get hide ↓` for Linux, for no suggestion, before validation, after any failure, and when the architecture hint excludes the installer.
On a phone (`phoneOf`: an iPhone, or an Android phone) the hero leads with the iPhone app on the App Store and offers `Get hide for your computer` second; an Android phone gets the computer download first and a line saying its app is coming to Google Play, with no store link until it is published (`tests/legal.test.mjs`).
After the files the section links what to do next on this site: the practice screen, the first ten minutes and connecting a phone.
The installer, checksum and version-specific release links appear only after validation.
The site links the published checksum for the visitor to verify; it does not download and cryptographically verify the installer itself.
API errors, timeouts, invalid JSON, no public release and ineligible releases leave useful GitHub Releases and source-install links available.
The same links remain usable without JavaScript.
Browser QA should simulate GitHub responses at that external boundary for ready, incomplete, draft, loading, rate-limit and network-error cases, without changing the shipped source or following test download URLs.

## Content and artwork authority

The brand line, approved owl mark, palette and voice come from [BRAND.md](../docs/BRAND.md).
`assets/hide-mark.png` is an unchanged copy of `design/brand/hide-mark.png`; keep its full square and proportions.
`assets/hide-icon-blinking.svg` is an unchanged copy of `design/brand/hide-icon-blinking.svg`, the animated app icon, and `assets/hide-icon.svg` is the same file with its `<animateTransform>` elements removed, which leaves the resting pose.
The page is the ADE for humans: "Agents do the work. You make the calls." The closing keeps "Run more agents. Hold less." It is kept short, one claim and one picture per section, in this order: the hero (a short loop of the Conductor film and the supported agents), Conductor (the Conductor film in one frame with its chapters Plan, Start, Ask and Report above it), bird's eye (one frame with six chapters above it: a lead pane's eleven workers drawn by the page, Tiled, Folded, Filed and Asked, then the focus film, From above and My turn), Hide AI (one frame and a list beside it: the switch that turns the same Tasks list from off to on, a session record, the PR list and the memory film), beside the agent (a Codex pane and its View area in three steps: the spec PDF it opened, the page it opened, and the page after its pointer pressed the button), every machine (a map of a laptop, a Linux server and the always-on core beside a drawn phone, with a letter running between agents on different machines until the phone gets the ask and answers it, a line saying the phone works in its browser today with apps coming, and three facts about pairing, notifications and the encrypted relay), pricing (the free plan, an Enterprise plan beside it that asks teams to get in touch, and a line for the Software Factory), the FAQ, and a closing that carries the download chooser.
Each section's claims follow the owning guide: the supported agents and their tiers [agent-hooks.md](../docs/agent-hooks.md), the four groups and status marks [status-model.md](../docs/status-model.md), agents handing work to each other [delivery.md](../docs/delivery.md), the core, devices and the phone [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [INSTALL.md](../docs/INSTALL.md), Hide AI [AI_PROVIDERS.md](../docs/AI_PROVIDERS.md), Project Memory, the backlog, the sidebar's lists, the Project tool, the Overview and the session record [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), the browser display and its commands [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md).
Every still picture of a hide screen is the app itself, never a drawing of it: `assets/shots/` holds captures of the shell's own window and crops of it, over invented example work (a Claude Code lead with Codex, Antigravity and OpenCode agents), each in Dark and Light at 2x, and the page picks one with `prefers-color-scheme`.
They are produced by the gallery's `landing` scene (`web/src/gallery/LandingScene.tsx`, with its text in `landingSceneData.ts`, the frozen sidebar and Overview fixtures reworded, and the project's tasks, counts and cost written for the story), so they match the shipped UI and carry no personal data; `/gallery?scene=landing&shot=hero|tasks|quiet|prs|map|projects|attention|plain|team|pr|record|conductor|memory|browser-spec|browser-page|browser|core&theme=dark|light` opens one; the lane shots' card, record and mission are in `landingLaneData.ts`.
Maintainer capture tooling is maintained separately from this public repository.
It starts this checkout's web dev server and a headless Chromium, fails on any Korean text or page error, and writes each shot as `<name>-dark.webp` and `<name>-light.webp`.
Set each image's `width` and `height` in `index.html` to the captured size, so the page does not shift while it loads.
Every captured picture is a crop of a gallery shot, so it shows the part the section talks about at a size that stays legible on a phone.
The `hero` shot (the whole window over the same story, 1456x900) is no longer on the page; it stays the ground of the link preview card.
`prs-list` is the sidebar's PR list cropped from the `prs` shot.
`rows-on` and `rows-off` are the sidebar's Tasks list cropped from the `tasks` and `quiet` shots, with Hide AI on and off, following [status-model.md](../docs/status-model.md), With and without Hide AI: off, a task is called by its provider's name unless it is a Claude Code session with its own title. The story keeps one or two pull requests per task and stars one task, so the list shows every group a person sorts by: Needs You, Unread, Revisit, Working and Waiting. Off follows the same table where it differs beyond titles: the release notes' question was only in a turn's plain text, so it waits under Revisit only when Hide AI is on; pull requests #438 and #439 were linked by the label (`created_prs`), not by the link reader, so they show only when it is on; and an unread stop keeps the plain dot, since only the label judges a turn finished. On, each row also carries the second line Hide AI writes: what a waiting ask wants, a finished turn's result, a working turn's progress and a read question; off, an ask keeps only its native question's first line, and only the lead's proven wait on its workers keeps a line.
`record-lane` crops the lane from the window with the lead's session record in it. `agent-spec`, `agent-page` and `agent-click` crop the Codex pane and its View area from the `browser-spec`, `browser-page` and `browser` shots: the spec PDF, drawn by the shell's own PDF viewer from bytes the scene answers as hided would, then the invoices page in a tab beside it, then the terminal's report after the click. The page stacks the three and steps through them; on the second the script draws the pointer hide draws on a driven page (`hided/assets/browser/overlay.js`): the same accent arrow glides to the button, ripples where it clicks and flashes it, and leaves two seconds after the last action, as the overlay does.
The machine map is drawn by the page, not captured, because Settings does not show agents talking across machines: each step names the sender, the recipient and the letter, every letter passes through the core as delivery does ([delivery.md](../docs/delivery.md)), and the laptop closes while the core keeps its agents going.
The bird's eye stage is drawn by the page too (`one-terminal.js`), because its first half shows terminals tiled the way hide does not draw them, and the focus film plays after it in the same frame.
Its hide half follows [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md): the pane head's second row with the child-tree button, `Waiting on children · N working` and a raised child's ask (`(child)`, its question and Open), the sidebar's Tasks and Projects lists with each worker under its parent, and a child's question raising its root into Needs You ([status-model.md](../docs/status-model.md)) and reaching the parent as a letter ([delivery.md](../docs/delivery.md)).
It names no model, draws the agents' own logos from `assets/agents/` (`antigravity.svg` an unchanged copy of `web/src/assets/agents/antigravity.svg`), and stays dark in both color schemes like the films.
It plays only while on screen and in a visible tab and resumes at the chapter it left; a chapter press plays from that chapter, the film's two from their second; under reduced motion it holds the last frame of Filed, a press holds that chapter's last frame, and the film's chapters show its still without loading it; without script the film's still stands in the frame.
`browser` is a Codex pane beside a browser display and the file it opened; a gallery tab has no desktop host to draw a page, so the scene stands in a host that answers nothing and gives the display an invented invoices page as its still.
The three films are the exception to captured pictures: `assets/video/kick-conductor`, `kick-focus` and `kick-memory` are drawn, rendered with HyperFrames from a film project kept outside this repository, because each tells a story across several screens and machines that one capture cannot hold.
They draw the shipped components' labels and states only (Mission n/6, Needs You, My turn, Go answer, Promote, Given and Pulled), over invented example work; the models they name are released ones, Gemini 4 Argon and Opus 5 among them, and a model appears in the app only when the agent's own CLI lists it for the signed-in account.
The hero plays its own film (`hero`, about 24 seconds; renamed from `kick-hero` so browsers holding the old film for a day fetch the new one): the hide mark, a bird's-eye view where only two agents need you, a dive into the Conductor's ten-step plan, every step landing in a box for its machine with reviewers on each pull request, and a pull back to the whole fleet; the Conductor section plays the whole film: a Conductor's plan drawn on its mission map, then each step flying to a worker on This Mac, linux-box or mac-mini, one question answered, every step ending in a pull request. The focus film is the multi-agent view: thirty-six agents from above, folded into the sidebar's Projects, Tasks and PRs lists, then the Overview's My turn leaving the two that wait on you. The memory film is Project Memory: a rule said to Codex on Monday, a fresh agent repeating the mistake beside Antigravity in hide starting with the card on Thursday, then the rule promoted into AGENTS.md.
Each plays over a still taken from its own film (`assets/shots/kick-*.webp`). A chapter's bar fills while the film is inside it and a press jumps the film there; Hide AI's list turns by itself while it is on screen, every 6.5 seconds and 16 on the memory film, until the first press, and a press always shows its slide, under reduced motion too. The films are dark in both color schemes, framed as a window. The script picks H.264 (`.mp4`) where the browser plays it and VP9 (`.webm`) where it does not, plays a film only while it is on screen and the tab is visible, and never loads it under `prefers-reduced-motion` or without script, where the still stays.
The Hide AI switch flips between off and on every 2.6 seconds while it is on screen and stops for good at the first press; it rests on "on" when it leaves the screen, never flips by itself under `prefers-reduced-motion`, and shows "on" without script. The browser pointer and the machine map's letters likewise run only while on screen and in a visible tab, and under reduced motion or without script the pointer stays away and the map holds its first letter.
The terminals hold what Claude Code would draw there (`HERO_TERMINALS`), written into each pane once its terminal has fitted; headless Chromium draws xterm's WebGL canvas blank at 2x, so the maintainer capture turns WebGL off and the terminals use xterm's DOM renderer, the one the app falls back to.
Every section owl is an `<img class="owl" data-owl="...">` whose `src` is the pose's still frame and whose `data-motion` is its loop, so a pose is replaced by changing those two files.
Supported agents are named in text; the machine map beside the names shows the marks of the three agents it uses, copied unchanged from the app's own bundle (`assets/agents/`: Claude Code's and Codex's artwork the app ships, OpenCode's MIT mark), whose terms `web/src/assets/agents/manifest.json` records. Antigravity's mark stays out of the page, because Google's brand terms ask for a request before a Google logo is used; the films draw Antigravity as a plain `A` monogram.

The Discord mark beside the invite links (header and download area; the footer link stays text) is Discord's own logo from Simple Icons (CC0), drawn inline in the text colour, as Discord's brand terms allow for linking to a server.
The section points follow the same guides as the copy: the parent's minute, the five-minute letter and the causes that reach the person [delivery.md](../docs/delivery.md) and [status-model.md](../docs/status-model.md), a card's check, promotion and pointer [ARCHITECTURE.md](../docs/ARCHITECTURE.md) and [agent-hooks.md](../docs/agent-hooks.md), and the `hide browser` and `hide file open` commands the browser shot's terminal shows [BROWSER_DISPLAYS.md](../docs/BROWSER_DISPLAYS.md).
`site.js` runs the one-time scroll reveals, opens the FAQ answer a link points at, and swaps each section owl between its still frame and its loop: the loop plays only while that owl is on screen and the tab is visible, and under `prefers-reduced-motion` the reveals show at once and no loop is ever loaded.
An owl's loop is CSS keyframes inside its own SVG, so it plays as a plain `<img>` with no player script; the in-file reduced-motion rule is a second guard, because an SVG inside `<img>` does not see the page's media queries in every browser.
The header icon's blink is the page's other repeating image: it is SMIL inside an `<img>`, which a hidden tab does not paint, and under `prefers-reduced-motion` a `<picture>` source shows `hide-icon.svg` instead.
The Software Factory is one line under the free plan; its link is an email to the site's contact address, because the site has no backend to receive a sign-up.
Pro carries no price, because none is decided; free covers everything a person runs themselves.
Enterprise shows no price either: its button is an email to the same contact address, and its list names only security and what a team shares (missions and memory), the paid side of the line between what a person runs and what a team shares.
The workflow and feature descriptions follow [UI_BEHAVIOR.md](../docs/UI_BEHAVIOR.md), with current main's agent launcher, project overview and Workspace components as implementation references.
Supported platforms, signing limitations, prerequisites and installation links follow [INSTALL.md](../docs/INSTALL.md).
The site has its own small CSS scale and brand palette; it does not consume or replace the product's generated UI tokens.
`assets/owls/` holds the owl character's poses, generated with the approved mark as the style reference, keyed to transparency and traced to SVG; [BRAND.md](../docs/BRAND.md#the-owl-character) owns what the character may and may not do, the header shows the blinking app icon and the favicon stays the unchanged `hide-mark.png`.
`assets/og-ade.png` is the link preview card (1200x630 PNG, since some preview readers do not take WebP): the hero headline over the `hero-dark` shot, with the `owl-agents` owl cropped to its head and rising over the window's top edge, so a new hero shot or headline means rendering the card again under a new file name, since `/assets` is cached for a day and link previews cache the old address.
The page is dark by default, on the Owl Field palette: the mark's blue-gray field taken down to night, lime as the one primary accent and the field color as the secondary on kickers, numbers and markers.
It follows `prefers-color-scheme` for a light variant that keeps the same primary and secondary.

## Remaining publication work

Source preparation does not complete issue #338's publication acceptance criteria.
The shots show invented example work drawn by the shipped components, and the films draw the same kind of work; a recorded demo of a real session is pending the related product fixes and capture review.
DNS ownership, hosting, HTTPS and live `withhide.dev` behavior have not been verified or changed by this source work.
A complete public stable release must exist before real archive downloads can be verified end to end.
Before publication, refresh feature and platform copy against current main, independently review the site and any media, test the live release links and checksums, and authorize deployment separately.
The page copy describes current main, which is ahead of the latest public release (v0.4.1 of 2026-10-16 predates Antigravity support and the four status marks); publish it with or after the release that ships those features.
