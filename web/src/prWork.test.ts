// The PRs lens's join of the core's board with the pull requests and what it
// says of GitHub reading (PRD work-tab-pr-lens B39, B41, B42, B43): the core
// decided the groups; this only gathers them and words how reading stands.

import { describe, expect, it } from "vitest";
import { blockedLines, hasPullRequests, prListModel, prReading, readingOfStatuses, readingWarning, type PrReading } from "./prWork";
import { prsStatusLines } from "./mobile/prModel";
import { initializeInterfaceI18n } from "./i18n/instance";
import type { PrBoard } from "./agentScope";
import type { GithubStatus, HostDeclaration, Workspace } from "./snapshot";

const t = initializeInterfaceI18n("en").getFixedT(null, "translation");
const status = (over: Partial<GithubStatus>): GithubStatus => ({ available: true, ...over }) as GithubStatus;

function project(id: string, github: Partial<GithubStatus>, board: PrBoard, numbers: number[]): Workspace {
  return {
    id,
    is_git: true,
    checkouts: [{ github: status(github) }],
    pull_requests: numbers.map((number) => ({ number, title: `PR ${number}` })),
    agent_scope: { prs: board },
  } as unknown as Workspace;
}

const row = (number: number, group: PrBoard["rows"][number]["group"], over: Partial<PrBoard["rows"][number]> = {}): PrBoard["rows"][number] => ({
  number,
  group,
  fold: null,
  reason: { kind: "merge" },
  agents: [],
  agent_pane: null,
  checkout_id: null,
  checkout_ids: [],
  issue: null,
  ...over,
});

describe("prReading", () => {
  const NOW = 10 * 60_000;

  it("blocks on a refusal of gh itself before anything was read", () => {
    expect(prReading([project("a", { failure_category: "not_logged_in" }, { rows: [], groups: [], needs_you: 0 }, [])], NOW)).toMatchObject({ state: "blocked", category: "not_logged_in" });
    expect(prReading([project("a", { failure_category: "outdated" }, { rows: [], groups: [], needs_you: 0 }, [])], NOW)).toMatchObject({ state: "blocked", category: "outdated" });
  });

  it("is reading until some project answered, then ready", () => {
    const board = { rows: [], groups: [], needs_you: 0 };
    expect(prReading([project("a", {}, board, [])], NOW)).toMatchObject({ state: "reading" });
    expect(prReading([project("a", { last_success_at_unix_ms: 1000 }, board, [])], NOW)).toMatchObject({ state: "ready" });
  });

  it("keeps the last values and says how old they are when the last read failed", () => {
    const board = { rows: [], groups: [], needs_you: 0 };
    expect(prReading([project("a", { last_success_at_unix_ms: 4 * 60_000, stale: true }, board, [])], NOW)).toMatchObject({ state: "stale", minutes: 6, unread: [] });
  });

  it("is partial, not stale, when the list was read and an extra question was not answered", () => {
    const board = { rows: [], groups: [], needs_you: 0 };
    const read = { last_success_at_unix_ms: 4 * 60_000 };
    expect(prReading([project("a", { ...read, unread_queries: ["merge_queue"] }, board, [])], NOW)).toMatchObject({ state: "partial", unread: ["merge_queue"] });
    // Two projects name the questions once each, in the order the core asks them.
    expect(prReading([project("a", { ...read, unread_queries: ["merge_queue"] }, board, []), project("b", { ...read, unread_queries: ["account", "merge_queue"] }, board, [])], NOW)).toMatchObject({ state: "partial", unread: ["account", "merge_queue"] });
  });

  it("keeps naming the unanswered questions beside a failed read", () => {
    const board = { rows: [], groups: [], needs_you: 0 };
    expect(prReading([project("a", { last_success_at_unix_ms: 4 * 60_000, stale: true, unread_queries: ["review_requests"] }, board, [])], NOW)).toMatchObject({ state: "stale", minutes: 6, unread: ["review_requests"] });
  });
});

