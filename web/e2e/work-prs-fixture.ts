// The pull requests a fake `gh` answers for the PRs tool specs (PRD
// work-tab-pr-lens): the web spec (`work-prs.spec.ts`) and the desktop host's
// native captures share them, so nothing reaches GitHub in either.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fixtureProgram } from "./platform-fixture";

export function git(cwd: string, args: string[]): string {
  return execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" });
}

const HOUR = 3_600_000;
const passing = [{ __typename: "CheckRun", name: "verify", status: "COMPLETED", conclusion: "SUCCESS" }];
const failing = [{ __typename: "CheckRun", name: "verify", status: "COMPLETED", conclusion: "FAILURE", detailsUrl: "https://github.com/acme/repo/actions/runs/7" }];

type Listed = {
  number: number;
  title: string;
  branch: string;
  author: string;
  state: "OPEN" | "MERGED";
  merge: string;
  review: string | null;
  checks: unknown[];
  draft?: boolean;
  asked?: boolean;
  /** A review was asked of the team `core` the account is in, and of nobody by name. */
  team?: boolean;
  hoursAgo: number;
};

const PULLS: Listed[] = [
  { number: 31, title: "머지할 수 있는 PR", branch: "feat/clean", author: "example-user", state: "OPEN", merge: "CLEAN", review: "APPROVED", checks: passing, hoursAgo: 3 },
  { number: 32, title: "base에 뒤처진 PR", branch: "feat/behind", author: "example-user", state: "OPEN", merge: "BEHIND", review: "APPROVED", checks: passing, hoursAgo: 5 },
  { number: 33, title: "내 리뷰를 기다리는 PR", branch: "feat/review", author: "ana", state: "OPEN", merge: "BLOCKED", review: "REVIEW_REQUIRED", checks: passing, asked: true, hoursAgo: 8 },
  { number: 34, title: "충돌이 난 PR", branch: "feat/conflict", author: "example-user", state: "OPEN", merge: "DIRTY", review: null, checks: passing, hoursAgo: 12 },
  { number: 35, title: "검사가 실패한 초안", branch: "feat/draft", author: "example-user", state: "OPEN", merge: "DRAFT", review: null, checks: failing, draft: true, hoursAgo: 20 },
  { number: 36, title: "Bump sha2 from 0.10.9 to 0.11.0", branch: "dependabot/cargo/sha2", author: "app/dependabot", state: "OPEN", merge: "BLOCKED", review: null, checks: passing, hoursAgo: 30 },
  { number: 37, title: "머지된 작업", branch: "feat/merged", author: "example-user", state: "MERGED", merge: "UNKNOWN", review: null, checks: passing, hoursAgo: 40 },
];

/** A teammate's pull request asked of the operator's team, for the specs that need a review ask only the search can place. */
const TEAM_ASKED: Listed = { number: 38, title: "팀이 리뷰를 요청받은 PR", branch: "feat/team", author: "bo", state: "OPEN", merge: "BLOCKED", review: "REVIEW_REQUIRED", checks: passing, team: true, hoursAgo: 10 };

/** The questions beside the pull request list that the fake `gh` can be told to fail. */
export type ExtraQuery = "account" | "review_requests" | "merge_queue";

/**
 * A `gh` that is signed in as `example-user` for `acme/repo` and keeps its
 * state in files: each pull request's head commit and merge state, and a log
 * of every write it was asked for. A merge or a branch update whose head
 * condition does not match the state's head is refused the way GitHub
 * refuses it. Anything it is not taught is refused.
 */
