// Native transcripts and a repository-local provider/GitHub fixture. The
// daemon still reads, judges, persists and queries through its production workers.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { declareParent, labelAgent, type FixtureLabel, type HerdrFixture } from "./herdr-fixture";
import { fixtureProgram } from "./platform-fixture";
import { expect } from "@playwright/test";
import { DatabaseSync } from "node:sqlite";

/** B13: fresh work starts only after its original repository was observed.
 * Read the private writer's actual connection facts, rather than inventing
 * repository metadata or waiting a fixed time for asynchronous discovery. */
export async function workRepositoryReady(stateDir: string, repository = "acme/repo"): Promise<void> {
  const file = path.join(stateDir,"links.sqlite3");
  await expect.poll(() => {
    if (!fs.existsSync(file)) return false;
    const db = new DatabaseSync(file,{readOnly:true});
    try {
      if (!db.prepare("SELECT 1 FROM sqlite_master WHERE name='ledger_project_connections'").get()) return false;
      return !!db.prepare("SELECT 1 FROM ledger_project_connections WHERE repository=?").get(repository);
    } finally { db.close(); }
  }, { message: "originating fixture repository reached the durable link writer", timeout: 30_000 }).toBe(true);
}

/**
 * How long after its creation time a fixture pull request merges: well after
 * the line of the turn that printed it, whose transcript ends 1 s after its
 * request, so a flow's merged fact never races that line (fact-status B1).
 */
export const MERGE_AFTER_MS = 3_000;

export const FIRST_GOAL = "세션 기록 화면과 결과물 연결 구현";
export const NEXT_GOAL = "놓친 일 이슈 생성과 검색 검증";
export const CHILD_GOAL = "자식 세션 결과물 연결 검증";
export const MISSED = "좁은 열에서 긴 한국어 요청의 줄바꿈을 확인해야 합니다";

