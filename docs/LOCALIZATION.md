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
That covers Settings (every tab), the sidebar rows, menus and close sheets, the shared Overview with its 백로그, Projects and Usage tabs and the ⇧⌘M quick add, the sidebar's 태스크 list, the lane's 보기 slot, session record, PR list and PR card, the Project tool, the window's top row, tab bars and pane chrome, the editor and file views, the Explorer and History, browser displays, the palettes, the start panel, the add-project and workspace dialogs, disk cleanup and the device rail.
Shortcut keycaps retain their host-specific chord glyphs in every language.
Text that a helper composes outside a render uses the `translate` function of `client.tsx`, which reads the confirmed language at call time; a helper that renders inside React takes the component's `t`.
A module the desktop host imports (`host.ts` and everything it reaches) stays free of the translator and names a label by key, as `revealExternal.ts` does.
Where `UI_BEHAVIOR.md` quotes shipped Korean wording (`요청`, `정리`, `맡기기`), that is the Korean catalog text; English and the other languages read their own catalog entries.
The core sends a code, never an English sentence, for what the screen words: an agent's `status_code` (`working`, `question`, `sleep_failed`, ...) is named by `agents.status.*` through `statusText` in `web/src/agentStatus.ts`, a GitHub `failure_category` by `issueSettings.gh*`, and the machine the core runs on as a device row with no `label` and its system in `host.platform`, which every shell words as This Mac, This PC or This computer (`systemMachineKey` in `web/src/screenMachine.ts`), the phone's start sheet included, which reads a null `place` as Home and a null `device_label` as that machine, named from the catalog's `core_platform`.
A delivery notice for a person is a `HumanNoticeKind` plus the names it concerns: the phone words it from its state key (`mobile.notice.*`, relayed to the service worker with the other notification words), while Herdr's own notification, which has no translating screen, shows the English title and body whose tail names the `hide request show` command.
Where the core still sends a sentence as data (a `reason` or `message` in a snapshot), the client does not translate it; each such family stays a boundary until it is converted to a code the catalogs can name, and a family's conversion removes its English from the wire in the same change.
A failure the core publishes as `last_error` is worded by its `kind` through `coreErrorText` in `web/src/coreError.ts`, with the catalog sentences in `web/src/i18n/resources/coreErrors.ts`; the same module words this machine's `status.herdr.state` and a device's `RemoteStatus.state`, and `web/src/startReason.ts` words the codes an agent start's reason carries.
The `message` beside a kind, Herdr's `message` and a device's `message` are diagnostic detail (Herdr's protocol answer, a socket path, an io error): the store's diagnostics, Settings' Recent list and the core's `error.reported` log keep them, the About tab's details show them untranslated, and no other screen draws them (design principle 13).
Hide's install kit sends each part's and each agent piece's `why` (`hide_kit::PartWhy`: a cause, and the one path, name or version its sentence names) and a machine's `unavailable` as a code; `web/src/kitWhy.ts` words them from `web/src/i18n/resources/kitReasons.ts`, along with a saved Herdr profile's `connect_refusal`, and the kit's English `reason` stays in its report for the log and the `hide` CLI, never on the snapshot.

A Local issue that was not made, changed or read, and a pull request's issue that was not made or linked, carry an `IssueFailure` (`local_issue.*` or `pr_link.*`, with the limit its sentence names) on `issue_work.update` and `pr_work.link`, and a Local issue's read failure is a `local_issue.*` code in `issue_work.detail.message`; `web/src/issueFailure.ts` words them from `web/src/i18n/resources/issueReasons.ts`, an unknown code leaves the headline alone, and the core's English reason goes to the `pr_link.failed` and issue diagnostics.
⌘K's GitHub search answers with a code (`github_search.failed`, `github_search.query_empty`, `github_search.query_too_long`) or, beside results that skipped repositories, `github_search.partial`, and a checkout's issue link names where it came from as a code (`manual`, `pane_token`, `branch_setting`, `pr_body`, `branch_name`).
Why the pane menu's Sleep agent or Fork is disabled comes as an `agent_action_refusal` code in `sleep_action.reason` and `fork.reason`, worded from `web/src/i18n/resources/agentActionReasons.ts`; a code this build does not word shows the item's general unavailable line.
An Explorer change that failed carries a `CodedFailure` in `explorer_operation.failure`: `explorer.*` for what the core refused, `host.<code>` for the checkout host's refusal (`hide_node_link::ErrorCode`) and `link.*` for the link (`node_access::link_failure_code`), and a device folder that could not be listed carries the same host code as `directory_unavailable.reason`; `web/src/fileFailure.ts` words them from `web/src/i18n/resources/fileReasons.ts`, and the host's sentence stays in the log.
History's `unavailable_reason` and `stale_reason`, the file index's `unavailable` and a download's failure reason are codes too (`changes.*`, `host.*`, `link.*`, `file_bytes_error` and `path_refused` reasons), worded by `readFailureText` and `downloadFailureText` in the same file; a read code this build does not word says only that it was not read.
A failed task carries its reason as a code in `task_operation.failure` (`task_operation.*`, `agent_start.*`, `checkout_purpose.*`, or a start reason), worded by `coreErrorText` and `startFailure` with the new sentences in `web/src/i18n/resources/taskReasons.ts`; a worker's sentence becomes the general `task_operation.failed`, and every reason goes to the `task_operation.failed` diagnostic with the core's own words.
A worktree's deletion gate (`WorktreeDeletionGateSnapshot`) sends codes and counts, never a sentence: why it is refused (`main`, `locked` with the name and Git's lock reason, `ignored_scan_unavailable`), each warning badge, what the branch loses and what the Discard checkbox accepts; `web/src/deletionGate.ts` words them from `web/src/i18n/resources/deletionGate.ts`, and a code this build does not know shows its group's general line.
`gh`'s own words for a failed GitHub read stay in the core: a checkout's `github.unavailable_reason` is not serialized (the card and Settings > GitHub word `failure_category` from `web/src/i18n/resources/githubReasons.ts`), and a PR's checks and review comments and an issue's body carry a `failure` code (`pr_feedback.read_failed`, `project_settings_repository_changed`, `issue_detail.read_failed` or a Local issue's `local_issue.*`), with the sentence in the `pr_feedback.failed` and `issue_detail.failed` diagnostics. A failed check GitHub gives no name arrives with `name` null, and the page words it (`githubReason.unnamedCheck`).
A worktree removal's answer is a `WorktreeRemovalNotice` (`removed` with the branch's fate, `stopped` with the host's `hide_node_link::worktrees::RemovalStop`, `still_running` with the programs holding the folder, `host_failed` with a `host.*` or `link.*` reason, and the core's own codes), worded by `web/src/removalNotice.ts` from `web/src/i18n/resources/removalReasons.ts`; the host's and the core's sentences stay in the `worktree_removal` diagnostics.
Why a History or View diff is short or absent is a `hide_node_link::git::DiffNotice` (`truncated` with the limit, `no_base`, `no_branch_changes`, `no_uncommitted_changes`, `unreadable`, `outside` with the folder History reads), worded by `web/src/diffNotice.ts` from `web/src/i18n/resources/diffReasons.ts`; Git's reason for a diff it could not take stays in the `diff_unreadable` diagnostic, and a helper of an older build that sends only its sentence reads as `unreadable`.
A failed wake's `sleep.reason` is an `agent_wake.*` code worded through `web/src/startReason.ts` from `web/src/i18n/resources/wakeReasons.ts`; a sleep record an older build saved keeps its English sentence, shown as sent.
The doorbell's bell line is the one sentence the core words itself, because it is typed into an agent's terminal rather than drawn by a screen: its kind and count follow the core's interface language, in `herdr-core/src/delivery/bell.rs` ([delivery.md, The bell line](delivery.md#the-bell-line)).
A device's project catalog sends only its state; why its helper could not be asked goes to the `catalog.unavailable` diagnostic.
A kit cause this build does not know shows the part's state alone.
A code the web does not know (a daemon of another build) is not hidden and not shown as the code: the surface shows its own general sentence (`coreError.other`, or the dialog's own fallback such as `workspace.createFailed`), and the code goes to the log and to the element's `data-core-error` attribute.
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

## Writing the copy

A catalog value is the screen's own sentence in that language, not a translation of the English key.
Write what happens on the screen first, then put it into English and into the other three languages one at a time.
A Korean value that reads as the English sentence with Korean words in it (`Hide가 자기 항목을 더해, 에이전트가 나를 부르는 순간을 알아챕니다`) fails this section even when every word is correct.
The rules below came from an audit of every catalog value, the native menus and dialogs, the `hide` command's sentences and the sentences the core sends as data.

### Korean

- **Register.** Guidance and errors end in 해요체 (`읽지 못했어요`), a request in `-세요` (`다시 시도하세요`), and a button, a status chip or a label in a noun or a short verb form (`계속`, `연결됨`, `알림 켜기`). 합니다체 and 반말 (`-다`, `-않는다`) are not used for screen text. One screen, one wizard and one dialog keep one register.
- **No particle after an inserted value.** A value is any name, number, path, branch, device or agent, so its last sound is not known. Never write `{{x}}` followed by 은, 는, 이, 가, 을, 를, 과, 와, 으로, 로 or 이라, and never write the pairs `을(를)`, `이(가)`, `은(는)`. Put a fixed noun after the value (`{{name}} 프로젝트를`, `{{device}} 기기가`, `{{agent}} 설정에`), use a particle that does not change (`에`, `에서`, `의`, `도`, `만`), or write the sentence so the value ends it (`선택한 기기: {{device}}`). A `*Euro` or `*Eun` key pair is not added for a new sentence.
- **No subject that does not act.** `Hide가`, `에이전트가` and `이 폴더는` open a sentence only when the sentence is about them. Say what the person sees (`저장하지 못했어요`), not what the product does (`Hide가 저장하지 못했습니다`). No personified verbs (알아챈다, 받아들인다, 부른다, 시킨다).
- **A button is quoted by its label.** `‘계속’을 누르기 전에는`, with the label copied from the button's own key.
- **The subtitle of a choice ends in a verb.** Two choices whose subtitles are both noun phrases read as the same choice (`인터넷에 있는 Git 저장소를 이 기기의 새 폴더로 내려받아요` / `빈 폴더를 만들고 Git 저장소로 시작해요`).
- **English only for names, commands, keys and paths.** Product names (Claude Code, Codex, Grok, OpenCode, Pi, omp, Antigravity, GitHub, Herdr, Hide, Tailscale, Git), commands, file names, shortcut keys and identifiers stay as they are. A general English word (`worktree`, `checkout`, `hook`, `pane`, `diff`, `Merge`, `Local`) is written in Korean or as the glossary says.
- **Product name.** `Hide` in a sentence, `hide` for the command, `hide.app` for the bundle; `BRAND.md` keeps the lowercase wordmark beside the mark.

### All languages

- **A value is words, never a code.** `{{state}}`, `{{reason}}` and `{{message}}` hold a sentence that is already in the viewer's language, not a status code (`not_installed`), not an English sentence and not a protocol answer. The core sends a code and the catalog words it (the boundary above). A failure the person cannot act on goes to the diagnostic log, not into a message.
- **The same thing has one name** in a language, from the glossary below. A new word for an existing thing is added to the glossary in the same change.
- **A sentence quoted in `UI_BEHAVIOR.md` changes with it.** `rg` the old text in `docs/UI_BEHAVIOR.md` and in `web/e2e`, `desktop/e2e` and the unit tests, and change those lines in the same pull request.
- **A message for an agent is not screen copy.** A prompt Hide types into an agent (`issue.prompt.*`), a hook line and a comment an agent leaves on GitHub are read by agents and by the people who later read their record; they are written in English, except where the operator's language is the point, and they are outside the register rule above.
- **English** is plain and short: the sentence a person would say, in the active voice, without a question the screen does not ask. **Japanese** uses です・ます for sentences and a noun phrase for labels, without 〜を行います or 〜することができます where 〜します works, and writes Latin letters beside kana without a space. **Simplified Chinese** uses 你, a full-width punctuation mark after Chinese text and a half-width one after Latin text, and avoids 进行… and 被… where a plain verb works.

### Glossary

One row is one thing the screen names. A word in the last column is not used for it.

| Thing | en | ko | ja | zh-CN | Not |
| --- | --- | --- | --- | --- | --- |
| A machine running Herdr | device | 기기 | デバイス | 设备 | 디바이스, 장치, マシン (the platform names `This Mac`, `This PC` and `This computer` stay as `common.*` says; Korean writes `Mac` in Latin letters) |
| One terminal of a tab | pane | 터미널 (`pane` only in `pane ID` and commands) | ペイン | 窗格 | 페인, pane in a sentence, 패널, 面板 |
| A project's folder on a device | checkout | 체크아웃 | チェックアウト | 检出目录 | checkout, 작업 폴더 |
| Herdr's workspace | workspace | 작업공간 (the tab named `Workspace` keeps its name) | ワークスペース | 工作区 | 워크스페이스 |
| Git worktree | worktree | 워크트리 | ワークツリー | 工作树 | worktree |
| An agent started by another | child agent | 자식 에이전트 (count: 개) | 子エージェント | 子智能体 | 하위, 형제 as a heading |
| A tab's document or browser page | view | 뷰 | ビュー | 视图 | 화면 |
| Folder | folder | 폴더 | フォルダ | 文件夹 | フォルダー |
| An agent the operator put to sleep | asleep | 재움 (action: 재우기) | 休止 | 休眠 | 휴면, 잠자기 |
| Copy of a session that goes on separately | fork | 포크 | フォーク | 分叉 | 분기 |
| The first agent of a delegation chain | root session | 맨 위 세션 | 最上位セッション | 根会话 | 루트 |
| Another machine's Hide | other Hide | 다른 Hide | 別の Hide | 另一个 Hide | 호스트 |
| An agent below a child | descendant session | 하위 세션 | 下位セッション | 下级会话 | 后代会话 |
| An agent is busy | working | 작업 중 | 作業中 | 工作中 | 일하는 중 |
| A unit of work in the sidebar | task | 태스크 | タスク | 任务 | 일감 |
| GitHub issue | issue | 이슈 | Issue | 议题 | 課題 |
| Pull request | PR | PR | PR | PR | 풀 리퀘스트, プルリクエスト, 拉取请求 |
| Merge | merge | 머지 | マージ | 合并 | Merge, 병합 |
| Copy a repository from a URL | clone | 클론 | クローン | 克隆 | 복제 |
| Join an issue to a PR | link | 연결 | 関連付け | 关联 | 잇기 |
| Revoke a paired phone | disconnect | 연결 해제 | 解除 | 解除 | 해지 |
| Read a QR code | scan | 스캔 | スキャン | 扫描 | 찍기 |
| The file Hide adds to an agent | agent setup | 연동 설정 | 連携設定 | 集成配置 | hook, 훅 in a sentence (a file named `hooks.json` stays) |
| Hide's helper on a device | helper | 헬퍼 | ヘルパー | 辅助程序 | 연결 도구 |
| The `hide` command | command | 명령 | コマンド | 命令 | 명령어 |
| Messages between agents | letter | 편지 | レター | 信件 | 手紙 |
| A phone | phone | 폰 | スマートフォン | 手机 | 모바일, 휴대폰 (the tab named `모바일` keeps its name) |
| The panel to the right of the tabs | lane | 레인 | レーン | 右侧面板 | 侧栏 for the lane (it names the sidebar) |
| The device list beside the sidebar | device rail | 기기 목록 | デバイス一覧 | 设备栏 | 레일 |
| An issue source kept on this device | local | 로컬 | ローカル | 本地 | Local |
| The feature that keeps memories for agents, and the rail tool that holds it with the suggestions | Memory, Hide AI | Memory, Hide AI | Memory, Hide AI | Memory, Hide AI | メモリ |
| One thing Memory keeps | memory | 기억 | 記憶 | 记忆 | card, 카드, カード, 卡片 (a PR's card and a conversation card keep the word) |
| What Hide AI suggests fixing | suggestion | 제안 | 提案 | 建议 | 추천 |
| Put a suggestion aside for now; it can rise once more | skip | 넘어가기 | スキップ | 跳过 | Don't do this, 안 함, しない, 不处理 for a suggestion |
| Where Hide AI reads its signals, in ⚙ | where to read | 읽을 곳 | 読む場所 | 读取哪里 | one switch per signal |
| A point where an agent lost time though nothing failed, and its four kinds | thing to improve: No doc, Searched again, Wrong guess, Tool or setting | 개선할 점: 문서 없음, 다시 찾음, 잘못 짐작, 도구·설정 불편 | 改善点: ドキュメントなし, 再検索, 推測ミス, ツール・設定の不便 | 可改进之处: 缺少文档, 重复查找, 猜错了, 工具或设置不便 | 문제, 실패 for a point (nothing failed) |
| Agent family names | Claude Code, Codex, Grok, OpenCode, Pi, omp, Antigravity | as written | as written | as written | 代理 for an agent in zh |
| Reasoning level | effort | effort | effort | effort | translated |

Words that stay in English on purpose: `core`, `stack`, `effort`, `Conductor`, `Hide AI`, `Memory` (tab name), `Factory`.

The rules a scan can decide are enforced by `web/src/i18n/copyRules.test.ts`: no particle that depends on the last sound after an inserted value, no `을(를)` pair, no 합니다체 or 반말 ending, no retired glossary word (Korean, Chinese and Japanese), and no bare English glossary word in a Korean sentence.
The prompts typed into an agent (`issue.prompt.*`) are the one allowlist; a new entry needs the reason in the test.
Naturalness, term choice beyond the banned words and ja/zh wording still need a reader.

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