describe("each host's reading", () => {
  const NOW = 10 * 60_000;
  const declared = (key: string, name: string, program: string): HostDeclaration => ({ key, name, operations: [], access: { program, minimum_version: "1", sign_in: `${program} auth login` } });
  const github = declared("github", "GitHub", "gh");
  const gitlab = declared("gitlab", "GitLab", "glab");
  const answered = { last_success_at_unix_ms: 4 * 60_000 };
  const both = (first: GithubStatus, second: GithubStatus) => [readingOfStatuses([first, second], NOW), readingOfStatuses([second, first], NOW)];
  const glabHost = (reading: PrReading) => ({ key: "gitlab", name: "GitLab", program: "glab", reading });

  it("names a host refused before its first read beside one that answered, in either order", () => {
    for (const reading of both(status({ ...answered, host: github }), status({ failure_category: "not_logged_in", host: gitlab }))) {
      expect(reading).toEqual({ state: "ready", hosts: [glabHost({ state: "blocked", category: "not_logged_in" })] });
      expect(readingWarning(reading, t)).toBe("Not signed in to glab");
      expect(prsStatusLines(reading, t)).toEqual(["Not signed in to glab"]);
    }
  });

  it("names a host whose first read failed beside one that answered, with no age, in either order", () => {
    for (const reading of both(status({ ...answered, host: github }), status({ failure_category: "network_or_rate_limit", host: gitlab }))) {
      expect(reading.hosts).toEqual([glabHost({ state: "stale", minutes: null, unread: [] })]);
      expect(readingWarning(reading, t)).toBe("GitLab read failed · last value");
      expect(prsStatusLines(reading, t)).toEqual(["Couldn't read GitLab · retrying"]);
    }
  });

  it("names a host whose first read was cut at the byte cap, by its state and with no age, in either order", () => {
    // hide-core sends a cut answer as a stale read with its reason and no category.
    for (const reading of both(status({ ...answered, host: github }), status({ stale: true, host: gitlab }))) {
      expect(reading.hosts).toEqual([glabHost({ state: "stale", minutes: null, unread: [] })]);
      expect(readingWarning(reading, t)).toBe("GitLab read failed · last value");
    }
  });

  it("names a host that failed after a success with its age, and the other host's partial read", () => {
    const [reading] = both(status({ ...answered, unread_queries: ["account"], host: github }), status({ ...answered, stale: true, host: gitlab }));
    expect(reading).toEqual({
      state: "stale",
      minutes: 6,
      unread: ["account"],
      hosts: [
        { key: "github", name: "GitHub", program: "gh", reading: { state: "partial", unread: ["account"] } },
        glabHost({ state: "stale", minutes: 6, unread: [] }),
      ],
    });
    expect(readingWarning(reading!, t)).toBe("GitHub read incomplete · could not read the signed-in account · GitLab read failed · value from 6 min ago");
    expect(prsStatusLines(reading!, t)).toEqual(["Couldn't read GitLab · retrying"]);
  });

  it("draws no line for a host still reading its first answer beside one that answered, in either order", () => {
    for (const reading of both(status({ ...answered, host: github }), status({ loading: true, host: gitlab }))) {
      expect(reading).toEqual({ state: "ready" });
      expect(readingWarning(reading, t)).toBeNull();
      expect(prsStatusLines(reading, t)).toEqual([]);
    }
  });

  it("blocks only when every host is refused, naming each", () => {
    const reading = readingOfStatuses([status({ failure_category: "outdated", host: github }), status({ failure_category: "not_installed", host: gitlab })], NOW);
    expect(reading.state).toBe("blocked");
    expect(blockedLines(reading as PrReading & { state: "blocked" }, t)).toEqual(["gh is too old; update it", "glab isn't installed"]);
  });

  it("names a host alone by its own declaration", () => {
    const blocked = readingOfStatuses([status({ failure_category: "not_installed", host: gitlab })], NOW);
    expect(blocked.state).toBe("blocked");
    expect(blockedLines(blocked as PrReading & { state: "blocked" }, t)).toEqual(["glab isn't installed"]);
    const stale = readingOfStatuses([status({ ...answered, stale: true, host: gitlab })], NOW);
    expect(readingWarning(stale, t)).toBe("GitLab read failed · value from 6 min ago");
    expect(prsStatusLines(stale, t)).toEqual(["Couldn't read GitLab · retrying"]);
    expect(readingWarning(readingOfStatuses([status({ ...answered, unread_queries: ["reviews"], host: gitlab })], NOW), t)).toBe("GitLab read incomplete · could not read reviews");
    // Alone, a first failure is still reading, as one host was before hosts.
    expect(readingOfStatuses([status({ failure_category: "network_or_rate_limit", host: gitlab })], NOW)).toEqual({ state: "reading" });
    expect(readingOfStatuses([status({ ...answered, host: gitlab })], NOW)).toEqual({ state: "ready" });
  });

  it("draws main's lines for GitHub alone, declared or not, an old payload mixed in", () => {
    for (const host of [undefined, github]) {
      const blocked = readingOfStatuses([status({ failure_category: "not_logged_in", host })], NOW);
      expect(blocked.state).toBe("blocked");
      expect(blockedLines(blocked as PrReading & { state: "blocked" }, t)).toEqual(["Not signed in to gh"]);
      const stale = readingOfStatuses([status({ ...answered, stale: true, unread_queries: ["merge_queue"], host })], NOW);
      expect(stale).toMatchObject({ state: "stale", minutes: 6, unread: ["merge_queue"] });
      expect(readingWarning(stale, t)).toBe("GitHub read failed · value from 6 min ago · could not read the merge queue");
      expect(prsStatusLines(stale, t)).toEqual(["Couldn't read GitHub · retrying"]);
      const partial = readingOfStatuses([status({ ...answered, unread_queries: ["account"], host })], NOW);
      expect(readingWarning(partial, t)).toBe("GitHub read incomplete · could not read the signed-in account");
      expect(prsStatusLines(partial, t)).toEqual([]);
      expect(readingOfStatuses([status({ ...answered, host })], NOW)).toEqual({ state: "ready" });
      expect(readingOfStatuses([status({ host })], NOW)).toEqual({ state: "reading" });
    }
    // An old payload beside a declared GitHub is one host.
    expect(readingOfStatuses([status({ failure_category: "not_logged_in" }), status({ ...answered, host: github })], NOW)).toEqual({ state: "ready" });
    expect(readingWarning(readingOfStatuses([status({ ...answered, stale: true }), status({ ...answered, host: github })], NOW), t)).toBe("GitHub read failed · value from 6 min ago");
  });
});

