# Release 1.2.1 fixtures

What the 1.2.1 daemon wrote and sent, kept so a test proves that a name renamed in the code since still reads and writes as 1.2.1 did.
They come from an isolated run of the released 1.2.1 `hided` (its own state folder under `/tmp` and a private HOME), with one project, two named tabs, a split and a stand-in Claude agent on a labelled session.

- `hook-facts.json`: the hook facts file, as written.
- `core-state-moved.json`: the UI store, as written, with `focus_from_herdr: true` added as a 1.2.1 core move's rekey adds it.
- `snapshot.json`: from one snapshot frame, the `status` section (lists and machine-specific values emptied), the first strip entry and the agent's pane.
- `owner-change.json`: a core move's owner change in the shape 1.2.1's `OwnerChange` serializes, written by hand, since the run made no move.