export function workFixture(herdr: HerdrFixture, uncertainCreate = false) {
  const cwd = path.join(herdr.root, "fixture");
  const git = (args: string[]) => execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd });
  // Match the GitHub fixture's head branch on every system, regardless of
  // the account's init.defaultBranch; its open PR keeps the session active.
  git(["init", "--initial-branch=main"]);
  fs.writeFileSync(path.join(cwd, "README.md"), "# fixture\n");
  git(["add", "README.md"]);
  git(["commit", "-m", "initial"]);
  git(["remote", "add", "origin", "https://github.com/acme/repo.git"]);
  const bin = path.join(herdr.root, "gh-bin");
  const calls = path.join(herdr.root, "gh-calls.jsonl");
  const contexts = path.join(herdr.root, "gh-contexts.jsonl");
  const issue = path.join(herdr.root, "created-issue.json");
  const repositoryUnavailable = path.join(herdr.root, "repository-unavailable");
  const repositoryGate = path.join(herdr.root, "repository-gate");
  const clock = path.join(herdr.root, "work-clock.json");
  fs.writeFileSync(clock, JSON.stringify({ first: Date.now(), next: Date.now() }));
  fixtureProgram(bin, "gh", `
const fs = require("node:fs");
const args = process.argv.slice(2);
const repo = process.env.GH_REPO || "acme/repo";
const calls = ${JSON.stringify(calls)}, issueFile = ${JSON.stringify(issue)};
fs.appendFileSync(calls, JSON.stringify(args) + "\\n");
fs.appendFileSync(${JSON.stringify(contexts)}, JSON.stringify({args,repository:process.env.GH_REPO || null,cwd:process.cwd()}) + "\\n");
const out = v => process.stdout.write(JSON.stringify(v).replaceAll("acme/repo",repo) + "\\n");
const clock = JSON.parse(fs.readFileSync(${JSON.stringify(clock)}, "utf8"));
const created = fs.existsSync(issueFile) ? JSON.parse(fs.readFileSync(issueFile, "utf8")) : null;
const MERGE_AFTER = ${MERGE_AFTER_MS};
const pull = (number, title, merged, failed, at) => ({number,title,state:merged?"MERGED":"OPEN",url:"https://github.com/acme/repo/pull/"+number,headRefName:"main",headRefOid:"1".repeat(40),isCrossRepository:false,baseRefName:"main",reviewDecision:"REVIEW_REQUIRED",isDraft:false,mergedAt:merged?new Date(at+MERGE_AFTER).toISOString():null,closedAt:null,createdAt:new Date(at+500).toISOString(),updatedAt:new Date(at+(merged?MERGE_AFTER:1000)).toISOString(),closingIssuesReferences:[],statusCheckRollup:[{__typename:"CheckRun",name:"verify",status:"COMPLETED",conclusion:failed?"FAILURE":"SUCCESS"}]});
const pulls = [pull(31,"기록 화면 구현",true,false,clock.first),pull(32,"이슈와 검색 검증",false,true,clock.next),pull(34,"자식 연결 검증",true,false,clock.first+200)];
const tracked = {number:41,title:"기록 화면 후속 검증",state:"OPEN",url:"https://github.com/acme/repo/issues/41",createdAt:new Date(clock.first+500).toISOString(),updatedAt:new Date(clock.first+1000).toISOString(),labels:[],author:{login:"reviewer"},assignees:[]};
const [a,b] = args;
if(a==="auth"&&b==="status") process.exit(0);
if(a==="api"&&b==="user") {process.stdout.write("example-user\\n");process.exit(0);}
if (process.argv.slice(2).join(" ").includes("review-requested:@me")) { process.stdout.write("[]\\n"); process.exit(0); }
if (process.argv.slice(2).join(" ").includes("HidePrFacts")) { process.stdout.write('{"data":{"repository":{"pullRequests":{"nodes":[]}}}}\\n'); process.exit(0); }
if(a==="repo"&&b==="view") {
 const deadline=Date.now()+15000;
 while(fs.existsSync(${JSON.stringify(repositoryGate)})) {
  if(Date.now()>deadline) {process.stderr.write("fixture repository gate timed out");process.exit(1);}
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,20);
 }
 if(fs.existsSync(${JSON.stringify(repositoryUnavailable)})) {process.stderr.write("fixture GitHub unavailable\\n");process.exit(1);} out({nameWithOwner:repo,id:"R_"+repo.replaceAll("/","_")});process.exit(0);}
if(a==="pr"&&b==="list") {out(pulls.filter(p=>args.includes("all")|| (args.includes("merged")?p.state==="MERGED":p.state==="OPEN")));process.exit(0);}
const open=created&&created.state==="OPEN"?[created]:[];
if(a==="issue"&&b==="list") {out([tracked,...(created?[created]:[])].filter(item=>item.state===(args.includes("closed")?"CLOSED":"OPEN")));process.exit(0);}
if(a==="issue"&&b==="close") {if(created){created.state="CLOSED";fs.writeFileSync(issueFile,JSON.stringify(created));}process.exit(0);}
if(a==="issue"&&b==="create") {
 const title=args[args.indexOf("--title")+1];
 const value={number:77,title,url:"https://github.com/"+repo+"/issues/77",state:"OPEN",createdAt:new Date().toISOString(),updatedAt:new Date().toISOString(),labels:[]};
 fs.writeFileSync(issueFile,JSON.stringify(value));
 if(${JSON.stringify(uncertainCreate)}) {process.stderr.write("fixture response lost\\n");process.exit(1);}
 process.stdout.write(value.url+"\\n");process.exit(0);
}
if(a==="api"&&b==="graphql") {
 const query=args.join(" ");
 if(query.includes("HideIssueDetail")) {
  const number=Number(args.find(value=>value.startsWith("number="))?.slice(7));
  const item=number===77?created:tracked;
  out({data:{repository:{issue:item?{...item,body:"## 검증 항목\\n\\n본문과 실제 댓글을 읽습니다.",author:{login:"reviewer"},assignees:{nodes:[]},labels:{nodes:[]},comments:{totalCount:1,nodes:[{author:{login:"reviewer"},createdAt:new Date(clock.first+2000).toISOString(),body:"실제 fixture 댓글"}],pageInfo:{endCursor:null,hasNextPage:false}}}:null}}});
 }
 else if(query.includes("states:[OPEN,CLOSED]")) out({data:{repository:{issues:{nodes:created?[{...created,viewerDidAuthor:true}]:[],pageInfo:{hasNextPage:false}}}}});
 else if(query.includes("issues(first:100")) out({data:{repository:{issues:{nodes:open,pageInfo:{hasNextPage:false}}}}});
 else if(query.includes("issue(number:")) out({data:{r0:{issue:query.includes("77")?created:tracked}}});
 else out({data:{}});
 process.exit(0);
}
if(a==="search") {out([]);process.exit(0);}
process.stderr.write("unsupported fixture gh arguments\\n");process.exit(1);
`);
  const env = { PATH: `${bin}${path.delimiter}${herdr.fixturePath}` };
  const setClock = (first: number, next: number) => fs.writeFileSync(clock, JSON.stringify({ first, next }));
  return { env, calls, contexts, issue, repositoryUnavailable, repositoryGate, clock: setClock };
}

export function seedFirstWork(herdr: HerdrFixture, at: number, goal = FIRST_GOAL) {
  const session = labelAgent(herdr, herdr.panes[0], {
    task: goal, at, cwd: path.join(herdr.root,"fixture"), end: "done", toolOutputs: ["https://github.com/acme/repo/pull/31", "https://github.com/acme/repo/issues/41"],
    request: "기록 화면 구현 요청", work: { missed: [{ kind: "follow_up", text: MISSED }], resolved: [], artifacts: ["세션 기록 화면 시안"] },
  });
  const child = labelAgent(herdr, herdr.panes[1], {
    task: CHILD_GOAL, at: at + 200, cwd: path.join(herdr.root,"fixture"), end: "done", toolOutputs: ["https://github.com/acme/repo/pull/34"],
    work: { missed: [], resolved: [], artifacts: [] },
  });
  declareParent(herdr, herdr.panes[1], herdr.panes[0]);
  return { session, child };
}

export function nextWork(at: number): FixtureLabel {
  return { task: NEXT_GOAL, at, end: "unfinished", request: "이슈와 검색 검증 요청", toolOutputs: ["https://github.com/acme/repo/pull/32"],
    work: { missed: [{ kind: "problem", text: "다른 저장소의 이슈 템플릿이 비어 있어 따로 고쳐야 합니다" }, { kind: "ask", text: "좁은 열 줄바꿈 기준을 정해 주세요" }], resolved: [], artifacts: [] } };
}
