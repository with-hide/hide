import { describe, expect, it } from "vitest";
import { RICH } from "./gallery/cmdkSceneData";
import { initializeInterfaceI18n } from "./i18n/instance";
import { githubEntries } from "./search";
import { githubRow, hasGithubProject, lastReadWords, ownAnswer, projectRead, projectToRead, startsSearch } from "./searchGithub";
import type { Checkout, GithubSearch, SnapshotRest } from "./snapshot";

const { t } = initializeInterfaceI18n("en");
const ko = initializeInterfaceI18n("ko").t;
const WORKSPACE = RICH.navigator!.workspaces![0]!;
const NEVER_READ = (() => {
  const rest = structuredClone(RICH) as SnapshotRest;
  for (const checkout of rest.navigator!.workspaces![0]!.checkouts) delete (checkout as { github?: unknown }).github;
  return rest;
})();
const front = (rest: SnapshotRest): Checkout => rest.navigator!.workspaces![0]!.checkouts[1]!;

describe("the one project read ⌘K asks for (PRD cmdk-navigation B8, B10, D-12)", () => {
  it("reads the local project in front once, when nothing has answered it", () => {
    const asked = new Set<string>();
    expect(projectToRead(NEVER_READ, front(NEVER_READ), asked)?.id).toBe("w1");
    asked.add("w1");
    expect(projectToRead(NEVER_READ, front(NEVER_READ), asked)).toBeNull();
  });

  it("leaves a project the core has answered alone", () => {
    expect(projectToRead(RICH, front(RICH), new Set())).toBeNull();
  });

  it("reads a node's project in front as its own, never a dialed device's or an Overview-less front (D-29)", () => {
    const onMini = (dialsIn: boolean) => {
      const rest = structuredClone(NEVER_READ) as SnapshotRest;
      const workspace = { ...rest.navigator!.workspaces![0]!, remote_target_id: "mini", device_id: "mini" };
      rest.navigator!.workspaces = [];
      rest.navigator!.focused_device_id = "mini";
      rest.navigator!.devices = [...(rest.navigator!.devices ?? []), { id: "mini", kind: "remote", dials_in: dialsIn } as never];
      rest.status = { ...rest.status, remote: [{ target_id: "mini", state: "connected", session: { workspaces: [workspace] } }] } as never;
      return rest;
    };
    expect(projectToRead(onMini(true), front(NEVER_READ), new Set())?.id).toBe("w1");
    expect(projectToRead(onMini(false), front(NEVER_READ), new Set())).toBeNull();
    expect(projectToRead(NEVER_READ, null, new Set())).toBeNull();
  });
});

describe("a project's read as its checkouts show it (B8, B9)", () => {
  const now = 1_000_000 + 5 * 60_000;
  it("is nothing for a project nobody asked for, even while the source says reading", () => {
    const rest = structuredClone(NEVER_READ) as SnapshotRest;
    rest.navigator!.workspaces![0]!.tasks!.source!.reading = true;
    expect(projectRead(rest.navigator!.workspaces![0]!, new Set(), now, t, "en")).toBeNull();
  });

  it("reads while ⌘K's request has no answer yet", () => {
    expect(projectRead(NEVER_READ.navigator!.workspaces![0]!, new Set(["w1"]), now, t, "en")).toEqual({ state: "reading" });
    expect(projectRead(WORKSPACE, new Set(["w1"]), now, t, "en")).toBeNull();
  });

  it("says a failed read with the last value's age and no reason (B9)", () => {
    const failed = structuredClone(RICH) as SnapshotRest;
    failed.navigator!.workspaces![0]!.checkouts[0]!.github = { failure_category: "network_or_rate_limit", available: false, loading: false, stale: true, last_success_at_unix_ms: 1_000_000, unavailable_reason: "gh timed out" };
    const read = projectRead(failed.navigator!.workspaces![0]!, new Set(["w1"]), now, t, "en");
    expect(read).toEqual({ state: "failed", tooltip: "GitHub read failed · last value: 5 min. ago · reason in logs" });
    expect(projectRead(failed.navigator!.workspaces![0]!, new Set(["w1"]), now, ko, "ko")).toEqual({ state: "failed", tooltip: "GitHub 읽기 실패 · 5분 전 값 · 이유는 로그에" });
    expect(JSON.stringify(read)).not.toContain("timed out");
  });

  it("says when its pull requests and issues were read", () => {
    expect(lastReadWords(WORKSPACE, now, t, "en")).toBe("Last read: 5 min. ago");
    expect(lastReadWords(WORKSPACE, now, ko, "ko")).toBe("5분 전 읽음");
    expect(lastReadWords(WORKSPACE, 1_000_000 + 30_000, t, "en")).toBe("Last read: Just now");
  });
});

