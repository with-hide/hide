import { emptyScope, legacyRest } from "../test/legacyAgentScope";
import { legacyAgentRow } from "../test/legacyAgentRow";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CHILD, PR_260, RICH, requestWith } from "./gallery/cmdkSceneData";
import { initializeInterfaceI18n } from "./i18n/instance";
import { filterEntries, fuzzyScore, groupEntries, numberQuery, openUrlEntry, recentEntries, searchEntries as drawSearchEntries, type SearchEntry } from "./search";
import type { SnapshotRest } from "./snapshot";

const { t } = initializeInterfaceI18n("en");
const ko = initializeInterfaceI18n("ko").t;

describe("fuzzy score", () => {
  it("needs the query's characters in order", () => {
    expect(fuzzyScore("src/main.rs", "nope")).toBeNull();
    expect(fuzzyScore("src/main.rs", "smr")).not.toBeNull();
  });

  it("scores a tight and word-boundary match above a scattered one", () => {
    expect(fuzzyScore("src/main.rs", "main")!).toBeGreaterThan(fuzzyScore("server/migrations/init.rs", "main")!);
    expect(fuzzyScore("readme.md", "r")!).toBeGreaterThan(fuzzyScore("docs/readme.md", "r")!);
  });
});

const REST = {
  navigator: {
    agents: [
      legacyAgentRow({ id: "a1", pane_id: "p1", identity_label: "Agent one", agent_kind: "claude", mark: "going", group: "working", status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false }),
      legacyAgentRow({ id: "a2", pane_id: "p9", identity_label: "Agent elsewhere", agent_kind: "codex", mark: "quiet", group: "seen", status_code: "idle", detail: "Waiting for review", changed_at_unix_ms: null, emphasized: false, unread: false }),
    ],
    workspaces: [
      { agent_scope: emptyScope(),
        id: "w1",
        label: "fixture",
        path: "/tmp/fixture",
        device_id: "local",
        registered: true,
        temporary: false,
        pinned: false,
        checkouts: [{ agent_scope: emptyScope(), id: "c1", workspace_id: "w1", label: "main", path: "/tmp/fixture", branch: "main", purpose: null, is_worktree: false, exists: true, has_panes: true, pull_request: null, tabs: [{ id: "t1", workspace_id: "w1", checkout_id: "c1", label: "Tab 1", empty: false, delegated: false, panes: [{ id: "p1" }] }], active_tab_id: null, strip: [], next_tab_label: "Tab 2" }],
        inactive_checkouts: { expanded: false, checkout_ids: [] },
      },
    ],
  },
} as unknown as SnapshotRest;

/** The one command every screen has; the tests below read the rows around it. */
const START_AGENT = "command:start-agent";
const without = (entries: SearchEntry[]) => entries.filter((entry) => entry.id !== START_AGENT && entry.kind !== "device");

describe("search entries", () => {
  it("lists agents, projects and checkouts with the ids that activate them", () => {
    const entries = without(searchEntries(REST, t));
    expect(entries.map((entry) => entry.kind)).toEqual(["agent", "agent", "project", "checkout"]);
    expect(entries[0]).toMatchObject({ kind: "agent", paneId: "p1" });
    expect(entries[3]).toMatchObject({ kind: "checkout", workspaceId: "w1", checkoutId: "c1" });
  });

  it("heads each entry under its kind: Agents, Projects, Checkouts (PRD cmdk-navigation B11)", () => {
    const heads = Object.fromEntries(without(searchEntries(REST, t)).map((entry) => [entry.id, t(entry.group.label)]));
    expect(heads).toEqual({
      "agent:p1": "Agents",
      "agent:p9": "Agents",
      "project:w1": "Projects",
      "checkout:c1": "Checkouts",
    });
  });

  it("offers Start an agent… as the only command, on every snapshot (B22)", () => {
    expect(searchEntries(REST, t).filter((entry) => entry.kind === "command")).toEqual([expect.objectContaining({ id: START_AGENT, title: "Start an agent…", subtitle: "Start an agent", command: "start_agent" })]);
    expect(searchEntries(REST, ko).find((entry) => entry.id === START_AGENT)).toMatchObject({ title: "에이전트 시작…", subtitle: "에이전트 시작" });
  });

  it("gives an agent row its mark's kind, where it works and its state sentence, else the status word", () => {
    const [one, elsewhere] = without(searchEntries(REST, t));
    expect(one).toMatchObject({ agentKind: "claude", subtitle: "fixture · Working" });
    // A pane no checkout holds has no place, and none is made up.
    expect(elsewhere).toMatchObject({ agentKind: "codex", subtitle: "Waiting for review" });
  });

  it("filters by the fuzzy score and keeps the best first", () => {
    const group = { id: "projects", label: "overview.projects" } as const;
    const entries: SearchEntry[] = [
      { id: "1", title: "Alpha", subtitle: "/a", kind: "project", group },
      { id: "2", title: "Beta", subtitle: "/b", kind: "project", group },
    ];
    expect(filterEntries(entries, "beta").map((entry) => entry.id)).toEqual(["2"]);
    expect(filterEntries(entries, "").map((entry) => entry.id)).toEqual(["1", "2"]);
  });
});