describe("readingWarning", () => {
  it("is quiet while GitHub reads whole, before an answer and when gh is refused", () => {
    expect(readingWarning({ state: "ready" }, t)).toBeNull();
    expect(readingWarning({ state: "reading" }, t)).toBeNull();
    expect(readingWarning({ state: "blocked", category: "not_installed" }, t)).toBeNull();
  });

  it("names each unanswered question in plain words", () => {
    expect(readingWarning({ state: "partial", unread: ["account"] }, t)).toBe("GitHub read incomplete · could not read the signed-in account");
    expect(readingWarning({ state: "partial", unread: ["review_requests"] }, t)).toBe("GitHub read incomplete · could not read review requests");
    expect(readingWarning({ state: "partial", unread: ["merge_queue"] }, t)).toBe("GitHub read incomplete · could not read the merge queue");
    // The facts query fails whole: the queue and the reviewers are named together (PRD pr-review-status B15).
    expect(readingWarning({ state: "partial", unread: ["merge_queue", "reviews"] }, t)).toBe("GitHub read incomplete · could not read the merge queue, reviews");
    expect(readingWarning({ state: "partial", unread: ["account", "review_requests", "merge_queue"] }, t)).toBe("GitHub read incomplete · could not read the signed-in account, review requests, the merge queue");
  });

  it("keeps how old a failed read's list is, and adds the questions it also lacks", () => {
    expect(readingWarning({ state: "stale", minutes: 6, unread: [] }, t)).toBe("GitHub read failed · value from 6 min ago");
    expect(readingWarning({ state: "stale", minutes: null, unread: [] }, t)).toBe("GitHub read failed · last value");
    expect(readingWarning({ state: "stale", minutes: 6, unread: ["merge_queue"] }, t)).toBe("GitHub read failed · value from 6 min ago · could not read the merge queue");
  });
});

describe("hasPullRequests", () => {
  it("is false for a folder that is not Git or has no GitHub remote", () => {
    expect(hasPullRequests({ is_git: false, checkouts: [] } as unknown as Workspace)).toBe(false);
    expect(hasPullRequests({ is_git: true, checkouts: [{ github: { failure_category: "no_github_remote" } }] } as unknown as Workspace)).toBe(false);
  });
});

describe("prListModel", () => {
  const board: PrBoard = {
    rows: [row(1, "needs_you", { checkout_ids: ["c1"] }), row(2, "needs_you", { fold: "team", checkout_ids: ["c1"] }), row(3, "waiting", { checkout_ids: ["c2"] }), row(4, "merged")],
    groups: [{ group: "needs_you", numbers: [1, 2] }, { group: "waiting", numbers: [3] }, { group: "merged", numbers: [4] }],
    needs_you: 1,
  };
  const scope = [{ workspace: project("p", { last_success_at_unix_ms: 1 }, board, [1, 2, 3, 4]), agents: [], device: null }];

  it("keeps the core's groups in the screen order and folds team asks out of the rows and the count", () => {
    const model = prListModel(scope, null, 0);
    expect(model.groups.map((group) => group.group)).toEqual(["needs_you", "waiting", "merged"]);
    expect(model.groups[0]!.entries.map((entry) => entry.pr.number)).toEqual([1]);
    expect(model.groups[0]!.team.map((entry) => entry.pr.number)).toEqual([2]);
    expect(model.needsYou.map((entry) => entry.pr.number)).toEqual([1]);
    expect(model.total).toBe(4);
  });

  it("narrows to one checkout by the ids the core named", () => {
    const model = prListModel(scope, "c2", 0);
    expect(model.groups.flatMap((group) => group.entries.map((entry) => entry.pr.number))).toEqual([3]);
    expect(model.needsYou).toEqual([]);
  });

  it("throws on a core row without its pull request instead of drawing a hole", () => {
    const broken = [{ workspace: project("p", {}, board, [1]), agents: [], device: null }];
    expect(() => prListModel(broken, null, 0)).toThrow(/without its pull request/);
  });
});
