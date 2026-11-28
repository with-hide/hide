# 0040. Splitting a moved core's two machines

- Status: Accepted
- Date: 2026-11-19
- Code: `hided/src/moved_core.rs`, `hided/src/node_role.rs`

## Decision

### Splitting a moved core's two machines

A node and the core it dials become two Hides, each with a core of its own and paired both ways (PRD device-connect-iroh D-46, B45), the first time M's node reaches C's core on M's own build.
Only a node a window started, which knows its state folder, does it (`node_role::splits`); a node no window started asks C nothing and stays the node it is, and a held screen's `core_link` frame says `splits: false`, which keeps its strip's reconnecting words.
C's side is behind the same switch: `look-ssh` and `accept-ssh` are pairing verbs, so a C without it answers neither and M's node, reading no running core of its build, asks nothing more.
M's node asks before it attaches, on the one sign-in of its dial (`node_role::try_link`, `split_from_core`): the SSH session is signed in first, the split runs on it, and the attach takes the same session when the split did not happen, so a login that asks, a password or a key's passphrase, is answered once for both.
Before anything is asked of C, the node reads the core C runs with `hide device look-ssh`, which changes nothing there and names a running core's node, build and release as its link's hello would (`pairing_ssh::core_on`, `Pairing::itself`).
Only the core the placement names, running M's build, is split from; any other is asked nothing and left to the link's handshake as before: an older core is updated first ([Updating the core](0008-updating-the-core.md#updating-the-core)) and split from on the dial that follows, a newer or another build is refused, and so is another node.
The move was the consent, so no link, words or allow window are shown on either machine.

The split is `moved_core::split`, and every step before its last leaves M the node it was and is the same when run again:

1. M makes or reads its machine key and asks C's Hide for a grant with `hide device accept-ssh`, run as the `hide` beside the core's program the placement record names, under one request id per machine key, so a repeat is answered the grant C saved the first time.
   An answer that does not name the placement's core is refused.
2. M saves C as a host it confirms with (`KnownState::Confirming`) and gives C an active grant in its own peer records, with no core running to hold them: the core role M starts next opens its endpoint on those records (`peer_host::endpoint_wanted`).
3. M records the folder its node ran panes in, `~/.hide/device-node`, as C's work on M (`HelperGuest` with `panes`, the installation those panes name; a folder another paired machine already took stays that machine's).
4. M asks C again with `--grant <id>`: C settles its grant for M and adopts M as a host by that grant (`Ask::AcceptSsh` in `hided/src/pairing.rs`).
   C takes a grant this way only from a machine its core already runs work for, one registered as a node that dials in (`hide_core::dialing_nodes`) or one it already opens, and refuses any other with `device_unknown`.
5. M meets C once on the connection two Hides use (`peer_host::meets`), since an SSH login that works says nothing of that connection: an endpoint of M's own key, opened as its core role would open it (`peer_host::Settings::open`) and closed again before the step ends, dials the addresses C answered and reads on the link that C names a grant for M.
   The meeting settles nothing: M's endpoint answers no stream C opens, C's confirm of the pairing among them, so C's record of M stays unconfirmed and C goes on taking M's link over SSH (a registration becomes a paired machine's only once that machine is confirmed with, [The peer connection](0041-peer-connection.md#the-peer-connection)).
   It proves the path and the two keys; that C's work is taken up on M over that connection is what the two cores do after the commit.
6. M sets `core-placement.json` aside as `moved-out/core-placement.json` (`placement::set_aside`), which is the commit: nothing the move left is deleted.

The link thread then ends and the supervisor's `node_turn` starts the core role in the same process, on the same seat, so M's windows reconnect to M's own core; their address loses the node's name once `/health` answers `core` (`addressForRole` in `web/src/ws.ts`).
M's core dials C and confirms, C's core registers M's connection, and on C the registration M held as a node that dials in becomes the paired machine's device in place (`reconcile_peer_devices`, `LinkOrigin::becomes_peer`, `switch_device_link`): the id, the name and every record that names the device stay.
A node that dialed in held no consent of C's, since it ran C's node itself; the switch gives the registration the consent a paired machine's device has (`register_peer_device`), without which C would never dial M and its work there would stay out of reach.
C's work on M then runs in C's own node there, a `hided` of C's build that M's daemon starts from C's guest install root on the handed-over folder ([A guest's panes on the host](0043-guest-node-on-the-host.md#a-guests-panes-on-the-host)), which takes up the holders as on any new link, so the terminals carry on and nothing closes.
C keeps everything a core holds: projects, missions, letters, work started from M's window while the core was moved, and the phone pairing, which names C's core.
No work is divided between the two cores, since no record says which machine started it.

The panes handed over name M's own installation (`HIDE_INSTALL=app`), not a guest's, so two things keep their `hide` and hooks reaching the node that holds them.
C's node answers as the installation the record names (`hide_host::kit::answer_as`, set once before the node binds its bridge or starts a pane), so its bridge declares it, its answers carry it and the panes it starts name it.
M's own daemon, asked first by such a pane, finds no pane of its own; before it falls back to binding the caller by its checkout it asks whether the caller runs under a live holder of the handed-over folder (`pane_auth::guests_pane`, `hide_kit::panes::live_holders`) and, when it does, refuses the request as another installation's, which sends the caller on to the node's bridge (`hook_request::install`).
Without that, a hook report from one of C's agents on M would be recorded by M's core, which does not run that session.

C's login item is kept through this: `always_on::paired` counts a core that a node still dials as kept on, so the item a move placed is not taken out as an unpaired machine's before M can reach it, and once the two are paired it is kept as any paired machine's.

A split that fails is tried once per connection, as a core update is (`State::split_failed`): M links as before on the session the split ran on, a screen the core draws is told the cause once (`core_link` frame with phase `live` and `split_failed`: `split_not_reached`, `split_refused`, `split_not_connected` when the two did not meet in step 5, or `split_not_saved`), M's own row in Settings › Devices says it with Retry, and the Retry ends the link and tries again (`NodeRole::connect_again` with a screen).
That Retry's dial holds no press, because the screen that pressed is drawn through the link and ends with it: a login that asks stops it as it stops any dial nobody pressed for, and the Retry on that wait signs in for the split and the link together.
While C is not reached at all, the link waits as it always did and the split is asked on the first dial that finds C on this build.
A move that was cut short before 1.2.0 is not split: `moved_core::unfinished` refuses the start and names the release that finishes it, for a move this machine left unfinished (`core-move.json`, `core-handover.json`) and for one it gave up or took back while its other machine still owed a step (`core-move-abandoned.json`), and for any of the three that does not read.
