# 0058. Herdr an earlier Hide left on a machine

- Status: Accepted
- Date: 2026-11-04
- Code: `hide-host/src/kit.rs`, `hide-core/src/herdr_import.rs`

## Decision

### Herdr an earlier Hide left on a machine

No package carries Herdr, and nothing Hide runs starts, adopts or hands over a Herdr server: every pane runs in its holder on the core's own engine ([The pane holder](0010-pane-holder.md#the-pane-holder)).
What an earlier Hide left is handled in two places, each a read path for an operator who skipped versions: the import of the Herdr session's saved work into the engine ([Importing a Herdr session into the core's own engine](0013-core-own-engine.md#importing-a-herdr-session-into-the-cores-own-engine)), and the records and files an earlier build wrote that this build still reads ([agent-hooks.md](../agent-hooks.md#herdrs-integration-an-earlier-hide-installed)).
This build takes nothing of Herdr's off a machine: an earlier build's Herdr copies under `<helper root>/herdr` on a device and its Herdr integration entries in agent hook files stay where they are, and the removal of a device or of Hide does not touch them (`hide-host/src/kit.rs`, `hide_kit::record`).
A device helper of an earlier build kept its pinned Herdr in `<helper root>/herdr/<id>/herdr`; the folder is left for the operator to delete once no Herdr server runs from it.