/** This Mac and `mini` (connected, with a Home and a project) and `build-box` (registered, not connected). */
const TWO_DEVICES = {
  navigator: {
    focused_device_id: "local",
    devices: [
      { id: "local", label: null, kind: "local", state: "local", message: null },
      { id: "mini", label: "mini", kind: "remote", state: "ready", message: null },
      { id: "build-box", label: "build-box", kind: "remote", state: "unavailable", message: null },
    ],
    agents: REST.navigator!.agents,
    workspaces: REST.navigator!.workspaces,
  },
  status: {
    remote: [
      {
        target_id: "mini",
        state: "connected",
        session: {
          agents: [legacyAgentRow({ id: "m1", pane_id: "remote:mini:pane:1", identity_label: "배치 감시", agent_kind: "codex", mark: "going", group: "working", status_code: "working", changed_at_unix_ms: null, emphasized: false, unread: false })],
          workspaces: [
            { agent_scope: emptyScope(),
              id: "remote:mini:workspace:web",
              label: "web",
              path: "/srv/web",
              device_id: "mini",
              inactive_checkouts: { expanded: false, checkout_ids: [] },
              checkouts: [{ agent_scope: emptyScope(), id: "remote:mini:checkout:web", workspace_id: "remote:mini:workspace:web", label: "main", path: "/srv/web", tabs: [{ id: "remote:mini:tab:1", panes: [{ id: "remote:mini:pane:1" }] }] }],
            },
            { agent_scope: emptyScope(), id: "remote:mini:workspace:home", label: "hide", path: "/Users/example/hide", device_id: "mini", is_home: true, inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [] },
          ],
        },
      },
      { target_id: "build-box", state: "not_connected", session: { agents: [legacyAgentRow({ id: "old", pane_id: "remote:build-box:pane:1", identity_label: "stale", group: "seen" })], workspaces: [] } },
    ],
  },
} as unknown as SnapshotRest;

describe("search entries across devices (PRD home-device-rail B40)", () => {
  // This window runs on a Mac, whatever system runs the test (#972 names it by its system).
  beforeEach(() => vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "" }));
  afterEach(() => vi.unstubAllGlobals());
  it("finds every connected device's agents, projects and checkouts and the devices themselves", () => {
    const entries = searchEntries(TWO_DEVICES, t);
    expect(entries.filter((entry) => entry.kind === "agent").map((entry) => entry.title)).toEqual(["Agent one", "Agent elsewhere", "배치 감시"]);
    expect(entries.filter((entry) => entry.kind === "project").map((entry) => entry.title)).toEqual(["fixture", "web"]);
    expect(entries.filter((entry) => entry.kind === "checkout").map((entry) => entry.checkoutId)).toEqual(["c1", "remote:mini:checkout:web"]);
    expect(entries.filter((entry) => entry.kind === "device").map((entry) => entry.title)).toEqual(["This Mac", "mini", "build-box"]);
  });

  it("chips a result only while its device is not the one in front, and never lists a device's Home as a project", () => {
    const fromLocal = searchEntries(TWO_DEVICES, t);
    expect(fromLocal.find((entry) => entry.id === "agent:p1")?.chip).toBeUndefined();
    expect(fromLocal.find((entry) => entry.id === "agent:remote:mini:pane:1")?.chip).toEqual({ label: "mini", local: false });
    expect(fromLocal.some((entry) => entry.title === "hide")).toBe(false);
    const fromMini = searchEntries({ ...TWO_DEVICES, navigator: { ...TWO_DEVICES.navigator, focused_device_id: "mini" } } as SnapshotRest, t);
    expect(fromMini.find((entry) => entry.id === "agent:p1")?.chip).toEqual({ label: "This Mac", local: true });
    expect(fromMini.find((entry) => entry.id === "agent:remote:mini:pane:1")?.chip).toBeUndefined();
  });

  it("lists nothing current from a device that is not connected, beyond the device itself", () => {
    const entries = searchEntries(TWO_DEVICES, t);
    expect(entries.some((entry) => entry.deviceId === "build-box" && entry.kind !== "device")).toBe(false);
  });
});

