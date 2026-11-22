import { emptyScope } from "../test/legacyAgentScope";
import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "./i18n/instance";
import { hostAccessRows, githubAccessLine as githubAccessLineIn, issueSourceChoices as issueSourceChoicesIn } from "./settings";
import type { Checkout, Device, GithubFailureCategory, GithubStatus, HostDeclaration, TaskSource, Workspace } from "./snapshot";

// Korean is the wording the Settings Issues tab shipped with; the rules read the same under it.
const t = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const githubAccessLine = (access: Parameters<typeof githubAccessLineIn>[0]) => githubAccessLineIn(access, t);
/** GitHub's row, which Settings always draws. */
const githubAccess = (workspaces: Workspace[], devices: Device[]) => hostAccessRows(workspaces, devices).find((row) => row.key === "github")!.access;
const issueSourceChoices = (workspace: Workspace, stored: string | undefined) => issueSourceChoicesIn(workspace, stored, t);

const status = (patch: Partial<GithubStatus>): GithubStatus => ({
  failure_category: null,
  available: true,
  loading: false,
  stale: false,
  last_success_at_unix_ms: null,
  ...patch,
});

const source = (patch: Partial<TaskSource>): TaskSource => ({
  kind: "github",
  label: "GitHub",
  name: "acme/app",
  reading: false,
  failure: null,
  last_read_at_unix_ms: 1,
  chosen: false,
  ...patch,
});

const project = (patch: Partial<Workspace> & { github?: GithubStatus }): Workspace => {
  const { github, ...rest } = patch;
  return { agent_scope: emptyScope(),
    id: "w1",
    label: "app",
    path: "/p/app",
    device_id: "local",
    remote_target_id: null,
    is_git: true,
    registered: true,
    temporary: false,
    pinned: false,
    checkouts: github ? [{ id: "c1", github } as Checkout] : [],
    inactive_checkouts: { expanded: false, checkout_ids: [] },
    ...rest,
  };
};

describe("Issue settings", () => {
  it("reads gh as connected once any project's read succeeded, and otherwise names gh's own refusal first", () => {
    const loggedOut = project({ github: status({ available: false, failure_category: "not_logged_in" }) });
    const offline = project({ id: "w2", github: status({ stale: true, failure_category: "network_or_rate_limit" }) });
    const noRemote = project({ id: "w3", github: status({ stale: true, failure_category: "no_github_remote" }) });
    const healthy = project({ id: "w4", github: status({ last_success_at_unix_ms: 5 }) });

    expect(githubAccess([offline, healthy], [])).toEqual({ state: "connected" });
    expect(githubAccess([offline, loggedOut], [])).toEqual({ state: "failed", category: "not_logged_in" });
    expect(githubAccessLine({ state: "failed", category: "not_logged_in" }).text).toBe("gh 로그인 안 됨");
    // A gh older than Hide's commands need is gh's own refusal too, and asks for an update.
    const outdated = project({ id: "w5", github: status({ available: false, failure_category: "outdated" }) });
    expect(githubAccess([offline, outdated], [])).toEqual({ state: "failed", category: "outdated" });
    expect(githubAccessLine({ state: "failed", category: "outdated" })).toEqual({ text: "gh가 오래됐어요. 업데이트하세요", tone: "warn" });
    // A repository with no GitHub remote says nothing about gh, and a dialed device's project is not read here.
    expect(githubAccess([noRemote], [])).toBeNull();
    const onMini = project({ remote_target_id: "mini", device_id: "mini", github: status({ available: false, failure_category: "not_logged_in" }) });
    expect(githubAccess([onMini], [{ id: "mini", dials_in: false } as Device])).toBeNull();
    // A node that dials the core has its repositories read with the core's gh, so its signed-out read speaks.
    expect(githubAccess([onMini], [{ id: "mini", dials_in: true } as Device])).toEqual({ state: "failed", category: "not_logged_in" });
  });

  it("draws a row per host so one host's success never hides another's sign-out, and another host's row only once it read a project", () => {
    const gitlab = { key: "gitlab", name: "GitLab", operations: ["commit_checks"], access: { program: "glab", minimum_version: "1.78.0", sign_in: "glab auth login --hostname gitlab.com" } } as HostDeclaration;
    const healthy = project({ id: "gh", github: status({ last_success_at_unix_ms: 5 }) });
    const signedOut = project({ id: "gl", github: status({ available: false, failure_category: "not_logged_in", host: gitlab }) });
    expect(hostAccessRows([healthy], []).map((row) => row.key)).toEqual(["github"]);
    const rows = hostAccessRows([signedOut, healthy], []);
    expect(rows.map((row) => [row.key, row.access])).toEqual([["github", { state: "connected" }], ["gitlab", { state: "failed", category: "not_logged_in" }]]);
    expect(rows[1]!.declaration?.access.sign_in).toBe("glab auth login --hostname gitlab.com");
    expect(githubAccessLineIn({ state: "failed", category: "not_logged_in" }, t, null, "glab").text).toBe("glab 로그인 안 됨");
  });

  it("names what Auto resolves to only where the core has shown it, and offers GitHub only to a Git project", () => {
    const onDefault = project({ tasks: { source: source({}), tasks: [], overflow: false } });
    expect(issueSourceChoices(onDefault, undefined)).toEqual({
      value: "auto",
      options: [
        { id: "auto", label: "자동 (GitHub)" },
        { id: "github", label: "GitHub · acme/app" },
        { id: "local", label: "로컬" },
      ],
    });

    // Chosen Local: the core has not said what Auto would be, so it is not guessed; the repository still names GitHub.
    const chosenLocal = project({
      tasks: { source: source({ kind: "local", label: "Local", name: null, chosen: true }), tasks: [], overflow: false },
      home_issues: { repository: "acme/app", issues: [], overflow: false },
    });
    const local = issueSourceChoices(chosenLocal, "local");
    expect(local.value).toBe("local");
    expect(local.options.map((option) => option.label)).toEqual(["자동", "GitHub · acme/app", "로컬"]);

    // A folder is always Local; a GitHub choice it cannot take reads as Auto.
    const folder = project({ is_git: false, tasks: { source: source({ kind: "local", label: "Local", name: null, chosen: true }), tasks: [], overflow: false } });
    expect(issueSourceChoices(folder, "github")).toEqual({
      value: "auto",
      options: [
        { id: "auto", label: "자동 (로컬)" },
        { id: "local", label: "로컬" },
      ],
    });
  });

  it("words the same choices and the gh state in English", () => {
    expect(githubAccessLineIn({ state: "failed", category: "not_logged_in" }, english).text).toBe("Not signed in to gh");
    expect(githubAccessLineIn({ state: "failed", category: "from a newer gh" as GithubFailureCategory }, english).text).toBe("from a newer gh");
    const onDefault = project({ tasks: { source: source({}), tasks: [], overflow: false } });
    expect(issueSourceChoicesIn(onDefault, undefined, english).options.map((option) => option.label)).toEqual(["Automatic (GitHub)", "GitHub · acme/app", "Local"]);
  });
});
