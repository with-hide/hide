# Design workflow

This document owns how a design change moves from an idea to shipped code: the authorities a change has to agree with, the scratch-to-PR flow, the sheet transplant procedure for parallel screen work, and how to add a token, a System part, or a Component.
[UI_BEHAVIOR.md](UI_BEHAVIOR.md) owns what the shipped UI does; this document owns how you get there.
Read [docs/README.md](README.md) first for which documents are current.

## Authorities

Three files each own one kind of truth, and none of them restates another's values.

- **Visual authority** is the Pen library, [design/hide-ui.lib.pen](../design/hide-ui.lib.pen).
  Its `System /` sheets are shadcn parts drawn 1:1 with the dev gallery: same name, same variants, one sheet per part with `Light` and `Dark` frames and one node per state.
  Its `Component /` sheets are hide composites built on top of `System /` masters (menus, rows, panels) that do not exist in shadcn.
  `Component / Agent graph box` is the one generated `Component /` sheet besides the two folded legacy masters: `componentSheets` in `scripts/pen-system.mjs` draws its master (a head and three rows a state turns on) and its states (primary, a worktree with an asking row, resting and dimmed, a row with a tucked badge, rows with cross-project chips, selected) as refs of that master, Light and Dark, and a new sheet lands where its `at` says until a person moves it.
  `Component / Action menus` records the approved context-menu B layout as System menu item refs: terminal, Agent and View tabs, session and task rows, submenus and a shared disabled reason in both themes.
  The approval came from an interactive HTML comparison; its source and run captures stay in the local run directory.
  `scripts/pen-system.mjs` owns the sheet; `web/src/components/entry-menu.tsx` owns grouping without changing command identities.
  `Component / Sidebar chrome` is generated the same way by `scripts/pen-sidebar-chrome.mjs`: the Search row and the section header masters, the Help menu, Light and Dark.
  Only `System /` and `Component /` top-level sheets belong in this file; screen proposals, audits, and scratch never do.
- **Numeric authority** is [design/tokens.json](../design/tokens.json): shadcn-named tokens (`--background`, `--foreground`, `--card`, `--popover`, `--primary`/`--primary-foreground`, `--secondary`, `--muted`/`--muted-foreground`, `--accent`, `--destructive`, `--border`, `--input`, `--ring`, `--sidebar-*`, and the four `--accent-choice-*` picks) each carrying a Dark value and a Light value, plus aliases.
  Those two values are the Neutral color theme's; a color the Hide color theme draws differently also carries `hide` with its own Dark `value` and `light`, and every other color is shared by both.
  `node scripts/gen-tokens.mjs` writes `web/src/tokens.css` (Tailwind v4 `@theme`, `:root`/`.light`, `.dark`, then the Hide values under `.palette-hide`, the class the shell puts on the document root) and `web/src/generated/accents.ts`.
  `node scripts/gen-pen.mjs` writes the same values into `design/hide-ui.lib.pen` as Pen variables on two theme axes: `Mode` (Light/Dark) and `Palette` (Neutral, the default, and Hide).
  Every `System /`, `Component /` and `Screen /` frame carries only a `Mode` tag, so it draws Neutral, as the dev gallery does; Foundations draws the Hide palettes beside the Neutral ones.
  `node scripts/check-token-contrast.mjs` fails a terminal text color or a secondary grey below 4.5:1 in either palette and mode.
  `tokens.json` is web-only: it generates `web/src/tokens.css` and nothing else.
- **Code authority** is `web/src/components/ui` for shadcn parts (one file per part, matching the gallery and the Pen `System /` sheets) and `web/src/components` for hide composites such as `entry-menu.tsx` and `settings-rows.tsx`, matching `Component /` sheets.

Screen designs are committed separately from the library, in [design/hide-screens.pen](../design/hide-screens.pen), as `Screen / <Area>` sheets (each with Light and Dark frames), importing the library rather than redrawing its masters.
Selected sidebar and 태스크 list rows use the library-authored `--color-selected`, the default `--primary` at `--opacity-selected-fill` in each theme, registered in `scripts/pen-token-map.json` and copied locally into the screens file.
Pen cannot derive an alpha tint from a color variable or follow the operator's runtime accent choice; this variable draws the default lime accent, while the shared neutral `--color-selected-wash` and selection on other surfaces retain their existing values.

## Dev gallery

The dev-only route `/gallery` (`web/src/gallery/`) renders every `System /` state as the real shadcn component, driven by `web/src/gallery/manifest.ts`, which lists each part and the exact state names its Pen sheet draws.
It is excluded from the production build `hided` serves.
`node scripts/check-pen-gallery.mjs` refuses a manifest and a library that name different parts or different states for either theme frame; see Checks below.

