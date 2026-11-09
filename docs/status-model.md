# Status Model

How the engine's raw agent state becomes a group in the sidebar and a count on a checkout row.

The state of an agent is five axes, not one word.
What it needs from the operator, whether it is running, whether it has reported a completion, whether the operator has looked at it, and whose work it is are independent, and mixing them into one string is what made the same agent read differently in different views.

- Demand: question, approval, error (shown as Blocked, see below), none.
- Activity: working, stopped, unknown.
- Completion: reported, not reported.
- Read: read, unread.
- Ownership: operator, delegated, or Hide AI's, for work a Hide-started agent does for a screen that owns it.

`hide-core/src/agent_state/axes.rs` is the single owner of all five.
`agent_state/turn.rs` derives the group, request verb, close and rest gates from them; `sidebar.rs` assembles the snapshot rows.
It also derives everything a view draws from them - the group, the mark, whether the row is emphasized, the status word, the descendant mark, and whether closing the pane needs a confirmation or a fresh status check - so no surface decides any of it a second time.

Ownership is not stored anywhere.
It is read back off the row: a row whose lineage depth is greater than zero is delegated, a root whose Hide AI snapshot names a home is Hide AI's, and every other row, including an orphan whose parent is gone, is the operator's.
`ownership_of` is the only function that makes that judgement, and `apply_lineage` reapplies every derived value once the lineage is known.

### Work Hide AI does for a home