describe("the explicit GitHub search (B16-B19)", () => {
  const answer = (patch: Partial<GithubSearch>): GithubSearch => ({ request_id: "r1", query: "sandbox", phase: "ready", results: [], message: null, ...patch });

  it("offers the row only while this Mac has a GitHub project", () => {
    expect(hasGithubProject(RICH)).toBe(true);
    expect(hasGithubProject({ navigator: { workspaces: [] } } as unknown as SnapshotRest)).toBe(false);
  });

  it("words the row for each state", () => {
    expect(githubRow("sandbox", null, t)).toEqual({ state: "idle", label: 'Search GitHub for "sandbox"' });
    expect(githubRow("sandbox", answer({ phase: "working" }), t).state).toBe("working");
    expect(githubRow("sandbox", answer({ phase: "failed" }), t)).toEqual({ state: "failed", label: "GitHub search failed · retry" });
    expect(githubRow("sandbox", answer({}), t)).toEqual({ state: "none", label: "No results on GitHub either" });
    expect(githubRow("sandbox", answer({ message: "github_search.partial" }), t).label).toBe("No results found · only some repositories searched");
    expect(githubRow("sandbox", null, ko)).toEqual({ state: "idle", label: 'GitHub에서 "sandbox" 검색' });
    expect(githubRow("sandbox", answer({ phase: "failed" }), ko)).toEqual({ state: "failed", label: "GitHub 검색 실패 · 다시 시도" });
    expect(githubRow("sandbox", answer({}), ko)).toEqual({ state: "none", label: "GitHub에도 없음" });
  });

  it("does not start the same query again while it runs, and does after a failure (B19)", () => {
    expect(startsSearch(githubRow("sandbox", answer({ phase: "working" }), t))).toBe(false);
    expect(startsSearch(githubRow("sandbox", answer({ phase: "failed" }), t))).toBe(true);
    expect(startsSearch(githubRow("sandbox", null, t))).toBe(true);
  });

  it("drops an answer for another request or another query, so changing the query clears the results (B19)", () => {
    const asked = { requestId: "r1", query: "sandbox" };
    expect(ownAnswer(answer({}), asked, "sandbox")).not.toBeNull();
    expect(ownAnswer(answer({}), asked, "sandbox2")).toBeNull();
    expect(ownAnswer(answer({ request_id: "r2" }), asked, "sandbox")).toBeNull();
    expect(ownAnswer(answer({}), null, "sandbox")).toBeNull();
  });

  it("shows a result a held row already is only once (B17)", () => {
    const results = [
      { kind: "pr" as const, repository: "acme/hide", number: 275, title: "Surface mailbox sandbox refusals", state: "open", url: "https://github.com/acme/hide/pull/275", pr_state: "pending" as const },
      { kind: "pr" as const, repository: "acme/hide", number: 12, title: "Old", state: "merged", url: "https://github.com/acme/hide/pull/12", pr_state: "merged" as const },
    ];
    const held = [{ url: "https://github.com/acme/hide/pull/275" }] as unknown as Parameters<typeof githubEntries>[1];
    const shown = githubEntries(results, held, t);
    expect(shown.map((entry) => entry.id)).toEqual(["github:pr:acme/hide#12"]);
    expect(shown[0]).toMatchObject({ external: true, kind: "pr", url: "https://github.com/acme/hide/pull/12" });
  });
});
