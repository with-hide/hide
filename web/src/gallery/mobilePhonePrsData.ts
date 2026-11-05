// Invented pull requests for the gallery's phone scenes (PRD mobile-companion-ui
// B37-B44; doop final board F9 and F10): two that can merge, one an agent is
// working on, one waiting for CI, one with a failed check and one nobody was
// asked to review. Names, numbers and times are fixtures, never operator data.
// Each row stands for the reason the core's board would give it
// (`herdr-core/src/agent_state/work/board.rs`): a merge-ready row has no agent
// beside it, because a finished agent is not listed on a pull request.

import { card, pr, project } from "../mobile/prFixture";
import type { PhonePr, PhonePrFeedback } from "../mobile/protocol";
import type { PrsState } from "../mobile/store";

export const READY_PR = "p1#2090";
export const AGENT_PR = "p1#2091";
/** The agent row of the list that works on the holder PR (`mobilePhoneSceneData.ts`, 홀더 메모리 측정). */
const HOLDER_AGENT = { device_id: "local", pane_id: "w2" };

const MINUTE = 60_000;
const HEAD = "7d3c1f0e9a8b4c2d6e5f10293847a6b5c4d3e2f1";

function counts(total: number, passed: number, running: number, failed = 0) {
  return { total, passed, failed, running, waiting: 0, skipped: 0, unknown: 0 };
}

function open(number: number, title: string, branch: string, over: Partial<PhonePr>): PhonePr {
  return pr(number, { title, head_branch: branch, card: null, ...over });
}

function work(over: Partial<NonNullable<PhonePr["work"]>>): NonNullable<PhonePr["work"]> {
  return { ...pr(1).work!, ...over };
}

export function phonePrs(): PrsState {
  const now = Date.now();
  const mergeable = open(2090, "fix: keep Windows paths on the wire", "fix/win-path-sep", {
    work: work({ check_counts: counts(3, 3, 0) }),
    card: card({
      head: HEAD,
      files: [
        { path: "hide-platform/src/path.rs", additions: 6, deletions: 2, generated: false },
        { path: "hided/src/mobile/wire.rs", additions: 71, deletions: 8, generated: false },
        { path: "hide-platform/tests/it/path.rs", additions: 7, deletions: 2, generated: false },
      ],
      files_total: 3,
    }),
  });
  const notes = open(2085, "docs: draft the 1.2.0 release notes", "docs/release-notes-1-2-0", {
    work: work({ check_counts: counts(3, 3, 0) }),
    card: card({ files: [{ path: "docs/release-notes-1.2.0.md", additions: 41, deletions: 0, generated: false }], files_total: 1 }),
  });
  const holder = open(2091, "refactor: tidy holder version folders", "refactor/holder-folders", {
    group: "working",
    state: "pending",
    checks: "pending",
    reason: { kind: "working", name: "홀더 메모리 측정", since_unix_ms: now - 4 * MINUTE },
    agents: [HOLDER_AGENT],
    work: work({ merge_state: "blocked", pending_checks: 8, check_counts: counts(22, 14, 8) }),
    card: card({
      head: HEAD,
      files: [{ path: "hide-kit/src/holders.rs", additions: 58, deletions: 19, generated: false }, { path: "hide-kit/tests/it/holders.rs", additions: 22, deletions: 4, generated: false }],
      files_total: 2,
    }),
  });
  const rehearsal = open(2093, "test: rehearse the 1.2.0 migration", "test/migration-rehearsal", {
    group: "waiting",
    state: "pending",
    checks: "pending",
    reason: { kind: "ci", count: 19 },
    work: work({ merge_state: "blocked", pending_checks: 19, check_counts: counts(22, 3, 19) }),
  });
  const failed = open(2088, "feat: show children on the phone list", "feat/phone-children", {
    group: "blocked",
    state: "failed",
    checks: "failed",
    review: null,
    reason: { kind: "failed", name: "web-e2e", count: 1 },
    work: work({ merge_state: "blocked", failed_checks: [{ name: "web-e2e", url: "https://ci.example/runs/2088" }], check_counts: counts(22, 21, 0, 1), reviewers: [], required_approvals: 1 }),
  });
  const unreviewed = open(2079, "feat: voice input in the reply bar", "feat/voice-reply", {
    group: "needs_you",
    state: "pending",
    review: "review_required",
    reason: { kind: "assign_reviewer" },
    work: work({ merge_state: "blocked", check_counts: counts(3, 3, 0), reviewers: [], required_approvals: 1 }),
  });
  return {
    projects: [project()],
    items: [mergeable, notes, unreviewed, holder, failed, rehearsal],
    feedback: null,
    omitted: 0,
  };
}

/** What the hand-over sheet's feedback read answers for the failing pull request. */
export function phoneFeedback(requestId: string): PhonePrFeedback {
  return {
    request_id: requestId,
    pr_number: 2088,
    phase: "ready",
    failed_checks: [{ name: "web-e2e", url: "https://ci.example/runs/2088" }],
    change_requests: [{ author: "tansfil", body: "하위 목록은 접어서 보여 주세요." }],
    failure: null,
  };
}