describe("an agent by its pane's Herdr id", () => {
  // Ids as `herdr pane list` prints them; mini's pane is that device's own, scoped by the core.
  const withIds = (local: string[], mini: string) => {
    const rest = structuredClone(TWO_DEVICES);
    rest.navigator!.agents = local.map((pane, index) => (legacyAgentRow({ ...REST.navigator!.agents![0]!, id: `a${index}`, pane_id: pane, identity_label: `Agent ${index}` })));
    rest.status!.remote![0]!.session!.agents![0]!.pane_id = `remote:mini:pane:${mini}`;
    return searchEntries(rest, t);
  };
  const ranked = (entries: SearchEntry[], query: string) => filterEntries(entries, query).map((entry) => entry.id);

  it("lists the agent in that pane first, on every device that has a pane by that id", () => {
    const entries = withIds(["w9J:p5", "w9J:p52"], "w9J:p52");
    expect(ranked(entries, "w9J:p52").slice(0, 2)).toEqual(["agent:w9J:p52", "agent:remote:mini:pane:w9J:p52"]);
    expect(ranked(entries, " w9J:p52 ")[0]).toBe("agent:w9J:p52");
  });

  it("finds every pane whose id holds a query with a colon, the whole id first", () => {
    const entries = withIds(["w9J:p52", "w9J:p5"], "w9J:p52");
    expect(ranked(entries, ":p5").slice(0, 3)).toEqual(["agent:w9J:p5", "agent:w9J:p52", "agent:remote:mini:pane:w9J:p52"]);
    expect(ranked(entries, "w9J:").filter((id) => id.startsWith("agent:"))).toHaveLength(3);
  });

  it("matches the id in its case, and not at all without a colon", () => {
    const entries = withIds(["w9J:pB", "w9J:pb"], "w60:p12");
    expect(ranked(entries, "w9J:pB")[0]).toBe("agent:w9J:pB");
    expect(ranked(entries, "w9J:pb")[0]).toBe("agent:w9J:pb");
    expect(ranked(entries, "w9j:pb").filter((id) => id.startsWith("agent:"))).toEqual([]);
    expect(ranked(entries, "p12").filter((id) => id.startsWith("agent:"))).toEqual([]);
    expect(ranked(entries, "12").filter((id) => id.startsWith("agent:"))).toEqual([]);
  });
});

describe("an agent by its Herdr name", () => {
  // Names as `hide agent spawn --name` and `herdr agent rename` give them; mini's agent is that device's own.
  const named = (local: string | undefined, mini: string | undefined) => {
    const rest = structuredClone(TWO_DEVICES);
    rest.navigator!.agents = [{ ...REST.navigator!.agents![0]!, herdr_name: local }, REST.navigator!.agents![1]!];
    rest.status!.remote![0]!.session!.agents![0]!.herdr_name = mini;
    return searchEntries(rest, t);
  };
  const agentIds = (entries: SearchEntry[], query: string) => filterEntries(entries, query).flatMap((entry) => (entry.kind === "agent" ? [entry.id] : []));

  it("finds an agent by its Herdr name on this Mac and on a device, and shows the name under the title", () => {
    const entries = named("observer-instant-pane-topology", "nightly-watch");
    expect(agentIds(entries, "observer-instant-pane-topology")).toEqual(["agent:p1"]);
    expect(agentIds(entries, "nightly-watch")).toEqual(["agent:remote:mini:pane:1"]);
    expect(entries.find((entry) => entry.id === "agent:p1")).toMatchObject({ title: "Agent one", subtitle: "observer-instant-pane-topology · fixture · Working" });
    expect(entries.find((entry) => entry.id === "agent:remote:mini:pane:1")).toMatchObject({ title: "배치 감시", subtitle: "nightly-watch · web · Working" });
  });

  it("draws no name for an agent Herdr has none for, and finds nothing by one", () => {
    const entries = named(undefined, undefined);
    expect(agentIds(entries, "observer-instant-pane-topology")).toEqual([]);
    expect(entries.find((entry) => entry.id === "agent:p1")).toMatchObject({ title: "Agent one", subtitle: "fixture · Working" });
    expect(entries.find((entry) => entry.id === "agent:remote:mini:pane:1")).toMatchObject({ subtitle: "web · Working" });
  });

  it("does not repeat a name the title already reads as", () => {
    const entries = named("agent one", undefined);
    expect(entries.find((entry) => entry.id === "agent:p1")).toMatchObject({ title: "Agent one", subtitle: "fixture · Working" });
    expect(agentIds(entries, "agent one")[0]).toBe("agent:p1");
  });
});

