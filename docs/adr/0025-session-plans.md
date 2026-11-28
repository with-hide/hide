# 0025. Session plans

- Status: Accepted
- Date: 2026-10-23
- Code: `hide-core/src/plans.rs`, `hide-core/src/dag.rs`, `hide-core/src/runtime/plans.rs`

## Decision

### Session plans

A session's own plan (PRD hide-plan) is a core-owned record keyed by the native session, `hide-core/src/plans.rs` for the model and its rules and `runtime/plans.rs` for the commands, the published view, the review question and the hook's summary ([delivery.md: Plans](../delivery.md#plans)).
Missions and plans check their graphs with one module, `hide-core/src/dag.rs`: the key rule, unknown dependencies, cycles, the 64-node cap, the walk upstream and the topmost node by depth; a mission's checks moved there unchanged, and a plan adds only its session key, its review, its change reasons and its update time.
Each `hide task` command is one typed event applied under `Mutex<Runtime>`; the plan is saved through the link worker (`LedgerUpdate::Plan`) into `ledger_plans` beside the session's record in `links.sqlite3`, schema 9, and read back once with `LedgerCall::Plans` when the worker starts, so no file of its own exists; the table is deleted with the session's other rows (`remove_sessions`), and the runtime drops a deleted or pruned session's plan in the same transition; a plan card of it keeps its tab until the operator closes it.
The book holds at most 128 plans, evicting the one updated longest ago whose session runs in no pane.
It is published as its own revisioned rest section, `plans`, one `PlanView` per pane whose session has a plan, made again on the snapshot read only when the book's change count or the panes' sessions moved (`refresh_plans`); a mission node's view carries its worker's `PlanView` (`worker_plan`), and the session record carries the same view, so the map, the chip and the node read one value.
A proposed plan is laid on its agent's row as a question in the local and device pipelines before the read axis and the lineage run (`lay_plan_reviews`), so it rides the ask's band, second line and notification paths with no new state ([status-model.md](../status-model.md#a-plan-waiting-for-review-is-a-question)).
The hook's summary (`PlanContext`, `hide_agent_hooks::plan_record`) is made under the lock from the book, the ledger's registrations and the missions, and rides the answer of the prompt hook's letter pull (`hook_request.rs`) and of every `hide task` command; the pre-tool hook reads only the record those wrote, so no tool call reaches the daemon ([agent-hooks.md](../agent-hooks.md#the-panes-plan-record)).
The map is a card, `DetailItem::Plan { session }`, a desk tab that draws the published section and starts no read.

Handing steps to children (PRD hide-plan-delegate) keeps the same owner and adds no worker.
Every status change, `node` and `set` alike, goes through `plans::judge` under the lock: the order, the evidence each output asks for and a handed step's report; evidence new to a step is not read there.
The command instead answers `PlanTurn::Prove` with what it still owes and the session's node link (`node_link`, the local node or the device's helper), hided runs `PlanProof::run` on its request thread (`gh pr view` through the node's allowlisted `gh`, `GitCommand::HasLocalBranch` for a head that is neither the current branch nor a child's, and `Call::PathFacts`, the same read the catalog uses, 20 s each), and dispatches the command again with the answers; the core judges the plan again as it is then, so nothing read off the lock is applied to a plan that moved, and three rounds bound it.
The spawn's admission (`plan_spawn_admission`) runs in `prepare_delivery` under the lock and hands the ledger the step's link and the main's sleeping children; the ledger counts the cap in its reserve under its own lock, as a mission's cap, so two spawns at once cannot both take the third place.
The B6 wake check reads the same count in `prepare_delivery`, so a spawn and a waking letter that land in the same instant may let a fourth child run until one ends; the cap is counted again at every spawn.
The step's delegation is folded from the published ledger (`fold_delegations`, on `publish_delivery` and when the plans are read back) and saved with the plan, each delegation keeping the last letter id it read, because letters are pruned and the step must say what its child did after they are gone.
`refresh_plans` keys its view on the book's change count, the panes' sessions and the panes and sessions of the children the shown plans handed steps to, so a child that opens, moves or ends remakes the view and a quiet burst does not; a shown plan brings its children's plans with it.