A Hide-started agent whose task kind has a home (a Memory card's promotion, `memory_promote`, and a 제안 line's application, `suggestion_apply`, whose home is the Hide AI tool's 제안 tab on the row the task's target names, `m72` for a card and `s12` for a line; PRD hide-ai-suggestions D-02, D-17) is not the operator's work, so it is a third ownership value, `HideAi`, not a flag on an operator row (PRD hide-ai-work-home D-13).
The coordination ledger's record of the spawn is the only source of the fact (`coordination::started`), and the task kind is a closed enum (`coordination::TaskKind`) whose exhaustive `home()` match makes a new kind fail to compile until it names its home.
A kind this build does not know, or a record with no task, loads with the rest of the ledger and reads as no home: that one agent stays an operator row with its mark and a `hide_ai.task_kind_unknown` diagnostic (D-19).
The row stays in the agent data, so ⌘K, tabs, the pane header's owl and `hide agent list` find it as before; what changes is which lists enumerate it.
`agent_state::listed_for_operator` is the one decision: a `HideAi` row is a member of the sidebar's tree roots, the device's tasks, Overview's requests and graph, the phone's list and the checkout counts only while it is in Needs You, and no tree draws it even then (the raised Needs You and the 태스크 list do).
Its group is Needs You or Seen and never Working or Done, so it moves no Working or Done badge, rail count or notification.
It is in Needs You when the agent itself asks (a question, approval or block, read like any row's) or when the home has a turn for the operator.
The home's turn is computed by the core from what the home's own tracker already follows (the Memory promotion module for `memory_promote`, `runtime/suggestion_apply.rs` for `suggestion_apply`), the attempt's pull request in the machine's PR catalog, and published on the row as `hide_ai.turn`:

| The attempt's pull request | Turn |
| --- | --- |
| No pull request; the agent committed on its branch and stopped, and the branch waits for the operator's decision (PRD hide-ai-branch-first-apply B3) | `branch_ready`, which stays after the row is read until the branch is merged, discarded or sent on |
| Open, not a draft, checks all passing or none, no change requested | `pr_review`, which stays after the row is read |
| Open, not a draft, checks failed, and the agent is not working | `ci_failed` |
| A draft, checks pending or unknown, a requested change, or failed checks while the agent works | none |
| Merged, closed, or no pull request | none; the attempt leaves the table |

A row has one line; its second line is the agent's ask or its block's cause, then `ci_failed`, then `pr_review` or `branch_ready`.
Entering Needs You notifies once, as for an operator row, and a turn changing inside it does not notify again, because the notification state machine follows the group and not the word.
Only a live agent has a row: an attempt with no agent (a failure with no pull request, an open pull request after the operator closed the pane) shows in the 제안 tab alone (D-15).
The shell words the turn and opens the home from the row (the Hide AI tool's 제안 tab with the target's row open); the phone opens the agent's detail.
Regression owners: `a_hide_started_row_follows_its_attempts_pull_request_into_and_out_of_needs_you` and `a_home_owned_row_is_listed_only_while_the_operator_has_something_to_do` in `hide-core/src/runtime/tests/`, `the_home_turn_follows_the_pull_request_and_the_agent` in `memory_promotion.rs`, and `a_home_owned_row_notifies_once_on_entering_needs_you_and_not_on_a_turn_change` in `hided/src/mobile/push.rs`.

### A held Memory card is no status

An agent reporting a Project Memory card stale (`hide memory stale`) changes no agent row, group, badge, notification or snapshot field, and the operator is told nothing (PRD memory-stale-cards D-11, B15).
The hold is the card's, not the agent's: it decides only what the hooks give, and `hide memory show` is where it is seen ([hide-private project-sessions-and-memory.md: A card an agent reports stale](ARCHITECTURE.md#project-sessions-and-memory)).

### A mission adds no status

A Conductor and its workers are ordinary agents: a Conductor's row, its workers' rows and their delegated tabs read and group as any agent's do, and a mission adds no attention state, group, badge or notification (PRD software-factory-conductor D-14).
The mission's own states (`planning`, `running`, `merge_wait`, `done`, `cancelled`) and each node's live only in the mission record and are drawn only on the map and the row's mission chip ([hide-private tasks-and-pr-list.md: A mission's map](UI_BEHAVIOR.md#tasks-the-보기-slot-and-the-pr-list)).
A node carries its worker's life as `hide agent show` gives it (`agent_runtime`, from `coordination::life`), so a node the Conductor still records as working whose worker Hide put to sleep is drawn asleep, never at work, on its card, the head, the pane chip and the summary's `지금`.
A decision the Conductor waits on reaches the operator through the Conductor's own row: the core lays it there as a question, as it lays a plan waiting for review (below), with `ask_subject` `{cause: "mission_decision", line}` carrying the decision's one line (needs-you-cleanup D-12).
The row then stands in Needs You, its line reads the decision's `<line>` with no verb before it (`mission.askDecision` names it in notifications and hints) and its row and mission chips keep the mission's progress with a `!` in the decision colour; a decision answered from the map or in the Conductor's pane, cleared by the Conductor, or ended with the mission is lifted from the row at once, and a new one replaces the line.
A wake the core sends the Conductor for a pull request change is a letter from `hide:conductor`, delivered and read like any other ([delivery.md: Missions](delivery.md#missions)).
A node of the mission shows only three states on the map, waiting, in progress and done, and what needs someone to look at is its one attention, never a status of its own; a node in progress also says whether its worker works now, rests while the node waits on something (`waiting_on`, said beside it), or has no worker (`NodeView.work`, from its registration and its agent row's activity, none while Hide cannot see the row); Hide settles the certain cases itself and raises the rest ([hide-private missions.md: What Hide settles in a mission by itself](delivery.md#missions)).
A worker that looks stuck (its review failed twice in a row, or no push for 90 minutes while the next move is its own) is warned to the Conductor alone: the node's card shows ⚠ and the Conductor gets one letter, and no attention, row question or notification reaches the operator ([hide-private missions.md: What Hide settles in a mission by itself](delivery.md#missions)).
An attention that reached the operator (PRD mission-node-simple D-05, B14) is laid on a row as a question, as a mission decision is: `ask_subject` `{cause: "mission_attention", node, reason}`, the node's title and one of the attention reasons (`merged_unproven`, `worker_ended`, `pr_closed`, `unplanned_pr`, `conductor_failed`, `wake_refused`, `close_refused`).
It is laid on the Conductor's row while the Conductor holds the mission, else on the row of the node's worker while its registration stands, and only on a row that rests with no demand of its own (`plans.rs::mission_attention_asks`, laid by `lay_reviews_on`); while no row shown carries it, a working one included, the mission is its own Needs You item (`on_row: false`, read from the laid rows) and the notice goes out; it is lifted from the row when the attention goes, whoever or whatever settled it.
The row then stands in Needs You and its line reads `<node> · <why>` (`mission.attention.reason.*`; `mission.attention` names it in notifications), with the node's title as the plain sentence for a surface that does not know the cause; the bell and the phone word it in their own language from the same subject, and a Desk card reads it as a decision.
Several asks on one row are oldest first with `외 N건` for the rest, as for a plan review and a mission decision.
With neither row standing, the mission itself is one Needs You item above the agent rows (its title and `<node> · <why>`, `외 N건` for the rest, with `확인하기`, led by `지휘 다시 열기` while no Conductor holds the mission), which opens its map and is counted in the device's Needs You number with the agents, and the operator is also told once by a notice; a toast or a push alone never stands in for it (PRD mission-node-simple D-19, [delivery.md](delivery.md#missions)).
It is a question the core lays, not one of the eight escalation causes of Delegated escalation, and adds no status, mark, colour or group.

### A handed plan step adds no status

A child a plan step was handed to (`hide agent spawn --parent here --plan-node <key>`, PRD hide-plan-delegate) is an ordinary delegated child: its row reads Working or Seen like any delegated row, its report or block reaches its parent as a letter and turns the parent unread as a descendant's change does, and its sleep follows the same rule.
Where the step stands, 보고 옴, 막힘 or 닫힘, lives only on the plan's step (`delegation.state`) and is drawn only on the plan's chip and map ([hide-private plans.md: Handing a step to a child](delivery.md#plans)); it adds no attention state, group, mark or notification.

### A plan waiting for review is a question

A plan an agent put up with `--ask` (PRD hide-plan D-12, D-30), and a mission decision on its Conductor's row (above), adds no status, mark, colour or group: the core lays it on the agent's row as the question it is, a `user_turn` of kind question with no text and the row's `ask_subject` `{cause: "plan_review", steps}`, before the read axis and the lineage run, so the row waits on the operator and the ask's own path carries it once.
The core sends what is asked, never the words: the row's line, its `ask` and the pane band carry the same `subject`, a bell or phone notice carries it beside `detail`, and the window, the bell and the phone word it in their own interface language (`plan.askReview`, `작업 단계 검토 · N단계` in Korean, `Task steps review · N steps` in English); the phone's service worker gets those words from the page with `{{count}}` left in.
The agent's own question, any other demand and a working agent come first: the plan's question is laid only on a resting row with none of them, and it is gone once the agent runs `hide task approve`, `set` without `--ask` over a finished plan, or `drop` ([delivery.md: Plans](delivery.md#plans)).
The same rule holds for a device's rows.
A mission decision, and a mission node's attention (above), follow the same path (`plans.rs::lay_reviews_on`), laid again whenever the plan book or the mission book moves; a decision's `detail` is its line, cut to a row's width, and an attention's is the node's title, so a phone page or service worker that does not know the cause draws the plain sentence.
A row that has both a plan waiting for review and a mission decision carries the one asked first and counts the other behind it, as the root's other asks are counted (`외 N건`, needs-you-cleanup D-13).
Input an agent has not confirmed follows the same path, laid on the pane rather than a session, since a child that never took its brief may have no session yet: `{cause: "input_unconfirmed", input: "brief" | "letter", spent, held}`, worded `브리프 확인 안 됨` or `편지 확인 안 됨`, `브리프 못 받음` or `편지 못 받음` only when a composer read saw the text still there (`held`), and `· 재시도 다 씀` once its typings are spent (`agents.input.*`); the core lays it only once no parent can clear the screen ([delivery.md: Input an agent did not take](delivery.md#input-an-agent-did-not-take)), and a delegated child's ask rises to its root as the seventh escalation cause.

### A question the core lays stays read

A question the core lays on a row (a plan waiting for review, a mission decision, a mission node's attention, input not taken) is known by the event that raised it, never by its words or when it was laid: the plan's instance and when it was put up (`Plan::asked_at_unix_ms`), the mission's question key, the node's raise key, the input's spawn or letter (`runtime/plans.rs::ask_id`).
The row carries the ids laid on it now (`laid_asks`), and reading the row keeps them in its read record (`PaneReadRecord::read_asks`, absent in a record written before, which is judged as it was).
While every question laid on the row is one the record read, the agent's own work and rest under it (its sequence, activity, completion and demand) are not news, so a Conductor that a letter wakes and that rests again keeps the raise the operator read read, and nothing is announced again (operator decision 2026-11-09).
A question not among them, another laid beside it or in its place, is news; so are a descendant's signal, the end of its checks and a person's turn, which are judged as for any row.
Reading removes the emphasis only: the demand, the read ask's place in 내 차례 and the Needs You count stay, and the mission's own Needs You item while the Conductor works is unchanged.
A question the record read and the row no longer carries beside the ones it does is trimmed, so the set holds only what one row carries.
A question laid or lifted reads every machine's rows again, a device's included, and the device's kept session is relaid with its published one.
Known limitation: an answer the agent gives the operator's own typed request while a read question stands does not light the row again; the operator is at that pane.
Regression owners: `a_raise_the_operator_read_stays_read_while_its_conductor_works_and_rests` in `runtime/tests/missions.rs`, `a_device_rows_read_plan_review_stays_read_through_its_agents_turns` in `runtime/tests/read_on_look.rs` and `a_read_core_question_lets_go_of_the_agents_own_turn_and_nothing_else` in `sidebar.rs`.

### A person's turn an agent declares

An agent at a step only a person can take, a sign-in, an unlock, an app or system permission, says so with `hide agent ask-human auth|permission <where>` (PRD ask-human D-02, D-03); it adds no status, mark colour or group, and Hide AI's own reading of a blocked turn stays for an agent that does not say it.
The declaration is the core's, one per registration in the delivery ledger (`Ledger::human_turns`, `coordination/human_turn.rs`), so it outlives a sleep and a restart, and nothing Hide AI reads takes it down (D-07, D-16); the place is kept as the agent wrote it on one line, a URL's query and fragment included, since they can name what the operator needs, such as a device code or a request number (D-04).
The place lives only while the declaration stands: it leaves the ledger with the declaration, and the Done letter and the diagnostic log never carry it, so a one-time code it holds is on the screen, a notification and the phone only until the step is done (D-04, B4).
It goes only when the operator presses Done, when the operator sends that agent a message of their own from its pane or the phone's conversation, when the agent runs `--clear`, or when its registration ends (D-08); a message typed and not sent leaves it.
The core lays it on the declaring row in a field of its own, `declared_human_turn` `{id, kind, place, device, since_unix_ms}` (`runtime/human_turn.rs`), on a working row and on one with its own question or block too; the row's own question, `ask_subject` and demand stay as they are, so a question the operator answers meanwhile does not come back when the turn goes, and every reading of the row looks at the field first: the row's ask leads with it (`state.ask.subject` `{cause: "human_turn", id, kind, place, device}`) and counts the rest behind it (`외 N건`, D-21), and the row's `human_turn` names the declaration a row leads with, its own or a raised one, for the key and Done.
A ledger the core could not read leaves the turns laid as they were and settles nothing, which the diagnostic log says (`ask_human.ledger_unreadable`).
A delegated child's declaration is the eighth escalation cause, `human_turn`, checked before every other: its lineage root stands in Needs You with the child's ask leading its asks, whatever the parents between are doing, and those parents take only the descendant signal (`human:<id>`), which turns them unread (D-06, D-17).
Several declarations under one root show the one declared first, with the rest behind it; Done on the root settles only that one, and each also stands on its own row in the tree (D-20).
The mark is the warning badge with a key in place of the `!` (`StatusMark` `human`), on the declaring row and the root only, named `사람 차례`; its line is `<kind word> · <place>`, `인증` or `허용` (`agents.human.*`), the place plain text the operator can select and never a link (D-09, D-15), with a copy icon beside it wherever Done stands that puts the whole place on the clipboard ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)), and the device's chip when it runs on a connected device.
The pane head of the declaring agent, of its root and of every parent between them says it with Done, as an own ask leads a pane's band and a raised one the root's (`agent_state/header.rs`; a parent between reads the turns raised below it from `human_below`, which its row never draws).
Done is one event (`human_turn_done`, the declaration's id) from the two lists, the pane head, the Overview, the tree and the phone, disabled with the connection's reason while the declaring agent's device is not connected; it sends the agent one report letter, which wakes it when it sleeps and waits for no answer, then takes the declaration down, so a press from two surfaces sends one letter ([delivery.md: A person's turn](delivery.md#a-persons-turn)).
A new declaration is one notice on the desktop and the phone under the existing switches, worded `사람 차례 · <kind word> · <place> · <device>`, and a replacing one is a new notice; the notification ledger keeps the declarations it announced (`Seen` in `turn.rs::push`), so a declaration raised under a root already in Needs You is announced once too, and a restart announces nothing again (D-11).
A mission worker's declaration rises to its Conductor's row like any delegated child's, and on the map it is only its node's waiting reason, a key and `<kind word> · <place>`, never a mission attention or a 내 차례 item of the mission (D-18).
Regression owners: `hide-core/src/runtime/tests/human_turn.rs`, the escalation tests in `agent_state/escalation.rs`, `each_persons_turn_is_announced_once_with_what_and_where` in `hided/src/mobile/push.rs`, and `web/e2e/ask-human.spec.ts`.

## agent_status provenance and background waits

The raw `agent_status` is the core's own engine's, with Herdr's names and meaning: each pane's holder reads its screen with its agent's definition-file rules, and the engine turns a stop after work into `done` and its `completion_seq` ([ARCHITECTURE.md](ARCHITECTURE.md#agents-on-the-cores-own-engine)); the core derives its axes in `agent_state/axes.rs`.
The screen rules follow Herdr's: for Claude Code and Codex, Herdr's [official agent reference](https://herdr.dev/docs/agents/) describes terminal-screen inference as the lifecycle source, rather than lifecycle state reported by their session hooks.
A native session reference or display-only hook token does not prove that a tool is running or that its command completed.
Other integrations can have full lifecycle reporting, so this screen-inference statement is specific to those two agents, not every agent.

Hide's current mapping calls raw `done` a reported completion and `idle` a ready stopped pane, as specified below.
That is a presentation signal read from the screen, not a verified task outcome, successful test, exited background command or durable delivery receipt.
Use a command's own exit/result or an explicit coordination receipt when a workflow needs completion evidence, rather than treating a quiet pane or Done group as that evidence.

[Issue #348](https://github.com/with-hide/hide/issues/348) records one actual run in which an agent waiting for a background command was read as `done`.
This is a known screen-inference limitation: a prompt-like quiet screen can cease to look working while background work is still outstanding.
It is one observed case, not a guarantee that every background wait produces `done` or that Hide detects outstanding commands independently.
The corresponding behavior for Monitor and for Codex's background terminal has not been verified by that observation.
The existing [waiting-on-descendants rule](#a-quiet-row-that-waits) uses proven lineage and child axes; it is not a general background-command detector.

## Hide owns the read axis, at pane level

Herdr's seen was tab-scoped: focusing a tab, or targeting it with pane focus or agent focus, marked every pane in that tab seen.
Three finished agents side by side in one tab therefore cleared together on a single click, which is the bug this model exists to fix.
The engine keeps no seen state of its own, so the read axis is the only record of what the operator has looked at.

So Hide keeps its own record instead.
A pane is read when it has held Hide's keyboard focus since its last state change, and on the engine the record was read on, a state change is the engine's `state_change_seq` moving **or** the derived demand and activity pair changing.
The pair matters because the engine's sequence does not move when only the core's own label changes: an analysis that reads a question at the end of a turn arrives after the agent stopped, so the demand appears with the sequence where it was.
An agent a phone has open is looked at the same way: from the moment a phone opens its detail until every phone that has it open closes it or goes away, it is read as the desktop's focus pane is, with the same record and the same ancestor and descendant rules, on the phone and the desktop at once, this machine's agent and a device's alike.
hided tells the core the whole list of agents its phones have open each time it changes (`phone_viewing`, the phones' input in `hide-core/src/runtime/look.rs`, at most 16), so a turn that ends while a phone looks is read, and one that ends after it closed is news again.
A phone's look moves nothing else: not the desktop's focus or tab, not an attach, and it does not wake a sleeping agent.
Regression owners: `hide-core/src/runtime/tests/phone_viewing.rs` and `an_agent_a_phone_opens_is_looked_at_until_every_phone_lets_it_go` in `hided/src/mobile/phone.rs`.

What the operator looks at is one value (`OperatorLook` in `hide-core/src/runtime/agents.rs`), made of three inputs that only `hide-core/src/runtime/look.rs` writes: this machine's focus pane as the operator chose it, the device in front, whose own focus counts while it is connected and its window shows the pane's tab, and every agent a phone has open.
A device that is not connected keeps its last rows but shows none of its panes, so nothing on it is looked at, even brought forward, until it answers again.
A selection is whatever moves those inputs together, such as a device brought forward and then a tab chosen there: an event's dispatch, the pass that settles the front Workspace and the tab a device's window shows (`sync_workspace_view`), and each worker step that brings something forward and then chooses in it run as one (`Runtime::change_look`), and a lone change of one input is a selection by itself.
When the outermost selection completes, under the lock it already holds, its look is compared once with the look the rows were last read with, and a look that moved reads every row again, whatever moved it: a pane, tab or checkout chosen here, a device brought forward or left, the tab a device's window shows, a device's own keyboard, a phone.
That read is part of the change the selection announces, whether or not a window reads.
A device's rows read this way take the rest of its tree with them in the same change: each pane's status, each tab's agent, each strip entry's identity and each pane's delegated-work chips.
The session the device last sent is read with the same look, so a catalog refresh derived from it keeps the read, and that session arriving again changes nothing.
A value a selection only passes through is never read: while one is under way, a row read for another reason, such as a session that arrives, is read with the look the last completed selection left, so a new turn on the pane still looked at is read and a pane passed through is not.
Two selections are two reads, in the order they completed, whatever runs between them.
A session is news, not a selection: its rows are read with the look as it stands when it arrives, a device's own keyboard included, and a look it moved is compared as it completes.
The one look a window's pull moves is a device window that follows its device's own keyboard to another tab: that pass runs on an event and on a pull, and until then no window shows the new tab, so its finished row stays in 읽을 것.
An event or a session that leaves the look where it was reads nothing again and announces nothing more; terminal input and a document's keystrokes are no selection and skip the comparison, and terminal output never reaches the core.
A device's own keyboard that already rests on a pane sends nothing new, so a device brought forward, or a device window that came to show that pane's tab, once left its finished row in 읽을 것 until the device's next session (with-hide/hide-private#685).
Regression owner: `hide-core/src/runtime/tests/read_on_look.rs`.
The record is `pane_read_records` in the persisted UI state, keyed by pane id, so it survives a restart.
A record is dropped only when the authoritative pane layout stops reporting that pane, scoped to the namespace that pass owns, so a temporarily incomplete agent list and a local sync can never drop a restored or remote pane's record.
The same pass drops the descendant signals other records keep for that pane and the pane's saved completion.
A corrupt store loads as an empty record, which reads as everything unread, and says so in a diagnostic; it is never silently treated as read.

Nothing reads the engine's `done` versus `idle` split to decide the read axis.
This is enforced by `INV-herdr-unseen-token`.

### Across an engine restart

`state_change_seq` is the engine's own count for one daemon process: it starts again when the daemon starts, and the engine's save keeps a pane's tokens except the server mark (below).
A restored pane reports in steps: first at sequence 0, then `unknown` until its holder has read the agent's screen again, then settled at a sequence of its own, which may land below, at or above the one before.
So a sequence is compared only with one read on the same engine, and no sequence value is taken as a restart signal.
A device's engine goes with its link, so the same holds for every link to a device; the hook's turns ([agent-hooks.md](agent-hooks.md#the-turn-the-hook-reports)), a watch's activity ([delivery.md](delivery.md#watch-activity-and-completion)) and the label worker's reads, with the stop and the notice each owed for a state, follow this rule too: a worker that starts lays nothing it read for an earlier engine's state, and takes its first sequence without reading the difference as a state that moved (`PaneRecord::forget_engine_state`).

Each record names the engine it was read on (`PaneReadRecord::server`).
Every connection Hide makes to an engine, this machine's or a device's, starts with a fresh mark, and once a saved record or completion names that mark the lineage writer writes it on the pane as the `hide_server` token (`engine::tokens::SERVER_MARK_TOKEN`).
A pane that carries a mark is on the engine that mark names; a pane that carries none is on the engine Hide is connected to now.
The agents' sequences start again with the daemon, so the engine keeps every token of a pane it saved except this mark, and a restored pane is judged as a pane on another engine is (`own/topology.rs`, `Topology::from_saved`).

A record saved on another engine is carried over instead of compared.
Each record holds its verdict (`PaneReadRecord::unread`), saved whenever it flips, and until the pane settles on the new engine the row keeps that verdict.
The first time the pane settles on the connection, a read record takes the new engine's mark and sequence and keeps everything else the operator read, and from then on it is judged like any record read on that engine: the row is unread while it shows something the record does not cover (another known agent session, a demand it did not have, a change between working and stopped), and read again if that goes away at the same sequence.
Nothing is decided from that first settled state alone, because it can still be on its way.
On 2026-11-09 a started daemon laid label summaries before its first usage read told it Hide AI was paused, so three read panes whose labels ended on a question showed that question for a moment, and a verdict taken once from that moment kept them unread until the operator read them again, on every restart.
A completion the restored engine reports again is the one the record held, so a restored agent that reports `done` again stays read.
An unread record stays unread through the restore and is decided again only when the operator reads the pane.
Running the same pass twice changes nothing, and a second restart starts from the records the first one left.
A record saved while its pane was still being restored (the operator looked during those steps) holds no state to compare, so it takes what the pane settles on as read.
Records written before this rule name no engine, so Hide cannot tell which one they were read on; each is judged by sequence as before the first time its pane settles and then names that engine.
Reading such a record as read on a guess would lose news, so an upgrade whose first connection reaches a restored pane with such a record can show a read row unread once.

Regression owners: `a_read_pane_stays_read_through_a_restored_servers_steps`, `a_restored_pane_is_read_by_what_it_rests_on_not_its_first_settled_state`, `a_restored_pane_reporting_done_again_stays_read`, `a_pane_unread_before_a_restart_stays_unread`, `a_restored_pane_showing_something_new_is_unread`, `a_hide_restart_on_the_same_server_shows_work_done_while_away`, `a_childs_signal_through_a_restore_lights_its_parent_only_when_new` and `a_record_saved_before_servers_were_named_is_judged_by_sequence_once` in `sidebar.rs`, `a_device_pane_read_before_its_server_restarts_stays_read` for a device, `a_server_mark_is_written_once_and_rides_with_lineage_keys` for the writer, and `web/e2e/read-across-restart.spec.ts` against a real restart of hided.

## Completion is separate from stopped

An agent can be stopped because it has completed a turn or because a newly opened pane is ready for its first instruction.
A completion is `agent_status: done`, or `idle` with a `completion_seq`: the engine keeps `done` and its `completion_seq` until the agent works again, and the core also reads `idle` with a `completion_seq` as a completion, a shape the own engine does not produce.
An `idle` lifecycle with no `completion_seq` reports a ready stopped pane and does not put it in Done, even though a missing read record still makes its read axis unread.
The engine has no seen state, so an unread finished pane stays Done until the operator reads it in Hide.
The completion fact is part of the pane read fingerprint, so a ready-to-completed transition becomes unread even when the engine's process-local sequence does not move.
The engine's done/idle distinction supplies completion evidence only; Hide's own pane record remains the sole read authority.

A completion is news only when it appears.
A record that holds a completion the row no longer shows still covers the row: a completion goes away only when the agent works again, which its own change already says, or when a restarted engine no longer reports it.
A demand is the same: one the row no longer shows is a question answered, whose work moves the sequence, or a label summary withheld while Hide AI is paused or summaries are off, and neither is anything to look at; a demand that appears, or turns into another, is news.
A session the row cannot name yet is no other session: another session is news only when the record and the row both name one.
Any other change to the fingerprint (the agent working again, a state change between two looks, another session in the pane) still turns the row unread.
Regression owners: `a_read_demand_stays_read_when_the_engine_marks_its_tab_seen_after_focus_left` and `a_read_demand_turns_unread_again_only_for_news` in `sidebar.rs`, and `web/e2e/blocked-state.spec.ts`, which finishes a blocked and an unfinished turn after the operator moved on.

A restarted engine reports neither `done` nor a `completion_seq` for a pane whose completion it lost; a restored Claude pane comes back `idle`.
So Hide keeps the completions it saw (`pane_completions` in the persisted UI state, keyed by pane), each with the agent session, engine mark and sequence it was last seen at.
A saved completion keeps showing while the same session has not changed since: on its own engine while the pane's sequence is the one it was saved at, and on a restarted engine when the pane first settles stopped.
It is kept before the lineage is built, so an ancestor sees the same descendant completion it saw before the restart.

## A descendant's change turns its ancestors unread

The fingerprint carries one more element: the outstanding demands and reported completions of the row's live descendants, each keyed by the descendant's pane (`PaneReadRecord::descendant_signals`).
A descendant entering a question, approval or error, moving between those three, or finishing a turn adds a signal the record does not hold, and every ancestor row becomes unread through the same comparison its own state change would use; there is no second store, timer or event.
A signal that goes away does not turn anything on: a question being answered, a finished child starting new work, or a child pane closing is trimmed from the record on the next projection, so the same descendant is news again the next time it asks or finishes.
A signal is trimmed only when the descendant is settled and no longer sends it: while a restored engine has not settled the descendant, or has not got its lineage tokens back, the record keeps it.
Reading the ancestor records the signals it shows at that moment and nothing more.
A descendant on a device counts the same as one on this machine: every pass that decides this machine's read axis, an engine update and an operator's focus move alike, reads the one lineage across machines (`Runtime::lineage_across_machines`), and trims by what the devices' agents send as well as this machine's.
Two passes that each built the tree their own way once read a parent with a device child as unread and read again in turn, so the row flashed into 읽을 것 on every focus move.
An agent on a device cannot start one on another machine, so a device row's own read axis needs no other machine's rows.

A descendant's signal never puts an ancestor in Needs You: a child's question leaves a working root in Working.
That is the boundary: a child is asked for its status through its parent, and a parent that has heard from a child is drawn bright until the operator looks at it.
The one way a descendant moves its root between groups is the waiting state below, and it moves the root into Working, never into an attention group.

Regression owners: `a_descendants_demand_or_completion_turns_every_ancestor_unread_and_nothing_else_does`, `an_ancestors_group_comes_from_its_own_axes_and_a_child_never_makes_it_needs_you`, `a_parent_read_with_its_device_child_stays_read_on_every_pass_until_the_child_finishes_again`.

## A View to look at

An agent that opens a View with `hide file open`, `hide diff open` or `hide browser open` opens it to show the operator, so the View is unseen until the operator looks at it (PRD unseen-agent-views D-05, D-06).
A View the operator opened, from the Explorer, ⌘P, a new tab's address field or a path or address clicked in a terminal, is never unseen, and an agent's `hide browser snapshot`, `click` or other command on a page leaves the View as it was.
A new target a page opens on its own (a link to a new tab) is the page's, not a new open.
The record is the View's own (`Display::unseen` in `view_layout.rs`, `Unseen` in `view_feedback.rs`): when the open happened, as a generation that moves on each open, and whether an ancestor has read it; it is stored with the checkout's View layout, so it survives a restart, and it goes with the View, so there are never more records than open Views (D-09, D-12).
A stored record that does not read leaves the View unseen and the load's repair says so in the diagnostic log; a View stored before the record existed reads as seen (D-27, B18, B33).

A View is seen when, in one window, all of these hold (D-07): the window holds the keyboard on the Workspace screen (the shell's `window_focus`, which hided aggregates across windows and lets go with a connection), the View's checkout is in front, the lane shows the documents and not Sessions or the 보기 slot, the View is the selected tab of the active View area, and the Files strip of the session in front shows it.
The core decides it on every snapshot (`settle_unseen_views` in `runtime/unseen_views.rs`), from state it already holds; the shell sends only the window's focus, and only when it changes.
An open of a View that is already looked at in the same event is never unseen, so nothing flickers (B5, B14), and an open again of a seen View is news again (D-11).

The View's last opener's row carries it (D-19): an operator-owned row with an unseen View is in Needs You, whatever its own axes say, and draws the `look` mark (The mark).
Seeing the View does not read the pane, and reading the pane does not see the View (D-20); when the last unseen View is seen or closed the row goes back to the group its own axes give it (B4, B13).
The row carries `look` on the wire: the newest unseen View's id and name, how many there are, the checkout it sits in and the group the row would be in with nothing to look at.
Its line is `볼 것 · <newest View>`, ending with ` · 외 N` for the rest, a tail a narrow row never cuts (`lineParts` in `web/src/agentRow.ts`); a question, approval, block or native question or plan on the same row takes the mark and the line, which then ends with `볼 것 N` (D-14, D-18).
Pressing the row focuses its pane and brings the newest unseen View forward in the same event (`focus_pane` with `reveal_unseen`), which sees it; a device's row focuses its pane on the device and the View through the checkout's focus.
A pane's `N views` list marks each View the operator has not looked at with the same mark (B10).
A View to look at is counted with the operator's turns on every badge and in the rail, and notifies as the row's other Needs You entries do, under the same desktop switch and the phone's own push settings (operator decision 2026-11-08, replacing D-13 and B12).
The notification ledger (`agent_state/turn.rs::push`) gives such a row its own state, `Look`, read from `look.group`, the row's group without the View: a row whose own group is Needs You (a question, approval, block or raised ask) keeps that state and its own notice, so the notice follows the row's mark.
Entering `Look` is one notice with the status `look` and the newest View's name as its sentence (`볼 것 · report.html · <project>`); a second open while the row is still unseen, the row's own turn moving under it, and an answered question that leaves the View to look at send nothing, and a View seen or closed takes the notice back.
A turn that finishes with a View to look at is that one notice and no Done beside it, and once the View is seen the row goes to Done with nothing new announced; an open again of a View already seen enters `Look` again, so it is news again.
The existing rules hold: the first projection only seeds, an agent back within ten minutes keeps its state, and the desktop host shows nothing while a Hide window has the keyboard.
A click on the desktop notification opens the row as pressing it does, its pane and the newest View; a tap on the phone's opens the agent's detail on 볼 것.

A delegated child's unseen View never puts a row in Needs You (D-10): the child stays in Working or Seen, and the View is a descendant signal (`view:<id>@<generation>`) that turns every ancestor unread, as A descendant's change turns its ancestors unread above.
An ancestor that reads the View through its own pane, with `hide view status <view-id>` or a `hide browser snapshot`, `screenshot`, `eval`, `console` or `network` on it, turns that signal off for every ancestor, and the View stays unseen in the child's `N views` (D-22, D-23); the child itself, an unrelated agent, the operator's shell and `hide view list` do not.
An ancestor that opens the View again from its own pane becomes its opener, so its own row has it to look at (D-21, B24).
`hide view list` says which Views the operator has not looked at (`unseen`), and a report letter lists the Views its sender opened and whether each was seen.

Regression owners: `hide-core/src/runtime/tests/unseen_views.rs`, `a_seen_record_that_does_not_read_is_unseen_and_a_missing_one_is_seen` in `view_layout.rs`, the look line in `web/src/agentRow.test.ts`, and `a_view_to_look_at_is_one_notice_per_news_and_yields_to_the_rows_own_ask` in `hided/src/mobile/push.rs`.

## A quiet row that waits

A row with no demand of its own, not blocked and stopped (idle or done) waits when something in progress will wake it: work that is still running, or an answer someone is still making for it.
It has not finished, so it sits in Working, draws the in-progress ring, has `status_code: waiting`, and is not emphasized.
In every list, the project tree included, its line leads with what it waits for, `자식 기다림`, `백그라운드 기다림`, `CI 기다림` or `답장 기다림` (`agents.wait.*`), and goes on with the label's sentence or, with none, the cause's detail; a row with no proven cause leads with `대기 · ` (Waiting), so a ring whose line is missing or does not start with one of these is an agent at work (status-marks D-18, B29; needs-you-cleanup D-05, D-14).
Why it waits is one additive wire value, `wait`, beside `group: working`; absent means the row waits for nothing.
No new group value reaches the wire, so a decoder that does not know the field draws an ordinary Working row.
`apply_lineage` decides it on the same pass that sums `descendant_counts`, by this precedence (`axes.rs::own_wait` and the children check), except whether a root waits on its children: that also turns on whether the operator read the root's own turn, which the read pass after the lineage decides, so the lineage keeps its two inputs and `axes.rs::settle_wait` decides it first thing each time `derive_from_axes` derives the row:

| `wait` | The row waits for | Proven by |
| --- | --- | --- |
| `children` | A live descendant that is working, or stopped on a background task its session proves, unless the root's own finished turn is not read yet (below). Only a lineage root waits this way. | The lineage, the descendants' own axes and the root's own read |
| `background` | A background command, monitor or task its own agent started and that is still running in the process that started it, on a turn the label did not read as finished | The session's own records, read for the engine state the label worker asked under (below) |
| `checks` | The running checks of a pull request the row is responsible for, or the GitHub read a turn that just printed a pull request's address set off (A row waiting on its checks, below) | GitHub's answer for the pull request, read by `gh`, and the turn's own sighting |
| `reply` | A Hide `request` or `block` letter the row's session sent whose answer is still awaited and still being made: the recipient has not taken the letter in yet, or it is working, or it or a descendant of it is on a proven background task, or a descendant of it works | The delivery ledger (`Ledger::awaited_answers`) for the letter and whether it was taken in, and the recipient's row for the rest |

A turn the label read as finished (`done`) is not hidden by the background tasks it left (status-marks D-16, B27): the row is Unread with the green dot, carries the count as `background_left`, and its line ends with `· 백그라운드 N` (`agents.line.backgroundLeft`), its hover and accessible name with `백그라운드 N개 남음`; the narrow row cuts the sentence and keeps that tail.
The Done notification goes out as for any finished turn, and opening the row reads it; a task that wakes the agent starts a new turn, which decides the mark again.
A root's turn that completed and that the label read as finished is not hidden by the children still working under it either, while the operator has not read that turn (B28, changed by the operator on 2026-11-09): the row is Unread with the green dot, carries how many work as `children_left` (the same count as `자식 N개 작업 중`), and its line ends with `· 자식 N개 작업 중` (`agents.line.children`), its hover and accessible name with the same words; the narrow row keeps that tail as it keeps the background one.
The Done notification goes out as for any finished turn.
Once the operator reads it, the root waits on its children again (`wait: children`, 작업 중 · 자식 기다림) until they end, and the Done notification is taken back; the label judging the same turn again as work left also puts the row back to waiting, but is no read and keeps the notification.
Unread here is the root's own turn: `own_unread` compares only the turn's own fields of the read record, its state, session, demand and completion (`axes.rs::record_covers_turn`), so a child finishing, a check ending or a person's turn turns a read root unread and leaves it waiting, never bringing the old result back; a record that cannot be compared (saved on another engine run, or a stale list) never lifts the wait.
Shown Done, the root still has work in progress (`has_work_in_progress` reads the lineage's `children_moving`), so Sleep, Reopen and Resolve stay refused while its children work.
A `waiting` or `unfinished` verdict, or none (Hide AI off included), keeps the row in progress on its background tasks and its children, and a reply being made or running checks keep it in progress whatever the verdict (B28).
A merely ready or finished descendant is quiet, and one whose activity the engine reports as unknown does not make its root wait, because a waiting state the projection cannot vouch for is not drawn.
A descendant holding a question, approval or error waits on its parent, not the other way round, so it does not make the root wait: it turns the root unread through the descendant signals above, and moves the root to Needs You only once one of the eight escalation causes raises it (Delegated escalation).
A delegated row that waits on its own (`background` or `reply`) is Working, like a delegated row that is running, and its ancestors' counts take it as working; its root waits on it only while it waits on a background task, because a reply is made by the agent that answers, not by the row that asked; an ordinary delegated row that waits for nothing keeps Seen.

A reply wait ends when nothing is making the answer, even while the letter is still open in the ledger.
Once the recipient took the letter in and stopped, having answered with a report letter or finished without answering, the sender shows its own state: a dot to read until it is read (green only when its label read the turn as finished), then quiet.
The recipient's own reply wait is not counted as making the answer, so two agents that wait on each other's answer never both draw a wait: a child that asked its parent by letter waits while the parent has not taken the letter in or works, and the parent never waits on that child.
The 24-hour answer deadline stays the outer bound, and a turn the sender's session began after the letter went out still ends that letter's wait.
A recipient the projection cannot vouch for holds no wait, taken in or not: a code-owned recipient, which has no row whose progress Hide can see, an agent on a device that is not connected, whose last state may no longer hold, an agent asleep, whose letters wait for its wake, and a pane that now runs another session than the one the letter went to.
The runtime lays each awaited letter on its sender's row (`lay_awaited_replies`, leaving those recipients out), and the lineage pass decides from the recipients' rows, so a recipient's step reaches the sender on the same projection; publishing the ledger rebuilds the lineage when a letter's sender, recipient or intake changes and not for its other delivery steps.

Mark precedence on a row is the operator's turn, then progress, then a stop to read, then quiet (`RowMark::of`, The mark below).
The `wait` value is only ever set on a row with no demand of its own that is not working, so the precedence is the order of the checks in `agent_group_for` and `RowMark::of`, not a second rule.
Only a lineage root waits on its children: an ordinary delegated middle row keeps its own mark and Working or Seen group, and the grandchild is drawn under it in the tree.
An active escalation uses the six-cause exception below.

### A row waiting on its checks

A quiet row also waits while a pull request it is responsible for has checks running (status-marks D-07): the row is in progress, not a stop to read, until the checks end.
`runtime/checks_wait.rs::lay_checks` lays the input before the lineage, from the pull requests `agent_state/work.rs::linked_pull_requests` links to the row and only the ones `assign_duty` gives it, so one pull request holds one row, the one nearest its root.
A pull request counts while it is open and its checks rollup is pending with at least one check actually running (`GhCheck::is_running`): a required check that has never reported (`EXPECTED`) may never run, so a pull request whose only pending checks are expected holds nothing.
A project whose GitHub status is stale, unavailable or unreadable holds nothing either, because its last answer may be long over.
Each exclusion writes one `checks_wait.excluded` diagnostic per pane, pull request and reason (`stale` or `expected_only`), so the log says why a row did not wait.
A device's rows never wait on checks: `lay_checks` places only this machine's panes, so a pull request a node reads for its own project reaches that node's rows through the link record and its PR card (`links.rs::link_projects`, `open_links`), never as a wait, and a device this core does not read links none.
The wait is `wait: checks` with `checks_wait`, the pull request's number, on the row (`own_wait` takes it after a background task and before a reply), and its line, with no sentence, says `CI 기다림 · PR #N`.

A turn that ends having just printed a pull request's address is not a stop until GitHub has answered for it (D-09): the sighting sets off a GitHub read (`projects.rs`, `Sighted.owed`), and until that read lands or fails the row waits on that pull request the same way, so a turn that opened a pull request with checks about to start is never announced finished first.
The hold has no timer of its own: the read landing, a failed read and the agent working again each end it.

While any row waits on running checks, its project is read again once a minute whatever the window shows (`request_view.rs::reread_pending_checks`), and GitHub's answer re-lays the checks and rebuilds the lineage and the read axis (`refresh_checks_waits`).
When the checks a row waited on end, in the same agent state, the row records how (`checks_ended`: the number and whether they passed), and the end is news (D-08): the read fingerprint carries it (`PaneReadRecord::checks_ended`), so the row turns unread, lands in Unread with a plain dot, and its line says `CI 통과 · PR #N` or, in the warning colour, `CI 실패 · PR #N`.
An end the operator has read is not news again, and the end is forgotten when the agent's state moves.

Regression owners: `a_checks_wait_is_in_progress_and_its_end_is_news` in `agent_state/turn.rs` and the runtime checks-wait tests.

**Background work is proven only by the session's own records, and only Claude Code's can do it.**
`hide-session/src/turns/wake.rs` reads, from each Claude Code record: the start text a tool returned, only when the tool's message opens with it (`Command running in background with ID: X`, `Command did not complete within its Ns timeout and was moved to the background (ID: X)`, `Monitor started (task X, expires in Nm ...)`, `Async agent launched successfully ... agentId: X`); the end (a `<task-notification>` naming the task with status `completed`, `failed`, `killed` or `stopped`, in the `queue-operation` enqueue record and in the `queued_command` attachment, or a `TaskStop` call whose result succeeded); and the process boundary (the `SessionStart:startup` or `SessionStart:resume` hook record; `clear` and `compact` keep the process).
`TurnTracker` folds them under their own offset, so a record is never folded twice, and keeps at most 32 devices (`WAKE_DEVICE_LIMIT`).
A device never outlives the process that started it, so a new process start clears them, and a session whose file shows no process start proves none (fail closed).
A monitor that announced its expiry stops counting at it.
A `TaskStop` call stops nothing by itself: the tracker keeps the call by its `tool_use_id` and ends the device only when the result of that call arrives without `is_error`.
The tracker keeps, in the same table, the calls to the tools that can start work (`Bash`, `Monitor`, `Agent`, and `Task`, the name an older Claude Code gave `Agent`) until their result arrives, at most 32 open calls and devices together; the table is what tells a result too large to read apart from a screenshot (below).
A refused stop (`Task … is not running`, `No task found`), a call with no result yet and a result of another process leave the device running until its notification or a successful stop.
A device is lost for one of two causes, and the first one since the process started is kept (`WakeLoss`): `capacity` when more than 32 devices, or calls waiting for their result, were open at once, and `lost` when a record that may have started, ended or stopped a device could not be followed.
Either proves none until the next process start, and neither fails the read the turn and label depend on.
The record the reader could not follow is one with more marks than the bound or an id too long to keep, or one over the 256 KiB line cap that is discarded without its text.
A discarded record is told by its structure and never by its words, because a text that is matched is wrong the day it is spelled differently: the bounded scan of the discarded record (`conversation_cursor.rs::LargeRecord`) keeps the record's `type`, the `name` and `id` of its `tool_use` blocks and the `tool_use_id` of its `tool_result` blocks, and `wake::Record` names the four kinds `wake.rs` takes marks from, so the large-record reader must decide each of them.
A kind that carries no mark changes nothing.
An `attachment` or `queue-operation` (a notification, a hook) is `lost`, and so is any record whose scan cannot certify its structure (a truncated line, an id that is not plain, no `type`).
An assistant record leaves the `Call` mark of each call it makes to a tool that can start work, so its result is still paired, and is `lost` when it holds a `TaskStop` (its task is in a field the scan does not keep) or such a call with no id it can keep.
A user record answers each call its tool results name without saying how (`Answered` with `Unreadable`), and the tracker reads that by the call: the answer to an open call to one of those tools, whose output may have been a task's start, is `lost`, the answer to a stop whose result was discarded is `lost` rather than taken as stopped, and the answer to any other call (a screenshot a file read returned, nearly every record that large in real sessions) changes nothing.
A result with no id the scan can keep is `lost`, because nothing says which call it answers.
The worker writes one diagnostic when a session first loses its devices, `wake_devices.capacity` (with the limit) or `wake_devices.lost`, so the log says whether to raise the limit or look at the record.
The count is laid as `RowFacts.wake_devices` by `LabelOverlay::apply`, only for the `state_change_seq` the read was made under, so a row never waits on a task a stale read proved.
When a process start finds devices that were proven alive, the tracker keeps `wake_vanished` until a turn or a device begins; a stopped row with a label then draws a plain dot to read with the word Stopped and its second line says, in the operator's language, that the work it waited for is gone (`line.mode: vanished`, text empty in the core; the web draws it, and only the sidebar row draws it).
Hide or the daemon restarting is not a process boundary: the tracker is stored with the label record and continues.
The declaration is `SessionFormat::reports_wake_devices` in `hide-agent-adapter`, true for Claude Code alone.
Codex writes no record when a session's command ends, and Grok, Pi, omp, Cursor and OpenCode keep no process boundary, so for them nothing proves a task is running and a label `waiting` with no device is a stop (below).
A turn that just ended is not a stop until its session read lands.
The wake read is bound to the engine state it was asked under, so from the moment the engine reports the stop until that read lands nothing says whether a task will wake the agent.
The label worker owes that read (`PaneRecord::turn_end_owed`, not saved) and the overlay lays it as `RowFacts.turn_end_unread` for that state; the activity axis (`agent_activity`) keeps such a row Working, so no group, mark or phone notification is decided from a stop that may be taken back.
It is owed only to an agent whose read reports background tasks, when its status goes from working to idle or done (or changes between them while still owed), and only while the read can run: a daemon that restarts and finds its record read, and every other agent owe nothing.
The hold ends on every path of the read and has no timer of its own: the read landing releases it, a read that fails (`ReadFailure`, itself bounded by `DEVICE_READ_TIMEOUT`) releases it with a `turn_end.unread` diagnostic, and the agent working again or a state with no session to read ends it.
A session no read has proved yet carries no facts, so its first turn end is not held.
The wait before the label's own verdict (`stopped_unfinished`) is not this gap: that verdict comes from an AI request that takes seconds, and holding the row for it would delay every Done.

Regression owners: `a_turn_end_is_not_a_stop_until_its_session_read_lands`, `a_read_that_finds_no_task_releases_the_turn_end_and_publishes_it`, `a_read_that_fails_releases_the_turn_end_and_says_so`, `a_stop_that_is_not_a_turn_just_ended_is_not_held`, `a_turn_ending_with_a_background_task_is_never_announced_done` and `a_turn_read_as_done_is_announced_done_past_the_background_task_it_left`, then `a_quiet_row_waits_on_children_then_a_proven_device_then_a_reply`, `a_quiet_root_waits_in_working_while_a_descendant_works_and_not_on_one_that_asks`, `a_root_with_finished_children_draws_its_own_state_past_a_request_a_stopped_child_took_in`, `a_child_asking_its_stopped_parent_by_letter_leaves_neither_row_waiting`, `a_letter_to_a_disconnected_device_holds_no_reply_wait`, `a_proven_background_task_keeps_a_stopped_claude_row_waiting`, `a_reply_is_awaited_until_answered_overdue_or_from_another_session`, the adapter contract `adapters::wake_devices` in `hide-session/tests/it` (the stop pairing, the discarded record that may carry a device, the one that cannot and the call too large to keep), the two `wake_devices.*` diagnostics in `hide-core/src/labels/tests.rs`, and the web `agentRow.test.ts`.

## Where each axis comes from

Activity, completion and approval are judged in one order: the agent's hook first, then its session file, then the engine (PRD engine-bundle-1 D-9).

1. The turn the hook of the pane's own session last reported (`working`, `done`, `waiting` or `idle`; [agent-hooks.md](agent-hooks.md#the-turn-the-hook-reports)), which the core keeps per pane and session (`agent_hooks::facts`) and lays on the agent as `SessionAgentPayload.hook`, never on the wire.
   `working` is Working; `done` is stopped with a completion; `waiting` is stopped with an approval demand; `idle` is stopped with no completion, the turn an interrupt ended.
   It stands until the same session's next hook event, so the engine's reading of the screen never overrides it: a turn waiting on a background command that the screen rules read as finished (#348) stays Working.
   No timer ends it (design principle 13); a turn no hook reports the end of is ended by the signal the agent does give, or by the session file: a read under a later engine state than the one the turn was first laid under, with the engine reading the agent at rest, that finds the turn interrupted (Claude Code's Esc, which sends no `Stop`) or completed ends it as resting or done ([agent-hooks.md](agent-hooks.md#the-turn-the-hook-reports)).
   The facts go when the engine lists no agent in the pane or the pane closes; an agent started again in its pane is followed by its new session's hook even while the engine still names the session that ended ([agent-hooks.md](agent-hooks.md#which-session-is-the-panes)).
2. The session file, for the waits no hook reports: a plan waiting for approval (below), with the hook's `working` standing in for the engine's `working` when the session has a hook turn.
3. The engine's `agent_status` and `completion_seq`, only for a session whose hook has reported no turn: `working` is Working, `idle` and `done` are stopped, `done` or `idle` with a `completion_seq` reports a completion, and any other value reads as unknown rather than idle.

These ranks are decided once, in `agent_state::turn_projection` (activity, an approval prompt, a question or plan approval of the agent's own), and every consumer reads that answer: the row's axes, the letter doorbell and the tool-letter handover ([delivery.md](delivery.md#safe-intake-and-manual-fallback)), the confirmation of a brief or letter Hide sent, typed or through the agent's own way in, and the retype of a typed one ([delivery.md](delivery.md#input-an-agent-did-not-take)), and agent sleep through the row.
A row the turn shows at rest is not by that a pane safe to type into: the engine's blocked prompt and readiness, Hide's input clocks and the composer read stay the doorbell's own gate, which a typed input's composer read shares (`doorbell::unheld_turn`, `doorbell::takes_keys`).
Regression owners: `runtime::tests::turn_authority`, which crosses the hook, the engine and the session read and asserts both the row and the letter's hold, and `runtime::typed_input::tests::a_child_whose_hook_says_it_works_owes_no_record_while_the_engine_reads_it_idle` with its reverse, which assert the typed input's notice and retype.

An agent-list transport failure makes the retained telemetry unconfirmed while the topology stream remains connected.
Until a successful agent list or bootstrap snapshot confirms it again, the agent keeps its identity and task title but has Unknown activity, no completion or current demand, and requires a status check before closing.
An identical successful list restores the confirmed state; a topology event alone does not confirm the entire agent list.
A pane whose terminal failed to start or restore (the engine's `restore_error`, the pane's `failed_terminal_id`, as when its folder was removed before a restart) runs no process, and the engine reports its agent `unknown` for good.
Its agent is read as stopped, with no completion, demand, block or start, whatever its hook, session read or label last said (`axes.rs::reports_nothing`): the row is `idle` with the `quiet` mark, needs neither a close confirmation nor a status check, and folds under `끝난 자식` in a parent's close sheet and tree popover like any finished child, while the pane header's `restore_failed` notice says what happened.
`sidebar::project_agents` lays that fact on each agent from its pane (`SessionAgentPayload::terminal_failed`) before any axis is read, so the row mark, the popovers, the close sheet and every local and remote close guard read one answer; none of them looks at the pane's failed terminal again.
Regression owner: `runtime::tests::tree_close::a_child_whose_terminal_failed_to_restore_rests_and_never_blocks_its_parents_close`.
No pane token is read for any of these.
Regression owners: `sidebar::tests::a_hook_turn_decides_the_axes_ahead_of_the_engine_s_state`, `agent_hooks::facts::tests`, `runtime::tests::labels::a_hook_turn_ends_when_the_session_file_reads_an_interruption_after_it`, `runtime::tests::labels::a_hook_turn_whose_prompt_the_session_file_shows_unanswered_ends_at_rest`, `runtime::tests::labels::an_approval_ends_when_the_session_file_reads_the_refused_calls_result`, `runtime::tests::agent_connection::an_open_approval_outlives_the_main_turns_end_and_closes_with_its_call`, `runtime::tests::agent_connection::an_agent_started_again_in_its_pane_follows_its_hook_over_the_engine_s_earlier_session`.
Demand has these sources:

- A question is the core's label verdict on the agent's last message (`label.question`, see Task identity below).
  It exists only while the label is proven for the pane's current session, and it ends when the agent starts working again or a turn ran between two looks at a stopped agent, so a question never outlives the turn that asked it.
- A native unanswered Claude `AskUserQuestion`, Codex `request_user_input` or `request_user_input_async`, Grok `ask_user_question`, omp `ask` or OpenCode `question` is a question even with summaries off.
  A Grok session whose `plan_mode.json` is `Active` and awaiting plan approval is a plan approval, with `plan.md` as its bounded content, until that file says otherwise.
  The current-session/current-state read carries the optional `user_turn` row fact, with `kind: question|plan_approval` and optional `content: {text, choices, truncated}`.
  Text is capped at 8 KiB, choices at eight and each choice at 256 UTF-8 bytes; cuts preserve character boundaries and set `truncated`.
  A correlated native result, a later human turn or an abort clears the pending question.
  Codex 0.162.1 asks with `request_user_input_async`, whose tool returns `{"accepted":true}` at once while Codex keeps working, so that result answers nothing: the question stays open, as an unread Question in Needs You even while the engine says the agent works (the demand axis does not read the activity), until a reply names it.
  The reply is a user message `<send_user_message_question_reply>` carrying `questionItemId`, a JSON array `[tool, call, question number]`; each question of the call waits for its own reply, and the match is on that id, never on the question or answer text.
  That message is scaffolding like the other injected prefixes, so it is not a human turn and feeds no label as a request; a person's own message still ends the wait, and a turn that starts without one does not (the question outlives the turn that asked it).
  A reply whose id cannot be placed, or an async call whose arguments cannot be read, leaves the wait unknown rather than nothing.
  Missing content remains absent, and a failed, incomplete or older-state read publishes no structured fact; the independent letter hold stays unknown rather than assuming an answer.
  Native calls are bounded to eight per turn with 256-byte identities; exceeding the bound fails the read instead of dropping a pending question.
- Approval is the hook's `waiting` turn: Claude Code's and Codex's `PermissionRequest`, Claude Code's `permission_prompt` and `elicitation_dialog` notifications, and an OpenCode permission ([agent-hooks.md](agent-hooks.md#the-turn-the-hook-reports)), whether or not the operator has read it.
  For a session whose hook has reported no turn, it is the engine's `blocked` lifecycle; a hook that has reported the turn has said whether it waits, so the engine's `blocked` no longer decides that session.
- Approval is also a plan waiting for the operator's approval that the engine reads as an ordinary stop, Codex's "Implement this plan?" (PRD codex-plan-approval-hold).
  The core reads it from the session file, not the screen: the label worker's session read folds the agent's turn records into a turn tracker (`hide-session/src/turns.rs`), and a plan-mode turn that proposed a plan and finished, with no later turn or person's message, waits.
  The answer holds only for the engine's `state_change_seq` the read was asked under and the session it proved, it is laid on the row's facts (`RowFacts.awaiting_operator`, never on the wire) and resolved by `agent_state::agent_blocked`, and it is not shown while the engine says the agent works, except for an agent kind whose adapter declares `waits_while_working` (`SessionFormat`).
  The engine reads Grok `working` while it waits for a plan approval or an answer, and Grok's waits are read from the session as it is now (`plan_mode.json`, a question call with no result yet) rather than inferred from how the last turn ended, so for Grok the wait holds the row, and raises a delegated child as `ChildBlocked`, whatever the engine says; it clears on the next poll after the operator answers.
  A wait no read has settled for the current state is not shown: the row shows what the engine says, and the letter doorbell holds instead (`docs/delivery.md`, A menu the engine reads as a stop).
  It needs no Hide AI: agent summaries off, the read still runs.
  "No, stay in Plan mode" writes nothing to the session, so after that answer the row stays in Needs You until Codex's next turn starts.
  The row's `blocked` value carries it, so it is held exactly like the engine's `blocked` below, including close confirmation and agent sleep, and the Enter that approves it still counts as the operator's submit, since Codex writes the next turn's message from it.

- A block is the label's verdict that the turn stopped on a cause it can name in one line (`label.blocked`, see Task identity below): a disk with no room, a CI run that failed, a permission or login the agent cannot get, a tool that will not run.
  It is the Blocked state, carried on the existing `error` demand value (`demand`, `status_code` and the mark counts keep the wire name `error`; the word is Blocked), because a block is a demand on the operator's attention and no new value is needed.
  Like a question it exists only while the label is proven for the pane's current session, and it ends when the agent starts working again.
  The engine's own `blocked` lifecycle is the approval above, not this.

A question outranks an approval, and a block outranks both for the mark.
Whether the operator has read a demand is Hide's own record, never a tab-scoped seen state.

## The four groups

The sections are named for whose move is next: Needs You (`내 차례`) and Unread (`읽을 것`) are the two the operator acts on; Working and Seen keep their names (status-marks D-11).

| Group | Membership |
| --- | --- |
| Needs You | An unread demand - question, approval or block - or a pane the engine reports as blocked, or whose native question or plan waits for the operator, right now, or a question the core laid on the row (a plan review, a mission decision), or an operator-owned row with a View to look at |
| Done (Unread) | No demand, stopped, completion reported, and unread |
| Working | Running, or a quiet row waiting on a working descendant, a proven background task or a reply still being made |
| Seen | Everything else: ready idle, read demands, read completions, unknown |

A blocked pane, or one whose native question or plan waits for the operator, stays in Needs You whether or not it has been read, and so does a row with a View to look at until the operator looks at it (A View to look at).
The prompt is still waiting, so it leaves the group when answered rather than when looked at.

A delegated row sits only in Working or Seen, and a Hide AI row for a home only in Needs You or Seen.
Each of the four groups above is then a group of the rows the operator's lists enumerate; a home's row is counted in none of them until it is in Needs You (see Work Hide AI does for a home).

Done is deliberately separate from Needs You: a stop to read is "look when you have a moment", an unread demand is "act now".

A read, unanswered question or approval is Seen in this group, so the app icon badge, desktop and phone notifications, which follow the group, count only unread asks and Views to look at (needs-you-cleanup D-09, D-10).
The lists the operator reads to answer, the 태스크 list's 내 차례 in 상태별 and in each project's band, the project chip `내 차례 N` and the phone's 내 차례, still stand it in Needs You, drawn read (A device's tasks), so the count there can be larger than the badge on purpose.

An archived session (PRD session-archive) is in no group and draws no mark: nothing of it runs, so it has no turn, no progress and nothing to read, and it is counted on no badge.
A resolved row rests in every list, so its mark is counted on no checkout badge either (`sync_checkout_agent_summaries`), as the 태스크 list's Needs You, the raised lists and the phone's list already leave it out.
The one list that still draws it is the sidebar's 프로젝트 tree: a resolved root with a live pane stays under its tab, dimmed whole with `해결됨 · ` leading its hover, and numbered by the digit shortcuts, while `shown`, `more`, `needs_you` and `turn_kind` of the tree are computed from the unresolved roots alone (`sidebar_tree` in `hide-core/src/agent_state/tally/lineage.rs`; PRD sidebar-every-tab-line).
It stands only in the 태스크 list's `보관됨` fold and its record, with its archive state (archiving, resuming, could not resume, deleting, unknown) as words; a member that came back is an ordinary row again, and a member still archived while its root is live is listed in that root's record, not as a row.
While a member is archived, its delivery registration and the watches on it are held as a sleeping session's are (`docs/delivery.md`), so its pane's absence does not end its work's ownership; resuming continues them.

## The mark

Every row draws one of nine marks, decided once in the core (`RowMark::of` in `agent_state/tally.rs`) and carried on the wire as `mark` (`turn`, `look`, `going`, `waiting`, `done`, `unread`, `quiet`, `unknown`, `asleep`); no surface derives a mark of its own (status-marks D-01, D-12), the dimming of an unknown row included (`rowDimmed` reads `mark`, never `status_code`; sidebar-quiet-mark-indent D-04).
`going` and `waiting` are the one in-progress mark in two shapes (D-25): every count and precedence takes them as one, and only the ring is drawn apart.
`turn` and `look` are the operator's turn in two shapes: every count takes them as one, and only the badge's glyph is drawn apart.
Four say whose move is next, and their shapes alone tell them apart; `unknown` says Hide cannot tell, and `asleep` is an agent Hide stopped that wakes when opened (D-21):

| Mark | Shape | When |
| --- | --- | --- |
| `turn` | A `!` in a 12 px warning badge | The operator's turn: the row is in Needs You or carries an escalation, or it is an own question, approval or block the operator has read (drawn at `--opacity-read-status`) |
| `look` | An eye in the same 12 px warning badge | The operator's turn to look at a View the agent opened, with nothing else asked of them on the row (A View to look at); its hover and name read `볼 것 · <View>` |
| `going` | A static three-quarter ring in the working colour | In progress: the agent works |
| `waiting` | The same ring dashed, also static (`2.2 2` over a path length of 19) | In progress while the agent waits (`wait`) on children, a background task, its pull request's checks or a reply, or a delegated child's own question, approval or block waits on its parent's answer; its hover and name read `진행 중 · 대기`, or the request's word for a delegated child's request (`진행 중 · 막힘`) |
| `done` | A 7 px green dot | Unread, and the label read the turn as finished |
| `unread` | A 7 px dot in the foreground colour | Unread, with work left (a wait nothing proves is work left), or no verdict |
| `quiet` | A faint hollow ring in the 12 px slot: radius 3.4 of 12, a 1.3 stroke in the muted foreground at `--opacity-read-status`, smaller and thinner than the in-progress ring (sidebar-quiet-mark-indent D-01, D-02) | At rest: read, idle |
| `unknown` | Nothing; the slot is kept so titles line up, and the row is dimmed whole | No demand and an activity Hide cannot read (`status_code: unknown`, unless a block, a raise or an escalation makes it `turn`); counted on no badge (sidebar-quiet-mark-indent D-04) |
| `asleep` | A grey moon (`☾`), not dimmed | An agent Hide put to sleep, waking, or one that could not be reopened; counted on no badge |

The ring does not turn (D-02): its dashes, not motion, tell waiting from work, on every surface, the ones with no second line (tabs, pane headers, the ⌃Tab switcher, ⌘K, the child tree popover) included (D-25, B36).
When the agent starts working again the dashes close, and when a turn ends with a wait left they open.
A delegated row draws only `going`, `waiting`, `quiet` or `unknown` (B24): its question is its parent's to answer, so it draws `waiting` in the working colour while its line keeps the request's warning colour, dimmed, and never the resting ring a parent's children button would show ahead of a working sibling; one of the eight escalation causes still makes it `turn`.
Colour never carries the distinction alone: the mark's name and the word that leads its line are its hover text and part of the row's accessible name (`markWords` in `web/src/agentRow.ts`), as in `내 차례 · 질문` or `읽을 것 · 일 남음`.

How a stop ended is the label's verdict (`axes.rs::stop_verdict`): finished, or unfinished (work left); a label `waiting` that nothing proves is running is work left too, since nothing will wake the agent (status-marks D-17).
Only a finished verdict draws the green dot and the word Done (`끝남`); work left is `Unfinished` (`일 남음`), so a plain dot means stopped with no confirmation that the work is done; a stop no label judged (summaries off, a failed or pending analysis) is `Stopped` (`멈춤`), said only on hover, in accessible names and in notifications, and its line is empty (D-03, D-04).
Read, a stop with work left keeps its line, subdued, with its word (`line.mode: kept`, B7).

A line leads with the word for how the turn ended or stuck (`막힘 · …`, `끝남 · …`, `일 남음 · …`); a question, an approval or a decision names no verb and says only what is asked, since the row's `!` and the warning colour already say it is the operator's turn, and the verb stays in its hover text, its accessible name and its notification; an Unread row's line is only ever `끝남 · …` or `일 남음 · …` (B6).
A working row says its sentence alone or has no line; a waiting row's line leads with what it waits for and its sentence, or with none the cause's detail: `백그라운드 기다림`, `자식 기다림 · 자식 N개 작업 중`, `답장 기다림`, `CI 기다림 · PR #N`, and `대기 · ` with a sentence when no cause is proven (B10, B29; needs-you-cleanup B17, B18).
The web splits a line into its lead word, sentence and background tail (`lineParts`), and a narrow row cuts only the sentence, so the lead and the tail always show.
A Needs You line's ask verb says the same word: an Answer reads `질문` (Question) and an Approval `승인` (Approval), followed by ` · ` and what to do.
A row of unknown state, or on an unreachable device, has no mark and is drawn dimmed whole, and its line says why (B16).
A slot with no live agent behind it, an ended session in a record or a link's timeline, a closed feedback recipient, draws no mark either (sidebar-quiet-mark-indent D-05): the ring says an agent is there and resting.
A row on a device that is not connected still carries the last state Hide knew of it, which may be long over; while `status.remote` says the device is not connected, the shell draws that row and a tree popover's child with no mark, dimmed, with the line `<device> 연결 끊김` (`agents.line.disconnected`), and its hover and accessible name say the same instead of the stale mark words.
A sleeping, waking or failed agent's row, and a saved sleeping conversation's row (`SleepingSessionRow`, the phone's sleeping row), draw the grey moon and are not dimmed (B31): a sleeping one has no line, and its hover and accessible name say `재움 · 열면 깨어남`; a waking one says `깨우는 중` and a failed one `재움 · 다시 열지 못함` on its line.
The operator's turn outranks the moon: a sleeping row with a raised ask draws `!`. These lines and the moon are the same in the Agents tab, the child tree popover, the pane header's child summary, the close list (where the moon is not dimmed) and the phone (`agentRow.ts::stateLine`, `phone::line`); a sleeping row keeps the place and close-list class its awake mark gives it (`RowMark::awake`).

Regression owners: `precedence_is_the_turn_then_progress_then_a_stop_to_read`, `a_dot_to_read_is_green_only_on_a_finished_verdict`, `a_finished_turn_is_not_hidden_by_the_background_tasks_it_left`, `a_sleeping_row_draws_asleep_and_counts_on_no_badge` and `a_waking_failed_or_unknown_row_keeps_a_state_line_on_the_phone` in `agent_state/turn.rs`, `a_finished_turn_is_not_hidden_by_the_children_still_working` in `sidebar.rs` (the lineage, the read axis and the notice together), `web/src/agentRow.test.ts`, and the render tests that every list draws the same line: `web/src/components/row-line-text.test.tsx` (the sidebar row), `web/src/AgentSessions.lines.test.tsx` (the Agents tab) and `web/src/PaneHeaderBand.test.tsx` (the child tree and the pane header) and `web/src/mobile/list.test.tsx` (the phone list).

## Close protection is separate from read state

`requires_close_confirmation` and `requires_close_status_check` are core-derived values carried by both the sidebar agent row and its pane projection.
The confirmation value is true for Working activity, an unresolved demand, a blocked pane, or an agent starting with the operator's first prompt; a stopped unread completion does not create a work-interruption prompt.
The status-check value is true only for Unknown activity with no demand and no block, on an agent that is not starting.
It prevents a destructive local or remote close from guessing that an unobserved agent is idle, and the caller-visible remedy is to refresh status before closing.
An agent is starting while the engine still holds open the start it made of it (`launch_pending`), from a confirmed agent list, unless it works or the engine saw a turn of it end since the start (`completion_seq`; `agent_state::agent_starting`).
The own engine holds a start open until its program takes input and its screen rests past the 3 s settle, so a row that already reads at rest, as a woken agent's does with the last run's screen and its hook's last report of a finished turn, is still `starting` (`시작 중`) rather than `idle` or `done`, and an agent read `unknown` in that time is `starting` too; it never needs a status check.
Whether the start carried the operator's first prompt is the core's own fact from the start request (`AgentStart::Starting { first_prompt }`): the runtime keeps the pane of a task start whose kept launch has a prompt (`lay_start_prompts`) until the engine reports that agent, the pane is gone or the start definitely failed, so the start's answer and the engine's report may arrive in either order.
A starting agent with a first prompt asks the ordinary Stop-work confirmation before its pane closes, because the agent took that prompt as its argument and a close would drop it without a word; one with no first prompt has been given nothing to do and closes at once.
The record lives in the daemon's memory only, so a daemon restart inside the few seconds of a start closes that pane as one started with no prompt.
A `hide agent spawn` brief is not a first prompt: it is submitted after the agent is ready, and a close before then refuses it back to its sender as `brief_not_submitted`.
A pane the engine restored carries no pending start and may hold running work, so it still needs the status check, as does a starting agent whose list was not confirmed.
A pane whose terminal failed to restore is not unknown: no process runs there, so it is stopped and needs no status check (Where each axis comes from, above).
Regression owners: `an_agent_still_starting_closes_without_a_status_check` and `an_unknown_agent_that_is_not_starting_still_needs_a_status_check` in `hide-core/src/session_sync/tests.rs`, `a_starting_agent_asks_before_closing_only_when_its_start_carried_a_first_prompt` and `an_agent_whose_start_is_pending_reads_starting_though_its_screen_rests` in `hide-core/src/runtime/tests/projects.rs`, and the starting case of `web/src/agentRow.test.ts`.
The shell presents that remedy as the existing read-only `Check status` action and keeps the destructive close confirmation separate from it.
An unknown row still belongs to Seen for sidebar grouping, so close safety never changes the read or ownership axes.

A row with live descendants also carries `close_descendant_pane_ids`: every live descendant's pane in the order a subtree close takes them, deepest first and each pane after every pane below it, siblings in the lineage's own child order.
`apply_lineage` derives it on the same pass as the tree, and `refresh_agent_lineage` drops the panes of a device that is not connected, so a close never lists what it cannot reach; a row with none omits the field.
A pane or tab close asks about the subtree only when some agent that closes lists a pane outside what closes (PRD close-agent-subtree D-16), and the status check above applies to each listed descendant: an unknown one keeps `모두 닫기` disabled, while `이것만 닫기` stays available.
Closing a parent alone does not close its children: they lose their parent on the next projection and become the operator's roots with their own state.
A subtree close is undone by one Reopen closed, as a single agent's close is: the parent and its descendants come back as the same registrations in their new panes, so the lineage tokens name the new parent pane and each child is delegated again under it, its descendants and the children left open included ([ARCHITECTURE.md: Reopening locally closed work](ARCHITECTURE.md#reopening-locally-closed-work)).
Regression owners: `each_row_lists_its_live_descendants_deepest_first_and_a_leaf_lists_none`, `a_descendant_on_a_disconnected_device_is_never_listed_for_a_close`, and the tree-close tests in `hide-core/src/runtime/tests/tree_close.rs`.

Needs You and Done belong to operator-owned roots.
A delegated row stays Working or Seen; under the eight conditions below `agent_state/escalation.rs` raises its lineage root to Needs You, with the child's ask on the root's line (Delegated escalation below).
The raised row keeps its own demand, mark, status word and group; what changes is that it is drawn bright and its root carries the ask.
Done is therefore scoped to the lineage root: a delegated child that finishes leaves a dimmed Seen row, and the completion the operator acts on is the root's.

## Where a parent comes from

Ownership, the tree, the ancestor path and the descendant mark all start from one fact per agent: its responsibility parent.
`hide agent spawn --parent here|<self id>` records delegation; omitting `--parent` hands the agent to the operator as an independent root.
The latter has no indentation, from hint, parent line, descendant contribution, ancestor unread propagation, waiting-on-descendants effect or inclusion in the spawner's subtree close.
Its own demand and completion enter Needs You and Done normally, and closing the spawner leaves it running.
The CLI's required `origin` field records the spawner in both modes (null for ordinary roots), but origin never enters the UI or any ownership calculation.
PR and issue panels still find the independent session through its own branch records without a delegated-by line.
The engine records no lineage, so hided writes the four pane tokens through the engine (`Engine::report_pane_tokens`): `parent_pane` names the parent's pane id, `parent_machine` names its machine when the parent is remote, and `child_session` and `parent_session` are the original session digests.
The value and lifetime contract is unchanged.
A fork made with Fork agent or Fork agent to the side is the operator's own root, not the source's child: its registration has no parent, names the source as its `origin`, and carries the `forked` fact (`AgentRecord::forked`), so `hide agent show` and `hide agent list` print a null `parent` and the source as `origin` (PRD fork-operator-root B1, B19, D-12).
Only the source's own live registration can register a parentless fork with itself as origin; any other caller is refused `parent_authority_required`.
The source is registered when the fork's agent starts, and the fork once the engine shows the fork's own session: Codex started as `codex fork <id>` reports its session only at its first turn, so a Codex fork left at rest has no session, no `forked_from` and no registration until the operator gives it a prompt.
Meanwhile the registration waits in the ledger, the source's consent kept as the wait, so the fork names the source as origin even when the source closed while it rested, and the wait ends with the fork's pane or agent ([delivery.md](delivery.md#a-registration-that-waits-for-its-agents-session)).
A fork that cannot be recorded keeps running without its mark: when the source's registration or the wait is refused (`identity_waits_full` past 64 waits, `capacity` at the 2048 registrations the ledger holds, `registration_conflict`), the fork's row is an ordinary root of the operator's with no `forked_from`, the source shows nothing of it, and the refusal is logged as `fork` `registration_failed` with both panes.
Nothing reaches the screen, by design principle 13 and because an alert is a PRD decision: the fork works, and no step the operator could take afterwards gives it the mark, since a fork is not asked again once a slot frees, so the log is where a lost mark is looked into.
Its pane names the source under three keys of its own, never `parent_pane`, which would make it a child again: `fork_pane` is the source's pane id, `fork_session` the fork's own session digest and `fork_source_session` the source's (`lineage::FORK_KEYS`).
The lineage writer writes them from the registration as it writes the four above, so a wake that continues the fork in another pane names the source there too, and a hided restart finds them on the pane and in the ledger (B17, B18).
They are read only while the fork's pane still runs `fork_session`, so a pane that hosts another conversation later is not a fork; `wire::fork_declaration` reads them, and `agent_state::apply_fork_sources` names the source while its pane still runs `fork_source_session` and names none once the source closed or moved on (`SidebarAgentSnapshot.forked_from`, `PaneForkSnapshot.forked_from`, B9, B10).
Because none of these is a parent token, the fork has no depth, so the source gets no child, unread, Working, descendant mark, child count or subtree close from it, and the fork's own demand and completion enter Needs You and Done (B12, B13, B14).
A fork of a fork names the fork it was made from, and a fork of a delegated child is still a root (B15, B16).
A fork made before forks were roots keeps its parent and its old `fork` token, which nothing reads any more: it stays a child and wears no mark (D-09).
The session tokens are lowercase hexadecimal SHA-256 digests of each original `agent_session.value`; this keeps even a path-valued session short and two sessions apart.
A same-machine relationship omits `parent_machine`.
A changed child session clears all four tokens, while an absent session proves neither a valid relationship nor a session change.
Ending registration leaves the tokens until the child pane changes or disappears.
A registration ends by itself when the host's authoritative pane topology stops reporting its pane, as a read record is dropped, and never from an incomplete or stale read ([delivery.md](delivery.md#agent-registration-and-spawning)).
`hide agent list` then reports it `ended` and `disconnected`, and its name is free on that host.
An agent Hide put to sleep is not ended: its registration stays, and `hide agent list`, `hide agent show` and `hide ps` all report it `asleep`, `hide agent list` with `disconnected`, until it wakes and runs again ([delivery.md](delivery.md#agent-registration-and-spawning)).
An agent whose pane the operator closed through Hide is not ended while its Reopen closed entry is listed: the three report it `closed`, `hide agent list` with `disconnected` and `hide agent show` with its activity stopped, its mission reads it as having no worker, and its reopen continues the registration in the new pane; an entry that leaves the list unreopened ends it ([delivery.md](delivery.md#agent-registration-and-spawning)).
`hide agent show`'s `state` reads the same row and adds no status: its activity is the row's, its `waiting` cause is the escalation cause, the core-laid question or the agent's own demand the row carries, in that order, and a row the core cannot read now is `unknown`, never stopped ([delivery.md](delivery.md#agent-registration-and-spawning)).
A relationship is accepted only when both panes still report the recorded session digests; without both digests the child is a root until a complete declaration is written.
A changed parent session cannot adopt the previous session's children, and an absent parent agent retains the existing orphan presentation.
A missing or unknown remote machine identity leaves the child a root until its matching device connects.
The operating-system identity is the platform UUID on macOS and the machine-id on Linux; cloned machines need distinct identities before they can safely resolve different parents.
`hide agent register`, delegated `hide agent spawn` and Hide's fork record the relationship, or for a fork its provenance, in the core's coordination ledger.
A delegated `hide agent spawn --machine <device id>` child is a remote child of a local parent: its tokens are written on the device's pane and `parent_machine` names the spawner's machine, the same resolution as any cross-machine relationship above.
The existing one-second agent refresh writes only panes whose tokens differ, with a complete reconciliation on startup or reconnect and an immediate write after spawn.
The same writer writes the read axis's server mark ([Across an engine restart](#across-an-engine-restart)), in the pane's lineage report when it has one.
`engine/tokens.rs` names this contract for every engine; `wire.rs` and `agent_state/axes.rs` remain its readers.
A connected device's immutable machine identity comes from its consented helper's connection greeting, outside the runtime mutex; an unavailable identity leaves the parent unresolved and records a diagnostic.
A declaration whose machine cannot be matched stays a root and records one diagnostic for that pane instead of guessing.

A pane outlives the agent it hosted and the tokens outlive the agent with it, so a declaration is only a claim until the sessions prove it: it holds while the child's pane reports the `child_session` and the parent's pane the `parent_session`.
`wire.rs` checks the child as it turns the tokens into a row, and `agent_state::apply_lineage` checks the parent because that is where both rows are known, on this machine or another.
An agent that took over a pane is therefore a root, and a parent pane taken over by another agent adopts none of the old children: no line, no descendant mark, and no orphan hint, since the child was never that agent's.
A pane the engine reports without a session cannot prove a match, so its relationship does not hold until it reports the recorded session again, and a `parent_pane` without the session tokens is not a relationship at all.
A parent whose pane no longer lists an agent is a different case: the child stays an orphan root with its hint.

An empty token is a cleared declaration, not a parent named by an empty string.
The token is display-only metadata and dies with the pane, so a closed child leaves no edge behind, and a parent that has gone makes the child an orphan root.

Fork regression owners: `a_fork_registers_as_a_root_whose_origin_is_its_source` (`coordination/mod.rs`), `a_fork_names_its_source_under_fork_keys_and_never_as_a_parent` and `a_fork_that_is_woken_in_another_pane_names_its_source_there` (`coordination/lineage.rs`), `a_fork_names_its_source_and_stays_the_operators_own_work` (`runtime/tests/lineage.rs`), `a_fork_whose_session_arrives_late_registers_as_the_operators_root_from_its_source` and `a_fork_whose_pane_closes_or_agent_ends_before_its_session_ends_its_wait` (`runtime/tests/fork_registration.rs`), the decisions in `coordination/identity_wait.rs`, and `web/e2e/agent-fork.spec.ts`.

Regression owners: `a_parent_declared_as_a_pane_token_is_the_lineage`, `a_child_whose_pane_now_hosts_another_session_declares_no_parent` and the `lineage_sessions` tests in `hide-core/src/session_sync/tests.rs`, `a_parent_pane_on_another_machine_reused_by_another_agent_adopts_no_local_child`, `a_sleeping_child_stays_under_its_parent_only_while_its_record_can_prove_it`, and `web/e2e/lineage-session.spec.ts`.

## The descendant mark and the tree popover

`agent_state/escalation.rs::lift` gives each row one `descendant_mark` over its whole subtree (PRD D-40): `raised` with the number of raised descendants if there are any, else `working` with the number of working descendants, else none.
The shell draws only the `raised` kind, as the turn mark and N, and only in the tree popover on a folded branch: a sidebar or 태스크 row draws no count, because its root already stands in Needs You with the raised ask on its line (needs-you-cleanup D-03, B7).
The `working` kind is still computed and draws nowhere: a parent waiting on its children shows its ring and the working child's line (PRD session-record-redesign D-04).
Idle, done and unknown descendants add nothing, so a mark never says only that children exist.
`row.state.tree_rank` orders siblings in every tree: asking or raised, itself or below it, first; then waiting on a reply or background work; then working; then finished and unread; then the rest.
The sidebar tree under an opened root is `agent_scope.sidebar_tree.visible_rows` from `agent_state/tally/lineage.rs::sidebar_tree`: children and grandchildren wherever they work, two levels only, with the digit shortcuts on roots alone.
The shell shows five siblings per parent and folds the rest behind `N개 더` (`web/src/sidebarTree.ts`).
A grandchild's own children open in the tree popover instead of a third level.
A delegated row whose checkout differs from its parent's carries `row.state.branch_badge` (`agent_state/axes.rs::apply_lineage`): that checkout's branch, or its label when it has none, the name the sidebar gives the checkout; a device checkout's label is its folder name, so the label alone would name a folder there.
The tree popover (`web/src/components/agent-tree-popover.tsx`) opens from the children button at the left end of the pane's second row (on the head before its tools while the turn row is off), a grandchild's chevron and an ask band's fallback `외 N건` when that button is unavailable.
The children button's face is the mark of the first direct child that has one, in tree order, and the number of direct children when there are several (a child of unknown state draws none), its border in the warning colour while the subtree's operator-raised `descendant_mark.count` is positive; its tooltip and accessible name spell out the live direct child total and every `direct_child_counts` state, and ordinary child requests are named only there, because they are the parent's to answer (status-marks B21).
Each child's own state is the mark beside its name on the same row, the mark its sidebar row draws, followed by its agent's logo in both logo view modes; unknown children are excluded from the resting remainder of the tooltip.
Its head names the parent with its direct child count, and its body draws the same rows two levels deep with five siblings each, the children that finished and were read, with nothing below them raised or working, folded below them behind `끝난 자식 N`; choosing a grandchild that has children re-roots the popover on it.
Arrows move, Right and Left open and fold a branch, Enter opens the agent's pane, and Escape closes it and returns focus to what opened it.
Opened from a pane button with a raised mark, or the fallback ask-band button, it starts with the raised branches open and draws each ask on the row it raised (`raised_pane_id`) with its verb, what to do and Open, which for a draft goes to the parent holding it.
A pending or failed move shows at its footer with the retry rules of the relation, and the last line, `개요에서 보기`, opens the Overview on the agent's device with its task's card selected and the agent's own record beside it.
Disappearing children leave immediately and an empty popover closes.

The one sidebar raises Needs You and Done above its project tree, at most five and three most recent roots respectively, with older rows behind More.
Those rows also remain under their checkouts, and a shortcut belongs to the first visible occurrence in physical sidebar order.
A checkout starts open; `session_collapsed_checkout_ids` remembers only explicit collapses, independently of older disclosure records.
A collapsed checkout still shows its Needs You rows.
Core `session_folds` puts agentless worktrees behind Worktrees with no agents, excluding the primary, front, dirty, unpushed and already inactive checkouts, lists in `child_only` the worktrees (same exclusions) whose unresolved agents are all delegated children, which the sidebar draws nowhere because the parent root's tree reaches them, and collects agent worktrees and missing folders behind Cleanup at the bottom.
`session_open_folds` remembers explicit openings across startup and device reconnects, including an intermediate empty catalog.
A confirmed project unregistration removes its project key; removing a device removes its project keys and Cleanup key.
Resolving a session removes it from sidebar membership without changing its pane, tab or task membership.

Regression owners: `web/e2e/sidebar-status.spec.ts`, `web/e2e/projects-sidebar.spec.ts`, `web/e2e/session-panel.spec.ts`, and `runtime::tests::agent_scopes`.

## Shared agent and Workspace status contract

The sidebar hierarchy is Project > Workspace > Agents.
A Workspace is one checkout path, including a plain folder; internal `WorkspaceSnapshot` names the project and `CheckoutSnapshot` names this Workspace.
A branch is the Workspace title when available; otherwise its real folder name is the title.
`primary`, `detached`, `missing`, temporary state, and uncommitted changes describe the checkout, not an agent lifecycle.
They remain separate badges or Git indicators and never select the agent status color.

### One meaning across surfaces

Agent rows, focused-agent tab marks, pane headers, the tree popover, the ⌃Tab switcher, the close sheet, ⌘K results, the Sessions tool, the Overview and its task cards, the pull request lens, the View comment menu, the Hide AI home row and the phone draw the same mark and the same words for the same row (status-marks B20), from the core's `mark` and `status_code` (`web/src/components/status-mark.tsx` draws the six shapes in one box of one size, and an empty box for `unknown` and for a slot with no live agent).
Working uses a fixed blue status token, independent of the user's accent color; the finished dot uses green, so running and completed work remain distinct.

A read question, approval or block keeps the turn mark and its request line at `--opacity-read-status`, so an unread one stands apart from one already looked at.
A blocked approval stays in Needs You and keeps full emphasis until it is answered.
Reading is acknowledgment, not evidence that a demand was resolved.
A read, non-blocked demand belongs to Seen unless the core places its running activity in Working; the mark still says it is the operator's turn.
Disconnected presentation overrides the retained mark and text on every affected surface without modifying demand, activity, read records, or the last known group: the row draws no mark, is dimmed, and its line names the device (`<기기> 연결 끊김`).
Unavailable-server tooltips describe the connection problem rather than presenting retained counts as current work.

A sleeping agent of a kind that sleeps in its pane (Claude Code, Codex, Antigravity) keeps its row, name, place, lineage and badge while the engine no longer lists it: the core draws the row from its own sleep record (`hide-core/src/agent_sleep.rs`), because the engine lists an agent only while its program runs in the pane.
Only a stopped agent of a kind that sleeps, on any machine (the core's, a node that dials it, or a device it dials; a kind that closes its pane on the core's machine only), with no demand and a reported conversation can sleep; the hours rule (24 hours unless the operator chose other hours or Off in Settings) also wants it seen and off screen, a finished mission node's agents and `hide agent sleep` want it off screen, and the pane menu wants neither ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)); a delegated child sleeps by the same rule as its parent and keeps its place under it.
A `request` or `block` letter wakes a sleeping agent exactly as Wake agent does, and its row passes from the moon through waking back to its own mark.
Each such letter wakes it once: one that already rang or already woke it does not wake it again after a later sleep, and its hour runs through that sleep.
A new agent in the pane, whether a wake, the operator or a restore started it, is awake: its `state_change_seq` differs from the one the record kept, and the record is dropped.

A provider with an implemented close-pane sleep dialect instead retains a dormant conversation keyed by `SleepId`, separate from the live agent projection.
Its core-owned group is Seen, and it contributes no active pane, lineage, unread state, doorbell target or executable input address.
Saving and closing do not duplicate the existing live row; confirmed closure publishes the separate sleeping row, and an uncertain operation publishes its explicit status action.
Wake resumes the saved native conversation in its own intent-marked tab, and current reader identity must confirm that conversation before the archived row is removed.
A delegated child that held a registration wakes into the same registration, so its row returns under its parent and its parent's watch and letters still reach it ([delivery.md](delivery.md#agent-registration-and-spawning)); the sleeping row itself never carries lineage.
A Cursor wake needs no session hook from Cursor: the core reports the resumed session to the engine itself, so the row returns under its parent like any other kind's ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)).
The Wake action is available only when this build declares the provider's resume capability.
A record an older build saved names no checkout owner ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)), so its Wake is offered only while the project list shows its checkout; until then the row has no Wake and its reason says to open Hide on that computer and add the project if it is missing, and the Wake returns once the checkout is listed.
A record this build saved wakes from the owner it kept, whatever the list shows.
Regression owners: `a_sleep_keeps_its_checkouts_owner_for_the_wake`, `a_sleeping_worktree_session_is_opened_from_its_repository_while_no_list_shows_it`, `a_sleeping_session_wakes_in_its_checkout_while_no_list_shows_it`, `a_sleeping_session_an_older_build_saved_offers_wake_only_while_its_checkout_is_listed` and `a_sleeping_session_offers_wake_only_over_a_connection` in `hide-core/src/runtime/tests/agent_sleep.rs`, and `a_kept_owner_names_its_repository_by_an_absolute_path` in `hide-core/src/agent_sleep/dormant.rs`.
A retained unsupported sleeping record stays visible without a Wake action or a loading spinner; only an active transition or status check spins.
Unknown work survives restart without replaying external effects; an intent saved before close admission can be released without touching a pane.
`agent_sleep/dormant.rs` owns the bounded archive and row values, while `runtime/agent_sleep/dormant.rs` owns its transitions and exact save/connection/execution fences.

### Workspace aggregation

`agent_state/tally.rs` owns Workspace aggregation from the canonical agent projection after pane-level read state is applied.
Each Workspace counts unique agent pane IDs physically owned by its tabs, independent of sidebar visibility, raised rows, parent collapse, or Workspace collapse.
The existing status synchronization indexes pane ownership once and visits each canonical agent once; it adds no timer, I/O, or per-frame work and publishes only changed summaries.
A descendant running in another checkout contributes to that checkout, even if its lineage row appears beneath a parent elsewhere.
An agent repeated in a raised section and the project tree counts once.
Plain terminal panes without agents do not create an Idle agent status.
A Workspace with no known agents draws no summary chip.
A populated Workspace places its representative status in a trailing chip, followed by `+N` for the remaining agents (omitted for a single agent).
The chip uses the canonical physical agent count, even when agents are also raised above the tree.
An agent whose lineage starts in another checkout also appears as a local root in the Workspace that physically owns it, so that Workspace can reveal the agents its chip counts.
Rendering and shortcut numbering share this ownership-aware tree projection.

The representative follows the same group order as the sidebar: Needs You > Done > Working > Seen.
Within a group, Blocked precedes Approval, then Question; otherwise Unknown precedes ordinary Idle within Seen, so missing information is not hidden by an idle sibling.
Equivalent candidates keep canonical agent order.
A read block in Seen cannot outrank an unread question in Needs You.
The Workspace draws the representative agent's exact mark, color, and emphasis through the shared presentation.

The web shell draws no representative chip; it draws a status badge in its place.
The same pass counts each agent under the mark its own row draws (`RowMark`): `turn` (a `look` row counted here too), `going` (a `waiting` row counted here too, D-25), `done` and `unread`.
A quiet row is counted in none of them, so the badge claims nothing but whose move is next.
The counts ride the summary as `marks`; the web sidebar draws them on a checkout's row only, and a project or folder row draws none.
The badge draws one mark and count per mark, in the order whose move is next (turn, in progress, finished to read, to read), zero ones left out, in the shapes and colors the rows use, so it says what opening the rows would show (B19).
A checkout's badge stands for its folded agent rows and leaves while they are open (docs/UI_BEHAVIOR.md, Sidebar hierarchy).
It is the only count badge: a folded parent row, in the 프로젝트 tree or the 태스크 list, counts no children; a hidden descendant that asks the operator puts the root in 내 차례 with its question, and the `! N` descendant mark is drawn only in the tree popover (needs-you-cleanup B7).
Regression owners: `workspace_summary_uses_physical_ownership_priority_and_unique_panes` for the counts, and `projects-sidebar.spec.ts` for where the badges are drawn.

The tooltip lists positive counts in group order: Needs You, Done, Working, Seen.
Unknown is reported as a subset of Seen, never added again to the total.
When the owning server disconnects, a Workspace with retained agents displays Disconnected and suppresses the stale activity breakdown.
Connection recovery resumes the current canonical projection; it does not mark agents read.

### Disclosure and selection

The disclosure chevron sits at the right edge of a Workspace row.
The left edge holds the checkout-kind icon, title, and checkout badges.
The representative status moves into the trailing agent summary chip immediately before the chevron.
A root agent status mark aligns with the Workspace branch icon; compact agent rows use a 4pt gap between status and title and draw no provider logo unless the operator's agent logo view mode shows it or an ⌥ hold peeks at it (the pane header always does; PRD agent-mark-quiet, PRD agent-logo-view-mode).
A Workspace with nested agent rows uses the entire row, including its name and empty space, as the disclosure hit area.
Its right-edge chevron is a non-interactive indicator within that same button, not a second small control.
A Workspace without nested agent rows shows no chevron and clicking its row opens the Workspace.
Select an agent to focus its pane; expanding or collapsing a populated Workspace only changes the tree.
Collapsing does not change the selected pane, tab, agent read state, running processes, or aggregated status.
`collapsed_checkout_ids` persists across launches.
The web shell starts every checkout open and keeps explicit collapses in `session_collapsed_checkout_ids`, changed by `checkout_agents_toggle` and the admitted `focus_checkout` event.
The older `expanded_checkout_ids` and `collapsed_checkout_ids` records do not determine web checkout disclosure.
Inside an open checkout, actual tabs group complete root subtrees as segments of one list: every root stands on one status axis, a guide per tab marks the boundary, and the tab line at a segment's head appears only when the tab has a sidebar root and holds no recognized agent pane.
The glyph's count describes physical agents in that tab, independently of root ownership or child disclosure; raised groups and sleeping sessions retain their existing positions.
Tab names use the core's explicit manual-name and stable-number metadata rather than an automatic task label, and no tab fold is saved.
Raised Needs You and Done rows remain available, while number shortcuts skip hidden tree rows.
In the web shell the fold controls of a project, a checkout and a parent agent all sit on the right of their row in slots kept at rest; a folded control is always shown and an unfolded one appears under the pointer, with focus inside the row, while the row's menu is open, or on an input with no hover.
The body of each row navigates (a project to its most recent usable checkout, or to its panel on the Overview's Projects when it has none (PRD sidebar-task-lens B41), a checkout to its Workspace, an agent to its pane); a project or checkout row also unfolds its children, or folds them when its scope is already in front and unfolded, and a fold control never navigates: folding from a chevron changes no screen, pane, tab, read state, group or process.

### Verification ownership

Core status tests own the Done mark, representative priority, unique-pane counts, cross-checkout ownership, and unchanged read semantics.
Core close tests own the separation between work confirmation and unknown-status blocking, including local and remote close refusal.
Manual acceptance in the desktop app covers fixed semantic colors and the shared disconnected override, mixed states, Done-to-Idle acknowledgment, right-side disclosure, unchanged terminal selection on collapse, empty and missing workspaces, and disconnect/recovery.
Run evidence belongs under `agents/runs/`, never in `docs/`.

## Uninstrumented is not an unknown activity

A pane whose agent Hide cannot see into is a different answer from a pane whose activity the engine reports as `unknown`, and the two are kept apart on purpose.

- Activity `unknown` is the engine saying it does not know what the process is doing. It is one of the three activity values and it groups like any other.
- Uninstrumented is Hide saying it cannot tell what that session has spawned. It is not an activity, it never changes a group, and it is projected as its own field beside the agent.

The reason is resolved once, by `hide_agent_hooks::diagnosis::instrumentation`, in a fixed order: config unreadable, remote host, hooks not installed, session predates install, hook outdated, unknown.
The first match wins and nothing falls through to an empty value or an invented cause.
Every projection carries the reason's stable code alongside its sentence, so no surface has to recognise its own operator-facing text.

The projection is made only for panes where an agent was detected, and no screen draws it as a mark: it would say Hide cannot tell how many subagents the session runs, which the operator can do nothing about, so the pane header, the sidebar row and the Overview line carry nothing for it (operator decision on issue #810, design principle 13; [UI_BEHAVIOR.md, Subagent count](UI_BEHAVIOR.md#subagent-count)).
The core still projects it (`PaneChildren.instrumented`, the reason and its code), because the connection below is judged from the code.
A count Hide cannot read is reported as unknown and never as zero, because a zero is a claim that the agent is working alone.
An agent whose adapter declares no subagent count (`subagent_counts: None`: Pi and Cursor) has nothing to read, and the core projects it with the reason `unknown` and no count.

### Not connected, and what fixes it

A Claude Code or Codex pane whose session Hide does not hear also carries a connection (`PaneChildrenSnapshot.connection`), read from the same observation as the projection above (PRD settings-cleanup D-09, D-11, B26 to B31).
It is judged by `sidebar::pane_connection` from `uninstrumented_code` for an awake agent whose native session the engine has recorded.
A process detected before its native session is not a failed integration and carries no connection judgement yet, without a timer or an assumption that it connected.
A sleeping agent has ended its process, so no hook can speak from its pane, and it carries no connection until it wakes.
Settings counts sessions and never these connections, so a pane that needs a Reopen says so only in its own header.

| Instrumentation | Connection |
| --- | --- |
| instrumented | `connected: true` |
| any of these, for an agent that is on whose machine's copy is refused as too old (`KitAgentSnapshot.update`) | `update_agent`, carrying that update (versions, path, command) |
| `session_predates_install`, for any agent, in a session that wore the `update_agent` badge on this machine | `started_before_hide` (restart it; Reopen only for Claude Code and Codex) |
| `session_predates_install`, a Codex and the machine's shared server on, or its daemon still answering | `codex_shared_server` |
| `session_predates_install`, any other | `started_before_hide` |
| `hooks_not_installed`, `config_unreadable`, `hook_outdated` | `setup_needed` (fix it in the agent's row in Settings; Reopen would change nothing) |
| `hooks_switched_off`, `unknown`, an agent with no hook, or an agent asleep | no connection, so no chip |
| any of the above, for an agent whose read reports turns and whose session Hide's read refuses for a reason reading again cannot clear ([delivery.md](delivery.md#letters-while-the-agent-works)) | `session_unreadable` (letters and mission answers wait; no Reopen, which resumes the same session) |

A restart of the engine does not move a connected pane to `started_before_hide`: the hook reports to the core, which keeps what it reported in its own `hook-facts.json`, so nothing the instrumented judgement reads goes with the engine's session, for a pane on a device as for this Mac's ([agent-hooks.md](agent-hooks.md#what-the-hook-returns-and-reports-back)).
`can_reopen` is false for `setup_needed`, `update_agent`, an agent other than Claude Code and Codex, and a pane on another device, because a Reopen restarts a Claude Code or Codex session through this Mac's engine and changes no agent's version.
The `update_agent` reason is read from the machine's last kit report and the pane's agent kind on the projection pass that already judges the pane, so it adds no per-tick work; the sessions that wore it are remembered per machine (`Runtime::update_badges`) until they connect or end, so a session started on the old copy reads "restart to connect" once the update has put the piece in, and a session that never wore it keeps the rows below ([agent-hooks.md](agent-hooks.md#updating-an-agent-that-is-too-old)).
The shared server is the machine's last kit read (`KitSnapshot::shares_codex_server`): a read that says the setting is on, or that a daemon still answers with the setting off, is what turns a Codex pane's reason into the shared server, and a later read that says neither turns it back into `started_before_hide` at once (PRD codex-daemon-apply D-07, B9).
After a turn-off that answered `stop_failed`, only a read that says no daemon answers ends the shared server, so a daemon answer Hide could not read keeps the retry on offer (B7).
A kit read happens at launch and on each device connection, on a Reinstall, when a Settings tab showing the kit opens and on this Mac every 5 seconds while it stays open, and in the answer to a turn-off request.
Nothing else polls for a daemon, so one that another app starts later reaches the pane's reason at the next of those reads, not live.

Reopen is one event, `pane_reopen { pane_id }`, and reuses the session-sleep path rather than a second one: the agent is ended the way sleep ends it and started again in the same pane with its own resume arguments, a Codex with `--no-daemon` first (`hide-core/src/pane_reopen.rs`).
Everything knowable before the agent is touched is refused before it is touched, so a refusal known at that point leaves the pane as it was: an agent that is working or waiting (`agent_busy`), a session no id was ever reported for or one with no resume arguments (`session_gone`), a Codex whose capability was never read (`codex_unread`), and a folder that is gone (`start_refused`).
The agent is read again just before it is signalled, because the list the decision used can be a second old: one that has started working or is waiting since is `agent_busy` too, with nothing signalled.
Any other end the engine or the agent refuses is `end_refused`, and a start the engine refuses after the end is `start_refused`.
The engine refuses a `start_agent` under a name another agent still holds as `agent_name_taken`, and an ended agent can hold its name for a moment; a Reopen, and a wake after a sleep, send the same start again on exactly that refusal for at most ten seconds (`agent_start::start_at_shell_reusing_name`), never before the end has been confirmed, and a name that is still held then is `start_refused`.
Every other start takes that refusal as the engine's answer, because its name is someone else's.
The core keeps one entry per pane, so a second press while one runs starts nothing; the pane's `connection.reopen` is `{"state":"pending"}` while it runs and `{"state":"failed","reason":<code>}` after a refusal, and the entry goes with the need for it: when the pane connects, when it leaves, or when it shows no chip.
A start that lands publishes nothing more by itself: the session's own hook reaching Hide is what turns the chip off.
A start refused after the end leaves the pane without an agent, so there is no chip to carry the failure: when the pane then has no agent row, the snapshot's `last_error` says once, as `pane_reopen.not_restarted`, that the agent was ended and could not be started again and that its conversation is kept; the engine's words stay in the diagnostic, and the shell raises the failure as a notice that outlives `last_error` (`web/src/errorNotice.ts`, [UI_BEHAVIOR.md](UI_BEHAVIOR.md) on Reopen).
It is not started a second time, because the wait for the ended agent's name above is the one retry a start has, and a second identical start would learn nothing.
The engine's words for a refusal go to the diagnostic log (`pane_reopen.answered`), never to the screen.

The Codex shared server is turned off by one more event, `codex_daemon_disable { device_id }` (`local` for this Mac), and only by it: no install pass ever does.
The operator confirms it first, because it also stops the daemon that is running, which disconnects every Codex attached to it (PRD codex-daemon-apply D-04, D-11).
It rides the machine's kit queue as `Scope::codex_daemon_off`, and the pass that carries it runs `codex features disable daemon_auto_start`, reads the setting back, stops the running daemon with `codex app-server daemon stop`, and answers in its report.
`KitSnapshot.codex_daemon_off` is `{"state":"pending"}` from the request until that report, then `{"state":"done"}` or `{"state":"failed","reason":<code>}` with `codex_missing`, `codex_refused`, `timed_out`, `unreachable` or `stop_failed`.
A failure before the setting went off leaves it as it was and stops nothing, while `stop_failed` says the setting is off but the daemon still answers, did not stop, or answered in a way Hide cannot read; the same request again then only stops it.
A plain read of the kit says nothing about the request and keeps the last answer, and a machine whose setting is off with no daemon answering, or whose Codex has no such setting, has nothing to turn off.
A Codex that Hide started runs with `--no-daemon`, is not attached to the daemon, and keeps running.

Regression owners: `runtime::tests::agent_connection` (the connection per reason, Reopen coalescing and refusals, the request's lifecycle), `pane_reopen::tests` (the order of end and start against `FakeEngine`), `runtime::tests::device_kit` (a device's turn-off over the confirmed connection only, its numbered runs), `hide-kit`'s `the_operators_request_turns_the_shared_daemon_off_once_and_says_so` and the stop tests beside it, and `hide_agent_hooks::codex_daemon::tests` (the stop through the account's own `CODEX_HOME`).

## GitHub status in the Workspace row

The PR icon is independent of agent status and Workspace disclosure.
It appears for a known pull request, an in-progress first lookup, or a GitHub lookup failure; a successful lookup with no matching PR leaves it absent.
A pull request restored from the previous run draws as soon as its checkout row exists.
Clicking opens details without selecting a pane, marking agents read, or folding the Workspace.
The popover shows the PR number, title, Open/Draft/Merged/Closed state, head and base branches, and CI rollup.
Its refresh action reloads one repository; its external action opens the PR URL through the existing external browser route.

The core reads GitHub for every registered local Git project from the moment a window attaches to the daemon (the reader sits in the coordinator's UI-attached block, so a daemon with no window reads nothing), whether or not Overview, the palette or any other screen was opened; a device's project and a plain folder are not read.
A project's status is read once when a window first attaches, again five minutes after its last answer, or sooner when that read failed to get the project's pull requests or its issues (`Runtime::reread_stale_github`, with the clock passed in so a test advances it without sleeping; a project whose ask is unanswered is not asked again, so a slow pass is never overtaken by the next), and on an explicit refresh, which advances that repository's generation.
When the operator looks at a project, the web shell sends `github_request` with `look` (the window gains focus or becomes visible, and a project is brought to the front; `web/src/githubLook.ts`), and the core reads that project again when its last answer worked and is at least 30 seconds old (`Runtime::read_when_looked_at`, `GITHUB_LOOK_AFTER`, the clock passed in), so a project the operator comes back to does not wait out the five minutes.
A look is the stale re-read's ask and nothing more: it asks nothing while an ask is outstanding, for a project in failure backoff, one whose last read missed its pull requests, or one answered "no GitHub remote"; it never sets `loading`; it is honored only for the local Git project in front, and a look the core does not honor is silent (no error, no frame); a request that names `refresh` is a refresh whether or not it names `look`.
While a window is attached, an advisory poll asks GitHub whether anything behind the marks changed, every 15 seconds for the project in front and once a minute for each other one, and only says so (`crate::github_poll`, `Runtime::ask_polled_changes`).
It watches the projects whose last read worked and holds an open pull request, and the project in front even with none open, by the repository its read named, so a pull request opened elsewhere shows within the minute (PRD git-freshness D-10); the project in front first and at most eight (a count past that is written as `poll.over_limit` when it changes): the repository's open pull request list, and the check runs and commit status of the head commit of up to four open pull requests whose checks had not finished (`PullRequestSnapshot::checks_unfinished`: pending, or failed while a workflow run or check on the head still runs, the rule a mission's ci line and its CI letter wait on), each by a conditional `gh api` read with the entity tag it last saw; a 304 means unchanged and costs no rate limit, and a tag seen for the first time is a baseline, not a change.
The check runs and the status of such a head commit are read the same way: a different tag only makes the poll read what the list adds up to (`passing`, `pending`, `failed`, by the rule the mark uses, GitHub Actions' runs read as [Checks of a head commit](#checks-of-a-head-commit) says) and whether anything on it still runs, and it counts a change only when that combined result or whether something runs, over the check runs and the status together, differs from the last one. One job of many finishing asks nothing; the last one finishing, a failure, or the last run ending behind a failure asks one read within the poll's interval for that project; another app's check or a status still running behind a failure of its own list is left to the five-minute read.
The open list moves its tag for a comment, a label, a review or a new `updated_at` as well, so a different tag there is not enough: the poll then reads one digest of the list (each pull request's number, head commit and draft flag, `gh api --jq`) and counts it a change only when that digest differs from the last one, which a pull request opened, merged or closed, a push, or a draft flip does.
A comment, a label, a review or an `updated_at` alone is left to the five-minute read and to a look.
A change is asked for through the look rule above without its thirty seconds, since an answer that just landed may predate the change GitHub reported and the poll's interval already bounds how often it asks; it never overtakes an ask, a failure backoff or a repository with no GitHub remote, and never sets `loading`; the poll writes no pull request state and never touches the five-minute clock or its backoff.
A failure of the poll (no `gh`, no login, the network, a reply it cannot read) pauses that project alone, 60 seconds and twice as long each time up to 15 minutes, and is written once per kind as `poll.failed`; the reason shows nothing on screen (design principle 13).
With fewer than a tenth of the account's requests left, nothing is polled until `x-ratelimit-reset` (`poll.rate_floor`).
A project the poll is not watching, paused or under the floor, has its GitHub status marked stale as after a failed read (`Runtime::note_poll_held`), so its marks dim with `GitHub 읽기 실패 · N분 전 값` until its next read answers (PRD git-freshness B23).
A change in the first interval after a project's baseline is read only by the five-minute clock, because the poll has no earlier tag to compare it with.
A local agent's session that prints the address of a pull request the last answer does not hold asks for its project as well, so a pull request an agent opens reaches its row when the label worker next reads that session (its turn ending, at the latest) rather than up to five minutes later (`Runtime::read_sighted_pull_requests`).
The project is the one whose pull requests are in that repository, else the printing pane's project when none of its pull requests names another repository, so a repository's first pull request is found and a fork's checkout whose agent opens a pull request upstream is not read for it; an address no read project owns is written to the diagnostic log as `read.sighted_unowned` and nothing more.
The sighting keeps the rule above: a project whose last read worked and has nothing asked since is asked at once and its clock waits for that answer; a project with any ask outstanding (a read in flight, its first read, a refresh, or an answer the clock has not taken yet) is not asked again, and is read once more after that answer lands only if the answer does not hold the pull request and the sighting is still under fifteen minutes old; a project whose last read failed to get its pull requests is left to its own retry, and one that got them and missed only its issues is asked as one that worked.
The read is quiet: unlike a refresh from the menu or the popover, it does not set `loading`, so the row keeps what it shows until the answer moves it.
Each address sets off this once while its sighting is under fifteen minutes old, so an address the read still does not return (another repository, or past the newest 200) is not asked for again; a sighting older than that sets off nothing, and an address an answer holds stops being remembered.
At most 64 addresses that a project owns are remembered at once, and one past that is not read and is written as `read.sighted_over_limit`; the label worker keeps at most 32 sightings between two hand-offs, the newest, and a read that finds more writes `read.sighted_capped` with how many it dropped; each owned address it remembers writes `read.sighted` with the projects read now and those waiting for an answer, both empty when every one of them is in failure backoff.
A repository with no GitHub remote is an answer, not a failure: when the remotes its node read with the catalog's path facts name no repository `gh` could read, the reader answers "no GitHub remote" (`failure_category` `no_github_remote`, neither stale nor an `unavailable_reason`) with no `gh` call and the screen draws no GitHub state for it; a `gh` that says the same for a repository whose remotes the node could not read (an older node, a config that includes another file, a remote URL in a form the node does not read such as `http://`) or that are not GitHub's gives the same answer. A device project whose `origin` its node reads as naming no GitHub repository gets the same answer (`available`, no `unavailable_reason`), so its checkout cards draw nothing for it either.
When the node read its remotes, its clock (`GithubClock::NoRemote`) never asks again on its own: the next read comes when the catalog's remotes for that repository change, written as `read.remotes_changed`, or on an explicit refresh; before, each read ran `gh pr list` twice to learn that and was retried every five minutes forever.
When the node could not, `gh`'s answer is asked again five minutes later, since a remote can be added where the node does not look.
A failed read (one that did not get the project's pull requests or its issues, whatever the reason: `GithubProjectSnapshot::read_outcome`) is asked again after 30 seconds, then 60, 120 and 240, and from the fifth failure in a row every five minutes; the first read that works starts the count over, and another project's clock is untouched: the reader hands back its cached entry for every project in a request, so an answer counts for a project only when that project's generation moved since its last answer.
The previous answer stays drawn while a retry waits, and nothing new is shown for it: the row keeps its muted stale state (design principle 13), and each retry writes one `github` `read.retry` diagnostic with the project, the attempt and the delay.
The failure count lives in the clock (`GithubClock`), not in a second delay, and a project is asked again only after its previous ask answered, so at most 64 projects each cost one read per 30 seconds in the worst case and an outage backs every one of them off to the five-minute pace within four retries.
At most 64 local Git projects are read: the project in front first, then the ones a screen named with `github_request` or `overview_refresh`, then the rest in path order; a count past that is stated in the diagnostic log as `projects.over_limit` each time it changes and the ones left out show no PR icon.
Every project shares `GithubReader`, its per-project cache, one active worker and coalesced pending requests.
Each `gh` command runs on the core's own node (`hide-host/src/gh.rs`), where the operator's GitHub login is, under its 15-second limit; the core sends the command line as a `gh` call, the node refuses any line `hide_node_link::gh::allowed` does not name without running it, and the core parses what it answers.
The last successful answer of every project is kept in `github-snapshot.json` in the state folder (`hide_kit::layout::github_snapshot`, written by `github_store.rs`) and restored when the daemon starts, so the first frame already draws it; the file also keeps what the wire leaves out of a pull request (its head commit, its repository and its closed time), because a settled pull request reconnects to its worktree only at that commit.
A restored answer is stale until a read replaces it: `stale` is set and `last_success_at_unix_ms` stays the time of the read that produced it, so the icon is muted and the popover says how old it is.
A file that cannot be read, decoded or is of another schema version is not used and is named in the diagnostic log as `snapshot.discarded`; the next answer replaces it.
`loading` is true only for a project with no answer to show whose first read has not yet answered; a project the reader answers nothing for is not loading either, and a project with a restored or earlier answer is loading only while an explicit refresh of it is in flight.
A project's pull requests are two [`gh pr list`](https://cli.github.com/manual/gh_pr_list) calls asked for at the same time, each under the 15-second limit: the 200 newest pull requests of any state without `statusCheckRollup` (with `headRefOid`, `isCrossRepository`, `createdAt`, `closedAt`, `author`, `mergeStateStatus` and `reviewRequests`), and the open ones with `number,statusCheckRollup` only, whose GitHub Actions jobs are judged by their workflow runs ([Checks of a head commit](#checks-of-a-head-commit)).
The rest of a project's read is asked beside that list rather than after it: the repository's name and the issues (their open and closed lists at the same time) start with the pull requests, and the review asks and the merge queue facts start together once the list has answered, so the issues no longer hold a merged pull request's state back by as long again.
The same open-check rollup also supplies optional `PullRequestWork.check_counts`: total, passed, failed, running, waiting, skipped and unknown, each check counted once.
Completed success is passed; neutral and skipped remain skipped; failure conclusions are failed; active CheckRuns are running, queued CheckRuns waiting, and an unread or unfamiliar state unknown.
Legacy StatusContexts keep SUCCESS, FAILURE/ERROR, PENDING and EXPECTED distinct; an absent rollup stays unknown while an explicitly empty rollup has zero checks.
This adds no GitHub read or polling clock and leaves the existing merge eligibility calculation unchanged.
The PR list (below) adds three reads to the same generation: the signed-in login (`gh api user --jq .login`, once per read of all projects, right after `auth status`), the open pull requests the account was asked to review (`gh pr list --search review-requested:@me`, numbers only), and one GraphQL query for each open pull request's place in the merge queue, its last ten review requests with whom each named, each reviewer's latest review (`latestOpinionatedReviews` and `latestReviews`, the first 20 of each, `totalCount` for the rest) and the approvals its base branch's classic protection asks for (`HidePrFacts`).
With that query, for each base branch of the open pull requests GitHub reports a review decision for (at most four, the newest pull requests' bases first; more is one `pull_requests.approval_bases_capped` diagnostic naming the repository and the count), one REST read of the approvals its ruleset rules ask for (`gh api --jq … repos/<repo>/rules/branches/<base>`, readable with read access; `hide_node_link::gh::required_approvals_arguments`); a failed one leaves that base's count unknown and writes one `pull_requests.approvals_unread` diagnostic, with nothing on screen (PRD pr-review-status B11).
From those the core gives each open pull request the query covers `PullRequestWork.reviewers` (login or team slug, state and time, at most 20, the rest counted in `reviewers_more`) and `required_approvals` (PRD pr-review-status D-07, D-08, D-12, D-13):
an approval, a change request or a dismissal stands over a later comment, a reviewer with only comments is `commented`, a pending review is not read, and a review still asked of someone (`reviewRequests`, the user's or the team's) makes them `requested` whatever they said before, with the time of their last ask;
`required_approvals` is the larger of the ruleset count and the classic count, drawn only when the base's rulesets were read (a failed read or a base past the cap leaves it unknown, because the classic count alone may be the smaller), kept only while GitHub's review decision is present and the count is at least one, because a 0 while GitHub still decides by review means the rule that asks is one this account cannot read.
`reviewers` is `None` (unknown, drawn as nothing) while the query has not answered for the pull request and on settled pull requests; GitHub's review decision alone says whether review still blocks, and `model::PrState::of` does not read the reviewers.
The pull request list alone decides whether a project's read succeeds (issue #1189, which replaces PRD work-tab-pr-lens D-25's rule that any of the three failing failed the read).
A `gh` older than `hide_node_link::gh::MINIMUM_VERSION` (2.72.0, the first that knows every field and flag of the allowed command lines) and `gh auth status` failing are still the machine's refusal and fail every project's read: each pass asks `gh --version` before `auth status`, and an older release is the `outdated` failure category, named in the reason with both versions, so it reads as one state to fix rather than as the first field it refused (gh 2.58.0 refused `closingIssuesReferences` and failed every pull request read of that machine).
A `--version` that fails or prints no release (an unversioned build's `DEV`) is written as `gh.version_unread` and the pass goes on to `auth status`, which names a missing `gh` or a lapsed login as before.
The rust lane runs every allowed command line against that release, offline (`hide-host/src/gh.rs`, `every_command_line_parses_on_the_oldest_gh_hide_supports`), so a field or flag a newer `gh` added fails there until the minimum moves with it.
When the pull request list fails and the issues answered, the project keeps those issues and the repository's identity (`issues.repository_id`) beside the failed status, and its previous pull requests stay as stale (`keep_previous_reads`): a mission admits a worker on another machine by that identity, and a refused list once took it away.
Each of the three is asked and judged on its own, and one that fails leaves the judgments derived from it unknown on every pull request, never "no": the login makes `mine` (`Option<bool>`, `None` when unknown), the review search with the login and the request list makes `asked_of_me` (`ReviewAsk::Unknown`), and the GraphQL query makes the queue place (`MergeQueuePlace::Unknown`), the time of the last review request and the reviewers (`None`).
What the list alone settles stays settled: a request naming the login is a direct ask without the search, and a request list with nobody who could be the operator or their team (any request when the login is unknown, a team's request when it is known) was asked of neither.
The read is a success with the failed questions named in the project's status (`unread_queries`: `account`, `review_requests`, `merge_queue`, and `reviews` beside `merge_queue` whenever the facts query fails), so the status is not stale and the list is current; each failure writes one `github` `pull_requests.extra_unread` diagnostic with the question and `gh`'s words, which the operator never sees.
A project that keeps its last list because a later read failed keeps the names of the unanswered questions that list was read with.
The re-read clock treats the read as answered: the list is current, and a question that fails every time (a GitHub Enterprise Server without the queue fields, a token too narrow for the search) costs two calls each normal interval rather than slowing the list.
Asking for the checks of every merged and closed pull request made one read take 11 - 14 seconds, so a settled pull request's checks are not asked for again: the checks read for the same number and head commit while it was open stay in the reader's cache, and without them its checks are `Unknown`, drawn as no mark.
The one exception is a pull request that settled in the last 15 minutes with checks last read pending or failed: a rerun can pass and the pull request merge between two reads, so each read asks its head commit's check runs, workflow runs and status again (the same reads the advisory poll makes), at most four such pull requests per read, the latest settled first, and takes a passing, pending or failed answer; any other answer or a failed read keeps the remembered checks and goes to the diagnostic log (`github.rs::recheck_settled`).
Either call failing fails the project's read, which keeps the last answer and states the failure, so half an answer is never a repository with no pull requests; `pull_requests.ok`, `pull_requests.empty` and `pull_requests.failed` carry the read's `duration_ms`.
Which pull request belongs to a checkout is one rule (`github::belongs_to_checkout`, chosen by `pull_request_for_checkout`), because a branch name is used again for new work: a pull request from a fork never belongs to a local branch of the same name, an open one belongs to the checkout on its head branch, and a merged or closed one only while the checkout's commit (the worktree reader's `head_sha`, or, before that reader has answered, the commit `Repository::head_oid` read from the repository's own files with no process) is exactly the pull request's head commit on that branch; a checkout that was amended, rebased or left behind after the last push therefore loses its merged pull request.
Several candidates resolve open, then merged, then closed, the most recently updated first among equals, and a checkout whose commit is not read yet takes no settled pull request.
The sidebar row, the worktree catalog, the agent rows' PR mark and the link and hand-off actions all ask that rule, and a catalog read that moves a HEAD decides the connection again.
The PR list and a worktree's base still list one pull request per head branch (`preferred_per_branch`).
The same generation also reads open issues, their Project Status, and PR closing references for Project Home.
The issue list uses `sort:updated-desc` and reads one sentinel beyond the 200-issue display cap so overflow is based on evidence.
Issue references accept a GitHub issue URL, `owner/repo#N`, or `#N` when the repository is known; unsupported hosts and malformed references are rejected.
A failed component read retains that component's last successful answer, including a successfully empty answer, while a successful PR read still advances if issue reading fails.
Hide stores no new credentials and runs no subprocess or file write under the runtime mutex; the five-minute ask is a comparison on the coordinator's existing wake, and the file is written by its own thread from an owned copy.
The project request event, result status and PR fields travel through revisioned `rest`; presentation reads that snapshot only.

| GitHub checks | UI |
| --- | --- |
| Every reported check succeeded, was neutral, or was skipped | Passing, green |
| Any failure, error, cancellation, timeout, or required action | Failing, red |
| At least one running, queued, waiting, pending, or requested check, with no failure | Running, yellow |
| An explicitly empty check list | No checks, gray |
| Absent check data, an unknown check kind, an unrecognized terminal result, or a merged or closed pull request whose checks were never read while it was open | Unknown, no mark |

Failure takes precedence over pending, then unknown, then passing.
A failed refresh preserves the last known PR and reports the lookup failure and last successful read time.
The visible stale notice qualifies both PR state and CI; old green checks are not presented as a fresh result.
Missing `gh`, logged-out authentication, network errors and rate limits use the reader's existing categorized diagnostics and user-facing reason.

The sidebar tree projection additionally indexes physical pane ownership when forming root rows and direct-select candidates.
This is linear in the visited project panes and agent projection per recomputation, with no queue or asynchronous work; repeated hover values do not trigger it.

### Checks of a head commit

The checks a pull request's mark and a mission's `ci` line show are what its head commit's checks add up to: a failure first, then anything unfinished, then anything unrecognised, then a pass (`github::runs::worse`).
Another app's check runs and the commit's legacy statuses count one by one, from the open list's rollup.
GitHub Actions' jobs do not: GitHub lists every run's jobs on a commit under the same names, a cancelled run's closing job reads failed, and a run queued before its first job is not listed at all, so a pull request whose first run was cancelled read failed beside a replacement that passed (with-hide/hide-private#546), and one whose replacement had not made its jobs yet read passing (with-hide/hide-private#394).
Each workflow is judged by its runs instead, read as check suites by one fixed GraphQL query for up to 50 head commits (`HideHeadRuns`, `hide_node_link::gh::head_runs_query`), after the open list answers and the repository's name is known: a run still queued or running keeps its workflow pending; otherwise the run that finished last (its suite's `updatedAt`) decides, a cancelled run is left out, and a workflow whose every run was cancelled failed.
Only runs the pull request or a push started count (`pull_request*` and `push` events), which are the runs GitHub's own pull request checks list; a run started by hand or on a schedule on the same commit is someone else's.
A commit with more than 100 suites, or one GitHub does not know, is `Unknown` (the first writes `checks.runs_capped` with the repository and the head).
When the runs cannot be read, every open pull request reads `Unknown`, since nothing then says no run is queued on its head, so neither a pass nor no checks is known and a mission's `ci` line is kept; one `pull_requests.runs_unread` diagnostic names the repository, the heads and the reason, and the rest of the read stands.
The card counts the same runs: its check counts, its failed checks by name and its running checks take GitHub Actions' jobs only from each workflow's unfinished runs, else from the run that decides it (the last cancelled one when every run was cancelled), each job placed in its run by the run number in the address GitHub gives it (`…/actions/runs/<run>/job/<job>`); a job whose address names no run is not shown to be the deciding run's and is not counted.
The advisory poll and the settled recheck read a head the same way: the check runs list with GitHub Actions' runs left out of its verdict, then the same query for that one commit.
Two facts beside the verdict are kept on an open pull request, core only: a failure that is only a workflow whose every run was cancelled, with nothing else failing (`PullRequestWork::checks_cancelled`), and a workflow run or check on the head that still runs, which a failure ranking first hides (`checks_running`); the verdict stays as it is, and a mission's ci line and its CI letter read the head's CI as ended only once nothing runs, by one rule (`PullRequestSnapshot::ci_end`), the letter calling the first failure cancelled (`docs/missions.md` in with-hide/hide-private, its paragraph on pull request changes waking the Conductor).

### Pull request visual states

A pull request has one state, decided in the core by one rule (`model::PrState::of`, from its lifecycle, draft flag, checks and review decision) and sent on the wire as `state` on every checkout pull request, as `pr_state` on a GitHub search result and as `state` on a link panel's pull request (`contracts/snapshot-wire-enums.json`: `pr_state`).
The rule reads in this order: merged, closed, draft, failed (failed checks or a change request), mergeable (passing checks with an approved or absent review decision), and pending for the rest (checks running, none or unknown, or a review still required).
A draft is draft even with failing checks, and absent or unknown checks never imply a pass.
Each state has one token and one icon, drawn the same on every surface (operator approval, 2026-10-10): failed red (`--pr-failed`), pending amber (`--pr-pending`), mergeable green (`--pr-mergeable`), draft grey (`--pr-draft`), merged purple with the merge icon (`--pr-merged`), and closed dim (`--pr-closed`).
A change request is red wherever its word is drawn, as failed checks are.
The web turns a state into its icon, colour and word in one module (`web/src/prMark.ts`) and draws it with one component (`web/src/components/pr-mark.tsx`); no screen keeps a colour table or a classifier of its own, and the checks and review words and the stale reading live in the same module.
The state word and the accessibility label keep the meaning without relying on colour.
A mark dims while GitHub cannot be read again, and its tooltip says when it was last read.

### Pull request groups

The PR list (`docs/tasks-and-pr-list.md` in with-hide/hide-private, The PR list and the PR card) lists pull requests in groups the core decides, in `hide-core/src/agent_state/work/board.rs`, from GitHub values the five-minute read carries and from facts only Hide holds: the link record's sessions and the agents' own state.
A pull request body's headings and a repository's own scripts decide nothing, so the same rules hold in every repository, and Hide AI never changes a group or a reason.
The result is the `prs` board of each project's `agent_scope`: rows (group, fold, reason, agents, the pane a reason is about, the checkouts it is the work of, its issue), the groups with their numbers in draw order, and the count of `needs_you` rows that are not folded.

Which pull requests are listed: the operator's own, those a session of theirs made or worked on, those a review was asked of them or of their team for, and bots'; a merged one needs the first two, a closed one that was not merged is never listed, and a teammate's pull request none of these reaches is not.
"Mine" is the pull request's `author.login` equal to the signed-in login.
A pull request is left out only when every judgment says it is not the operator's: while the login is unknown, every open or merged pull request may be theirs and is listed, and while a review ask is unknown, the open pull request it concerns is listed (issue #1189).
A pull request's agents are the live agents whose session the link record names as having made or worked on its branch; sharing a folder alone ties nothing.
An agent that finished and is not yet looked at moves no group, because the unread check is the 태스크 list's (읽을 것).

Each pull request stops at the first slot that fits:

| # | Fits when | Group | Reason |
| --- | --- | --- | --- |
| 1 | the author is a bot (GitHub's `is_bot`) | Bots | the reason its other rules give |
| 2 | its agent is asking (a question, an approval or an error) | Needs You | `reply` with the agent and the question's first line |
| 3 | its agent is working but has not moved for the watch's first inactivity window (`INACTIVITY_MS`, 20 minutes) | Needs You | `check` with the agent and the time since it moved |
| 4 | its agent is working | Agent is working | `working` |
| 5 | it is in the merge queue | Merging · merged, first | `queued` with the place |
| 6 | it is a draft | Drafts | `draft` with its age |
| 7 | it is owned (mine, a session of mine touched it, a bot's) and checks failed that GitHub's `mergeStateStatus` does not call `UNSTABLE`, or a change request stands, or the state is `DIRTY` | Blocked · no agent | `failed` with the first check's name and how many more, `changes`, or `conflict` with the base |
| 8 | it is mine and the state is `CLEAN` or `HAS_HOOKS`; `UNSTABLE` too, with the failed required-or-not check named | Needs You | `merge` |
| 9 | it is mine and the state is `BEHIND` | Needs You | `update` with the base |
| 10 | it is mine and the state is `BLOCKED` | Waiting with `ci` while checks run, Waiting with `waiting_review` while a review is requested, else Needs You | `assign_reviewer` (worded `리뷰어 지정 · 요청한 사람 없음`, with `승인 0/m` when the count is known) while GitHub's review decision is `REVIEW_REQUIRED`; `assign_reviewer_optional` (worded `리뷰어 지정 · 필수 리뷰 없음`) otherwise (no decision, or approved and blocked by another rule), because that block is not review's and the row must not say nobody was asked (PRD pr-review-status B4) |
| 11 | a review was asked of the operator by name | Needs You | `review` with the author and the time since the ask |
| 12 | a review was asked of a team of the operator's (the search finds it, the request names no user) | Needs You, folded as `팀 리뷰 요청 N` | `team_review` |
| 13 | anything else open | Waiting | `ci` while checks run, else `waiting_review` |

A judgment that is unknown is never read as "no", and a pull request that cannot be judged is never put in a group it may not belong to.
The slots that read an unknown judgment instead place the pull request in Waiting with the reason `unjudged` and the question that was not answered (`account`, `review_requests`, `merge_queue`), in the plain colour of `checking`, and it counts toward no Needs You:

| Unknown | The pull request that waits unjudged | What still holds |
| --- | --- | --- |
| the login | any open one that no earlier slot placed and that has no session of the operator's and is not a bot's; a session's pull request still stops at slot 7 | slots 1 - 6 (a bot, an agent, the queue place, a draft) |
| the review ask | a teammate's open one the request list cannot settle (a team's request is on it) | a request naming the login (slot 11), and the pull requests of slots 7 - 10 |
| the queue place | the operator's own at slot 8, 9 or 10 (merge state clean, has hooks, unstable, behind or blocked), the states a queued pull request can be in | a conflicted, draft or still-computing one; a queued one when the queue is read |

Slot 5 fires only on a known place.
A merged pull request is Merging · merged below the queued ones, newest merge first.
When GitHub has not computed `mergeStateStatus` (`UNKNOWN`) the pull request keeps the group it stood in on the previous board with the reason `checking`, and waits when it is new, so a pull request never jumps while GitHub thinks.
The reviewers and the required count move no group and no reason (PRD pr-review-status D-02): the web adds the reviewer marks and `승인 n/m` to the row the board already placed.
A pull request nothing has changed on for 7 days (`OLD_MS`) in any group but Merging · merged is folded as `오래된 PR N` inside its group.
Folded lines (a team's review asks, old pull requests) are in no count: the PRs icon's number is these unfolded Needs You rows alone (PRD sidebar-task-lens B29), and the sidebar's Needs You is unchanged.
Within a group the order is the reason's urgency, then the newest update (the newest merge for Merging · merged), then the larger number.
Times in a reason are the instants they began, never durations, so time passing publishes nothing; the web counts them on its one clock.

`<checkout> only` keeps a row whose `checkout_ids` hold the checkout: the one on the pull request's branch, the ones its recorded sessions ran in and the ones its live agents sit in.
A project with no GitHub remote has no board, and `gh` missing or signed out is a state of the project's GitHub status, not an empty board.

### A task's stage

A task's four-step stage (`작업 → PR → 검토 → 머지`), which feeds the task's state and which no screen draws since the session record's `지금` box replaced its bar (PRD pr-card-and-session-record-redesign B14), reuses this cached PR answer through the agents' PR summaries, reading only the live ones, so a pull request merged before the operator's last request never counts (`agent_state/tally/tasks.rs::stage`): with any pull request the bar stands at the least advanced one that is not merged, a draft, pending, failing or closed one at two steps and a mergeable one at three, and at four once every one merged; with none it is one step while a member works in a linked worktree, and a task that is only a conversation has no bar.
A failed or unrequested lookup never becomes a merged or empty stage: a member whose PR state is unread adds nothing.
The workspace inspector uses the canonical representative agent and disconnected override; inspection itself never acknowledges an agent.

## Task identity

Moved to a private document: `docs/task-identity.md` in with-hide/hide-private.

## Row presentation and phone notifications

`agent_state/turn.rs::row_state` supplies each row’s attention, title emphasis, line mode, semantic tones, priority, search tone, close state and request timing.
The shell maps semantic tones to its existing tokens and translates status words; a tab carries only the mark tone it needs.
A waiting root intentionally keeps its hollow row glyph with a working tone, and a subdued chip and search result.
`row_tests::read_question_keeps_its_request_line_and_hue_without_operator_attention` and `row_tests::waiting_root_keeps_distinct_row_and_chip_decisions` pin these surface differences.

`agent_state/tally.rs::phone` projects the phone’s groups, roots, places and safe row values from the published snapshot.
`agent_state/turn.rs::push` owns the effective notification state and transition ledger; hided keeps pairing, push encryption, transport and delivery policy.
A read root question clears the server’s effective push state, while the phone’s existing open-page rule keeps its notification while the demand remains.
The phone reads the core’s `holds_notification` value, and `emphasized` controls its compact title.
When a new turn's read finishes before that pane's previous analysis, the label worker keeps one entry in its existing analysis queue and reconsiders the current turn after the old result lands.
The old result cannot release the new turn's notice, and the new turn needs no further read to receive its judgment (`labels::notification_tests::a_late_analysis_cannot_release_the_next_turns_notice`).
The projection and push regression tests remain in `hided/src/mobile/{projection,push}.rs`, exercising the same public core functions the transport calls.
The desktop app's agent notifications run the same transitions on their own ledger (`hided/src/desktop_notices.rs`, PRD first-run-onboarding D-09), so a desktop notification and a phone push agree on what is new without either one's seed or mode moving the other.
A Hide AI row for a home is on the phone's list, and in these transitions, only while it is in Needs You, so the notification follows the group: one notice when it enters, none when the home's turn changes inside it, and none for its progress (PRD hide-ai-work-home B2, B10); the phone opens the agent's detail from it.

### With and without Hide AI

Here “on” means Agent summaries is enabled and the current session has a proven label.
Before a proven label exists, an unavailable provider or failed analysis adds no generated title or sentence; an existing proven label can remain while an analysis is unavailable.
“Off” means Agent summaries is disabled, including Hide AI being off; native session facts, engine lifecycle, lineage, read state and notification settings still apply.
The table describes operator-owned roots; delegated rows and the eight escalation causes keep their ownership rules above.
Sentences keep their original language in every mode.

| Surface or condition | Hide AI on | Hide AI off |
| --- | --- | --- |
| Row title | Every adapter: manual title, then Hide AI goal, then proven native automatic title, then provider. See [Task identity](#task-identity). | Every adapter: manual title, then proven native automatic title, then provider. No workspace label or control name becomes a title. See [Task identity](#task-identity). |
| Pending native question | The turn mark, Question, Needs You until answered, including after reading. Second line: turn-end `expected_reply`, otherwise the question's first non-empty line (40 characters including a truncation ellipsis), otherwise `progress`. | The same turn mark, Question and Needs You until answered; its first non-empty line supplies the second line without an AI call. With no question text the status word stands in. |
| Plain-text question at the end of a turn, with no native wait | If the analysis identifies a question: the turn mark, Question, Needs You while unread, with `expected_reply`. Reading moves it to Seen unless it is still running; the request sentence remains until resolved. | Text alone creates no question, no turn mark and no Needs You. A stopped, unread engine `done` is in Unread with the plain dot, the word Stopped on hover and no sentence. An engine `idle` is Seen, not Unread. |
| Approval / engine blocked pane / native plan wait | The turn mark, Approval, Needs You until answered, including after reading. The second line follows `expected_reply`, native question, then `progress`; a plan document's heading is never extracted as a sentence. | The same turn mark, Approval and Needs You; without a native question sentence the word Approval stands in, including for a plan document. |
| AI-identified error or block with a named cause, without a native wait | The turn mark, Blocked, Needs You while unread; the second line is the cause in `progress`. Reading leaves the dimmed mark and cause in Seen unless it is running. | No cause is inferred from plain text and no Blocked mark is invented. The engine's own lifecycle decides the group: an unread `done` is Done with no sentence, an `idle` is Seen. A native blocked pane still uses Approval above. |
| Working second line | `progress`; with an unresolved demand, `expected_reply`, native question, then `progress`. A quiet working row's display can hide its sentence until hover or selection. | No generated progress; Working stands in when there is no sentence. A working row with a native question still carries that question. Proven background tasks, working children and replies still being made still supply Waiting independently of AI. |
| Unread (unread) mark and second line | The green dot only for a finished verdict, the plain dot for work left or an unproven wait; the line leads with `끝남` or `일 남음` (an unproven wait is work left, D-17) and then `expected_reply`, otherwise `progress`, ending with `· 백그라운드 N` when a finished turn left background tasks (D-19), and with `· 자식 N개 작업 중` when a root's finished turn not read yet has children still working. A turn that ended on its pull request's running checks is in progress until they end, and their end is the line. | The plain dot, no line; the word Stopped is said only on hover, in accessible names and in notifications. No inferred unfinished verdict; proven waits, running checks included, still prevent a premature stop. |
| Seen second line | A read unresolved demand keeps its sentence; a read completion, idle or unknown row has none. | A native unresolved question keeps its question; ordinary read completion, idle and unknown rows have no sentence. |
| Whether a root enters Needs You | Native question/approval/block, an unread AI question or named block, or one of the explicit delegated escalations. Working and Done alone do not enter it. | Native question/approval/block and the same explicit escalations; a plain-text end question alone does not enter it. |
| App icon count | Sum of root Needs You rows while its desktop switch is on and unread Done rows while its own switch is on, across devices and including folded rows. Reading Done removes it from the count. | The same count rule, with fewer inferred demands. macOS uses the Dock, Linux a supported launcher, Windows a taskbar dot with the count in its description. |
| Desktop notification title | The root's row title, capped by the host at 120 characters. | The root's row title by the off priority above, with the same cap. |
| Desktop notification body | With `detail`: translated row `status_code` word ` · ` exact `detail` ` · ` project. A View to look at is `look`, worded as the eye mark, with the newest View as `detail` (`To look at · report.html · my-app`). Examples: `Question · Choose staging? · my-app`, `Approval · Review the plan · my-app`, `Blocked · Login required · my-app`, `Done · Build fixed · my-app`, `Unfinished · Three tests remain · my-app`, `Stopped · my-app` for a stop no label judged. Without `detail` the row's word and the project (`Approval · my-app`); a frame with no `status_code` says its section (`Needs You · my-app`, `Unread · my-app`). | The same composition. Native question: `Question · Choose staging? · my-app`. Approval without a sentence: `Approval · my-app`. Plain-text end question on an unread completed turn: `Done · my-app`. |
| Phone push title and body | Title is the same root row title. With `detail`, the phone language's row status word, exact sentence and project use the desktop format. Without `detail`, the row's word and the project; a payload with no `status_code` says the section (`내 차례 · my-app`, `읽을 것 · my-app` in Korean). | The same composition from native facts only. Native question in Korean: `질문 · Choose staging? · my-app`; approval without text: `승인 · my-app`; a plain-text question on an unread completed turn: `끝남 · my-app`. |

The status prefix is the row's translated `status_code`, not a new classification: English uses Question, Approval, Blocked, Done and Stopped; Korean uses 질문, 승인 대기, 막힘, 완료 and 멈춤.
An empty project omits its separator, and the desktop host's existing 400-character body cap still applies.
Both transports copy the root's published `detail`, even when the phone's compact row hides that detail; neither recomputes a sentence or takes a descendant's text as its root's.
Each desktop group has its own switch: Needs You enables demands and its icon count, Done enables completions and its unread icon count, and either enables the permission/test row; the phone keeps its independent push mode.
Only a transition into an enabled group announces anything: a sentence changing by itself sends no second notification, and replacement tags, withdrawal, the desktop backlog of 16 and each channel's existing mode and focus rules are unchanged.

This is the operator-approved reversal (2026-10-13) of mobile-companion's “never a sentence” rule: a phone lock-screen preview can now show the agent's or generated sentence, according to the phone's preview setting.
First-run-onboarding D-11 changes with it: both channels use the same status/sentence/project order, while their existing no-sentence translations remain intact.
The payload is encrypted, terminal content is still excluded, and neither transport logs the sentence.
The phone's serialized JSON is limited to 3,993 bytes before encryption; the 86-byte aes128gcm header, one-byte delimiter and 16-byte authentication tag make the largest accepted HTTP body exactly 4,096 bytes.
An oversized payload, including project, identifiers and the existing `clear` list, fails delivery before any POST with a fixed size-limit diagnostic; the sentence is never silently shortened to fit.
The status and sentence fields are additive: a cached old worker ignores them and keeps the previous body; a new worker with an older page's cached words falls back to that page's group word, and with no stored words it can show only the sentence and project.
Human delivery notices keep their existing words and carry no row sentence.
Regression owners: `hided/src/desktop_notices.rs`, `hided/src/mobile/push.rs`, `web/src/agentNotices.test.tsx`, `web/src/mobile/language.test.tsx` and `web/src/mobile/serviceWorker.test.ts`.

### Scope projections

`agent_state/tally/scope.rs` publishes `agent_scope` on the navigator, each device, each project and each checkout.
Physical group totals preserve every reported row; Overview members use the first checkout owner and deduplicate a pane within each project, while checkout marks retain the existing last-owner rule.
Physical row references include an occurrence index so duplicate pane IDs keep their distinct labels and states; Overview and task membership retain the first source occurrence.
The retained request projection keeps its previous internal scope semantics.
The device's tasks separately take delegated children only under their root.
Disconnected devices have empty physical totals but retain their last Overview members, matching the existing rail and board behavior.
The scope cache compares owned agent rows, device connection facts and checkout membership and summaries; it restores cached values after a catalog rebuild and recomputes only when those inputs change.
The frozen screen counts are asserted by `agent_state::tally::scope_tests::physical_groups_root_headings_and_requests_keep_the_frozen_screen_values`; ownership, unchanged projection and disconnect retention are asserted by `runtime::tests::agent_scopes`.

`agent_state/tally/lineage.rs` projects the list headings, the sidebar tree, folded checkout badges and checkout trees with their two card representatives.
The sidebar uses the connected-device tree.
A done descendant turns a card yellow only when it is a root relative to that checkout.
Folded checkout lines carry status-priority tiers; the browser keeps its existing locale-aware alphabetical placement inside a tier so Korean and English labels keep their displayed order.
The cross-checkout counts, relative-root highlight and fold transitions are asserted by `runtime::tests::agent_scopes::checkout_trees_and_folded_badges_preserve_cross_checkout_lineage_and_priority`.

`agent_state/work/board.rs` owns the PR list's branch and maker association, issue chip source, ancestor rows, attention ordering, groups and open count.
The maker remains listed after moving to other work, but only the branch's agents can make the PR read as fixing or needing review.
`runtime::tests::agent_scopes::pr_board_keeps_branch_turn_separate_from_its_maker_and_tracks_issue_changes` pins that distinction and GitHub-only invalidation.
`agent_state/work.rs::row_work` supplies the selected PR, additional-PR count and ordered issue keys for expanded and folded request rows.
It preserves live-PR-first selection, checkout-task precedence and issues closed after the request; issue facts and checkout task changes invalidate the scope cache.
The agent palette receives its checkout and closing-issue keys from `tally/relations.rs`.
The device scope also carries raised sections (five Needs You rows and three Done rows before overflow) and each numbered agent's first checkout owner.
`runtime::tests::agent_scopes::raised_sections_keep_five_questions_three_completions_and_first_number_owner` pins those limits.

`agent_state/tally/close.rs` publishes the existing pane, tab, checkout and project close targets' confirmation decision, stop-work rows and outside-descendant lists and counts.
The first unknown target still takes priority over confirmation; a subtree sheet includes every target while counting only descendants outside it.
The shell chooses the current target and translates the published states; runtime close enforcement is unchanged.
A removal dialog whose target has left the catalog keeps its existing result without requesting live close consequences.
`runtime::tests::agent_scopes::close_consequences_keep_unknown_priority_and_outside_descendant_counts` pins these rules and pane-only invalidation.

`agent_state/tally/tasks.rs` publishes each project's tasks with the scope: cards in state order, their members on every device root first, the devices and projects they delegated into, the incoming entries of another device's roots, and the Needs You count.
A pane is placed once, at its first occurrence across the local and last-known remote catalogs, so the same task is never counted on two devices, and a child whose parent's machine cannot be resolved is a root of its own.
`runtime::tests::agent_scopes::a_task_is_its_root_and_every_descendant_with_their_projects_and_devices` and `runtime::tests::agent_scopes::tasks_stand_by_state_and_fold_a_merged_worktree_as_cleanup` pin these rules.

`tally.rs` also owns local and remote device counts and project-removal running-agent totals.
Device scopes publish the current listed rows by source index, preserving duplicate physical rows and local-first order, and the first checkout place for each pane.
The overall scope concatenates connected listings; a disconnected device keeps its catalog places but marks them unavailable to the live sidebar.
The PR board publishes its turn/fixing/blocked counts and the review/draft/finished-agent breakdown, including the existing precedence of a finished agent over draft status.
The scope ownership/disconnect and PR-board tests above assert these values.

The command palette reads `tally/relations.rs` groups and row depths for the selected agent, including its in-checkout ancestors, outside parent and descendants in other checkouts.
It translates their tags and resolves PR/issue labels without rebuilding lineage membership.
The relation fixture in `runtime::tests::agent_scopes` covers both the parent caption and the cross-checkout child group.

`turn.rs::AgentUse` and `tally/cleanup.rs` retain checkout-removal use counts, including the last known busy descendants on disconnected devices.
Their unknown-state and per-ancestor counting rules remain distinct from live close-sheet consequences.
The existing cleanup tests in `runtime::tests::lineage` continue to exercise the public runtime entrypoint.
Delegated-tab placement, active-agent project context, request descendant-question counts and the cleanup row's working indicator also read module-owned answers.