export function fakeGh(root: string, options: { failing?: ExtraQuery[]; teamAsk?: boolean } = {}): { bin: string; state: string } {
  const bin = path.join(root, "gh-bin");
  const state = path.join(root, "gh-state");
  fs.mkdirSync(bin, { recursive: true });
  fs.mkdirSync(state, { recursive: true });
  const now = Date.now();
  const sha = (number: number, generation: number) => `${number.toString(16)}${generation.toString(16)}`.padStart(40, "0");
  const listed = options.teamAsk ? [...PULLS, TEAM_ASKED] : PULLS;
  const heads: Record<number, string> = {};
  const merges: Record<number, string> = {};
  for (const pr of listed) {
    heads[pr.number] = sha(pr.number, 0);
    merges[pr.number] = pr.merge;
  }
  fs.writeFileSync(path.join(state, "heads.json"), JSON.stringify(heads));
  fs.writeFileSync(path.join(state, "first-heads.json"), JSON.stringify(heads));
  fs.writeFileSync(path.join(state, "merges.json"), JSON.stringify(merges));
  fs.writeFileSync(path.join(state, "failing.json"), JSON.stringify(options.failing ?? []));
  const pulls = listed.map((pr) => ({
    number: pr.number,
    title: pr.title,
    headRefName: pr.branch,
    isCrossRepository: false,
    baseRefName: "main",
    state: pr.state,
    reviewDecision: pr.review,
    isDraft: pr.draft ?? false,
    url: `https://github.com/acme/repo/pull/${pr.number}`,
    mergedAt: pr.state === "MERGED" ? new Date(now - pr.hoursAgo * HOUR).toISOString() : null,
    updatedAt: new Date(now - pr.hoursAgo * HOUR).toISOString(),
    createdAt: new Date(now - (pr.hoursAgo + 24) * HOUR).toISOString(),
    closedAt: null,
    closingIssuesReferences: [],
    author: { login: pr.author, is_bot: pr.author.startsWith("app/") },
    reviewRequests: pr.asked ? [{ __typename: "User", login: "example-user" }] : pr.team ? [{ __typename: "Team", name: "Core", slug: "core" }] : [],
  }));
  const checks = listed.filter((pr) => pr.state === "OPEN").map((pr) => ({ number: pr.number, statusCheckRollup: pr.checks }));
  const asked = listed.filter((pr) => pr.asked || pr.team).map((pr) => ({ number: pr.number }));
  const script = `const fs = require("fs");
const path = require("path");
const state = ${JSON.stringify(state)};
const args = process.argv.slice(2);
const file = (name) => path.join(state, name);
const read = (name) => JSON.parse(fs.readFileSync(file(name), "utf8"));
const out = (value) => process.stdout.write((typeof value === "string" ? value : JSON.stringify(value)) + "\\n");
const fail = (why) => { process.stderr.write(why + "\\n"); process.exit(1); };
const pulls = ${JSON.stringify(pulls)};
const checks = ${JSON.stringify(checks)};
const asked = ${JSON.stringify(asked)};
const [a, b] = args;
const log = () => fs.appendFileSync(file("writes.log"), JSON.stringify(args) + "\\n");
const failing = (query) => read("failing.json").includes(query);
if (a === "auth" && b === "status") process.exit(0);
if (a === "api" && b === "user") { if (failing("account")) fail("HTTP 502: Bad Gateway (https://api.github.com/user)"); out("example-user"); process.exit(0); }
if (a === "repo" && b === "view") { out({ nameWithOwner: "acme/repo" }); process.exit(0); }
if (a === "pr" && b === "list") {
  if (args.includes("merged")) { out([]); process.exit(0); }
  if (args.includes("review-requested:@me")) { if (failing("review_requests")) fail("GraphQL: API rate limit already exceeded for user ID 1"); out(asked); process.exit(0); }
  const heads = read("heads.json");
  const merges = read("merges.json");
  if (args.includes("number,statusCheckRollup")) { out(checks); process.exit(0); }
  const first = read("first-heads.json");
  // A list read after a push says so in the title, which is how a spec sees the list was read again.
  out(pulls.map((pr) => ({ ...pr, title: heads[pr.number] === first[pr.number] ? pr.title : pr.title + " (moved)", headRefOid: heads[pr.number], mergeStateStatus: merges[pr.number] })));
  process.exit(0);
}
if (a === "issue" && b === "list") { out([]); process.exit(0); }
if (a === "pr" && b === "merge") {
  log();
  const number = args[2];
  const head = args[args.indexOf("--match-head-commit") + 1];
  if (read("heads.json")[number] !== head) fail("Head branch was modified. Review the changes here and try the merge again.");
  const merges = read("merges.json");
  merges[number] = "MERGED";
  fs.writeFileSync(file("merges.json"), JSON.stringify(merges));
  process.exit(0);
}
if (a === "api" && args[1] === "--method" && args[2] === "PUT") {
  log();
  const number = args[3].split("/")[4];
  const expected = args[args.indexOf("-f") + 1].split("=")[1];
  const heads = read("heads.json");
  if (heads[number] !== expected) fail("expected_head_sha does not match the pull request head");
  heads[number] = (heads[number].slice(0, 39) + "f");
  fs.writeFileSync(file("heads.json"), JSON.stringify(heads));
  const merges = read("merges.json");
  merges[number] = "CLEAN";
  fs.writeFileSync(file("merges.json"), JSON.stringify(merges));
  out({ message: "Updating pull request branch." });
  process.exit(0);
}
if (process.argv.slice(2).join(" ").includes("HideHeadRuns")) { const repository = {}; for (let i = 0; i < process.argv.slice(2).join(" ").split("object(oid:").length - 1; i++) repository["c" + i] = { checkSuites: { totalCount: 0, nodes: [] } }; console.log(JSON.stringify({ data: { repository } })); process.exit(0); }
if (a === "api" && b === "graphql") {
  const query = args.join(" ");
  if (query.includes("HidePrFacts")) { if (failing("merge_queue")) fail("GraphQL: Field 'mergeQueueEntry' doesn't exist on type 'PullRequest'"); out({ data: { repository: { pullRequests: { nodes: [] } } } }); process.exit(0); }
  const match = /pullRequest\\(number:(\\d+)\\)/.exec(query);
  if (query.includes("HidePrCard") && match) {
    const number = match[1];
    const heads = fs.existsSync(file("card-heads.json")) ? read("card-heads.json") : read("heads.json");
    out({ data: { repository: {
      mergeCommitAllowed: true, squashMergeAllowed: true, rebaseMergeAllowed: false, viewerPermission: "ADMIN",
      defaultBranchRef: { name: "main" }, mergeQueue: null, attributes: { text: "dist/** linguist-generated\\n" },
      pullRequest: {
        headRefOid: heads[number], baseRefName: "main", body: "## Summary\\n\\n본문입니다 #" + number + "\\n\\n## Test plan\\n\\n- [x] checked\\n",
        files: { totalCount: 3, nodes: [{ path: "src/app.ts", additions: 12, deletions: 3 }, { path: "src/lib.ts", additions: 4, deletions: 0 }, { path: "dist/app.js", additions: 900, deletions: 20 }] },
        reviewThreads: { nodes: [] },
      },
    } } });
    process.exit(0);
  }
  out({ data: {} });
  process.exit(0);
}
fail("unsupported: " + args.join(" "));
`;
  fixtureProgram(bin, "gh", script);
  return { bin, state };
}

export function writes(state: string): string[][] {
  const log = path.join(state, "writes.log");
  if (!fs.existsSync(log)) return [];
  return fs.readFileSync(log, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line) as string[]);
}

export function head(state: string, number: number): string {
  return (JSON.parse(fs.readFileSync(path.join(state, "heads.json"), "utf8")) as Record<string, string>)[number]!;
}

/** The card's own read answers another head than the list's, as a push between the two reads does. */
export function skewCardHead(state: string, number: number, to: string): void {
  const heads = JSON.parse(fs.readFileSync(path.join(state, "heads.json"), "utf8")) as Record<string, string>;
  heads[number] = to;
  fs.writeFileSync(path.join(state, "card-heads.json"), JSON.stringify(heads));
}

export function moveHead(state: string, number: number, to: string): void {
  const file = path.join(state, "heads.json");
  const heads = JSON.parse(fs.readFileSync(file, "utf8")) as Record<string, string>;
  heads[number] = to;
  fs.writeFileSync(file, JSON.stringify(heads));
}
