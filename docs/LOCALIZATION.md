# Interface localization

Issue #339 defines the interface language policy.
The core owns one explicit choice, shared by clients of that daemon.
`ui_state.interface_language` is `en`, `ko`, `zh-CN`, `ja`, or null.
Null and an absent field mean each client uses its primary OS or browser language.
Supported regional English, Korean and Japanese locales resolve to their base language.
Chinese locales that resolve to the Hans script use Simplified Chinese (`zh-CN`); other Chinese scripts and unsupported languages use English.
The client does not search a secondary preferred-language list or persist a resolved system language.

## Ownership and persistence

Only `interface_language_set {language}` edits the choice; null explicitly returns to system language.
The parameter is required, and unsupported event values are refused without changing the choice.
A stale `ui_state_update` cannot change the language.
Changes use the existing coalesced UI-state save and snapshot publication, with no new timer, worker or storage file.
Repeating the current choice publishes no change.
An invalid stored value produces `ui_state.interface_language_invalid`, publishes English and remains in the store through unrelated saves.
The diagnostic never includes the invalid value.
Old state files need no migration.

## Translation boundary

`web/src/i18n/locale.ts` owns client locale resolution.
`instance.ts` creates an independent translator from the embedded four-language catalogs, validating keys and interpolation placeholders before initialization.
`client.tsx` owns one in-memory translator per shell page and follows confirmed snapshots and browser `languagechange` events.
It sets the document language and subscribes React surfaces through react-i18next.
Language selection is disabled while disconnected, and displays only the confirmed core value while connected.
A client never translates conversations, terminal bytes, file contents, paths, branch names, device labels, task labels or other user-owned text.
Such values are interpolation data, rendered as text.
Numeric identifiers, including Memory's PR numbers, are passed to translation as digit strings so locale grouping never changes their spelling; quantities retain locale number formatting.
Diagnostic payloads and copied diagnostic records remain technical source data.
Missing keys and missing interpolation values fail explicitly; English is a locale-resolution fallback, never a missing-key fallback.

## Current integration coverage

