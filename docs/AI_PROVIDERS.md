# Background AI providers

`hide-ai/` is the one boundary through which a Hide feature asks a language model for something in the background.
Its consumers are the core's agent label analyzer (`herdr-core/src/labels/analyzer.rs`), Project Memory extraction under `hide-memory/`, and the Start dialog's worktree name; later features reuse the same boundary rather than a provider client of their own.
This guide owns the boundary's rules; the code under `hide-ai/src/` and the tests under `hide-ai/tests/` are its executable authority.

## Ownership split

Optional product statistics count each backend invocation through a content-free `AiUsageSink::started` callback, separately from local token and cost accounting.
Only the `hide_ai_called` event, random installation ID and time leave through that channel; [product usage statistics](ARCHITECTURE.md#product-usage-statistics) owns its complete privacy and opt-out contract.

A feature owns its prompt, its output schema, and the parsing of the answer into its own type.
It submits an `AiRequest` (feature id, caller-made request id, subject id, system prompt, input, schema, deadline, schema version, output language) and receives an `AiResult`: a `serde_json::Value` already validated against that schema, and the provider that produced it.
There is no output token ceiling in the request, because no provider contract honours one; the prompt and the schema are what keep an answer short.

### The output language

What Hide AI writes for a person is in the operator's language (PRD ai-output-language).
`AiRequest.output_language` is `en`, `ko`, `zh-CN`, `ja` or none, and has no default: a request that does not name it, a peer of an older protocol included, is refused when it is read.
`AiRequest::system_prompt()` is the prompt every provider is given: the feature's own, then one sentence asking for every text a person reads in that language whatever the input's language, keeping code, commands, paths, identifiers, names and quoted text as they are; none adds nothing.
No feature's prompt names a language of its own, and `scripts/check-ai-prompt-language.py` fails on one that holds Korean, Chinese or Japanese text or a fixed language instruction.
The language is part of the duplicate key, so a request made after the language changed never joins one from before it and is never handed an answer in the old language.
The core decides it in one place, `Runtime::output_language` (`herdr-core/src/runtime/output_language.rs`): the explicit interface language (`ui_state.interface_language`, English for a stored value that is invalid), else this machine's primary language as a shell resolves it ([LOCALIZATION.md](LOCALIZATION.md)), else English, with `language.system_fallback` in the diagnostic log.
The machine's language is read once per daemon before the runtime exists, because macOS answers it through a child process; a request that runs on a connected device carries the core's language, never the device's.
Labels with their missed work, pull request summaries, Memory's extraction, re-verification and promotion check, the Memory promotion agent's reasons ask in that language; the Start dialog's worktree name asks in none, because a Git name is lowercase ASCII whatever the operator reads.
A text already written stays in the language it was written in, and only what is asked after a change follows the new language.
The answer is checked against its schema only: an answer in another language is shown as it is, never refused or asked again.

`hide-ai` owns everything about providers: which are installed and logged in, the child process and its protocol, timeouts and cancellation, the retry policy, fallback between providers, duplicate suppression, and the structured error a caller branches on.
No feature type lives in the crate, and no provider detail leaks out of it.

The session label requests use `context_label.v21`, retaining nullable goals at both boundaries, with the goal and line lengths of the request's language (8 to 30 and 40 characters in Korean, Simplified Chinese and Japanese, 8 to 60 and 80 in English).
Both boundaries also return `goal_update` (`keep`, `refine` or `replace`; a replace opens a new goal flow when what is being made changes, PRD session-record-goal-flow D-09) and `requests`, a one-line summary for each request the input lists as unsummarized, each with the request it `answers` when it replies to the agent, at most eight per call, the turn's own request always among them, and `closed_result`, with a `replace` one line saying how the flow it closes ended and otherwise empty (PRD session-record-fact-status D-02); they add output tokens to calls already made and no call.
The END request adds a work judgment: at most three short missed items (a problem found outside the assigned work, a proposed follow-up nobody has taken, or a question or decision that waits on the operator), resolved item ids and up to three outputs other than pull requests (`artifacts`).
The fixed instructions consolidate repeated goal, line, stop-state and work-judgment rules; the model effort, request boundaries and context limits are unchanged.
There is no additional call per turn, and requests from periods with summaries Off or failed judgments are never backfilled by the provider.
END waits for a known ledger context instead of treating missing parent authority or open work as empty.

The Start dialog of the Overview uses feature id `worktree_name` (`herdr-core/src/ai.rs::suggest_worktree_name`): it sends the issue's title and at most 2,000 characters of its body, asks for `{slug}` against a schema, and the core keeps only lowercase ASCII words joined by hyphens, at most 48 characters, behind the issue-number prefix the dialog passed; the deadline is 30 seconds, a failure leaves the dialog's own name in place, and Settings › Issues turns the feature off.

A pull request's card asks Hide AI nothing: the `pr_summary` feature was retired (PRD pr-card-and-session-record-redesign D-19), and a `pr_summary` field an older Hide left in the settings file is read past and not written back.

Project Memory uses feature id `project_memory` with Hide's own extraction prompt (`hide-memory/src/engine.rs`, under 8 KiB) and strict schema: each card's text, a one-line title, aliases, kind, extraction confidence and salience, source offsets, cited repository paths, and a `new`, `same`, `supersedes`, `conflicts` or `discard` relation to one of the cards it was shown, plus titles and aliases for shown cards that have none.
No third-party memory engine or prompt asset is part of it.
The write service validates those proposals against source events, Project identity, privacy, provenance, lifecycle, and resource caps; only an operator's own words can supersede a card.
It sends locally redacted, normalized human and assistant events after the durable session cursor, or an agent's note from `hide memory add`, plus the existing cards the model-free matcher finds most like them, so a same-meaning card is found across the operator's language and English: a card is written in the output language and its aliases add the English word for each one that is not English.
The comparison set is bounded by cards and serialized bytes so it and one event group fit the 64 KiB request-input cap, and both pass through the current local redactor again at the final provider egress boundary.
Background analysis counts against a machine-wide 1,000 calls a UTC day and is asked only of an agent under the operator's usage limit (see [The usage limit](#the-usage-limit)); `hide memory add` counts against the daily cap only, and its provider tries end within 50 s.
The feature layer verifies the relation and provenance before the single-writer store changes anything; provider output never has direct write authority.
The `preference` kind records personal choices without proposing repository changes.
`project_memory_creation_docs` is one comparison batch call for lexically matched new or superseding candidates: one repository excerpt per candidate, at most 3 KiB each, with the complete input capped at 15% of the 64 KiB extraction-input ceiling in characters and bytes.
Its same/differs/correction/exception/unrelated verdict distinguishes duplicate documentation, an unresolved disagreement, an enduring operator correction and a task-local exception.
A confirmed enduring correction remains available as untrusted context with its pending document update, while unresolved disagreements are withheld and task exceptions do not revise standing rules.
Reconciliation records each admitted attempt before asking and certifies a document fingerprint only after a usable answer.
An intent-local failure rotates the correction and lets remaining card verification and suggestion checks continue within the same maintenance quantum; the runtime retains the failure for its normal Project backoff.
Authentication, account usage and resource-cap refusals still stop provider work.
Completed check changes and observed failures survive a later admission, local or cancellation stop.
A Project with an observed maintenance failure leaves its discovered extraction pending for runtime backoff, so a same-pass success cannot clear the failure.

Conflicting quotes must appear in the exact excerpt sent, and direct operator evidence is required to confirm correction authority.
The call uses the existing analysis admission and router usage limit; `analysis.session_cards` reports Project/session IDs, counts and both input sizes without content.
Two more features use the same boundary and the same call cap and usage limit (PRD memory-card-quality D-07, D-08, D-11): `project_memory_verify` asks whether a card whose evidence moved still holds, and `project_memory_promote_check` asks whether a rule, lesson or durable decision about to be suggested for the repository's documents is already said there (`hide-memory/src/verify.rs`).
Each sends the card, what the repository answers now for its evidence, and bounded excerpts (at most five cited files of 8 KiB for the first; at most five instruction documents of 8 KiB and three of the operator's own global instruction files of 5 KiB for the second), each a whole file when it fits and otherwise the sections (a heading and what follows) that share the most words with the card, read from at most the first 96 KiB of the file, all passed through the local redactor, within the 64 KiB request input (the last excerpts are dropped before a request is refused); a file named for credentials (`.env*`, keys and certificates, names containing `secret` or `credential`) is never read into a request, whatever a card cites; an answer that arrives but cannot be used (not the schema, a correction with no text, a document never shown) spends the card's day and the pass goes on to the next card, while a provider failure spends nothing and holds the Project; the answer is a strict-schema verdict and a corrected card when it is outdated, and a document named by an `in_docs` verdict must be one the request carried.
A card is asked about at most once a day.
The logs carry card numbers and verdicts, never card text, file text or a path.
Memory analysis reuses this boundary's selected provider, availability, bounded same-provider retry, duplicate suppression, process ownership, cancellation, and budgets.
It may fall through to a provider the operator added under fallback (see [Selection and fallback](#selection-and-fallback)), and only to one, and the disclosure names both that possibility and the fact that the stored analysis batch records which provider answered.
The disclosure also names active-Memory retransmission, and its stored version disables previously enabled Projects when this material data-egress description changes so that the operator must opt in again.
Not-authenticated, unavailable, usage-limited, and exhausted outcomes pause new analysis without disabling existing local Memory search or Sessions browsing.
Logs retain request, feature, provider, Project/session subject IDs, counts, duration, and outcome, but never transcript text, prompt text, Memory body, file path, credential, or provider thread ID.

The existing verification and promotion-check answers include `docs_disagree` with a shown repository path and exact conflicting quote; no extra conflict call is made.
Authority is decided from direct operator evidence and semantic scope; source and file times are provenance and never select the winner.
An unresolved disagreement needs an operator answer.

## Providers

The user's installed and logged-in CLIs are the only credentials for background AI requests.
Hide's integration code reads no token file: the codex backend references the user's `auth.json` only through a symlink in the private `CODEX_HOME` it sets, and OMP delegates credential discovery and use to its public SDK (see [Oh My Pi](#oh-my-pi)).
It sets two environment variables: that `CODEX_HOME`, pointing the app-server at the private directory, and `MAX_THINKING_TOKENS=0` on a claude model turn (see the claude bullet).
`hide-ai` is also the only code that starts a background AI child of any agent: the Weekly Usage reader below asks the same `ClaudeCliBackend` for its `/usage` text rather than running the CLI itself.

The providers are a registry (`hide-ai/src/registry.rs`), in this fixed order: `claude`, `codex`, `grok`, `opencode`, `pi`, `omp`, `antigravity`.
`ProviderId` is a copyable name for one entry, written in `ai.json` as its id; `PROVIDERS` lists them, `ProviderId::descriptor()` gives the label, the program names looked up on the login `PATH`, the default model and `agent`, the id of the same agent in the install kit (`claude-code`, `codex`, `grok`, `opencode`, `pi`, `antigravity`), and `build_backend` makes the backend of one entry.
An empty model means the CLI's own default: no `--model` flag is sent.
There is one place that finds a CLI and says what it is run with (`hide-ai/src/program.rs`): a program is looked for on the account's search (`hide_platform::programs`: the login shell's folders, the daemon's own `PATH`, the usual install folders), and the child is started with that same value as its `PATH`.
That covers every child, the availability and model-list probes and the Weekly Usage read as much as a request, and the Codex app-server, because a CLI installed as a script (pnpm's `codex`, a Node CLI under nvm) starts its interpreter by name from its environment, and a CLI found on the shell's folders and run with the daemon's would be reported ready and then fail.
A backend's config can carry a `search_path` of its own, which is how a test gives a stand-in script an interpreter only that value reaches without starting the developer's login shell (`hide-ai/tests/it/search_path.rs`); a CLI named by file with no `search_path` is run with the process's `PATH`, since nothing searched for it.
The tests that need a missing CLI name an absolute path that does not exist, because a bare name is looked up on the account's search, which asks the login shell.
A new agent is one registry entry and one backend; the router, the settings and the core iterate `PROVIDERS` and name no provider.
The backends run on the core's own node, never inside the core: the core's routers hold a `NodeBackend` per provider (`herdr-core/src/ai.rs`), which carries each probe, model list, request and measurement to the node as a call and reads the answer back.
The node keeps one real backend per core instance in a registry capped at 64 (a few routers, each with one backend per registered provider) (`hide-host/src/ai.rs`), refuses the next, and drops an instance when the core's `NodeBackend` is dropped, so a router rebuilt for a changed model choice ends the old child.
A request reports progress once a second while it runs, and a cancel in the core stops the node's request through that report.

- `codex`: `codex app-server --listen stdio://`, the official JSON-RPC surface of the Codex CLI, driven with an ephemeral read-only thread, the feature's system prompt as the base instructions, every optional feature disabled, and the feature's output schema attached to the turn as `outputSchema` in `Schema` mode or named in the base instructions in `PlainJson` mode (see the answer modes below).
  The default model is `gpt-5.6-luna`.
  Availability is read from `account/read`; the thread leaves nothing in `~/.codex/sessions`.
  `codex exec` is not used.

  `hide-ai` owns this process (see [Process ownership and budgets](#process-ownership-and-budgets)).
  It runs the app-server under a private, owner-only (`0700`) `CODEX_HOME` in a temporary directory (`$TMPDIR/hide-ai-codex-home-<pid>-<nanos>`) that is created holding nothing but a symlink to the user's `auth.json` (from the environment's `CODEX_HOME`, or `~/.codex/auth.json`), and no `config.toml`.
  The app-server then writes its own state beside the symlink (`installation_id`, `models_cache.json`, its sqlite databases, `shell_snapshots/`, `skills/`, `tmp/`; 2 to 5 MB a session, measured 2026-09-20), but no `sessions/` rollout, so the ephemeral thread leaves no transcript.
  With no config the app-server starts none of the MCP servers the user's real config declares, which measurement (2026-09-17) showed `-c mcp_servers={}` and a per-thread `config` override both failed to prevent.
  The directory is removed when the session ends; if it or the symlink cannot be created the request fails with `ProviderUnavailable(codex_home_unavailable:<stage>:<kind>)` rather than falling back to `~/.codex`.
  An owner that ends without running `Drop` (a `kill -9`, or a process exit while a session is open) leaves its directory behind, so every `CodexAppServerBackend` sweeps them when it is built, on a thread of its own (`hide-ai/src/codex_home.rs`).
  The sweep removes a `hide-ai-codex-home-<pid>-<nanos>` directory under `$TMPDIR` only when it is a plain directory the current user owns and no process has that pid; a pid that is alive keeps its directories even if it was reused by an unrelated process, because a dead owner's leftover costs disk while deleting a live owner's breaks its app-server.
  A name that does not parse is never touched, symlinks are never followed, so the user's `auth.json` stays, and one sweep removes at most 32 directories so a backlog (344 dead owners' homes on one machine on 2026-09-28, counted by checking each name's pid with `kill -0`) is cleared over several starts.
  A sweep that removed, failed to remove or could not read anything logs `ai.codex_home.swept` with `removed`, `bytes`, `failed`, `unreadable` and `capped`, plus `first_error=<kind>` when a removal or the directory listing failed (a listing error ends the sweep early); a temporary directory it could not read, or a sweep thread that could not be started, logs `ai.codex_home.sweep_failed` with the stage and the error kind.
  The credential file is referenced through the symlink and never read.
  codex's own token refresh writes through it to the real file: the default `file` credential store opens `CODEX_HOME/auth.json` for truncating write, no temp file and no rename, so the symlink is followed and survives (read from `codex-rs/login/src/auth/storage.rs` at `rust-v0.155.1`).
  A `keyring` or `auto` credential store keys the entry by the canonical `CODEX_HOME` path, so a private home has no entry and the provider reports `needs_login`; that mode is not supported here.
  Shutdown is one graceful path on every exit (`Session::drop`, a session swap, an over-budget restart): close stdin (its EOF is the app-server's own shutdown signal and ends the whole tree), then SIGTERM, then SIGKILL, each after a three-second grace.
  After ten idle minutes with no completed request the app-server is shut down and `ai.app_server.idle_exit` is logged; the next request starts a fresh one.
- `claude`: `claude -p --output-format json`, print mode, one child process per request.
  Print mode is Claude Code's only official structured-output surface; there is no `app-server` equivalent in `claude --help`.
  It was excluded by decision until 2026-09-10, when that exclusion was withdrawn; terminal scraping and token reuse remain excluded, and the background AI boundary reads no credential file.
  The default model is `sonnet`.
  Availability is read from `claude auth status --json`: `loggedIn` decides `Ready` against `NeedsLogin`, a binary that is not on `PATH` is `NotInstalled`, and a probe that answers nothing readable is `Unavailable` rather than either guess.
  In `Schema` mode the answer is the result frame's `structured_output`, the field the CLI validated against the feature's `--json-schema`, and the `result` string is never parsed.
  In `PlainJson` mode (see below) the `result` text is parsed, which print mode prints as the object inside a fenced code block or plain, and the router's check against the feature's schema is what stands in for the CLI's.
  The prompt body travels on stdin, so no transcript reaches an argument vector or a process listing.

  The child's argument vector is not a preference.
  Measured on claude 2.1.267, a bare `claude -p` carries the whole agent harness into the system prompt, 32,903 cached input tokens, and answers the wrong question: it reviews the transcript instead of classifying it.
  Adding `--system-prompt` with `--tools ''` and `--setting-sources ''` takes the same request to 1,188 input tokens and returns a schema-validated answer.
  Dropping any of those three is not a cost regression, it is a wrong answer, so `hide-ai/tests/it/claude_cli.rs` asserts the vector the child actually received.

  A model turn runs with thinking off where the model honours it: the child gets `MAX_THINKING_TOKENS=0` whatever the operator's environment says, and the same test file asserts the value the child received.
  Print mode thinks by default, and a background answer is a short JSON object; on 2026-10-02 the context label spent a median 2,656 output tokens (at most 9,425) to return about 100, which put its median at 28 seconds and 43 of 252 requests past the 60-second deadline.
  No flag turns it off: `--settings '{"alwaysThinkingEnabled":false}'` left the thinking in place, and the variable removed it.
  Measured on claude 2.1.287 with the same label-style prompt, haiku with thinking answered in 56 to 81 seconds, haiku without it in 5 to 8 seconds but broke the prompt's rules (step verbs, listed items), and sonnet without it in 3.6 to 7 seconds with about 165 output tokens and the best answers, which is why `sonnet` is the default.
  That measurement does not hold for `claude-haiku-5-5`.
  Measured 2026-10-16 on claude 2.1.301 with a label-style answer of about 60 tokens, `claude-haiku-5-5` ignored the variable and `--effort low` alike and reported 384 to 403 `thinking_tokens` inside 460 to 739 output tokens, while `claude-sonnet-5-5` reported 0 and answered in 84 to 184.
  The result frame's `usage.output_tokens_details.thinking_tokens` is read for every Claude turn, and the attempt line carries it when it is above zero (see [Logging](#logging)), so a model that thinks anyway shows in the log instead of in the usage.
  A Hide AI setting that names a haiku therefore buys a cheaper token and several times as many of them; which of the two the subscription's limit counts is not documented, so compare the limit, not the token count, before choosing it.
  The rest of the vector keeps the turn from reaching anything else or outliving itself: `--strict-mcp-config`, `--disable-slash-commands`, `--no-session-persistence`, `--permission-prompts none`.
  `--bare` cannot be used: it reads `ANTHROPIC_API_KEY` only and never the OAuth keychain, so it is incompatible with the subscription login that is the user's own credential.

  A request asks for its answer one of two ways (`AiRequest.answer`, `AnswerMode`).
  `Schema` is `--json-schema`: Claude Code answers it with a synthetic tool call, which is a second model turn and a larger prompt, and the answer is the frame's `structured_output`.
  `PlainJson` drops the flag, names the schema in the system prompt and reads the one object from the `result` text (plain, in a code fence, or the span from the first `{` to the last `}`, the same reader as the text-mode CLIs); a `result` that holds no JSON is `InvalidOutput`, retried like any other, and the router validates the object against the schema either way.
  Measured on claude 2.1.301 with the same label-style input, `Schema` made two API turns, wrote 2,826 cache tokens and printed 184 output tokens on sonnet, and `PlainJson` made one turn, wrote 2,264 and printed 84.
  The schema rides in the system prompt, which is an argument of the child, so `PlainJson` is for short schemas only.
  Agent summaries, pull request summaries and worktree names ask `PlainJson`, because their answers are a few short fields; Project Memory keeps `Schema`, whose answers are long and strictly shaped.
  The mode is one contract for every provider that has a schema surface, not a Claude setting: Codex honours it the same way.
  In `Schema` mode Codex attaches the schema to the turn as `outputSchema`, which the Responses API enforces in strict mode; in `PlainJson` mode it names the schema in the thread's base instructions, sends no `outputSchema`, and reads the object from the agent message with the same reader.
  Strict mode is why: measured on codex-cli 0.163.0, a schema that uses `oneOf` or `uniqueItems` (the agent summary's turn-end schema uses both) fails every turn with HTTP 400 `invalid_json_schema`, so a `PlainJson` feature that sent its schema there never got an answer from Codex.
  The router's validation of the answer against the full schema is what stands in for strict mode, and an answer that fails it is `InvalidOutput`, never drawn.
  The text-mode CLIs have no schema surface, so the mode changes nothing for them.

  A feature may also name the reasoning effort it asks of a provider (`AiRequest.provider_efforts`, one entry per provider), which applies whenever that provider answers, first or as a fallback, unless the caller picked an agent and effort of its own (`AiPick`), which replaces it; a provider with no entry runs at its default (`AiRequest::effort`).
  Agent summaries ask Codex for `medium` and Claude for nothing, so Claude's thinking stays off: measured 2026-10-18 by re-asking real label turns on gpt-5.6-luna, `medium` turned none of 104 label answers from a question into done, while Codex's default turned one of 31 and `low` turned a done into a question.

  One child per request, never a persistent one: a second request to a live child reuses its `session_id` and the conversation accumulates.
  A cancelled or expired request kills the child, which is safe because print mode holds no state worth draining.

  The model list is the CLI's own.
  `ClaudeCliBackend::models()` starts `claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence --tools '' --setting-sources '' --strict-mcp-config --disable-slash-commands` with the same neutral directory, writes one `initialize` control request to stdin, reads the `control_response`, and ends the child; no model turn is made.
  Only `response.response.models[].value` is read, and `default` is left out because it is the empty choice every menu already has.
  The same response carries the logged-in account, which the parser never reads and nothing logs, stores or snapshots.
  The list is held for ten minutes (a menu opening is not a child start) and the child is bounded at twenty seconds; a list the CLI could not give is `ModelCatalog::Unknown` with `claude_models_unreadable:<stage>`, never a guess.
  The shape was observed on 2026-10-06 against the installed, logged-in CLI; the observed values (`opus`, `fable`, `sonnet`, `haiku` and full names) are the account's, not a contract, which is why they are read each time.
- `grok`, `pi` and `antigravity` answer in text through one shared one-shot runner (`hide-ai/src/runner.rs`, `text_cli.rs`), and `opencode` is registered but unsupported.
  See [The text-mode CLIs](#the-text-mode-clis).

## The text-mode CLIs

Grok, Pi and Antigravity have no output-schema surface the way Claude Code and Codex do (agy's `--json-schema` cannot be used; see [Antigravity](#antigravity)).
Each is asked, in its system prompt, for exactly one JSON object matching the feature's schema, and the object is read back from the text it printed: the whole text, the body of a code fence, or the span from the first `{` to the last `}`, in that order.
The router validates every answer against the feature's schema, so a wrong shape is `InvalidOutput` whatever the CLI said.
A failure is classified from the CLI's documented exit codes and what it printed to stderr: an authentication word or HTTP 401 or 403 is `NotAuthenticated`, a rate or quota word or 429 is `UsageLimited`, an overload word or 5xx is `Transient`, and anything else is `CompletionUnknown`, because the child ran and the prompt may have been submitted.
Nothing the CLI printed on failure is kept; the error carries the provider and the exit status.

The shared runner starts every child through `hide_platform::process::OwnedChild`, so it dies with its owner and on every exit path.
It bounds standard output at 8 MiB and standard error at 64 KiB (more is `InvalidOutput`, never a larger buffer), ends the child at the deadline or when the caller cancels, and gives it either the process environment or the login variables plus the few it names.
A prompt or system prompt that must be a file is written by `PrivateFile` into a `0700` folder of its own as a `0600` file, removed on every path; a folder whose owner died is swept at the next start by pid, the same rule as the codex home.
The transcript never reaches an argument vector.

| Agent | Command | Prompt, system prompt | Read-only by | Models | Login |
| --- | --- | --- | --- | --- | --- |
| `grok` | `grok --prompt-file <f> --output-format json --json-schema <s> --system-prompt-override <sys> --tools "" --no-subagents --disable-web-search --max-turns 1 [--model <m>]` with `GROK_MEMORY=0` and `GROK_DISABLE_AUTOUPDATER=1` | transcript in a `0600` prompt file | no tool, no sub-agent, no web search, one turn | `grok models`: the `* default` line and `- <id>` lines | `grok models` printing "not authenticated" is `NeedsLogin` |
| `pi` | `pi -p --no-tools --no-session --no-extensions --no-mcp --no-skills --no-prompt-templates --no-context-files --thinking off --system-prompt <sys+schema> [--model <m>] <instruction>` | transcript on stdin | no tool, no extension, no MCP, no session | `pi --list-models`, read tolerantly | `pi auth check --provider <p>` when the model is `provider/id` (exit 0 ready, 1 or 2 not), otherwise a non-empty model list |

Grok's answer is `structured_output` when present and its `text` otherwise.
Each CLI's own flags are the contract; a flag one of them stops accepting is a failed request that names the exit status, not a silent change of behaviour.

`opencode` uses `UnprovenReadOnlyBackend`.
The CLI documents no mode that guarantees a one-shot, tool-free, read-only run from a script, and a background feature must never be the reason an agent edits a file, so it is listed and never asked: availability is `NotInstalled` when no program is found and otherwise `Unsupported { reason: "cannot_guarantee_read_only" }`, `models()` is `Unknown` with the same reason, and `execute` returns `Unsupported`.
Settings shows it as installed and not selectable, with that reason.
Making it selectable is a change to its backend and to this table, with a documented read-only mode cited, not a setting.

### Oh My Pi

`omp` background AI requires the user's **npm SDK installation** of `@oh-my-pi/pi-coding-agent` and a compatible Bun on the account's PATH.
Hide neither installs nor bundles either dependency.
Homebrew and official standalone binaries alone are not supported: they contain no importable SDK, and the CLI's prompt flags do not disable every ambient instruction, MCP and executable extension path.
That gap closes with an upstream CLI contract that guarantees complete isolation, or an officially supported importable SDK distribution beside those binaries.
An existing interactive OMP installation is not evidence that background AI can run.

The supported exact SDK versions are `18.7.0` and `18.12.2`; an unverified version fails before authentication or inference.
Each official package declares `engines.bun >=1.3.14`; the bridge checks the installed manifest's requirement against the running Bun, rather than inferring compatibility from a successful probe.
The bounded locator follows Unix npm executable symlinks and Windows npm's sibling `node_modules`; an SDK or Bun installed elsewhere must be made visible beside `omp` or on PATH respectively.
The Rust API also accepts explicit package and runtime paths.
Missing SDK/runtime, package identity, mismatched dependency versions, inaccessible public exports and incompatible runtime all return actionable `ProviderUnavailable` reasons.

`hide-ai/src/omp.rs` starts the private `omp_bridge.mjs` through the existing owned runner for each probe or turn.
It imports the installation's public `@oh-my-pi/pi-ai`, `@oh-my-pi/pi-ai/auth-broker/discover`, `@oh-my-pi/pi-catalog/models` and `@oh-my-pi/pi-utils/{logger,env,dirs}` exports after checking their package identities and matching versions.
It never imports the coding-agent harness, a user model module, settings loader or extension registry.
`discoverAuthStorage` and `storage.keys.getWithCredential` own login discovery, refresh and authorization; Hide does not parse, export, copy or log credentials.
The child receives the platform's restricted login environment, so this boundary supports OMP's default profile and stored login, not inherited API-key or custom-profile environment variables.
Auth config values beginning with `!` are refused rather than executed; custom credential commands, model aliases, extension providers and `models.yml` overrides are outside this contract.
The public SDK's native addon loader remains part of the trusted user installation, not an operating-system sandbox.
Compiled-mode and native-directory dotenv overrides are refused before the eager native root import; normal official native loading and its platform cache remain SDK-owned.
SDK usage/ranking probes and file/console logging are disabled.
Raw SDK request debugging is forced off before imports, including when home/profile dotenv files enable it.
After the SDK resolves the initial opaque credential, its public directory API moves diagnostic state to an existing private file root, so the OS refuses request-dump writes, including HTTP 200 streams containing 400/413 errors.
The opened auth store retains its own source; no credential is repacked or exported.
The private root makes the SDK's installation-ID lookup use its in-memory random fallback; ordinary protocol session identifiers and OS/CPU headers remain SDK-owned.
Synthetic fixtures do not establish paid-service acceptance of those changing installation identifiers or broker-backed refresh behavior.
The bridge fails before inference if the private SDK root cannot resolve to that file, for example when Windows home and temporary files are on different volumes.
The SDK may maintain its own credential state; Hide writes no OMP conversation or session.

Model discovery uses the bundled chat/tiny catalog for providers with a concrete configured credential source, filtered to `anthropic-messages`, `openai-completions`, `openai-responses` and `openai-codex-responses`.
Other transports are excluded because the single-request fetch boundary does not cover them.
The list is `Fixed`, not an assertion of the account's model entitlements; `Ready` means usable SDK/runtime and configured credentials, with service authentication checked only on a turn.
No credential means `NeedsLogin` and no model request.
The default model is `anthropic/claude-haiku-4-5`; choose a listed model for a different provider's login.

The feature supplies its system prompt and input unchanged, with only the shared JSON-schema instruction added to the system prompt.
The official transport's fixed protocol framing is preserved; it is not an agent harness or permission to add ambient instructions.
For Anthropic OAuth, upstream `buildAnthropicSystemBlocks` prepends the fixed Claude Code identity and a billing header before the feature system block.
The billing version fingerprint uses SDK constants and characters of the already supplied user input, and its attestation hashes the outgoing request body.
The bridge removes the ambient version override before inference and never overrides the catalog's `injectClaudeCodeInstruction` compatibility setting.
The sources are the pinned [Anthropic transport](https://github.com/can1357/oh-my-pi/blob/v18.12.2/packages/ai/src/providers/anthropic.ts) and [fingerprint constants](https://github.com/can1357/oh-my-pi/blob/v18.12.2/packages/ai/src/providers/claude-code-fingerprint.ts); the contract permits only this transport framing and unchanged feature content, with no user/project instructions, context, personal data or tools added.
The bridge submits one message and an empty tool list directly to `completeSimple`, with no project/personal context discovery, MCP, tools, session harness or auxiliary model work.
WebSocket transport and hidden inference retries are disabled; the fetch boundary aborts after the first HTTP refusal or transport error and refuses a second send.
HTTP 401/403, 429, other 4xx and 5xx become the existing authentication, quota, rejection and transient variants; a lost response after submission is `CompletionUnknown`, never silently replayed.
The existing router alone owns any visible retry/fallback and validates the JSON against the feature's schema.
Usage remains one unknown call, never totals counted from duplicate stream events.

System and input travel in a private temporary request file, never process arguments.
The runner owns cancellation, deadline, process-tree cleanup and bounded stdout/stderr; request files are removed on every return.
Bun's Unix parent-death watch and the platform's Windows kill-on-close job end the bridge if its owner crashes.
An abrupt owner exit skips temporary-file destructors; private files can remain until the system cleans its temporary directory.
Requests are capped at 1 MiB and inference response bytes at 8 MiB.
The `omp-contract` CI lane runs both real SDK versions, synthetic stored credentials and a substituted localhost catalog on macOS, Linux and Windows; it proves the SDK boundary, not a paid service login or the operator's installation.

### Antigravity

agy (`hide-ai/src/antigravity.rs`) has no switch that turns its tools off, so a request runs in a folder Hide keeps for it, `~/.hide/ai/antigravity-read-only`, whose own `.agents/hooks.json` refuses every tool call: one `PreToolUse` hook with matcher `.*` whose command is `exit 1` (PRD antigravity-first-class D-09).
agy refuses a tool call whose `PreToolUse` hook exits nonzero, prints what agy cannot read, times out or is missing, so that hook is fail-closed, and agy 1.4.0 loads a folder's own hooks only when the folder is in `trustedWorkspaces` of `~/.gemini/antigravity-cli/settings.json`.
The install kit makes the folder, writes the hook file and adds the folder's exact path to that list when it installs agy's hook, and a pass repairs any of the three that went missing or changed; switching agy off removes the folder and that trust entry ([agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block)).
The backend refuses a request unless the folder, its hook file byte for byte and its trust are all as the kit wrote them, and availability says why while they are not (`Unsupported`, with `antigravity_read_only_missing`, `antigravity_read_only_hook_changed`, `antigravity_read_only_untrusted` or `antigravity_settings_unreadable`), so the provider stays dimmed rather than running with tools.
An answer whose stream shows a tool step that finished (`step_update` with `step_type` `tool` and state `DONE`) is refused as `InvalidOutput("antigravity_tool_ran")` as a second line.
Measured against agy 1.4.0 on 2026-10-16 with a real account and the real home: in that trusted folder every tool call the model tried (`view_file`, `run_command`, `write_to_file`, `manage_task`) ended refused and the proof file was never written, and in an untrusted folder the folder's hook was not loaded.

The command is `agy --log-file <log> --input-format stream-json --output-format stream-json --disable-slash-commands --print-timeout <deadline>s [--model <m>] -p ""`, run in that folder.
The transcript and the schema request go on stdin as agy's one input message, `{"event":"user","message":{"content":...}}`; plain `-p` ignores stdin, and the transcript never reaches an argument vector.
`--json-schema` is not used: agy ends such a turn through a `finish` tool, which the folder's hook refuses, so the schema is asked for in the prompt as for the other text CLIs and the object is read from the `result` event's `response`.
A `result` whose `status` is not `SUCCESS` is classified from its `error` text by the shared rules.
Every child gets the login variables, `AGY_CLI_DISABLE_AUTO_UPDATE=1`, and a `PATH` naming a folder that does not exist: a signed-out agy would open the operator's browser to sign in through a program it finds on `PATH`, and with nothing there every hook, the operator's global ones included, fails to start its shell, which agy also reads as a refused tool (B32).

Every agy child Hide starts (a request, `agy models` and the `/usage` read below) writes its log to one file per kind in `~/.hide/ai/antigravity-logs/` (`request.log`, `models.log`, `usage.log`), which Hide empties before the child starts, so the folder holds the last runs of each kind and never more.
Two children of one kind can overlap (a label request and a Memory request, or two model-list probes), and the later one's emptying can then drop lines the earlier one wrote; nothing reads these logs, so they are not serialized for it.
Without `--log-file` agy leaves a log of about 20 KB in `~/.gemini/antigravity-cli/log/` for every process and never removes one: measured on agy 1.4.0 from 2026-10-16 to 10-18, the five-minute `/usage` read alone left 226 there.
agy appends to a file it is given, which is why Hide empties it rather than only naming it; it is emptied in place, so a child of the same kind still running keeps writing to it.
`--log-file` has been run against agy 1.4.0 on macOS only (`agy models` and `-p /usage`, 2026-10-18); on Windows and Linux it is unverified ([INSTALL.md](INSTALL.md#what-each-system-supports)).
A log file that cannot be made the account's own empty file starts no child: a request is `ProviderUnavailable` with `antigravity_log_unavailable:<error>`, the model list `Unavailable` with `antigravity_models_log_unavailable:<error>`, and the usage read fails with `antigravity_usage_log_unavailable:<error>`.

Availability is `NotInstalled` when no `agy` is found, `NeedsLogin` while `~/.gemini/antigravity-cli` does not exist (agy has never been signed in there), `Unsupported` while the read-only folder is not ready, and otherwise what `agy models` answers: its `id<TAB>label` lines are the model list and make the provider `Ready`, a sign-in answer is `NeedsLogin`, and anything else is `Unavailable` with the reason.
The default model is the CLI's own, so no `--model` is sent until the operator picks one.
The result's `usage.input_tokens` and `output_tokens` are the call's counts; no model is attributed here, because the request's conversation is also in agy's history, where the session usage reader counts it under no project.
Each request leaves a conversation in agy's history, because agy has no flag that skips it; it is filed under the read-only folder, which no checkout reads, so no project's Sessions, labels or usage page shows it.

Proof level, as of 2026-10-16: Claude, Codex and Antigravity have been run live (Antigravity's tool-free turn, above, and its `/usage` read); Grok and Pi are proven by scripted stand-ins (`hide-ai/tests/it/text_clis.rs`, which records the argument vector, the standard input, the working directory, the environment and the file contents the child received) because none is installed on the machine that built this, so their flags, exit codes, output shapes and login probes follow the CLIs' published documentation and are not yet observed.
Grok's `--tools ""` giving a tool-free run, where the validated object sits in Grok's JSON output, and the format of `pi --list-models` are the unobserved parts; the parsers accept more than one shape and an unreadable list is `Unknown`, not empty.

## The operator's choice

Which agent answers, and which of its models, is a setting.
It lives in `~/Library/Application Support/hide/ai.json`, a sibling of Hide's own `state.json`, and `hide-ai/src/settings.rs` owns the path, the schema and the rule that turns a choice into a `RouterConfig`.
The file belongs to the machine that runs the core: a core move carries it in its copy and places it where the machine taking the core keeps it, keeping that machine's own file when it chooses the same and refusing one that chooses otherwise, and once the move commits the file the core left is set aside in `moved-out/<intent>` with the rest of its state (`herdr-core/src/node_migration/copy.rs`; `ARCHITECTURE.md`, Moving the core).

Hide writes it: the Settings Hide AI tab (`web/src/settings/HideAiTab.tsx`, rules in `web/src/hideAi.ts`) dispatches one `ai_settings` event per intent, the core applies it to the snapshot at once and queues the write, and the session-sync coordinator performs the write off the runtime mutex.
The core reads the file once, when this Mac's session-sync coordinator starts, and from then on holds the choice in the runtime, so a file edited by hand while `hided` runs is read at the next start.
The core's label analyzer reads the runtime's choice before each analysis, so a choice made in Settings reaches the next label without restarting anything, and an analysis already running finishes on the choice it started with.
A changed choice rebuilds the router, because a backend is constructed with its model; an unchanged one leaves the router and its sticky failover state alone.

The file names a provider, a model per provider, whether Hide AI is on, an ordered fallback list, and whether agent labels are made at all:

```json
{
  "provider": "claude",
  "models": { "claude": "sonnet", "codex": "gpt-5.6-luna" },
  "enabled": true,
  "fallback": [{ "provider": "codex", "model": "gpt-5.6-luna" }],
  "agent_summary": true,
  "usage_limit": 95
}
```

`usage_limit` is a percent from 1 to 100 or `"none"`, written only when it is not the default 90; a file without it reads 90, and any other value makes the file malformed, which is reported and not guessed around.

`enabled` is the Settings › Hide AI switch and is on when the field is absent.
Off, `AiRouter::execute` returns `AiError::Disabled` at once and asks no provider anything, and a feature treats it as nothing to do, never as a failure.
`fallback` is the list of providers a request may move to, in order, each with the model it is asked with (absent means that provider's own default); it is empty when the field is absent.
A provider the registry does not know is ignored wherever it appears, so a file written by a newer Hide still reads, and `models` keeps the keys it does not know.
This includes retired `gemini-cli`, `gemini` and `cursor` ids: a saved retired choice becomes unchosen, with the internal default `claude` but request routing disabled until a supported agent is selected through the existing first-run or operator choice.
Retired fallback entries are ignored, their raw per-provider model entries survive rewrites, and neither their program nor a replacement provider is asked a model request before that selection.
`AiSettings` is the one owner of the rules: `set_provider` removes the chosen provider from `fallback` and adopts the model it had there, `add_fallback` refuses the chosen provider and a duplicate (`FallbackRefusal`), `remove_fallback` and `set_fallback_model` edit one entry, and `models_by_provider()` is the model of every provider for building backends.

`agent_summary` is the Settings › Hide AI › Features Agent summaries switch (PRD overview-request-view D-11), on when the field is absent.
Off, the label analyzer asks nothing and cancels the request it is running, and no surface shows a label; worktree naming and Project Memory keep their own switches.
Turning it does not rebuild the router, because it is not part of the router's choice.

The defaults are the registry's own constants: `claude` with `sonnet`, `codex` with `gpt-5.6-luna`, and the CLI's own model for every other agent.
`provider` is in the file only once it was chosen (`AiSettings.chosen`), so a file that only turned a switch makes no choice for the operator, and a settings value with nothing chosen asks no model anything (`router_config().enabled` is `enabled && chosen`).
`AiSettings::provider_for_first_run(availability)` is the one rule for what a first run chooses: the first provider in the fixed order that is `Ready`, and none when none is, which leaves the feature off until the operator chooses.
The core applies it, not the settings crate: while nothing is chosen and Use Hide AI is on, the coordinator asks only the agents that are switched on in this Mac's kit and installed whether they are signed in (`AiRequest.selecting`, availability only, no model list, every five minutes while the tab is closed and at once while it is open), and the first of the fixed order that answers `ready` is chosen with its default model and the choice is stored (D-18, D-27).
A stored choice is never replaced by that rule, an agent that is not switched on in the kit is never chosen by it, and with none signed in nothing is chosen and Hide features run without a model; signing in later is noticed without any setting being touched (B45, B47, B48).
A file Hide wrote before this default changed names `haiku` for claude like any other choice, so it keeps haiku until the operator picks a model in Settings.
A file that is not there means nobody has chosen, so the defaults stand and nothing is reported.
A field that is missing takes its default and a field the crate does not know is ignored, so an older Hide reads a file a newer one wrote.
A file that exists and cannot be read is never taken as the defaults in silence: Hide states the reason on the Hide AI tab and writes an `ai_settings` `settings.unreadable` record to its diagnostic log, and only then do the defaults apply.
A write that fails says so on the same tab, because a choice the operator made and the file on disk must not silently disagree.
A write that succeeds marks the choice as chosen at once, so the tab stops calling it the default without waiting for the next launch to read the file back.
A session with no home directory to write to reports that on the tab for the same reason: the choice has already left the runtime, so it cannot be dropped quietly.

Choosing a provider changes which provider is asked first and nothing else.
Every feature, Project Memory included, asks the chosen provider and then the fallback list under the policy below, while the router's retry, cooldown, cancellation, duplicate-suppression, process, and budget rules apply to all of them.

The Hide AI tab reads `status.background_ai`, one section the core publishes (`herdr-core/src/model.rs`, `BackgroundAiSnapshot`): `enabled`, `provider` (null until one is chosen), `chosen`, `agent_summary`, one `providers` row per registered agent in the fixed order (`agent` is the shared agent adapter id; `state` one of `ready`, `needs_login`, `usage_limited`, `not_installed`, `unavailable`, `unsupported`, `unread`, `installed`, `login_checked`, `selectable`, `retry_at_ms`, `model`, `models`, `models_fixed`, `cli_default`, and `models_unavailable_reason`), the ordered `fallback` list, and `refusal`.
Every reason in it is a code the shell turns into words, never prose, an account, a path or a conversation, and the cause of a failed model-list read stays in the diagnostic log (B68) as an `ai.models.unavailable` record with the agent and a `reason=` code, written when the reason first appears or changes and not on every read.
`ai_settings` carries `enabled`, `provider` (Runs on), `model` (for Runs on, or for an agent in the fallback list, which keeps its own), `fallback_add`, `fallback_remove`, `agent_summary` and the two observation hints; the core refuses an agent that the last read did not find selectable (`ai_settings.provider_not_selectable`), an unknown one (`ai_settings.unknown_provider`), the Runs on agent as its own fallback (`ai_settings.fallback_is_runs_on`) and a listed one twice (`ai_settings.fallback_listed`).
`refusal` is how the label analyzer last found the chosen agent (`AiStanding`, written on the analyzer's thread after each job): its reason class, the end of a usage limit when the agent said, and the listed agent a request made now would run on (`using`), or none when no listed agent can answer, which is the Runs on row's reason (B41, B42).
It is absent while the chosen agent answers and while Hide AI is off.

With Hide AI off or no agent chosen, the label analyzer answers every job as stopped without asking anything and the runtime says `agent_summary()` is false, so rows show the session's own text and a turn is made once it is on again; worktree naming returns the dialog's own name; Project Memory analysis waits (the project stays enabled, its saved Memory and Sessions keep working).
A request already running when the switch turns off finishes or is cancelled by the analyzer's own check, and `AiError::Disabled` reaching a turn is a wait (`AnalysisFailure::retry_after`), not a parked failure.

Project Memory's disclosure is version 3: analysis may go to the agent Hide AI runs on or one added under fallback, and with it the repository files a card cites and the instruction files the checks read (the repository's `AGENTS.md` and `CLAUDE.md`, the documents they link, and the operator's global instruction files).
Moving from version 2 to 3 turns every enabled Project off once; its stored Memory stays, and enabling it again accepts the new text.
A Memory analysis builds its router once, so one already running is stopped when Use Hide AI turns off or Runs on, its model or the fallback list changes (`Runtime::stop_memory_analysis_if_ai_moved`); the stored Memory stays and Retry starts a run on the new choice.
Opening a Memory store from the earlier version disables the projects that accepted version 1 until the operator accepts again; their stored Memory, search and Sessions stay.

Settings shows each provider's availability and the models it offers.
Both come from asking the provider, so no model list is written into the core or the shell; `AiRouter::availability()`, `AiRouter::models()` and `AiRouter::statuses()` are the source, and they cover every registered provider whether or not it is chosen.
`statuses()` answers each as a `ProviderStatus { provider, availability, selectable, parked_for }`: `selectable` is true for `Ready` and for a provider parked by a usage limit (it will answer again), and false for `NotInstalled`, `NeedsLogin` and `Unsupported`, whose `availability` carries the reason the screen shows.
An agent that is installed and not selectable is shown with that reason, never hidden.
Asking costs child processes, so the probe is a capability reader like the project panel's: the session-sync coordinator drives it, the work runs on a worker thread, the runtime mutex is never held across it, and it asks nothing at all while the group is off screen.
In the web shell the daemon holds that flag for every connected page: each page reports only its own demand, and a page that closes or disconnects releases it (`hided/src/demand.rs`).
`scripts/check-capability-readers-off-lock.sh` asserts that structurally, by name.

### What leaves the machine

Hide AI sends redacted conversation summaries, Project Memory batches and a pull request's title, body start, file names and check names (for a card's summary) to the agent the operator chose in Settings (Runs on) and to the agents the operator added under fallback, and to no other.
A registered agent that is neither chosen nor in the fallback list is never asked a request: its program may be installed and logged in, and it is only inspected, for the Settings screen, which starts no model turn.
Because the list is the operator's, adding an agent to it is the consent for that agent, and Project Memory's disclosure names it (its version moves when this description changes, so earlier opt-ins are asked again).

## Process ownership and budgets

A background feature asks a resident process for an answer, so `hide-ai` owns that process the way `oh-my-principle`'s resident-process practice requires: one spawn helper, one shutdown path, a child that dies with its owner, and caps that turn a leak into a reported failure rather than a larger number.
This exists because on 2026-09-17 a single label watcher (the retired plugin's process) held 1,699 `codex app-server` descendants and 11.6 GB for two idle days with no signal at all.

`hide stop`, and `hide connect` replacing a daemon of another build, send `hided` SIGTERM, then SIGKILL after five seconds if it has not exited (`hided/src/cli.rs`); `hided` turns SIGTERM and SIGINT into its graceful stop (`hided/src/lib.rs`), and dropping the core cancels the running label analysis, which ends its provider child, and joins the analyzer thread (`Core::drop`), so on that path no label request outlives the daemon.
On the SIGKILL path, and for a child no explicit shutdown reaches, a background AI child's end relies on the OS closing the inherited stdin pipe when the owning process exits.

Every child the crate starts goes through one spawn helper (`hide-ai/src/process.rs`), and the one-shot CLIs through the runner on top of it (`hide-ai/src/runner.rs`).
The codex app-server is owned through the stdin pipe it inherits: when the owner dies, the pipe closes and the whole tree ends, which is what makes a `kill -9` of the owner leave no survivors.
The claude backend and the text-mode CLIs start one child per request and kill it on every path.

The caps live in `RouterConfig` as measured constants, not settings (a setting with no value is unlimited, which is what a cap removes):

| Cap | Default | Measured against |
| --- | --- | --- |
| `max_in_flight` | 1 | requests running at once across the router |
| `max_per_minute` | 30 | requests admitted in any 60-second window |
| `max_app_server_descendants` | 4 | processes under the child the crate started, transitively, after a turn |
| `max_app_server_rss_bytes` | 1 GiB | resident size of that child and everything under it |
| `max_consecutive_restarts` | 3 | over-budget restarts before the provider is failed |

The two request-rate caps are checked before a request is submitted; crossing one returns `AiError::OverBudget { cap, measured }` at once and logs `ai.budget.exceeded`.
`OverBudget` is a refusal: it is safe to retry and it does not move to another provider, because the cap is the account's, not the provider's.
An in-flight request and any other provider are untouched.

The two process caps are measured after each codex turn (macOS `libproc`: `proc_listchildpids` and `proc_pidinfo`, no subprocess).
Crossing one logs `ai.app_server.over_budget` with the measured value and the cap, ends the app-server through the graceful shutdown path, fails the request with `OverBudget`, and lets the next request start a fresh app-server.
A request that completes under the cap clears the restart count; three consecutive restarts that do not clear it fail with `ProviderUnavailable(app_server_restart_cap)`.
On a platform without the kernel query the measurement is `Unavailable`: the process caps are not enforced and the log line says `measurement=unavailable` rather than a zero.
The label worker treats `OverBudget` as an environmental failure: it keeps the goal, drops the turn's line and end, and asks again after ten minutes, the same as any other environmental failure (`AnalysisFailure::retry_after`).

## Usage capability declarations

`hide-agent-adapter/src/usage.rs` declares all nine usage cells for every adapter: four limit scopes, tokens, cost, model breakdown, session attribution and project attribution.
Every unsupported cell names its reason and the condition that would change it; `NotBuilt { layer }` is refused by the contract test once that layer ships.
`UsageDeclaration::cells()` destructures every field exhaustively and is the source of `contracts/usage-coverage.json`; `export_usage_coverage` regenerates that contract, while `export_web_contract` includes the same declaration in the shell's adapter catalog.
The independent `hide-agent-adapter/tests/fixtures/support.json` matrix pins the expected support for all seven agents.
Layer 4 includes the six historical token and cost readers, with model and source attribution retained from each native usage record; Cursor is retired from the supported adapter list.
The table is generated by `export_usage_coverage --markdown`; the adapter contract test compares it byte for byte.

<!-- usage-coverage:start -->
| Agent | 5h | Weekly | Scoped | Monthly | Tokens | Cost | Models | Session | Project |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Claude Code | Supported | Supported | Supported | No such data | Supported | Supported | Supported | Supported | Supported |
| Codex | Supported | Supported | Not built (L5) | No such data | Supported | Supported | Supported | Supported | Supported |
| Grok | No such data | Not built (L5) | No such data | No such data | Supported | Supported | Supported | Supported | Supported |
| OpenCode | Excluded | Excluded | Excluded | Excluded | Supported | Supported | Supported | Supported | Supported |
| Pi | No such data | No such data | No such data | No such data | Supported | Supported | Supported | Supported | Supported |
| omp | No such data | No such data | No such data | No such data | Supported | Supported | Supported | Supported | Supported |
| Antigravity | No such data | Supported | Supported | No such data | Supported | No such data | No such data | Supported | Supported |
<!-- usage-coverage:end -->

## Usage limits

The sidebar footer's Usage chips and popover ([UI_BEHAVIOR.md: Usage](UI_BEHAVIOR.md#usage)) show a separate read-only capability owned by `herdr-core/src/usage.rs`.
It uses the user's existing CLI logins to read each declared provider's five-hour and seven-day account windows, and it never routes a model request through `hide-ai`.

For Claude Code, the core's own node runs `claude -p "/usage" --output-format json --no-session-persistence --setting-sources ''` through `ClaudeCliBackend::usage_text` (`hide-host/src/usage.rs`, the `UsageLimitFetch` call, which reports once a second so the reader's cancel ends the child) and the core parses the `result` text the CLI prints.
The core asks its own node for `UsageLimitFetch { source, root }`; a device refuses this call.
Codex's `auth.json` read, HTTPS request and session-file fallback all run on that node; the core keeps the schedule, parsing and rows.
The credential type is private to `hide-host` and cannot be serialized.
The node bounds credential reads at 64 KiB and HTTP bodies at 256 KiB, returns only numeric window fields from Codex, and reads at most 2 MiB from each candidate session tail.
The transport timeout is ten seconds; HTTP errors carry only a fixed diagnostic code, status and bounded Retry-After header.
`/usage` is a local command: the CLI authenticates against its own keychain item, makes no model turn (`duration_api_ms` 0, cost 0), and with `--no-session-persistence` leaves nothing under `~/.claude/projects/`, in `claude --resume`, or in Hide's Agent Conversation list.
Hide holds no Claude token at any point and never opens the keychain itself; the earlier direct keychain read is gone because an ad hoc signed dev build has a new code identity on every rebuild, so macOS revoked "always allow" and the row fell to a three-second timeout.
The child receives exactly the variables `hide_platform::process::LOGIN_CHILD_VARIABLES` names (`hide_ai::USAGE_ENVIRONMENT`): `HOME`, `PATH`, `USER`, `LOGNAME` and `TMPDIR` on macOS and Linux, and `PATH`, `PATHEXT`, `SystemRoot`, `USERPROFILE`, `USERNAME`, `TEMP`, `TMP`, `APPDATA`, `LOCALAPPDATA`, `ComSpec`, `windir`, `SystemDrive`, `ProgramFiles`, `ProgramFiles(x86)`, `ProgramData`, `HOMEDRIVE`, `HOMEPATH` and `CLAUDE_CODE_GIT_BASH_PATH` on Windows, where Node reads its home from `USERPROFILE` and does not start without `SystemRoot`, and the Claude CLI needs Git Bash, which it finds through `CLAUDE_CODE_GIT_BASH_PATH` or `ProgramFiles`.
It runs in Hide's state directory (`~/.hide/state` by default).
`USER` is what lets the CLI find its keychain account; without it the CLI prints `/cost` text as if logged out.
`HERDR_*` and `CLAUDECODE` are withheld on purpose, so even a hook that did run would see no Herdr pane.
`--setting-sources ''` reads none of the operator's settings, so none of their hooks or plugins runs for the read: a session-start hook may leave a folder in `~/.claude/session-env/` for every session, which Claude Code never removes, and on 2026-10-18 (claude 2.1.302) the read with the settings left one every five minutes and without them none, with the same windows in the answer.
The read reaches no network: with every outbound connection denied (`sandbox-exec`, 2026-10-18) it answered the same windows, so a proxy or certificate the operator sets only in `settings.json`'s `env` block is not needed for it.
A Hide AI request reads no settings either and does reach the network, so for it such a proxy has to be in the environment Hide is started with.
`--bare` cannot be used, because it never reads the keychain.
`CLAUDE_CONFIG_DIR` is no longer an environment key Hide reads: the child does not receive it, so the CLI uses its default configuration directory.

The pure parser (`herdr-core/src/usage/limits/`) reads `Current session` as five hours and `Current week (all models)` as seven days; other weekly scopes become model rows.
Percentages may say `used` or `left`; the latter is converted to used percent.
A zero session line without a reset is `NotStarted`, never a displayed zero; another missing or malformed reset is `Unreadable` for that window alone.
A reset that passed becomes `Expired` and carries no percent.
Missing Codex five-hour data is `NotOffered`, because that window depends on the account plan; durations of 18,000 and 604,800 seconds identify its two windows regardless of their primary/secondary position.
A whole response with no recognized window fails as `FormatChanged`, immediately discarding retained values.
Readers cap the result at sixteen windows and a model label at 64 bytes.

Claude reset times use the printed year and IANA zone, or the next wall-clock match when the year is absent; a match that passed within the last window is the reset that just passed.
`herdr-core/src/zoneinfo.rs` uses `jiff`, with system zoneinfo on macOS and Linux and its bundled database on Windows.
The observed 2.1.274 output is fixed as a scrubbed fixture under `herdr-core/tests/fixtures/claude-usage/`.
Only English output is parsed; an unrecognized response is unavailable.
A `/cost` response with no `Current` line means signed out; transport failures and timeouts retain a recent success for up to fifteen minutes.
The snapshot carries cause codes, not screen wording: the web gives a sign-in action only for an authentication failure and keeps network diagnostics out of the screen.

For Antigravity, the core's own node runs `agy --log-file <log> -p /usage --output-format json` through `AntigravityCliBackend::usage_text`, with the same environment as a request and no model turn (`hide-host/src/usage.rs`).
Each read starts agy's whole runtime, its language server included, and takes about 5.5 s (measured 2026-10-18 on agy 1.4.0); its log goes to `usage.log` as described under [Antigravity](#antigravity).
The parser (`herdr-core/src/usage/limits/antigravity.rs`) reads `/command/data/groups`: each group of models shares weekly buckets that say what fraction is left and when it refills, the `gemini-weekly` bucket is the account's weekly window, and every other group's weekly bucket is a window scoped to that group's models; agy has no five-hour or monthly window.
A sign-in answer is `NotAuthenticated`, and a frame with no groups is `FormatChanged`.

The child runs on a `BackgroundRead` worker, so the coordinator thread that applies every Herdr pane event never waits on it; one child runs at a time, and dropping the reader cancels a child still running at shutdown.
The failure event carries the provider and the failure kind only; no terminal text, token, or account identifier is logged or placed in a snapshot.

For Codex, the core's own node reads `auth.json` under `CODEX_HOME` or `~/.codex`; an invalid explicit `CODEX_HOME` disables that read instead of falling back to `HOME`.
The access token exists only long enough to build the HTTPS authorization header, and Hide never refreshes, persists, logs, or includes it in a snapshot.
The Codex usage endpoint is called outside `Mutex<Runtime>`; a 429 honors `Retry-After` in either delay-seconds or HTTP-date form, or falls back to bounded 5, 10, and 15 minute delays.
An offline response keeps a non-expired success for at most 15 minutes; Codex can then fall back to the latest timestamped five-hour and weekly windows in a local session JSONL file.

The registry schedules one read per declared source, deduplicated across its windows.
Non-hidden providers are first read one second after launch, every five minutes while a shell window is visible, and at once when the popover opens on a read older than a minute.
A hidden provider stops scheduling unless Memory needs its limits; these consumers use the same `weekly()` and `exhausted_until()` accessors.
The driver cancels and joins its workers when the coordinator ends, outside the runtime lock.
The web shell reports its page visibility and its popover through the two `ui_state_update` hints `contracts/hided-ws.schema.json` declares as `uiStateUsageHints`.
The previous Claude `.usage-cache.json` input is not read.
Failures produce one structured event containing only provider, HTTP status where there is one, and error kind.

## Local historical usage readers

`hide-session::usage` reads content-free usage events from the local Claude Code, Codex, OpenCode, Pi, omp and Grok session stores.
It opens agent files and the OpenCode database read-only, does not invoke an agent, and does not reuse conversation parsing.
Candidates are confined to the selected agent's native root, including Claude subagent files and Codex archived sessions; Cursor is unsupported.
Discovery reports its 10,000-candidate cap and unreadable entries.
One read returns at most 1 MiB of source data, a bounded checkpoint, normalized events and skip counts; a checkpoint never contains a partial JSON line or conversation content.
JSONL parsing yields after 20 ms and caps the event batch; the read budget includes the prefix and consumed-tail hashes that fence a rewritten checkpoint.
File identity, truncation, parser version and a same-size rewrite also trigger a rescan, whose events still pass the ledger's first-value deduplication.
The executable contracts are `hide-session/tests/it/usage.rs` and the scrubbed Grok fixture under `hide-session/tests/fixtures/usage/`.

Input, output, cache read and cache write are disjoint components.
Reasoning is a subset of output, and one-hour cache writes are retained as a subset of cache writes.
Native identities are hashed with full SHA-256 digests so copies and repeated notifications keep one identity.
Claude streaming updates retain their message/request identity, while its subagent files count separately.
Pi and omp use session/message identities and retain their provider metadata and recorded cost.
Codex prefers native per-response `TokenUsageRecord` items, excluding inherited records by thread identity, and ignores subsequent duplicate `TokenCount` notifications.
Older rollouts use the last response's usage, with cumulative counters only as the repeated-notification identity; compaction estimates and context-window-full synthetic totals never count.
A copied fork prefix without a native boundary is reported as skipped.
Grok prompt totals include children: `summary.json`'s `session_kind` beginning with `subagent` excludes a child's own records, and preserved prompt/model identities deduplicate top-level fork copies.
Grok incomplete usage or partial cost retains the reported token lower bound with an incomplete flag and no cost.
OpenCode reads completed assistant messages' `step-finish` parts, whose input already excludes cache and whose output excludes reasoning, so reasoning is added once.
Each part supplies its own cost; the accumulated message cost is used only for a single-step message without a part cost, and message tokens never feed totals.
Its bounded keyset cursor revisits the last timestamp for late rows and restarts on file replacement or a changed schema version, including `VACUUM`.

### Historical cost estimates

Native recorded costs remain authoritative for Grok, OpenCode, Pi and omp.
Claude and Codex use exact model-id matches from the public [LiteLLM price JSON](https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json), restricted to first-party Anthropic and OpenAI rows.
That root-level dataset is outside LiteLLM's enterprise subtree and covered by its [MIT license](https://github.com/BerriAI/litellm/blob/main/LICENSE); the copyright and permission notice travel with the cached table.
LiteLLM was selected because its native one-hour cache-write fields avoid guessing that price from a five-minute rate.

The estimate uses standard API rates, including disjoint cache reads, five-minute and one-hour writes, and whole-request context tiers; it is not subscription billing or a reconstruction of an unrecorded fast, batch or regional mode.
Daily rollups retain token-only bands at the supported 100,000, 200,000 and 272,000 prompt-token thresholds, counting input plus both cache categories before choosing a band.
This lets a later table reprice old days after individual events expire.
An unknown model, new unsupported context threshold, missing nonzero token-category rate or incomplete native usage produces a partial lower bound, never an invented zero price.

The worker fetches one constant URL, with no credentials, usage payload, model query or redirects, and a five-second global timeout and 16 MiB response cap.
It records the attempt in the ledger before starting the request, including failed attempts, so restart cannot exceed one attempt per day.
The retained table is capped at 2 MiB and 2,048 models and is stale after 60 days; a failed refresh keeps the last table, and an offline first run leaves tokens available while cost waits for a table.
The price worker asks only the own node for the fixed public URL; devices refuse this call and the core has no HTTP client.
The price worker is joined on shutdown, and diagnostics contain fixed codes and counts rather than source paths or response bodies.

## Counting Hide AI's own usage

Hide AI's background calls (agent summaries, Memory, pull request summaries and worktree names) run through the provider CLIs without leaving a session file, so the historical readers above never see them.
Each router built for one of those features carries an `AiUsageSink`, and the router tells it once per attempt that reached a provider.
The Settings probe has no sink: it asks a CLI whether it is ready and is not a feature call.
An agent pane that Hide starts (a Memory promotion) is a native session, so its own transcript already counts it and the sink never hears of it.

What counts is the attempt, not the request:

- A response with usage is told with its per-model tokens, whether or not the answer then passes schema validation. A rejected answer cost the same tokens, and a retry or a fallback is its own attempt.
  Several model entries still count as one backend attempt; only their tokens and costs are added separately.
- Callers joined onto one in-flight request are told once.
- A `Timeout`, a `CompletionUnknown`, a `Cancelled` or a backend-side `InvalidOutput` is told as an **unknown call**: the backend had the prompt, so a model turn may have run, and the error carries no usage.
  Each is its own attempt: the second of two `InvalidOutput` attempts is a second dispatch, told once more.
  A Codex turn that reports `interrupted` is `Cancelled`, and `responseStreamDisconnected` is `CompletionUnknown` regardless of its optional HTTP status: both may follow model work, count once, and are never retried automatically.
  After `turn/start`, a failed turn with missing or unfamiliar `codexErrorInfo` is also `CompletionUnknown`; only a typed refusal or a failure to establish the HTTP/response-stream connection can remain retryable without an unknown call.
  The one exception is an `other` failure whose `message` is the Responses API's own error body as JSON with a 4xx `status` (codex-cli 0.163.0 forwards it that way): the API refused the request before any model work, so it is `Rejected(codex_request_refused:http_<status>[:<code>])`, with the body's `error.code` kept only when it is a short identifier such as `invalid_json_schema`; a 401, 403 or 429 there is `NotAuthenticated` or `UsageLimited` as a typed one is, and the body's prose is never read.
- A response that arrives without usage (Pi and Grok today) is also an unknown call.
- A refusal before the prompt went out (`ProviderUnavailable`, `NotAuthenticated`, `UsageLimited`, `Unsupported`, `Transient`, `OverBudget`) and a `Rejected` request (the provider refused the request itself: an HTTP 4xx, a prompt that does not fit, a Codex bad request, full context window or API-refused request) are not told: they did no model work.
  Neither is a cancel that returns before a backend is asked, nor a caller joined onto another's flight that gives up, since neither dispatched anything.
- A schema failure of an answer that did arrive is the answer's own attempt, told once with its usage; the failure adds nothing.

An unknown call is a count on its day, provider and feature. It adds nothing to an amount and makes the day's known amounts a lower bound; the popover marks them `≥` and the table footnotes them.

Claude's tokens come from the result frame.
With one model in `modelUsage`, the frame's whole `usage` is that model's, including its cache categories.
With several models, each model's own `modelUsage` entry is used, and their sum has to equal `usage` in every category or the call is unknown rather than guessed.
When several models ran and `usage.cache_creation` reports one-hour writes, the one-hour share cannot be attributed to a model: the call keeps its writes as five-minute writes and is marked a lower bound, so its price is partial.
Input and output are required; missing cache fields count as zero; one-hour writes above all cache writes make the call unknown.
Codex's tokens come from the last `tokenUsage.total` of the app-server turn and the model from the `thread/start` response: input is `inputTokens` minus cached and cache-write tokens (a negative result makes the call unknown), and reasoning above output makes the call unknown.
Grok reports no usage the code can read, so its calls are unknown (D-06 of the `hide-ai-usage-split` PRD).

The ledger worker owns the writes.
A router thread only appends to a bounded in-memory queue (256 attempts) under its own mutex: no I/O, no runtime lock, no time zone lookup and no clock read beyond the attempt time.
Past the bound the oldest attempt is dropped and counted by the quarter hour it was made in, and the worker dates it as an unknown call with an `ai_usage_queue_full` diagnostic.
The worker drains the queue at most once a second and writes the batch in one transaction into `ai_daily(day, provider, model, feature)`, a table of the same `usage.sqlite3` that holds totals and an unknown count and nothing that names a request, prompt, subject or path.
Each row's totals JSON also keeps an `attempts` count separate from the per-model call count used to price its tokens.
One attempt increments only its first successfully stored model row, so an earlier model refused at the row cap cannot hide the call or the surviving day's tokens.
Feature ids outside the closed list (labels, memory, factory, issue) are counted as `other` and named once in an `ai_usage_feature_unmapped` diagnostic, so a new feature is counted from its first call.
A write that fails loses its batch with the transaction, so each failed write names the number of calls it lost in a diagnostic; a Hide AI table that cannot be read at open is named in a diagnostic and does not stop the session ledger.
A usage report that contradicts itself (an invalid model name, one-hour writes above all writes, reasoning above output) makes that attempt one unknown call with an `ai_usage_tokens_invalid` diagnostic.
The table is capped at 40,000 rows; a combination that has no row yet is refused past the cap with `ai_usage_row_capacity`, an existing row still takes calls, and the earliest refused day is kept in a one-row `ai_state` table so every day since reads as incomplete until it ages out of retention.
Rows age out with the daily retention horizon, 512 per maintenance turn.
`user_version` is not raised: the tables are created `IF NOT EXISTS` on open, and a store from before this change simply has none.

Nothing is backfilled: a call made before this change left no record, so the table counts from its first row.
The full history names that day, and an earlier day reads "Before counting" rather than "No records".
The projection prepares Hide AI on the all-agents range only, adds it to the headline day amounts, bars, grass levels and totals, and computes every agent's contribution, day and lifetime share, over the agents' session tokens alone.
A day with only Hide AI calls is still a day of the calendar: it counts toward active days, streaks, the peak and the grass levels, as the headline does.
Costs go through the same price table as a session call, by provider, and the popover shows tokens and a share but no money.
Per call, Hide AI adds one queue append on a router thread; the worker's extra work is one batched write a second at most, and a publish only while a usage page is visible and the rows changed.
Those costs are by construction, not measured: no CPU, memory or snapshot-size figure has been taken for this feature.

## Where the tokens go

Measured from the usage ledger (`ai_daily`) on 2026-10-15 and 2026-10-16, per call, with the list prices the cost estimate uses (`price_cache`).

| Feature | Cache read | Cache write | Read share | What is written |
| --- | --- | --- | --- | --- |
| agent summaries | 3.7k to 4.0k | about 1.3k | 70 to 75% | the turn's own events |
| Project Memory | 2.2k to 2.4k | about 8.0k | about 22% | the session events and the cards compared with them |
| Factory judgments | 0.8k to 1.0k | 12.9k to 14.4k | 5 to 7% | the issue, pull request or diff under judgment |

A new `claude -p` process reads the previous call's cache when the tools, the system prompt and the messages before the changing part are identical and the model is the same, so the fixed prefix (the system prompt and, in `Schema` mode, the schema) is read and the part that differs per call is written and not read again.
Subscription turns are written with a one-hour lifetime, which prices a write at twice the input rate, so at the list prices the writes of the changing part are the larger share of a call's cost: about two thirds of an agent summary's and about nine tenths of a Memory call's, with the output second.
The share is a statement about prices; how the subscription's limit counts a cache write or read is not documented.
Setting `CLAUDE_CODE_PROMPT_CACHE_TTL=5m` is documented to write at 1.25 times instead, and has not been tried on a call whose input differs, so it is not used.

## Selection and fallback

A request is asked of the chosen provider and then of each provider in the `fallback` list, in that order, once each, and of nothing else (`RouterConfig::priority` is exactly that list).
A provider that is registered and not on it is inspected for Settings and never asked, so a login the operator never meant to use is not spent.
While nobody has chosen, the internal default is `claude` and request routing is disabled until the existing selection rule or the operator chooses a supported provider.

One request has one overall deadline: its own deadline times `overall_deadline_factor` (2), capped at `overall_deadline_cap` (90 seconds) and never less than its own deadline.
Every provider and every retry gets what remains of it, and once nothing remains the request ends with the refusal it last saw (`ai.fallback` is not logged for a hop that was never taken).
A fixed 30 second request therefore spends at most 60 seconds across all providers, where the old one-deadline-per-provider rule could spend that many times over.

`AiRouter::provider_state()` answers the standing question a result alone cannot: the `selected` provider is the one the priority puts first, `active` is the provider a request made now would run on, and `degraded` says why `selected` stepped aside and what remains of its wait.
`active` is `None` when every provider on the list is degraded, which is a reachable state, and is reported rather than filled in with a provider that cannot answer.
`AiRouter::last_answered()` names the provider that most recently produced an answer, which is the only way to say "answered by the fallback" after the request that did so has returned.
`provider_state()` is a query: it never logs and never runs a request, though it may refresh a stale availability answer, which is what makes a return to the selected provider visible.
`availability()` still reports each provider's own reason next to it.

Every failure is one of two families.
A refusal (`ProviderUnavailable`, `NotAuthenticated`, `UsageLimited`, `Unsupported`, `Transient`) means the provider never took the request, so asking again repeats nothing.
A `Rejected` request is a refusal that is retried like an `InvalidOutput` and reported to callers as the same `invalid_output` class.
An `InvalidOutput` from a backend is not a refusal: the backend had the prompt and its answer was unusable, so a model turn may have run, and the usage ledger counts it as an unknown call (see Counting Hide AI's own usage).
A completion-unknown outcome (`Timeout`, `Cancelled`, `CompletionUnknown`) means the request was submitted and its fate is not known: the deadline passed, the caller cancelled, or the connection or child was lost after `turn/start` went out.
The codex backend draws that line at the `turn/start` write: a lost child before it is `ProviderUnavailable`, a lost or unanswered `turn/start` and anything after it is `CompletionUnknown`.

A request moves to the next provider only on `ProviderUnavailable`, `NotAuthenticated`, `UsageLimited` or `Unsupported`, and the move is written to the log as `ai.fallback` with `from` and `to`.
`Unsupported` is what `opencode` answers, so a fallback list naming it moves on to the next entry.
A text-mode CLI's failure it cannot classify is `CompletionUnknown` and ends the request: moving it on could ask two agents the same question.
`Disabled` is neither a refusal nor a completion-unknown outcome; nothing was asked.

That move is sticky, so the reason outlives the request that found it.
A provider under an account-wide cooldown is not asked for an answer and is not asked whether it is logged in either: the cooldown already answers, and probing a provider that cannot answer for hours is the poke stickiness exists to stop.
An outage or a missing login is remembered by the availability cache for its window instead.
When the cooldown expires the router drops that provider's cached availability with it, asks the provider once, and returns to it when it can answer; nobody has to tell the router the limit reset.
Because a sticky failover has no second provider to try, `ai.fallback` never fires again, so entering and leaving failover each get their own event: `ai.provider.degraded` names the selected provider, its reason and the provider now active, and `ai.provider.recovered` names the return.
Each is logged once per transition, not once per request.
A completion-unknown outcome is final: one call, no second attempt on the same provider, no attempt on another, because a duplicated completion is worse than a missing label.
An answer the schema refused is retried on the same provider only.
There is no silent fallback: the result names the provider that answered, the log names it too, and a failure says why none did.

## The usage limit

One setting, `usage_limit` (`hide-ai/src/usage_limit.rs`), says how much of an agent's week Hide AI may use for what it starts on its own: agent labels, Memory analysis, the card recheck and the pre-promotion check.
An agent whose weekly figure is at or past the limit cannot answer such a request, so the router moves on to the next agent of the same priority list as for any agent that cannot, and logs the move as `ai.usage_limit` with `reason=usage_limit`, the agent, its usage, the limit and `action=fallback` or `action=paused`, never any content.
It is not a second order and not a second list.
The judgment is a request-time check against a shared `UsageLimits` cell, not a flag on `AiRequest` (which crosses to the node): the core writes the limit and each agent's reading when either moves, a router reads them on its own worker thread, and a router built without the cell, or a request that names its agent with `pick`, is never judged.
The routers for what the operator waits on (worktree names, issue intake review), `hide memory add` are built without the cell, so the limit never delays them (PRD hide-ai-usage-limit D-03).
`statuses()` and `availability()` stay ungated, so Settings still lists a past-limit agent as signed in; only `active` in `provider_state()` skips it.

The figures are the weekly rows the toolbar's usage read already produces (`usage` through `weekly_reading()`, the seven-day account window with a percent and a future reset); no new read command exists.
With no window attached the same reader also runs while a limit is set and Hide AI is on and either the order is paused (only a fresh figure can lift it) or an agent worked in the last 30 minutes, so labels are not judged by a figure that is days old; that is at most one read per agent every five minutes and none while nothing runs.
Until the first read lands (one second after launch) an agent has no figure and is not limited.
An agent with no figure, or whose reading belongs to a window that has already reset, is never past the limit, and No limit judges nothing.
When every agent of the list is past the limit, or the list is empty because Hide AI is off, nothing is sent: `Runtime::usage_paused()` folds into the same states as Hide AI off, so labels show each session's own text, Memory analysis reports `AiNotReady` and the pass builds no router, and both resume on the next usage read, settings change or coordinator wake after the limit is raised or the week turns.
Entering and leaving the pause is one `usage_limit.paused` or `usage_limit.resumed` diagnostic each.
The limit belongs to the core machine like the rest of `ai.json`, so a window on another device shows the core's value and its choice is applied there.
The snapshot carries `status.background_ai.usage_limit`, each agent row's `weekly` and `past_limit`, and the refusal's `past_limit` reason with `used_percent` and `limit`; they change only when a figure, the limit or the refusal changes, so an unchanged usage read publishes nothing.

## Retry policy

- `Transient`: exponential backoff, at most four attempts on the same provider.
- `InvalidOutput`, `Rejected`: at most two attempts, because the input is deterministic; a backend-side `InvalidOutput` may have spent tokens and is counted as an unknown call, while an answer rejected by schema validation keeps its reported usage.
- `Timeout`, `Cancelled`, `CompletionUnknown`: no retry anywhere; the caller decides what to do with the turn.
- `Internal`: the request's leader thread panicked; every caller waiting on the same intent receives this error, the intent's key is freed, and the next call starts fresh.
- `NotAuthenticated`: no retry; availability is re-read after its cache window.
- `UsageLimited`: an account-wide cooldown on that provider, the provider's reset time when it reports one and ten minutes otherwise; every subject waits together.
  `codex` reports its reset time from `account/rateLimits/read`; `claude` reports none, so it takes the default (see Known gaps).
- Duplicate suppression: a second request with the same feature id, subject id and input hash joins the in-flight one instead of starting a new call.
  The in-flight entry is owned by a guard, so a leader that panics still wakes its joiners and releases the key.

What the caller does after the router gives up is the caller's decision.
The label worker parks a turn for good on a settled failure and asks again after ten minutes on an environmental one (after the provider's reset time when a usage limit reports one); a turn end that timed out is asked once more before it is parked (PRD overview-request-view D-33), so a turn costs at most three requests.

## Logging

The router emits `ai.attempt`, `ai.request.finished`, `ai.request.joined`, `ai.fallback`, `ai.usage_limit`, `ai.provider.degraded`, `ai.provider.recovered`, `ai.budget.exceeded`, `ai.app_server.over_budget` and, once per UTC day, `ai.daily_rollup`; the codex backend emits `ai.app_server.idle_exit`, `ai.codex_home.swept` and `ai.codex_home.sweep_failed`.
A line carries the request id, feature id, provider, outcome class, attempt, duration, input length, output tokens and schema version; an `ai.attempt` line also carries `thinking_tokens` when the provider reported some above zero, and `effort` when the attempt asked its provider for one.
Every `ai.request.finished` of a provider that declares itself measurable (`AiBackend::measurable()`, today only codex, whose app-server is resident) also carries the app-server pid and the process measurement (`app_server_pid`, `descendants`, `rss_bytes`), or `measurement=unavailable` where the platform cannot measure it.
It never carries the prompt, the input, the generated text, a token, a file path from a transcript, or a provider thread id.
Hide's core writes the events of its own routers and backends (the label analyzer, the Settings probe and Project Memory) to its diagnostic log, `Logs/core.jsonl` beside `state.json`, as records with `component` `ai` and the event name as `kind`.
A backend's events are produced on the node; the node holds at most 256 of them per backend and returns them with the next answer to that backend, where the core writes them, and a held overflow arrives as one `ai.log.dropped` event with its count.

## Known gaps

- A Claude usage limit carries no reset time the code can read.
  The 429 result frame names the window only inside a localised sentence in `result` ("You've hit your session limit, resets 6:42pm (Asia/Seoul)"), so the backend reports `UsageLimited { retry_after: None }` and the router's ten minute default cooldown applies to a window that is really five hours or seven days.
  Keeping the shared default is a decision taken on 2026-09-10, not an oversight: a 429 result frame reports `usage` zeros and `total_cost_usd` 0, so the park is self-correcting at the cost of about a second of latency and one log line every ten minutes, and it is what notices the reset soonest.
  Closing this properly needs a structured reset field in the result frame, not a parser for that sentence.
- The Claude backend does not check that the configured model is one the account offers; the codex backend checks `model/list`.
  A model the account cannot use is therefore discovered as a request failure rather than as an availability state.
  The list `models()` offers is the CLI's own, read through `initialize`, so no model list is written into the code.
  The Settings model control always offers the configured model even when it is not on the provider's list, so reaching the screen never silently changes the operator's choice.
- While nobody has chosen a Hide AI agent, each agent switched on in the kit is asked whether it is signed in every five minutes (see [The operator's choice](#the-operators-choice)), and one that answers `needs_login` is asked again on the same timer, though nothing on that timer says its login changed.
  For an agent whose check starts its CLI (`claude auth status`, Codex's app-server, `grok models`, `pi auth check` or `pi --list-models`, and `agy models`, which starts agy's runtime for about 5.5 s) that is one process every five minutes for as long as it stays signed out.
  It has not been seen on a machine where an agent is chosen, since the timer asks nothing then.
  It closes when a `needs_login` answer is asked again only on a sign of a new login (Settings opened, or that agent's own login files changing), which needs each agent's login files measured, and Grok and Pi have not been run on the machine that would measure them.
- Grok and Pi have not been run live by this repository's checks; see the proof level under [The text-mode CLIs](#the-text-mode-clis).
- OpenCode cannot be selected until a read-only one-shot mode is documented for it.
- Antigravity's read-only rule rests on Hide's own folder and its trust in the operator's agy settings, not on a flag of agy's: if the operator takes that trust out, the provider dims until the kit's next pass puts it back, and a future agy that loads a folder's hooks differently would show as tool steps refused by the second check rather than as a silent tool run.
  Each request also leaves a conversation in agy's history under that folder; this closes when agy offers a print mode without tools or without history.
- A Claude usage limit has not been observed against the live account.
  The mapping was measured end to end instead, by answering the CLI's own API request with each HTTP status and reading the frame it printed; the run that measured it is local evidence, not a tracked file.
- Hide AI's counting is partial by design.
  An `InvalidOutput` or `Cancelled` attempt that did spend tokens (for example a result with no structured output or `error_max_structured_output_retries`, a Codex turn that completed with no JSON message) is counted as an unknown call, not with its tokens, because the error path carries no usage; an error frame's usage is not read.
  A `PlainJson` answer that holds no JSON (`claude_result_not_json`) is the same case, and the router's two attempts for it are two full-price processes where `Schema` mode retried inside one.
  Pi and Grok report no usage the code reads, so their calls are only counts of unknown calls.
  Unreleased experimental rows written before the separate `attempts` field keep their original per-model call count when read or extended; the original attempt boundaries cannot be reconstructed, so their historical count can exceed actual attempts.
  New attempts add exactly one known call regardless of model count; no migration guesses a correction to older experiment data, and a downgrade to that experimental writer can discard this new count when it rewrites a row.
  Calls before this feature are not backfilled.
- Weekly usage in the toolbar reads Codex's usage endpoint, with Codex session JSONL as an offline fallback only, and Claude Code's `/usage` text; the router's daily rollup is a log line, not a popover value.