describe("grouping (issue 154)", () => {
  const agentsHere = { id: "agents:w1", label: "overview.agents" } as const;
  const agentsThere = { id: "agents:w2", label: "overview.issues" } as const;
  const projects = { id: "projects", label: "overview.projects" } as const;
  const entry = (id: string, group: SearchEntry["group"]): SearchEntry => ({ id, title: id, subtitle: "", kind: "agent", group });

  it("stands each group where its best entry ranked and keeps the rank inside it", () => {
    const ranked = [entry("a", agentsHere), entry("p", projects), entry("b", agentsThere), entry("c", agentsHere), entry("q", projects)];
    const sections = groupEntries(ranked);
    expect(sections.map((section) => section.group.id)).toEqual(["agents:w1", "projects", "agents:w2"]);
    expect(sections.map((section) => section.entries.map((row) => row.id))).toEqual([["a", "c"], ["p", "q"], ["b"]]);
  });

  it("draws no group for no entries", () => {
    expect(groupEntries([])).toEqual([]);
  });

  it("keeps the best match first once grouped", () => {
    const ranked = filterEntries(searchEntries(REST, t), "fixture");
    expect(groupEntries(ranked)[0]?.entries[0]?.id).toBe(ranked[0]?.id);
  });
});

describe("issues and pull requests (PRD cmdk-navigation B11-B13, D-14)", () => {
  const found = (query: string) => filterEntries(searchEntries(RICH, t), query).map((entry) => entry.id);

  it("holds this Mac's issues and pull requests, a pull request once however many places name it", () => {
    const entries = searchEntries(RICH, t);
    expect(entries.filter((entry) => entry.kind === "pr").map((entry) => entry.id)).toEqual(["pr:w1:275", "pr:w1:260"]);
    expect(entries.filter((entry) => entry.kind === "issue").map((entry) => entry.id)).toEqual(["issue:github:acme/hide#273"]);
  });

  it("finds a pull request by number, title or branch and an issue by number or title, not by words only a body would hold (B13)", () => {
    expect(found("#275")).toContain("pr:w1:275");
    expect(found("sandbox refusals")).toContain("pr:w1:275");
    expect(found("sandbox-letters")).toContain("pr:w1:275");
    expect(found("273")).toContain("issue:github:acme/hide#273");
    expect(found("internal")).toContain("issue:github:acme/hide#273");
    expect(found("zzzzz")).toEqual([]);
  });

  it("puts the exact number first, an issue and a pull request of the same number each as its own row (B12)", () => {
    const withTwin = structuredClone(RICH) as typeof RICH;
    withTwin.navigator!.workspaces![0]!.pull_requests = [{ ...(RICH.navigator!.workspaces![0]!.pull_requests![0]!), number: 273, title: "Unrelated 275 title" }];
    const ranked = filterEntries(searchEntries(withTwin, t), "#273").map((entry) => entry.id);
    expect(ranked.slice(0, 2)).toEqual(["issue:github:acme/hide#273", "pr:w1:273"]);
    // A bare number holds only rows whose text holds the digits.
    expect(filterEntries(searchEntries(RICH, t), "273").every((entry) => `${entry.title} ${entry.subtitle}`.includes("273"))).toBe(true);
  });

  it("lists after the pull request the agents the core links to it, and not the ones it does not", () => {
    // The child works on pull request 275 in its checkout; nothing in its name, place or state holds the digits.
    const sections = (rest: typeof RICH, query: string) => groupEntries(filterEntries(searchEntries(rest, t), query)).map((section) => [section.group.id, section.entries.map((entry) => entry.id)]);
    expect(sections(RICH, "#275").slice(0, 2)).toEqual([["pull-requests", ["pr:w1:275"]], ["agents", [`agent:${CHILD.pane_id}`]]]);
    expect(sections(RICH, "275").slice(0, 2)).toEqual([["pull-requests", ["pr:w1:275"]], ["agents", [`agent:${CHILD.pane_id}`]]]);
    // A merged pull request the agent no longer owns is still its link, though its PR mark no longer draws it.
    const tied = structuredClone(RICH) as typeof RICH;
    const settled = requestWith("idle", [PR_260]);
    settled.pull_requests = settled.pull_requests.map((pull) => ({ ...pull, badge: "merged", live: false, duty: false, settled_at_unix_ms: 1 }));
    tied.navigator!.agents = tied.navigator!.agents!.map((row) => (row.pane_id === "p-dag" ? { ...row, request: settled } : row));
    expect(sections(tied, "260").slice(0, 2)).toEqual([["pull-requests", ["pr:w1:260"]], ["agents", ["agent:p-dag"]]]);
    // An issue's number is not a pull request's: the issue row alone.
    expect(sections(RICH, "273")[0]).toEqual(["issues", ["issue:github:acme/hide#273"]]);
    expect(filterEntries(searchEntries(RICH, t), "273").map((entry) => entry.id)).not.toContain(`agent:${CHILD.pane_id}`);
  });

  it("reads #273 and 273 as numbers and nothing else", () => {
    expect(numberQuery("#273")).toBe(273);
    expect(numberQuery(" 273 ")).toBe(273);
    expect(numberQuery("#273a")).toBeNull();
    expect(numberQuery("sandbox")).toBeNull();
  });

  it("carries the state the core decided for each pull request, and names it as the row's status", () => {
    const [open, merged] = searchEntries(RICH, t).filter((entry) => entry.kind === "pr");
    expect([open?.prState, open?.status]).toEqual(["pending", { tone: "muted", label: "CI running or review pending" }]);
    expect([merged?.prState, merged?.status]).toEqual(["merged", { tone: "muted", label: "Merged" }]);
  });

  it("keeps the 80 row cap", () => {
    const many = structuredClone(RICH) as typeof RICH;
    many.navigator!.workspaces![0]!.tasks!.tasks = Array.from({ length: 120 }, (_, index) => ({ key: `github:acme/hide#${index + 1000}`, source: "github", id: `#${index + 1000}`, url: null, title: `sandbox issue ${index}`, open: true }));
    expect(filterEntries(searchEntries(many, t), "sandbox").length).toBe(80);
  });

  it("finds an agent by where it works", () => {
    expect(found("sandbox").includes(`agent:${CHILD.pane_id}`)).toBe(true);
  });
});