The web shell renders every product-owned sentence of its main screens from the catalogs.
That covers Settings (every tab), the sidebar rows, menus and close sheets, the shared Overview with its 백로그, Projects and Usage tabs and the ⇧⌘M quick add, the sidebar's 태스크 list, the lane's 보기 slot, session record, PRs tool and PR card, the window's top row, tab bars and pane chrome, the editor and file views, the Explorer and History, browser displays, the palettes, the start panel, the add-project and workspace dialogs, disk cleanup and the device rail.
Shortcut keycaps retain their host-specific chord glyphs in every language.
Text that a helper composes outside a render uses the `translate` function of `client.tsx`, which reads the confirmed language at call time; a helper that renders inside React takes the component's `t`.
A module the desktop host imports (`host.ts` and everything it reaches) stays free of the translator and names a label by key, as `revealExternal.ts` does.
Where `UI_BEHAVIOR.md` quotes shipped Korean wording (`요청`, `정리`, `맡기기`), that is the Korean catalog text; English and the other languages read their own catalog entries.
The core sends a code, never an English sentence, for what the screen words: an agent's `status_code` (`working`, `question`, `sleep_failed`, ...) is named by `agents.status.*` through `statusText` in `web/src/agentStatus.ts`, a GitHub `failure_category` by `issueSettings.gh*`, and the machine the core runs on as a device row with no `label` and its system in `host.platform`, which every shell words as This Mac, This PC or This computer (`systemMachineKey` in `web/src/screenMachine.ts`), the phone's start sheet included, which reads a null `place` as Home and a null `device_label` as that machine, named from the catalog's `core_platform`.
A delivery notice for a person is a `HumanNoticeKind` plus the names it concerns: the phone words it from its state key (`mobile.notice.*`, relayed to the service worker with the other notification words), while Herdr's own notification, which has no translating screen, shows the English title and body whose tail names the `hide request show` command.
Where the core still sends a sentence as data (a `reason` or `message` in a snapshot), the client does not translate it; each such family stays a boundary until it is converted to a code the catalogs can name, and a family's conversion removes its English from the wire in the same change.
A code the web does not know (a daemon of another build) is shown as the code itself rather than hidden.
The desktop host draws its native menu, its role items (Undo, Quit, Services and the rest, labeled explicitly because the system's own follow the OS language), the connection screen, the folder picker and the browser display dialogs from the same catalogs (`native.*`, `commands.*`).
It keeps only the last core-confirmed explicit choice in its profile (`interface-language.json`) and resolves an unset choice on its own host, as this document requires of any native or closed-window consumer; `ARCHITECTURE.md`, The desktop host, owns the details.
The phone app (`web/src/mobile/`) draws every sentence from the same catalogs.
It has no snapshot, so the daemon puts the core's explicit choice on each `agents` frame (`interface_language`, null for none) and the phone resolves it as the shell does; before the first frame (the pairing, unpaired and refused screens) it follows its own browser language.
`web/src/i18n/translator.tsx` holds the translator both pages share, so the phone bundle does not read the shell store through it.
Failures and refusals the phone keeps in its store are a key and its values, translated where they render, so a shown line follows a language change.
A push notification is composed on the phone, not by the daemon: the payload carries the agent's title and place (user data) and the state (`needs_you` or `done`), and the service worker joins the state's word with the place.
The page hands the worker the two words (`mobile.group.needs_you`, `mobile.group.done`) in the language in effect on load, on every language change and after a subscription is made, and the worker keeps them in the Cache API so a closed app still notifies in the right language.
A subscription made before the words were ever stored shows the place alone rather than an invented English word.
The sentence a human-delivery notice carries is composed by the core in English and rides along as the place, untranslated, like the other core-sent text below.
Hide AI is a closed-window reader in the core: what it writes for a person (agent labels and their missed work, pull request summaries, Memory cards and their re-verification and promotion reasons) is in the core's explicit choice, else the language `hide_platform::host::primary_language` names for the core's machine, resolved by `InterfaceLanguage::from_system` with the shell's rules, else English (`Runtime::output_language`).
The request carries that language and the Hide AI system prompt names it; no prompt names a language of its own ([AI_PROVIDERS.md](AI_PROVIDERS.md#the-output-language)), and a text already written keeps its language after the choice changes.
Git branch and worktree names Hide suggests stay lowercase ASCII in every language.
The manifest has no description, since a static file cannot follow the language.

## Hardcoded text guard

Product text outside the catalogs is caught by two rules in `web/eslint.config.js`, which `pnpm lint` and the `checks` lane run on `web/src` (tests, `src/i18n` and `src/gallery` excepted).
`i18next/no-literal-string` flags JSX text and the attributes a person reads or hears (`aria-label`, `aria-description`, `aria-roledescription`, `title`, `placeholder`, `alt`, `label`, `description`).
It lets a keycap name, a product name (Claude, Codex, OpenCode, GitHub, Herdr, Hide, Tailscale, Git), a symbol, a unit and an example path or address through, and does not look inside the `<Kbd>` component or a `t`, `data`, `cn` or label-lookup call; a new allowance is added to that file with the reason, never as an inline disable.
`no-restricted-syntax` flags any Korean, Chinese or Japanese character in a string or template of product code, so a sentence written in a `.ts` helper is caught too, which is how the shell shipped its Korean text.
`web/src/i18n/guard.test.ts` runs both rules on sample sources, so a change that makes them stop matching fails a test instead of reading as clean.
Hide AI prompts have their own guard, `scripts/check-ai-prompt-language.py` in the `policy` lane: production string text in a file that builds a Hide AI request may hold no Korean, Chinese or Japanese character and no fixed instruction about the language to write in, because the request carries the operator's language.
What the guard cannot see is an English sentence returned by a `.ts` helper or a template; review a change to one for it, and prefer a key table or a function that takes `t`.
The audit at the time the guard landed read every non-test source under `web/src` for JSX text, read attributes, CJK characters and English phrases in helpers; it found the two device-label fallbacks (`startTargets.ts`, `search.ts`) and the phone's, all now catalog text, and leaves the English `title` of each shortcut in `shortcuts.ts`, which is the search key and the English source `commandTitle` replaces on screen.

## Verification owners

`runtime::tests::appearance` checks all four choices, reset, invalid events, restart, stale UI saves, and preservation of invalid stored values, and that Hide AI's output language follows the choice, the system language and the invalid-value fallback.
`web/e2e/interface-language.spec.ts` drives the selector against a private daemon and Herdr, with separate Korean and Japanese browser contexts.
It checks shared choices, four-language headings, refresh, daemon restart, system reset, unsupported-system fallback and invalid-store diagnostics.
It also checks Overview scope labels, sidebar tabs, navigation command labels and unchanged shortcut keycaps in all four languages.
Catalog, translator and locale unit tests remain beside their modules.
`desktop/src/main/language.test.ts` and `menu.test.ts` hold the host's resolution, persistence and four-language menu labels, and `desktop/e2e/language.spec.ts` drives the real app: stored choices in all four languages before any daemon answers, the system fallback, and a choice made in Settings relabeling the menu, persisting and returning to the system language.
`web/src/mobile/text.test.ts` and `language.test.tsx` hold the phone's four-language tables and language resolution, `web/e2e/mobile.spec.ts` follows the core's choice from the desktop Settings to a paired phone and checks the notification payload and the worker's words cache, and `hided/src/mobile` tests hold the language on the `agents` frame and the payload's shape.
Run artifacts stay under `agents/runs/`; browser checks do not establish native menu behavior.
