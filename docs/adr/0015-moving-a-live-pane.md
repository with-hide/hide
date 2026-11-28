# 0015. Moving a live pane

- Status: Accepted
- Date: 2026-10-21
- Code: `hide-core/src/runtime/pane_move.rs`, `hide-core/src/runtime/pane_move_preview.rs`

## Decision

### Moving a live pane

`runtime/pane_move.rs` admits one typed intent with captured checkout, device, group, source and destination references, reserving both participating tabs before any native mutation.
It saves its intent in the existing Workspace view journal off the render lock and waits for the durable acknowledgement before using the engine's move and swap primitives.
It never reconstructs a terminal by applying a layout (`Engine::apply_layout`).
A same-tab move uses a bounded temporary tab; intermediate topology stays behind the reservation while the terminal projection remains attached.
Creating a destination tab reserves a slot in the same 64-tab admission count as ordinary tab creation, including an unresolved move after reconnect.
Ambiguous answers and recovered pending journals query native placement instead of resending a possibly completed effect.
The final native snapshot remains the affected tabs' publication baseline until the coordinator starts a newer authoritative bootstrap, so a queued older read cannot undo the committed placement or release its terminal.
That bootstrap is a read, never a replay of the move, and its newer topology can retire a terminal another client subsequently closed.
The move follows terminal identity into the pane's new place, and preserves the xterm instance under the new routing address.
The snapshot exposes that native terminal identity alongside the pane address; it is not itself a caller credential.
`runtime/pane_move_preview.rs` reads bounded text on an off-lock worker for an unattached tab, with one running and one latest pending request; it attaches, focuses and resizes nothing.
The shell otherwise copies its cached terminal into an inert preview.
Moves and hover reads use finite request workers holding a weak runtime owner, with no resident polling loop; their admission, deadlines and retained state are described in [the pane movement cost contract](../PERFORMANCE_TESTING.md#pane-movement-cost-contract).