// PRD cmdk-recent: the Recent rows are the core's recent checkouts, read
// against the catalogs ⌘K holds. A record is `device:checkout`.
describe("recent entries (PRD cmdk-recent)", () => {
  const record = (device_id: string, checkout_id: string, project_name = "fixture", branch = "main") => ({ device_id, checkout_id, project_name, branch });
  const withRecent = (records: ReturnType<typeof record>[], base: SnapshotRest = TWO_DEVICES) => ({ ...base, ui_state: { recent_checkouts: records } }) as unknown as SnapshotRest;
  const ids = (entries: SearchEntry[]) => entries.map((entry) => entry.checkoutId);

  it("lists the record's checkouts newest first, as the checkout rows, under Recent (B1, B2)", () => {
    const entries = recentEntries(withRecent([record("mini", "remote:mini:checkout:web"), record("local", "c1")]), new Set());
    expect(ids(entries)).toEqual(["remote:mini:checkout:web", "c1"]);
    expect(entries.every((entry) => entry.group.id === "recent" && entry.kind === "checkout")).toBe(true);
    // Branch over project, the device's chip only off the device in front.
    expect(entries[0]).toMatchObject({ title: "main", subtitle: "web", chip: { label: "mini", local: false } });
    expect(entries[1]).toMatchObject({ title: "main", subtitle: "fixture", chip: undefined });
    expect(entries.some((entry) => entry.dimmed)).toBe(false);
  });

  it("leaves out the checkouts already shown and fills the five from the rest of the record (B4)", () => {
    const many = Array.from({ length: 8 }, (_, index) => record("local", `gone-${index}`));
    // Records the catalog does not list are not drawn; only the live ones count toward the five.
    expect(recentEntries(withRecent([record("local", "c1"), ...many]), new Set()).map((entry) => entry.checkoutId)).toEqual(["c1"]);
    expect(recentEntries(withRecent([record("local", "c1"), record("mini", "remote:mini:checkout:web")]), new Set(["c1"])).map((entry) => entry.checkoutId)).toEqual(["remote:mini:checkout:web"]);
  });

  it("shows at most five", () => {
    const base = structuredClone(TWO_DEVICES) as unknown as { navigator: { workspaces: { checkouts: { id: string }[] }[] } };
    const template = base.navigator.workspaces[0]!.checkouts[0]!;
    base.navigator.workspaces[0]!.checkouts = Array.from({ length: 8 }, (_, index) => ({ ...template, id: `c${index}` }));
    const rest = withRecent(Array.from({ length: 8 }, (_, index) => record("local", `c${index}`)), base as unknown as SnapshotRest);
    expect(ids(recentEntries(rest, new Set(["c0"])))).toEqual(["c1", "c2", "c3", "c4", "c5"]);
  });

  it("draws a checkout of a device that is not connected dimmed, from the names the record kept and the device's own row (B9)", () => {
    const entries = recentEntries(withRecent([record("build-box", "remote:build-box:checkout:api", "api", "release")]), new Set());
    expect(entries).toEqual([expect.objectContaining({ title: "release", subtitle: "api", dimmed: true, deviceId: "build-box", chip: { label: "build-box", local: false } })]);
    expect(entries[0]?.workspaceId).toBeUndefined();
  });

  it("is the live row again once the device is connected, and drops a checkout its connected catalog no longer lists (B9, B10)", () => {
    const connected = withRecent([record("build-box", "remote:build-box:checkout:api")], {
      ...TWO_DEVICES,
      status: { remote: [{ target_id: "build-box", state: "connected", session: { agents: [], workspaces: [{ agent_scope: emptyScope(), id: "remote:build-box:workspace:api", label: "api", path: "/srv/api", device_id: "build-box", inactive_checkouts: { expanded: false, checkout_ids: [] }, checkouts: [{ agent_scope: emptyScope(), id: "remote:build-box:checkout:api", workspace_id: "remote:build-box:workspace:api", label: "main", branch: "main", path: "/srv/api", tabs: [] }] }] } }] },
    } as unknown as SnapshotRest);
    const live = recentEntries(connected, new Set());
    expect(live).toEqual([expect.objectContaining({ title: "main", subtitle: "api", workspaceId: "remote:build-box:workspace:api" })]);
    expect(live[0]?.dimmed).toBeUndefined();
    expect(recentEntries(withRecent([record("local", "deleted")]), new Set())).toEqual([]);
  });

  it("has no rows without a record, or for a device that was removed", () => {
    expect(recentEntries(REST, new Set())).toEqual([]);
    expect(recentEntries(withRecent([record("removed-device", "x")]), new Set())).toEqual([]);
  });
});