The gallery also renders production screens on synthetic scenes, one document per scene because a scene seeds the app's own stores.
`lang=en|ko|ja|zh-CN` initializes the gallery language, including the real action menus in `pane-header`, `area-focus`, `projects-sidebar` and `session-panel`.
`/gallery?scene=projects-sidebar` is the shell's real `Sidebar` and `createActions` over `web/src/gallery/sceneData.ts`, a snapshot with the content `Screen / Projects Sidebar` draws; its folds go through the real actions and the scene answers them as the core would.
Its query takes `theme` (`light`, `dark`), `width` (the sidebar's CSS width, seeded as the core's `ui_state.sidebar_width`), `scale` (the interface text scale, which reaches only what lies outside the sidebar, since the sidebar keeps its sizes at every scale) and `content` (`reference`, or `long` for long Korean titles).
`/gallery?scene=factory` is the shell's real `Sidebar` beside the real `FactoryScreen` over `web/src/gallery/factorySceneData.ts`, a `FactorySummary` of two Factories with the content `Screen / Factory` draws; the scene answers the screen's requests as the engine does (the settings tab's `config` read, and an answer, merge, retry or cancel taking its item off 내 차례).
Its query takes `theme` (`light`, `dark`), `tab` (`turn`, `board`, `graph`, `settings`), `task` (a Task id such as `t-412`, which opens that Task's page) and `content` (`reference`, or `long` for a long Korean title and a long issue number).
`/gallery?scene=onboarding` is the shell's real `Sidebar` and Home behind the real first-run wizard, start panel or Settings sheet over a synthetic This Mac with the eight agents and a stand-in desktop host (`web/src/gallery/OnboardingScene.tsx`), with the content `Screen / Onboarding` draws.
Its query takes `theme` (`light`, `dark`), `state` (the step and what is open or answered; `ONBOARDING_STATES` lists them), `system` (`mac`, `windows`, `linux`), `lang` (`en`, `ko`, `ja`, `zh-CN`), `content` (`reference`, or `long` for the Korean copy), `scale` (the interface text scale) and, for a `settings-*` state, `page` (the Settings page it opens, Notifications by default, any id of `SETTINGS_GROUPS` or `about`); the root carries `data-scene-ready` once the presses a state needs have landed.
The notification states `settings-off`, `settings-on`, `settings-done-only` and `settings-both` draw the four Needs You/Done choices, alongside the permission and system states.
`/gallery?scene=memory` is the real lane on its Memory tool over invented cards, one state per document; its query takes `theme`, `lang` (`ko` by default, or `en`, `ja`, `zh-CN`), `width` (the lane's CSS width) and `state` (`workspace`, `off`, `ai_not_ready`, `analyzing`, `empty`, `unreadable`, `promote`, `failed`, `start_failed`, `not_processed`, `building`, `unbuilt`, `forgotten`, `verifying`, `reconsent`, `checking`, `check_budget_exhausted`), the states the Memory screen's design draws, `start_failed` for the `failed` card whose status names the question its new pane's shell stopped at, the same row with other words, `not_processed` for a card its agent could not process, opened on the agent's reason, `building` and `unbuilt` for a decision the code does not have yet, with and without a branch that builds it, and `verifying` for waiting cards that reuse the dimmed-row treatment and a shared explanation above the list.
The additional Memory states reuse the existing empty/status slots and put a shared waiting explanation once above the list, with no new token, System part or Component; the gallery is the rendered state reference.
Waiting cards reuse the existing dimmed-row treatment, while only the running check retains a status mark.
`/gallery?scene=view-opener-feedback` is the real Workspace over one checkout with two agents, one that opened a page and a diff and one that opened a PDF (`web/src/gallery/ViewFeedbackScene.tsx`); its query takes `theme`, `scale` and `state` (`opener-tip`, `views-list`, `comment-browser`, `comment-diff`, `comment-pdf`, `send-menu`, `sent`), the seven boards of `Screen / Workspace`'s `vof-*` windows, and the scene makes the PDF's bytes itself.
A page's own pixels and Comment overlay are drawn by the desktop host, which the gallery does not have, so `comment-browser` shows the page's View controls in Comment mode, and the page's pins are judged on the board and in the e2e.
Every name, path and count in a scene is invented example data, so a capture of it is safe to publish.

Onboarding is `Screen / Onboarding`, built by `onboardingBoards` in `scripts/pen-screens-onboarding.mjs`.
It draws the first run as one board per subject, each in both themes: the wizard as a dialog over the dimmed shell, step 1 with its tiles, What changes and its failed, no-agent and reopened states, step 2 in every answer on macOS, Windows and Linux, step 3, the start panel it closes into, the empty Home, the sidebar's Add project on the Projects label, Settings > Notifications on each system (drawn with `Screen / Settings`' sheet), the one-time line on Factory, the startup and failure screen, and the longest copy (Korean at the largest text, Light only).
Each board's themed frame keeps the node id of the approved bundle (`<board>-light`, `<board>-dark`), so a design review exports the same node from the committed file.
Its web files are `web/src/FirstRunWizard.tsx`, `web/src/NotificationControls.tsx`, `web/src/firstRun.ts`, `web/src/StartPanel.tsx`, `web/src/MainScreen.tsx`, `web/src/sidebar.tsx`, `web/src/settings/NotificationsTab.tsx`, `web/src/ScreenIntro.tsx` and `desktop/static/status.css`.

## The flow: scratch to one PR

1. **Scratch.** Create an ignored, library-linked scratch document with `node scripts/design-scratch.mjs <task-slug>`.
   It requires the pinned Pen CLI version and an existing Pen login, links this checkout's `design/hide-ui.lib.pen` read-only, refuses to overwrite an existing scratch, and refuses a symlinked scratch directory.
   The command prints the scratch path (`agents/runs/<task-slug>/design/scratch.pen`) and the exact edit command.
2. **Edit.** Use an independent Pen CLI headless session per file: `pen interactive --in <scratch-path> --out <scratch-path>`.
   Never use the shared desktop MCP or `--app desktop` for agent editing; an explicit MCP `filePath` did not reliably isolate the active desktop document in verification.
   `scripts/pen-cli.mjs` pins Pen CLI 0.3.10 to support SDK-authored library documents and `contain` image fitting.
   Changing that pin requires actual import, rendering, and reopen proof with the new version.
   Inside the CLI, read `read_skill()` and its schema/execute guides, then confirm `get_app_state()` and `list_libraries()` before editing.
   `list_libraries()` returns each imported library's ID; reference a component as `<id>:<component-id>` and a variable as `$<id>:--token-name`, discovering the ID fresh in each scratch rather than reusing another task's alias.
   Only one writer edits a shared library document at a time.
   Save with `save()` and exit with `exit()` before human review; before resuming CLI edits of a document the user opened, the user saves and closes it in the desktop app.
   Never resolve a conflicting design edit by line-merging the `.pen` JSON; keep both versions and reapply the approved change through Pen.
   Library changes become visible in an importing document only after it is closed and reopened; per-instance overrides survive that reopen.
   Each worktree reads its own committed library revision, not another checkout's mutable library path.
3. **Pen toolchain limits to design around**, not to invent workarounds for:
   - Pen is not CSS. Unsupported alignment, margin, and percentage properties must not be invented.
   - Width and height use numeric literals; variable references have rendered at zero in this toolchain.
   - Variable-bound node opacity also renders invisible in this toolchain; the state bindings in `pen-token-map.json` materialize their numeric values during generation and checking.
   - The canvas substitutes JetBrains Mono for unavailable SF Mono and plain Inter because `ss03` does not travel on a token; these substitutions are not a pixel-comparison acceptance gate.
   - A cross-library `ref`'s own internal `$--token` fills and strokes resolve to the imported library's own default (Light) value, regardless of any `theme: {Mode: 'Dark'}` tag anywhere in the importing document.
     A bare local `$--token` on a node the importing document authors itself, and an explicit property override placed at the ref site using the importing document's own local `$--token`, both resolve correctly per theme.
     A descendant-override key on a cross-library ref also needs the alias prefix (`hideui:btn-lb`, not `btn-lb`), not only the ref's own target.
     `design/hide-screens.pen` carries its own local copy of every token, read live through `pen-tokens.mjs`, and `scripts/pen-screens.mjs`'s `themedOverrides()` restates a ref's effective colors by walking its actual master, nested component targets, and inherited instance patches in `design/hide-ui.lib.pen` and copying every `$--token` name it finds into a local override of the same name; a hand-typed color recipe is only for a real per-variant design decision (Button and Badge import `BUTTON_VARIANTS`/`BADGE_VARIANTS` from `pen-system.mjs` for exactly that reason), never for a master's own default.
     `scripts/check-hide-screens.mjs` refuses a ref or a descendant-override key whose colorable properties are not restated this way.
     Nested override paths cross each referenced component instance, with an alias prefix on every imported segment; the checker resolves those paths within the owning master and rejects missing targets, active master cycles, and unrelated nodes.
     An explicit empty paint list (`[]`) clears the imported color in both themes and counts as a local override; an absent property still fails the check.
4. **Human approval.** Show alternatives in the task's scratch document, let the user choose and revise them, and get approval before implementing.
   Record the approved behavior and any proposed system addition in the PRD so the decision survives scratch cleanup; a scratch is not a merge deliverable.
   Keep the chosen design as a reference bundle (see Reference bundle and review run below) before anything else edits it, marked `user` when the operator chose it and `delegated` when it is a proposal made under delegated authority.
5. **Library or screen file.** Promote only the approved reusable components or tokens into `design/hide-ui.lib.pen`, or the approved screen into `design/hide-screens.pen`, with one writer handling the shared update.
   Preserve master IDs when editing so existing instances keep their identity: a component state is a `ref` of its reusable master with descendant overrides, never a redrawn copy.
   Run `node scripts/gen-pen.mjs` after editing the library; it refreshes mapped variables and the Foundations sheet without moving authored sheet positions.
6. **Code.** Implement the change in `web/src/components/ui` or `web/src/components`, reaching every color, spacing, and radius value through a token.
7. **Compare in both themes.** For a `System /` part, compare the gallery's rendering against the Pen export, Light and Dark.
   For a `Component /` or a screen, compare an actual app capture against the Pen export, Light and Dark, because Pen cannot reproduce real app state or Radix behavior.
   For a screen with a review target, `node scripts/design-review.mjs review` does this in one run against the reference bundle.
   Pen has no code export and does not model Radix interaction; the comparison is a human visual judgment, not an automated pixel diff.
   Keep every capture under `agents/runs/<slug>/`; never commit a screenshot, scratch file, or comparison image (see AGENTS.md, Evidence Belongs Outside The Repository).
8. **One PR.** Land the token/library/screen change and the code change together.
   The PR body follows `.github/pull_request_template.md`: one before/after image of the actual screen above the fold, captioned with where to look, and the one visual judgment the reviewer has to make; the rule results as automated facts in Evidence.
   The review run's other comparison images and state captures go in Evidence's folded block (uploaded, never committed); `prd_ship.js screenshots` puts every image it uploads under Summary, so pick the one that stays there and move the rest before publishing.
   A failed rule, an INCOMPLETE item, or a required check that was not run stays visible in Evidence, never only in a folded block.

## Reference bundle and review run

A design change is judged against the design that was chosen, not against whatever the Pen file says by the time the code lands: a later edit to Pen, or the same wrong change made to Pen and code together, would otherwise hide the drift.
`scripts/design-review.mjs` keeps that choice and checks the production screen against it.
The `workspace-servers` gallery scene renders the production top row's Workspace path and icons with invented reachable-address fixtures.
A frame may name `referenceNode` when the approved scratch uses a different node ID; baseline exports that reference node while review exports the committed `node`, retaining both identities and their actual dimensions in the comparison.
A target in `design/review-targets.json` names what one run covers: the committed Pen file and its `Screen /` sheet, the Pen nodes it pairs with the screen and each node's width, theme, text scale, content and state, the gallery scene, the states the scene can be put in, the conditions to measure, the default layout rules, and the questions left to a person.
Targets exist for `session-record`, `sidebar-task-lens`, `pane-header`, `projects-sidebar`, `workspace-servers`, `area-focus`, `factory`, `core-move-devices`, `core-move-window`, `first-run-onboarding` and `view-opener-feedback`, and `design/review-targets.json` is the inventory; add one when a change touches another screen, not before.
`factory` pairs the Light and Dark `Screen / Factory` windows with the Factory scene's 라인, graph and Task page states, takes the delegated proposal's Light picture e3 as the graph's reference of both themes (the 라인 and Task page frames are their own reference), and measures the screen with `web/e2e/factory-geometry.mjs` (no overlapping parts, no sideways overflow).
Its selector is the scene root, not `[data-factory-screen]`, because the Pen frame is the whole window and a comparison image has to be as wide as its frame; `[data-factory-screen]` is its `readySelector`, what a capture waits for after the scene loads.
A target's `urlStates` lists the states its scene sets up from its address (`state=<name>`), or maps each state to the query it sets, as `first-run-onboarding` does to put a state on Windows or Linux.
`first-run-onboarding` pairs the approved boards (PRD first-run-onboarding D-20, D-27) with the onboarding scene at rest and captures every state the scene draws in the state sheets, since one board draws several states; it measures the wizard with `web/e2e/onboarding-geometry.mjs` (no overlapping parts, no sideways overflow, no letters cut off at the bottom, no squeezed agent mark, and the wizard inside the window) at text scale 1 and at 17/13, the largest Appearance offers (B41).
`area-focus` measures the production shared area renderer and View tabs with read-only Korean content, enforcing one keyboard area, retained selections, stable geometry and unfiltered content; real terminal and native browser behavior still requires an isolated app capture.

### Keep the chosen design: `baseline`

```sh
node scripts/design-review.mjs baseline <slug> --target projects-sidebar --approval user|delegated --reference "agents/prd/<slug>/prd.md D-08" [--name <name>] [--from <file.pen>] [--rules <rules.json>]
```

- It copies the target's Pen file (or `--from`, such as a scratch proposal), every library it imports and every image it uses into `agents/runs/<slug>/design/baseline/<name>/`, rewriting the references so the bundle opens in Pen from any directory or machine (two different files that would share one name in the bundle are refused), and exports the target's nodes from that copy, so the images prove the copy is whole.
- `manifest.json` records when, from which file and commit, with which Pen version, each frame's conditions and image, the layout rules, and a hash of every file.
- `--from` has to name a file inside this checkout, because the bundle records whether it has uncommitted changes with `git status`; a proposal made in another worktree is copied under `agents/runs/<slug>/design/` first, where the library paths it imports keep resolving.
- `--approval` separates the operator's choice from a delegated proposal, and `--reference` points at the decision in the PRD instead of restating it.
- A bundle is never overwritten: a rerun with the same name is refused, and a failed export publishes nothing.
- `node scripts/design-review.mjs show <bundle>` prints the approval, frames and rules and checks every hash, with neither Pen nor a browser, so an implementor on a machine without Pen still reads the numbers and opens the PNGs.

### Check the screen against it: `review`

```sh
node scripts/design-review.mjs review <slug> --baseline <bundle> [--theme light,dark] [--width 292,240] [--content reference,long] [--scale 1,1.25] [--state rest,hover-parent,...] [--no-pen]
```

One run, in this order, with each step's real result in `agents/runs/<slug>/design/review/<run>/report.md` and `report.json`:

1. The static contract: `node scripts/check-design-contract.mjs`, as CI runs it.
2. The current Pen: the bundle's nodes exported from the target's Pen file in this checkout, so a proposal bundle is set against what the checkout now carries.
3. The production screen: the command starts its own Vite dev server and headless Chromium, opens the target's gallery scene under each selected condition, and measures the bundle's rules there through `web/e2e/sidebar-geometry.mjs`, the same geometry the e2e specs assert on.
   The rules are the bundle's, never values read back from the code under review: `childIndentPx` (a child's status mark and title start that many pixels per level right of its list's roots), `rootsAligned`, `stableUnderHover` and `stableUnderFocus` (no row moves or resizes while any one row is hovered or keyboard-focused), `noOverlap` (no part of a row overlaps another or runs past the row, no text spills), `sharedColumns` (one column for every time and badge end, one for every chevron) and `noSidewaysOverflow`.
4. Comparisons: for each bundle frame, the reference, the current Pen and the actual screen side by side, captured under that frame's width, theme, text scale, content and state (a target without `scales` in its conditions is measured at scale 1, as `projects-sidebar` is, because the sidebar does not follow the scale) and labelled with them; an image whose width is not the frame's is reported as a different condition, never scaled to look alike.
5. State sheets: every selected condition and state no Pen frame draws (a narrow width, a larger text size, long titles, hover, focus, a folded parent, a closed checkout) as captures of the actual screen only, labelled as such.

The exit status is 0 for PASS, 1 for FAIL (the static contract or a rule failed) and 3 for INCOMPLETE: a missing Pen or Pen login, a node Pen did not export, a bundle file that no longer matches its hash, or a browser step that did not finish.
An INCOMPLETE report names the cause and the command to rerun on a machine that can render; it is never read as a pass.
A PASS covers only the rules; the comparisons and the report's judgment list are for a person.
The run owns its dev server, browser and Pen session and closes them on every exit, including an interrupt at any step, which ends the whole run; a process it cannot close is reported, and it touches no other app, pane or server.
Cancellation joins the pending Pen command before the review exits; its guard reaps the CLI before ending the remaining owned process group, with a five-second cleanup cap.

Only the selected conditions are measured, so a change names what it touched (`--theme dark --width 240 --content long`), and a bundle frame is always captured under its own conditions so its comparison exists.

## The screen transplant procedure

`design/hide-screens.pen` is one shared file, so parallel PRDs working on different `Screen / <Area>` sheets do not line-merge its JSON.
Instead, a branch transplants only its own sheets into main's copy of the file, node by node:

```sh
node scripts/pen-transplant.mjs --from <branch-file> --into <main-file> --sheet <sheet-id> [--sheet <sheet-id> ...]
```

- `--from` is the branch's `design/hide-screens.pen` (or another `.pen` file holding the authored sheets); `--into` is the target file to write, ordinarily main's current `design/hide-screens.pen` after a rebase.
- `--sheet` names one or more top-level sheet IDs to move; the script refuses a sheet ID that does not exist in `--from`.
- The script replaces (or adds) exactly those sheets in `--into` by node identity and leaves every other sheet in `--into` untouched, so two branches that touched different `Screen /` areas both land cleanly.
- Run `node scripts/gen-pen.mjs` and `node scripts/check-design-contract.mjs` against the result before committing; a transplant that leaves stale Foundations or an unrecognized sheet name fails the same checks a normal edit would.
- Never resolve a `hide-screens.pen` conflict by hand-merging JSON; re-run the transplant with the correct `--sheet` list instead.

For an owned screen regeneration, `node scripts/gen-screens.mjs --sheet <sheet-id>` generates the candidate and uses the same node-identity transplant internally, keeping every other sheet byte-for-byte and refreshing the shared local token variables.
Omitting `--sheet` still regenerates all screens.

## Web screen list

The Projects Sidebar screen and production gallery draw tabs as segments, not depth: one root status axis on the checkout name's column, a 1px guide per tab centered under the checkout's kind glyph, and for a tab of two or more agents, or a named tab, one quiet line inside the guide, its glyph on the status-mark column and its manual name or `Tab N` on the title column.
The segments separate with `--spacing-sm` rather than another card, hang `--spacing-xxs` under the checkout row and end with `--spacing-sm` before the next row, keep the existing 18px lineage step, and use neutral `--secondary` selection; a plain folder's glyph and name stand on the project row's columns.
`sidebar-status-axis` is a production Sidebar gallery scene with invented 2-1-2 tab facts (a named tab, a one-agent tab, an unnamed tab), paired with the `psb-axis-l` and `psb-axis-d` Pen cutouts by `design/review-targets.json`.
The `sidebar-status-axis` target captures the real checkout row with 8px of surrounding sidebar padding to match the 240px cutout.
Its 900px square browser viewport keeps the device rail and every row visible; `capturePadding` changes the screenshot bounds only, never the rendered layout.
`sidebar-tab-groups.spec.ts` owns the live tab geometry and `sidebar-root-only.spec.ts` owns the visible singleton lineage disclosure to children in other tabs.
`scripts/pen-screens.mjs` owns the composed segment, guide and root offsets (`segment`, `tree`); `web/src/gallery/sceneData.ts` supplies explicit tab naming metadata.
No `System /` or `Component /` master carries a tab part, so `design/hide-ui.lib.pen` is unchanged: the segment is drawn from primitives in the screen file.

`design/hide-screens.pen` draws every area the web shell shows today, one `Screen / <Area>` sheet per area, imported from `design/hide-ui.lib.pen` the same way a `Component /` sheet is drawn from `System /` masters.
Each sheet carries a `Light` and a `Dark` frame and uses realistic content, including Korean labels and a long path, to show real wrapping and truncation rather than an abstract state.
`scripts/check-hide-screens.mjs` enforces the shape (`Screen / ` naming, both theme frames, every reference resolving against the library, every cross-library color restated locally, and local variables matching `design/tokens.json`) and `scripts/gen-screens.mjs` regenerates the file from `scripts/pen-screens.mjs`.
Because the file is generated and several megabytes of JSON, `.gitattributes` marks it `-diff linguist-generated=true`: `git diff` reports only that it changed and GitHub folds it in a pull request, so review reads `scripts/pen-screens.mjs`; `git diff --text` still shows its lines when they are needed.

The device Overview is `Screen / Overview`, drawn by `scripts/pen-screens-overview.mjs` (PRD overview-device-tasks, sidebar-task-lens D-20): one device's `백로그 · 프로젝트 · 사용량` under the title and the device switch, the panel beside the tabs with an item or, at Home, a session's record, the device that does not answer, and the ⌘⇧M quick add.
Its backlog rows and Projects rows are drawn there from primitives, as the web draws them, with the library's System masters as refs for the controls.
`Screen / Main` is the window with no Workspace in front: the Overview as the page beside the sidebar, opened on 백로그, whose Overview row is marked.
Their web files are `web/src/MainScreen.tsx`, `web/src/Overview.tsx`, `web/src/BacklogTab.tsx`, `web/src/ProjectsTab.tsx`, `web/src/OverviewRecord.tsx` and `web/src/QuickAdd.tsx`.

The sidebar's `프로젝트 | 태스크` switch and 태스크 list, the lane's 보기 slot and the PRs tool (PRD sidebar-task-lens) are eight `Screen / Task lens` sheets: A the window on 태스크 by status, B the sidebar column's states, C a PR mark pressed with the card in the 보기 slot, D the PR card and its cases, E a session record in the slot, F the rail and the PRs tool with its states, G the Overview without Tasks, and I a changed file opened from the card and the way back.
They were moved in from the operator-approved v4 boards with `scripts/pen-transplant.mjs`, keeping the boards' node ids so the `sidebar-task-lens` review target exports the same nodes the reference bundle holds, rather than drawn by a builder in `scripts/pen-screens.mjs`; `scripts/gen-screens.mjs` leaves a sheet it does not generate in place, so they change by a new transplant.
The proposal's before-and-after board and the PR card's old column were left out, since they drew the screen this change replaced.
The seven `Screen / Work` sheets of the retired Work tool and the Sessions panel on `Screen / Main` were removed in the same change; `RETIRED_SHEETS` in `scripts/pen-screens.mjs` names the Work sheets, and a scoped `gen-screens.mjs --sheet` run removes a retired sheet as a full run does.
Their web files are `web/src/TaskLens.tsx`, `web/src/taskList.ts`, `web/src/DetailPanel.tsx`, `web/src/PrsPanel.tsx`, `web/src/PrList.tsx`, `web/src/PrCard.tsx`, `web/src/PrLinks.tsx` and `web/src/prReason.tsx`.
`/gallery?scene=session-panel` renders the production 태스크 list alone, so the windows, the slot and the PRs tool are judged against an isolated app capture rather than a scene.

Workspace is `Screen / Workspace`.
It draws Agent Views, the right lane and the tool rail beneath the window's top row (PRD right-lane D-17; PRD sidebar-titlebar-refresh D-15, D-16): the traffic lights, the sidebar's switch and This Mac over the device rail and the sidebar, and over the center the Workspace toolbar master as the `project / checkout` path with Overview and Open server, using the Workspace columns, toolbar and divider masters in the library; the sidebar beside it opens with the Search row and has no Home row.
The tool rail carries Memory and PRs over the divider and Files and Changes under it with the pressed tool's bar, the Files badge while the lane does not show Files, PRs' warning badge with its Needs You count, and the `…` menu; the lane carries its `tool · scope` header with Pin, Expand and Close, and the files tool its tree button and ` › ` path.
The board draws the approved v3 states: the lane closed, over the agents, pinned and docked, the tree on over the agents, the tree alone with no document, expanded, PRs pinned, and a narrow window with the pin filled over the agents.
The pane header keeps the agent-hierarchy round 2 identity and ask-band patterns: the ancestor path, fork mark and own PR remain on the identity row.
Its second row follows the approved 2026-10-18 pane-header-progress design: a tree icon, direct-child number and chevron at the left open the existing popover; one operator request or the pane's own live progress occupies the rest, and a raised request names its child origin.
The fork mark is the issue 916 addition, chosen by the operator as a small icon after the title and drawn in the board's Forked pane section.
The pane-header main and edge boards and `Component / Agent tree parts` draw the twelve approved situations in Light and Dark, including progress-only, tree-only, empty, finished, open popover and narrow text; `design/review-targets.json` names both boards as the `pane-header` target.
The approved local Pen sheet is the frozen visual reference, and the production gallery and isolated native capture supply implementation evidence.
The reference includes the lane's and the tree's dividers at rest, under the pointer, focused and being dragged.
Light and Dark frames carry Korean text and long paths, including the one-line shortened document path and its full-path tooltip.
The server-picker comparison retains its distinct reachable endpoints, and the area-focus comparison retains two View areas with one keyboard owner and independently readable selections.
The remote Workspace keeps the device-colored band at the start of the top row's path and the agent pane frame; This Mac has neither.
The Explorer tree's rows use the shell's indent (`--size-tree-indent`), chevron and mark column (`--size-tree-mark`) and hairline guides.
The shell's marks are Material Icon Theme SVGs, which Pen draws empty, so the sheet draws lucide glyphs in the theme's hues in their place; the production gallery's `right-lane` scene (`state=tree-only`) is the reference for the marks themselves.
Its web files are `web/src/WorkspaceScreen.tsx`, `web/src/TabBar.tsx`, `web/src/ViewAreas.tsx`, `web/src/Tools.tsx`, and `web/src/ExplorerTree.tsx`.

PR and session cards are the authored `Screen / PR card` and `Screen / Session card` sheets, transplanted from the operator-approved local proposals with the screen transplant procedure above.
They replace the generated `Screen / Session record` sheet and supersede the standalone card arrangements in Task lens D and E; those task-lens sheets still describe their surrounding navigation.
The PR sheet carries wide and narrow cards in Light and Dark, plus failed and ready cases.
The session sheet carries wide and narrow cards in both themes, plus a child expanded in place.
These sheets are not rebuilt from a screen generator; edit them through the same authored-screen procedure and keep their stable frame ids.
`/gallery?scene=pr-card` renders production `PrCardView` over `web/src/gallery/PrCardScene.tsx`, with `state=rest|failed|ready|unknown|skipped` and `web/e2e/pr-card-geometry.mjs` as its measurement.
`/gallery?scene=session-record` renders production `SessionRecord` over `web/src/gallery/SessionWorkScene.tsx`, with summaries Off, pending judgment, question, loading and failed cases, measured by `web/e2e/session-record-geometry.mjs`.
Both galleries accept `width`, `theme`, `lang`, `scale` and long content for wrapping checks; gallery values are invented examples, and isolated daemon tests own runtime behavior.

Settings is `Screen / Settings`, built by `settingsBoards` in `scripts/pen-screens-settings.mjs` from the boards the operator approved (PRD settings-left-nav D-11).
It draws the ⌘, sheet with its list on the left, App (General, Notifications, Devices, Shortcuts) and This Mac (Agents, Hide AI, Mobile, GitHub) with About at the list's foot, and each page under its title alone: General as Appearance, Notifications with This Mac and Phone, Agents with its machine switch, Hide AI, GitHub and About.
Its States board draws a remote device's chip beside the page title, the smallest window (720×480), and the longest copy, Korean at the largest interface text, Light only.
The list, the title and the pages are drawn on local tokens and library refs, not as a `Component /` master, because they are one screen's parts; `settingsFrame` in the same file is the sheet around any page, so `Screen / Onboarding`, `Screen / Core move` and `Screen / Mobile` draw Settings with the same list and title.
Agent marks that ship only as SVG (Pi, Cursor and the others in `web/src/assets/agents/`) draw empty in Pen, which fills an image only from a raster.
Its web files are `web/src/SettingsSheet.tsx`, `web/src/settings.ts` and the pages under `web/src/settings/`.

Palette is `Screen / Palette`.
It draws the sidebar's Search row with its ⌘K chord, the ⌘K palette's wide list-and-detail layout with its relation list, grouped results, collapsed, no-match and GitHub states, and the ⌘P file palette on the same shell.
`/gallery?scene=search-palette` is the real ⌘K over `web/src/gallery/cmdkSceneData.ts` (`front=agent|agent-short|terminal|none`, `github=results|pending|failed|empty`), which answers `github_search` as the core would.
Its web files are `web/src/Palette.tsx`, `web/src/SearchPalette.tsx`, `web/src/search.ts`, and `web/src/components/sidebar-header.tsx`.

Dialogs and Sheets is `Screen / Dialogs and Sheets`.
It draws every Dialog and AlertDialog surface the shell opens: New worktree, Delete worktree, Remove project, Purpose, Unsaved drafts, Add a project, Keyboard shortcuts, New issue (with `만들고 바로 시작` unchecked and checked), and Start from an issue; the committed sheet still draws the removed sidebar New workspace panel in Add a project's place until the screen is redrawn.
Its web files are `web/src/WorkspaceDialogs.tsx`, `web/src/AddProjectDialog.tsx`, `web/src/DraftRecovery.tsx`, `web/src/ShortcutSheet.tsx`, and `web/src/IssueDialogs.tsx`.

Menus and Overlays is `Screen / Menus and Overlays`.
It draws the sidebar row menu, the Explorer context menu, the hidden rail's device menu (the devices, Add device, Show rail), and the Explorer root row's Git status marks (loading, a failure the operator can repair, none), each anchored in its real screen context.
Below them it draws the ⌘N start panel floating over the window as ⌘K does: one line to write, the target, agent kind and model dropdowns, and the start button with its Enter key.
The panel is drawn at rest and with each dropdown open: the target menu (Home first, the front device's checkouts, then each other device's Home, an unreachable one dimmed with `연결 안 됨`), the kind menu (Claude, Codex), and the model menu for Claude and for Codex.
Its web files are `web/src/entry-menu.tsx` and the start panel under `web/src/`.

Projects Sidebar is `Screen / Projects Sidebar`.
It draws the sidebar that follows the device rail.
The rail is its full-height left column with This Mac and each registered device as 32 tiles with no name under them (the laptop glyph or the device's monogram), and `+` directly under the last tile; the selected tile is ringed, one mark at a tile's top-right shows the most urgent state (the Needs You count, `9+` from ten, else a dot for unseen Done), Working has no mark, and an unreachable device dims its glyph and wears a cross at the bottom-right with no mark.
The rest frame is This Mac in front under the window's top row, which carries the traffic lights, the sidebar's switch and the device name over the rail and the sidebar (PRD sidebar-titlebar-refresh D-15): the Search row with its ⌘K chord, the shared Overview row with its count and shortcut, then the single sidebar's Needs You and Done groups, `Pinned · 1` led by the pin, `Projects · Recent activity · 5` ending in Add project, the projects, and `No agents 3` and `Inactive projects 3` folded; there is no Home row (D-06).
A working project's name is 600 and a resting one's 400, and two projects whose agents all sleep are folded behind the moon cell with its age (D-02, D-03); the footer carries the usage chips, Help and the gear.
Beside it, cuts of the list draw the weight, a sleeping project folded and opened, `+ ⋯` over the badge and over the moon under the pointer (D-04), No agents opened, `Projects` stuck to the list's top with its hairline while rows pass under it (D-05), and the Help menu above the footer with `?` pressed (D-12).
Four more frames draw a remote device in front with the `9+` pill and its Needs You past the cap (five rows and `More 7`), a device that is not connected (its name, `연결 안 됨` and one reconnect action, no tree), the one-device window (the rail with This Mac alone), and the rail hidden (the top row's name with a chevron over the open device menu).
In the list, pinned and activity-ordered projects contain checkout rows with their kind glyph, age and agent line, an opened checkout's agent rows, and the checkout folds.
The project, checkout, folder and agent row menus stand beside it; the checkout and folder menus carry `Open in VS Code` before the reveal (D-13), and the project menu is drawn opened from `⋯`.
The shared Overview row owns selection on its page or modal; there is no per-project Overview child.
A project row opens its last focused usable checkout, falling back to primary and then first usable, and unfolds; activation while its target Workspace is already in front and unfolded folds it.
A checkout row opens and unfolds, then folds on activation while already selected and unfolded.
Beside each theme's sidebar it draws a pull-request row under the pointer with its card (`Component / PR hover card`) opened to the right, the state the row's tooltip has become.
Its web files are `web/src/sidebar.tsx`, `web/src/components/sidebar-header.tsx`, `web/src/projects.ts` and `web/src/components/pr-card.tsx`, plus the rail component under `web/src/`.

Usage is `Screen / Usage`, composed from `Component / Usage windows` and `Component / Usage history`.
The operator selected the compact A HTML proposal with cost first, thirty contribution squares and a date/agent table; the generated sheets record that accepted direction, rather than treating the previous year-calendar boards as current approval.
The history component carries light/dark popover rest, day, unpriced, backfill and sparse states, and the Overview table.
Its grass uses the shared secondary zero level, four primary intensity tokens and 12-pixel cells with a 3-pixel gap.
The screen combines the compact account rows with the popover and the full history table; Overview's account details start collapsed.
The `usage-history` review target checks the table in wide and narrow/large-text states; `scene=usage-history` opens the production component with invented reference data.
Its web owners are `web/src/components/usage-history.tsx`, `web/src/components/usage-history-table.tsx`, `web/src/components/usage-amount.tsx` and `web/src/UsageOverview.tsx`.

Mobile is `Screen / Mobile`.
It draws Settings > Mobile as a page of `Screen / Settings`' sheet, blocked on a failing Tailscale check with only that step lit, one action beside it and no QR, then ready with every check passing, the QR, the ts.net address, the code countdown with 새 코드, and the connected phones (2 / 4) with 해지; the push modes are on Settings > Notifications.
Beside it, the phone app: the pairing confirm the QR opens, the list in four groups (내 차례, 읽을 것, 작업 중, 확인함), the detail with its read-only scrollback, five quick keys and one-line reply, the unreachable state over the dimmed last list, the empty state, the push banner, and the start sheet the list's `+` opens (text box, target, kind, model and `시작` over the dimmed list).
The machine name and tailnet are placeholders, and the QR is a drawn pattern, never a real code.
The Settings > Mobile frames are stale against the shipped tab: they draw the QR open with a `새 코드` button as soon as the checks pass, and one row per Tailscale check.
The shipped tab opens with no code and a `Show QR` row that turns into `Hide QR`, offers `Show QR` again in place of `Hide QR` once the code has expired, and collapses a passing Tailscale check to one ready line.
The phone app frames are unchanged.
Its web files are `web/src/MobileTab.tsx` and `web/src/mobileSettings.ts`, and the phone app under `web/src/mobile/` with its entry `web/mobile.html`.

Disk Cleanup is `Screen / Disk Cleanup`.
It draws the Overview facts line with the disk number's breakdown tooltip and the low-disk cell, the cleanup sheet as a checkout x layer table with every checkbox scope (cell, row, column, top-left, the fold of small checkouts) and the Checkbox's Indeterminate, the states the table carries (measuring, not measured, in use with its reason, usage unreadable, another cleanup running, no row for the filter), the confirm step a worktree pick adds, the running step and the result.
The library Checkbox and Button are refs; the table is a screen-local grid drawn in `scripts/pen-screens-disk.mjs`, with no `System / Table` behind it.
Its web files are `web/src/DiskCleanupSheet.tsx`, `web/src/diskCleanup.ts` and `web/src/ProjectsTab.tsx`, the Overview's Projects tab it opens from.

Factory is `Screen / Factory`.
It draws the Factory place in the window, from one invented data set so the 결정 필요 count, the 라인 tab number and the sidebar badge agree.
The sidebar carries the Factory row under Overview with that number in warning and ⇧⌘F, each Factory under it, and the 비서 row beneath them once a Factory exists.
The header is Factory, the project filter, the pause, `+ Factory 만들기` and `비서에게 묻기`, then the tabs `라인 · 보드 · 그래프 · 설정` with the Factory's state at the row's end: the last GitHub read (dimmed once it is old), `main 깨짐` and the daily AI limit; there is no flow bar.
The 라인, 결정 필요 and Task page frames (`fx-loop-*`, PRD factory-human-loop) are the operator's approved design and lead the code, which still draws 내 차례 until that PRD lands.
라인 opens with 결정 필요, only what the person moves, every item one shape: a sentence, what it stopped, two or three choices with their results and the recommended one, `다른 답`, the default and its deadline, or one button for a to-do (with a copyable command where the person runs one), and the evidence folded at its foot.
Under it is one table row per Task (title, issue and PR, the steps `접수 · 작업 · 검증 · 머지` with the current one named, what it does now in one sentence, elapsed time, how many answers Factory AI gave), the person's turn alone in the warning band, and 후속 후보 folded at the foot; a second frame draws every 결정 필요 kind at once.
The Task page reads top to bottom: title and state sentence, the track with who decided where, summary, the criteria checklist, decisions split `내가 정한 것` / `AI가 정한 것` with `다른 답`, the activity timeline with four-part reports, the original issue rendered, and the Task's 후속 후보.
The 라인 with no Task, its first read and a Task page that could not be read are frames of their own.
보드 has the four columns with the person-turn cards in the warning border on top, a state mark on every card and old completions folded into one line; 그래프 lays the Tasks out in layers with arrows, draws none for a dependency a longer path implies, dims finished work and puts the unrelated Tasks below.
The create sheet is drawn detecting, with the required checks, with verify command candidates and with nothing detected (auto unavailable, manual), each listing what it writes on GitHub; with no Factory the screen offers only `+ Factory 만들기`.
The cards, rows, 결정 필요 items, track and sheet are drawn on local tokens and library refs in this sheet, not as `Component /` masters, because each is one screen's part (`web/src/factory/`) rather than a composite under `web/src/components`.
Its builder is `scripts/pen-screens-factory.mjs`, and its web files are `web/src/factory/FactoryScreen.tsx`, `MyTurn.tsx`, `FactoryBoard.tsx`, `FactoryGraph.tsx`, `FactoryCard.tsx`, `TaskPage.tsx`, `CreateSheet.tsx` and `web/src/components/sidebar-header.tsx`.

Core move is `Screen / Core move`, the design the operator approved for moving the core between machines (PRD core-host-node-move).
Its first board draws the Devices rows with the crown `core` badge, the move entry in a connected device's ⋯ (dimmed with `연결 안 됨` on a device that is not connected), and this Mac's ⋯ after the move with the return and the disconnect.
The second draws the move dialog over Settings in its five states: checks failed, confirm, moving, failed and done.
The third draws the rail with the crown notched into the core machine's tile, in the MacBook's window and in the mini's.
The fourth draws the window states outside Settings: moving, updating the core, cannot connect, and the host status page for an app older than the core.
Beside them, the disconnected strip with its `다시 연결` link (B16) is drawn by analogy with the cannot-connect strip and is not part of the approved bundle.
Its builder is `scripts/pen-screens-core-move.mjs`, the gallery scenes `core-move` and `core-move-window` (`web/src/gallery/CoreMoveScene.tsx`) draw each frame's state for the review, and its web files are `web/src/settings/DevicesTab.tsx`, `DeviceRow.tsx`, `CoreMoveDialog.tsx`, `web/src/components/device-rail.tsx`, `web/src/badge.tsx`, `web/src/connection.ts` and `web/src/coreMove.ts`.

## How to add a token

1. Add the entry to `design/tokens.json`, with both a Dark (`value`) and a Light (`light`) value for a color token, or use `type: "alias"` to point at another token.
   When the Hide color theme draws the color differently, add `hide: {"value": <Dark>, "light": <Light>}` too; an alias takes its target's Hide value and carries none of its own.
   A `hide` that repeats both Neutral values is refused by the generator, so a later Neutral change cannot leave Hide on the old color.
2. Run `node scripts/gen-tokens.mjs` to regenerate `web/src/tokens.css` and `web/src/generated/accents.ts`.
3. Run `node scripts/gen-pen.mjs` to carry the same value into the Pen library's `Mode` and `Palette` variables and Foundations sheet, and `node scripts/gen-screens.mjs` for `design/hide-screens.pen`'s local copy.
4. Reference the token from Tailwind classes or CSS custom properties in `web/src` (`check-web-tokens.mjs` refuses a literal hex, `rgb()`/`hsl()` or `px` value anywhere in a source, including a CodeMirror theme object or another stylesheet, and Tailwind's own default spacing/text scales, since those are also unchosen literals).
   The generated `agent-tab-count-room` container variant compiles two `--size-control-sm` lengths into its condition, so Agent tab counts use the same numeric authority without a runtime `var()` in the query.
5. A visual case the token system does not cover is a proposed addition, reviewed and approved before use, never settled with a one-off value.

## How to remove a token

1. Delete the entry from `design/tokens.json` once nothing in `web/`, `scripts/` or `docs/` reads it (`rg` the name).
2. Run `node scripts/gen-tokens.mjs`; the token leaves `web/src/tokens.css` and `design/hide-screens.pen`'s local variables with it.
3. Generation never deletes a canvas variable on its own, so list the name in `retired` in `scripts/pen-token-map.json`, run `node scripts/gen-pen.mjs` to drop it from `design/hide-ui.lib.pen`, then delete the entry from `retired`.

## How to add a System part

A `System /` sheet is a shadcn part redrawn with hide tokens, name and variant 1:1 with shadcn's own naming.

1. Start from the shadcn source for the part and copy its structure onto hide's tokens rather than inventing a new visual language.
2. Add or extend the part's section in `web/src/gallery/manifest.ts`: the section name matches the sheet name (without the `System / ` prefix) and the state list matches exactly what the sheet's `Light` and `Dark` frames draw.
3. Draw the `System / <Name>` sheet in `design/hide-ui.lib.pen` with `Light` and `Dark` frames, each holding one node per state named identically to the manifest list.
4. Implement (or confirm) the part in `web/src/components/ui/<name>.tsx`.
5. Run `node scripts/check-pen-gallery.mjs` to confirm the sheet and the manifest agree, and `node scripts/check-design-contract.mjs` before delivery.

## How to add a Component

A `Component /` sheet is a hide composite assembled from `System /` masters (never redrawn from scratch).

1. Confirm the composite is actually needed in the current web screens; do not add a Component for a screen that has no approved local contract or public review plan.
2. Compose it in the scratch document from `System /` masters as refs with descendant overrides, so it inherits token updates automatically.
3. Implement it as a hide composite in `web/src/components/<name>.tsx`, built from the same `web/src/components/ui` parts the sheet composed.
4. `Component /` sheets are not covered by the gallery; compare them against an actual app capture in both themes instead (see step 7 of the flow above).

## Checks

`node scripts/check-design-contract.mjs` is the entrypoint `design-contract.yml` runs, and it runs:

- `check-pen.mjs` - refuses a Pen library that is not what the token generator would produce: a stale token value, a stale Foundations sheet, a top-level sheet whose name carries neither the `System /` nor the `Component /` prefix, or an id that names more than one node.
  A node written whole inside a ref's `descendants` counts: its key already addresses the node, so an `id` of its own makes Pen's loader report duplicate ids.
- `check-pen-gallery.mjs` - refuses a `System /` sheet and `web/src/gallery/manifest.ts` that name different parts, or a state drawn on one side and not listed on the other, for either theme frame.
- `check-web-tokens.mjs` - refuses a web source file that reaches a color, size, or radius through a literal instead of a token: a hex, `rgb()`/`hsl()` or `px` literal anywhere in the file, an inline color style, a Tailwind arbitrary `[...]` value, or one of Tailwind's own default spacing/text scale classes.
- `check-token-contrast.mjs` - refuses `design/tokens.json` when a terminal color used for text falls below 4.5:1 on the terminal background, or `--muted-foreground`, `--subtle-foreground` or the Seen mark's `--status-seen` below 4.5:1 on the background, the sidebar or a card, in Neutral or Hide, Dark or Light; terminal black in Dark and white and bright white in Light are background colors and are left out.
- `check-hide-screens.mjs` - refuses a `design/hide-screens.pen` top-level node not named `Screen / `, an id that names more than one node, a `Screen /` sheet missing a `Light` or a `Dark` frame, a local `$--variable` the file does not define, a ref or descendant-override key whose import alias or target id does not resolve against the imported library, a cross-library ref that leaves one of the imported master's own colors un-restated locally, or a local variable block that differs from what `design/tokens.json` generates.

`--staged` reads the exact staged content of the design inputs and checker files into a temporary directory and checks that, without touching the index or working tree; the tracked `.githooks/pre-commit` runs it and is opt-in (`git -c core.hooksPath=.githooks commit`).
The node test suites `scripts/tests/pen-gallery.test.mjs`, `scripts/tests/pen-transplant.test.mjs`, `scripts/tests/design-scratch.test.mjs`, `scripts/tests/design-review.test.mjs`, `scripts/tests/hide-screens.test.mjs` and `scripts/tests/token-contrast.test.mjs` exercise the gallery comparison, the transplant script, the scratch creator, the reference bundle and the review rules, the screen-list checker and the contrast check respectively against fixtures; they do not prove Pen rendering or browser layout, which stay a local `design-review.mjs review` run with the real Pen CLI and Chromium.

These are static source checks, not a rendering engine or an aesthetic evaluator: they catch drift between the files that are supposed to agree, not whether a composition looks right.
Visual approval is always a human judgment made from a gallery-or-app capture beside the Pen export, recorded under `agents/runs/<slug>/` and never as a committed image; the review run's rules make layout drift a failure, not an approval.
