// Pull requests as the phone receives them (`hided/src/mobile/prs.rs`), for
// the phone's own tests: one open pull request that can merge, and the card
// the core read for it, changed one field at a time.

import type { PrReasonKind } from "../agentScope";
import type { PhonePr, PhonePrCard, PhonePrProject } from "./protocol";

export const HEAD = "a".repeat(40);

export const project = (over: Partial<PhonePrProject> = {}): PhonePrProject => ({
  id: "p1",
  label: "with-hide/hide",
  device_id: "local",
  device_label: null,
  github: [{ failure_category: null, available: true, loading: false, stale: false, last_success_at_unix_ms: 1_000 }],
  ...over,
});

export const card = (over: Partial<PhonePrCard> = {}): PhonePrCard => ({
  reading: false,
  head: HEAD,
  files: [
    { path: "hide-platform/src/path.rs", additions: 6, deletions: 2, generated: false },
    { path: "hided/src/mobile/wire.rs", additions: 71, deletions: 8, generated: false },
  ],
  files_total: 2,
  merge: { methods: ["squash", "merge"], queue: false, can_merge: true },
  failed: [],
  write: null,
  ...over,
});

/** An open pull request of the operator's own that is clean and approved: the board says `merge`. */
export function pr(number: number, over: Partial<PhonePr> = {}): PhonePr {
  return {
    key: `p1#${number}`,
    project: "p1",
    number,
    title: `Change ${number}`,
    url: `https://github.com/with-hide/hide/pull/${number}`,
    badge: "open",
    review: "approved",
    is_draft: false,
    checks: "passing",
    state: "mergeable",
    head_branch: `feat/change-${number}`,
    base_branch: "main",
    work: {
      author: "me",
      author_is_bot: false,
      mine: true,
      merge_state: "clean",
      review_requests: [],
      asked_of_me: "none",
      failed_checks: [],
      pending_checks: 0,
      check_counts: { total: 3, passed: 3, failed: 0, running: 0, waiting: 0, skipped: 0, unknown: 0 },
      queue: { state: "not_queued" },
      review_requested_at_unix_ms: null,
      reviewers: [{ login: "tansfil", team: false, state: "approved", at_unix_ms: null }],
      reviewers_more: 0,
      required_approvals: 1,
    },
    group: "needs_you",
    reason: { kind: "merge" },
    agents: [],
    checkout: true,
    card: card(),
    ...over,
  };
}

/** The same pull request standing for another reason, as the board would decide it. */
export function standing(number: number, kind: PrReasonKind, over: Partial<PhonePr> = {}): PhonePr {
  return pr(number, { reason: { kind }, ...over });
}