describe("Open URL in Browser", () => {
  it("reads an http(s) address and a loopback host as a web address to open", () => {
    expect(openUrlEntry("https://example.com/a?b=1", null, t)?.url).toBe("https://example.com/a?b=1");
    expect(openUrlEntry(" http://example.com ", null, t)?.url).toBe("http://example.com");
    expect(openUrlEntry("localhost:5173", null, t)?.url).toBe("http://localhost:5173");
    expect(openUrlEntry("127.0.0.1:3000/app", null, t)?.url).toBe("http://127.0.0.1:3000/app");
  });

  it("leaves a name, a bare host, another scheme and a path to the search", () => {
    for (const query of ["fixture", "agent one", "#273", "example.com", "ftp://example.com", "mailto:a@b.c", "/tmp/fixture"]) {
      expect(openUrlEntry(query, null, t)).toBeNull();
    }
  });

  it("stays listed and dimmed with its reason when it cannot run", () => {
    const entry = openUrlEntry("localhost:5173", "Pages open in the Hide desktop app.", t);
    expect(entry).toMatchObject({ dimmed: true, subtitle: "Pages open in the Hide desktop app." });
    expect(openUrlEntry("localhost:5173", null, t)).toMatchObject({ dimmed: false, subtitle: "http://localhost:5173" });
  });
});

function searchEntries(rest: SnapshotRest | null, ...args: Parameters<typeof drawSearchEntries> extends [unknown, ...infer R] ? R : never) {
  return drawSearchEntries(rest ? legacyRest(rest, rest.navigator?.agents ?? []) : rest, ...args);
}
