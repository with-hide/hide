# 0034. Stored primary checkout

- Status: Accepted
- Date: 2026-09-27
- Code: `hide-core/src/runtime/projects.rs`, `web/src/snapshot.ts`

## Decision

### Stored primary checkout

A local registered project stores `primary_checkout_id` on its `WorkspaceRegistration` in `core-state.json`.
An absent choice keeps the root default for existing registrations; a stored choice remains stored while its checkout is unavailable.
The single `set_primary_checkout` event carries `{workspace_id, checkout_id}`, sent by a checkout row's `Set as default checkout`, and accepts only an existing checkout of that registered local Git project.
Missing projects or checkouts, plain folders and remote registrations are refused with a diagnostic, without changing the selection or presenting an alert.
The catalog projects `CheckoutSnapshot.is_primary` after assigning registration identities and adding Git worktrees; sorting and inactive folding consume that flag, and the web reads it for the house glyph rather than comparing paths.
A change updates the accepted catalog immediately, so stale worker results cannot restore the old choice, then uses the existing coalesced off-lock UI save.
An unchanged choice publishes and saves nothing; there is no additional worker, subprocess, timer or queue.
Checkout fields remain in the hand-maintained `web/src/snapshot.ts` shape; the WebSocket frame generator does not generate this part of the core snapshot.
SSH catalogs currently read the device's session and the device helper, not another hided core's registrations, so they retain the remote root default and do not synchronize a primary choice stored by another daemon.
