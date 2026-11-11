// The sample host's content (docs/UI_BEHAVIOR.md, Phone demo mode): the frames a Mac would send, built from the same protocol types
// and the same pull request fixture the phone's own tests use, worded from the catalogs so they follow the phone's language. Pure:
// `demoSocket.ts` holds the state a visit changes and sends these frames the way hided would.

import type { TFunction } from "i18next";
import type { MessageKey } from "../i18n/catalogs";
import { PROVIDER_KINDS } from "../agentPicker";
import chartTemplate from "./demoChart.svg?raw";
import guideTemplate from "./demoGuide.html?raw";
import { card, pr, project } from "./prFixture";
import type {
  AgentGroup, ConversationMessage, GroupRow, LookRow, PhoneAgent, PhoneDevice, PhoneMission, PhoneMissionNode, PhonePr, PhonePrFeedback, ServerFrame, StartCatalog,
} from "./protocol";

type T = TFunction<"translation">;

export const DEMO_DEVICE = "local";
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

/** One thing said in a conversation this visit added to: a catalog sentence (so a language change rewords it) or the person's own text. */
export type Said = { who: "you" | "agent"; key: MessageKey } | { who: "you" | "agent"; text: string };

/** An agent the visit started from the start sheet or a pull request's hand-over. */
export type DemoStarted = { pane: string; title: string; prompt: string };

/** Everything a visit changes; the rest of the sample host never moves. */
export type DemoState = {
  /** When the visit began: the sample times are counted back from it. */
  t0: number;
  started: readonly DemoStarted[];
  /** The mission's decision has been answered. */
  answered: boolean;
  merged: readonly number[];
  feedback: { requestId: string; pr: string } | null;
  seenLooks: readonly string[];
  said: Readonly<Record<string, readonly Said[]>>;
};

export const START_STATE: DemoState = { t0: 0, started: [], answered: false, merged: [], feedback: null, seenLooks: [], said: {} };

/**
 * The kind of every sample agent and conductor is the word "agent" in the phone's language: a kind Hide has no adapter for,
 * so a row and a detail draw the neutral terminal mark and no vendor's name or logo (a store screenshot of the demo shows no
 * other company's mark), and VoiceOver reads a word of the language it speaks. A step names no worker.
 */
const sampleKind = (t: T): string => t("mobile.demo.kind");

const LOOK_PANE = "a2";
export const LOOK_HTML = "v1";
export const LOOK_IMAGE = "v2";

function agent(t: T, pane: string, title: string, ago: number, state: DemoState, over: Partial<PhoneAgent> = {}): PhoneAgent {
  return {
    device_id: DEMO_DEVICE, pane_id: pane, root_pane_id: pane, group: "working", mark: "going", wait: null, descendants_working: 0,
    agent_kind: sampleKind(t), title, place: null, device_label: null, changed_at_unix_ms: state.t0 - ago * MINUTE,
    line: null, status_code: "working", demand: "none", emphasized: false, holds_notification: false, ...over,
  };
}

const row = (value: PhoneAgent): GroupRow => ({ kind: "agent", ...value });
const asking = (text: string): PhoneAgent["line"] => ({ text, mode: "request", tone: "warning" });

/** Every agent of the sample list by pane, so a conversation can name the one it belongs to. */
function agents(t: T, state: DemoState): Record<string, PhoneAgent> {
  const allSeen = state.seenLooks.includes(LOOK_HTML) && state.seenLooks.includes(LOOK_IMAGE);
  const child1 = agent(t, "a1c1", t("mobile.demo.a1c1.title"), 2, state, { line: { text: t("mobile.demo.a1c1.line"), mode: "kept", tone: "subtle" } });
  const child2 = agent(t, "a1c2", t("mobile.demo.a1c2.title"), 9, state, { group: "done", mark: "done", status_code: "idle", line: { text: t("mobile.demo.a1c2.line"), mode: "news", tone: "news" } });
  const parent = agent(t, "a1", t("mobile.demo.a1.title"), 3, state, {
    group: "needs_you", mark: "turn", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, place: "demo · fix/review",
    line: asking(t("mobile.demo.a1.ask")), descendants: { asking: 0, working: 1, done: 1 }, children_open: true, children: [child1, child2],
  });
  const listed: PhoneAgent[] = [
    parent,
    agent(t, LOOK_PANE, t("mobile.demo.a2.title"), 11, state, {
      group: "needs_you", mark: allSeen ? "turn" : "look", status_code: "idle", demand: "question", emphasized: true, holds_notification: true, place: "demo · main",
      line: asking(t("mobile.demo.a2.ask")),
    }),
    agent(t, "a3", t("mobile.demo.a3.title"), 4, state, { place: "demo · perf" }),
    agent(t, "a4", t("mobile.demo.a4.title"), 22, state, { place: "demo · main" }),
    agent(t, "a5", t("mobile.demo.a5.title"), 60, state, { group: "seen", mark: "quiet", status_code: "idle", place: "demo · site" }),
    agent(t, "a6", t("mobile.demo.a6.title"), 35, state, { group: "done", mark: "done", status_code: "idle", line: { text: t("mobile.demo.a1c2.line"), mode: "news", tone: "news" }, place: "demo · main" }),
    agent(t, "a7", t("mobile.demo.a7.title"), 180, state, { group: "revisit", mark: "quiet", status_code: "idle", place: "demo · notes", starred: true }),
    agent(t, "a8", t("mobile.demo.a8.title"), 240, state, { group: "seen", mark: "quiet", status_code: "idle", place: "demo · notes" }),
    ...state.started.map((started, at) => agent(t, started.pane, started.title, at, state, { place: "demo · main" })),
  ];
  const all: Record<string, PhoneAgent> = {};
  for (const one of listed) all[one.pane_id] = one;
  all.a1c1 = child1;
  all.a1c2 = child2;
  return all;
}

export function agentGroups(t: T, state: DemoState): AgentGroup[] {
  const by = agents(t, state);
  const working: GroupRow[] = [...state.started.map((started) => row(by[started.pane]!)), row(by.a3!), row(by.a4!)];
  return [
    { group: "needs_you", count: 2, rows: [row(by.a1!), row(by.a2!)] },
    { group: "working", count: working.length, rows: working },
    { group: "done", count: 1, rows: [row(by.a6!)] },
    { group: "revisit", count: 1, rows: [row(by.a7!)] },
    { group: "seen", count: 2, rows: [row(by.a5!)], more: [row(by.a8!)] },
  ];
}

export function devices(state: DemoState): PhoneDevice[] {
  return [{ device_id: DEMO_DEVICE, label: null, local: true, connected: true, needs_you: 2, working: 2 + state.started.length }];
}

/** The `agents` frame: the list, with no language of its own, so the phone follows its own. */
export function agentsFrame(t: T, state: DemoState): ServerFrame {
  return { type: "agents", groups: agentGroups(t, state), devices: devices(state), interface_language: null, color_theme: null };
}

/** The conversation of one pane: what it opened with, then what this visit said. */
export function conversation(t: T, state: DemoState, pane: string): ConversationMessage[] {
  const by = agents(t, state);
  const opening: Said[] =
    pane === "a1" ? [{ who: "you", key: "mobile.demo.a1.m1" }, { who: "agent", key: "mobile.demo.a1.m2" }, { who: "agent", key: "mobile.demo.a1.ask" }]
      : pane === LOOK_PANE ? [{ who: "you", key: "mobile.demo.a2.m1" }, { who: "agent", key: "mobile.demo.a2.m2" }, { who: "agent", key: "mobile.demo.a2.ask" }]
        : state.started.some((started) => started.pane === pane)
          ? [{ who: "you", text: state.started.find((started) => started.pane === pane)!.prompt }, { who: "agent", key: "mobile.demo.started" }]
          : [{ who: "you", text: t("mobile.demo.convoYou", { title: by[pane]?.title ?? "" }) }, { who: "agent", key: "mobile.demo.convoAgent" }];
  return [...opening, ...(state.said[pane] ?? [])].map((said, at) => ({
    id: (at + 1) * 100,
    who: said.who,
    text: "key" in said ? t(said.key) : said.text,
    truncated: false,
    at_ms: state.t0 - (opening.length + 1 - at) * MINUTE,
  }));
}

const ESC = "\u001b[";
const paint = (code: string, text: string): string => `${ESC}${code}m${text}${ESC}0m`;
const dim = (text: string) => paint("90", text);
const green = (text: string) => paint("32", text);
const red = (text: string) => paint("31", text);
const cyan = (text: string) => paint("36", text);
const ask = (text: string) => paint("1;33", text);

/** What a sample agent's pane ran: the file it read and changed with its diff, the command and its output. Code text, so never worded. */
type PaneWork = { read: string; edit?: { file: string; added: number; removed: number; diff: readonly string[] }; run?: { command: string; output: readonly string[] } };
const PANE_WORK: Record<string, PaneWork> = {
  a1: {
    read: "src/review.ts",
    edit: { file: "src/review.ts", added: 9, removed: 3, diff: ["-  return path.split(\"\\\\\");", "+  return path.split(/[\\\\/]/);"] },
    run: { command: "pnpm test", output: [green("✓ 412 passed") + dim(" (3.1s)")] },
  },
  [LOOK_PANE]: {
    read: "INSTALL.md",
    edit: { file: "INSTALL.md", added: 6, removed: 4, diff: ["-3. hide pair --qr", "+1. brew install --cask hide"] },
  },
  a3: {
    read: "bench/holder.rs",
    run: { command: "cargo bench -p hide-holder", output: ["rss   idle   182 MB  →  146 MB", "rss   busy   311 MB  →  248 MB"] },
  },
};

/**
 * The pane of a sample agent as its terminal tab reads it: the request, what it read and changed with the diff in the terminal's
 * colours, the command it ran and its output, then where it is now (its question, or still at work). The agent's own words follow the
 * phone's language like the rest of the sample; file names, commands and their output are code text and stay as written.
 */
export function terminalText(t: T, state: DemoState, pane: string): string {
  const said = conversation(t, state, pane);
  const prompt = said.find((message) => message.who === "you")?.text ?? "";
  const by = agents(t, state)[pane];
  const work = PANE_WORK[pane];
  const rows: string[] = [`${dim("›")} ${prompt}`, ""];
  if (work) {
    rows.push(`${green("✓")} ${t("mobile.demo.term.read", { file: work.read })}`);
    if (work.edit) {
      rows.push(`${green("✓")} ${t("mobile.demo.term.edit", { file: work.edit.file })} ${green(`+${work.edit.added}`)} ${red(`-${work.edit.removed}`)}`);
      for (const line of work.edit.diff) rows.push(`  ${line.startsWith("+") ? green(line) : red(line)}`);
    }
    if (work.run) rows.push("", `${dim("$")} ${work.run.command}`, ...work.run.output.map((line) => `  ${line}`));
    rows.push("");
  }
  const agentSaid = said.filter((message) => message.who === "agent");
  const question = by?.line?.mode === "request" ? by.line.text : null;
  for (const message of agentSaid) if (message.text !== question) rows.push(`• ${message.text}`);
  rows.push("", question ? ask(`? ${question}`) : by?.status_code === "working" ? cyan(`⠋ ${t("mobile.demo.term.working")}`) : dim(t("mobile.demo.term.waiting")));
  return `${rows.join("\n")}\n`;
}

export function looks(state: DemoState, pane: string): LookRow[] {
  if (pane !== LOOK_PANE) return [];
  return [
    { id: LOOK_HTML, kind: "html", name: "install-guide-mockup.html", unseen: !state.seenLooks.includes(LOOK_HTML), at: state.t0 - 9 * MINUTE },
    { id: LOOK_IMAGE, kind: "image", name: "memory-chart.svg", unseen: !state.seenLooks.includes(LOOK_IMAGE), at: state.t0 - 12 * MINUTE },
  ];
}

const escapeText = (text: string): string => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** The one HTML result: a small page the phone draws in its sandboxed frame. Its markup is a file, not a style this module draws. */
export function lookHtml(t: T): string {
  const steps = (["htmlStep1", "htmlStep2", "htmlStep3"] as const).map((key) => `<li>${escapeText(t(`mobile.demo.look.${key}`))}</li>`).join("");
  const title = escapeText(t("mobile.demo.look.htmlTitle"));
  return guideTemplate.replaceAll("{{title}}", () => title).replace("{{steps}}", () => steps);
}

/** The one image result: a chart drawn as SVG, which the phone's image reader accepts. */
export function lookChart(t: T): string {
  return chartTemplate.replace("{{title}}", () => escapeText(t("mobile.demo.look.chartTitle")));
}

/** UTF-8 text as the base64 a `look_chunk` carries. */
export function base64Of(text: string): { data: string; size: number } {
  const bytes = new TextEncoder().encode(text);
  return { data: btoa(String.fromCharCode(...bytes)), size: bytes.length };
}

function step(key: string, title: string, status: PhoneMissionNode["status"], over: Partial<PhoneMissionNode> = {}): PhoneMissionNode {
  return {
    key, title, kind: "change", depends_on: [], status, blocked: null, delivery: "pr", pr: null, pr_state: null, base_pr: null, stage: null,
    attention: null, landing: null, agent_runtime: "running", worker_agent: null, agent: null, escalation: null, ...over,
  };
}

const merged = (key: string, title: string, number: number): PhoneMissionNode =>
  step(key, title, "done", { pr: number, pr_state: "merged", agent_runtime: "ended" });

function mission(t: T, id: string, title: string, state: DemoState, over: Partial<PhoneMission> = {}): PhoneMission {
  return {
    id, title, device_id: DEMO_DEVICE, device_label: null, status: "running", decision: null, question: null, answer: null, answer_failed: null,
    landing_order: [], created_at_unix_ms: state.t0 - 3 * HOUR, started_at_unix_ms: state.t0 - 3 * HOUR, closed_at_unix_ms: null, stale: false,
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: sampleKind(t), agent: null }, nodes: [], ...over,
  };
}

export function missionsFrame(t: T, state: DemoState): ServerFrame {
  const asked = state.t0 - 6 * MINUTE;
  const release = mission(t, "mission-release", t("mobile.demo.m1.title"), state, {
    // The list's first agent conducts it, so its row wears the mission chip as a Conductor's desktop row does.
    conductor: { present: true, takes_answers: true, name: "Conductor", agent_kind: sampleKind(t), agent: { device_id: DEMO_DEVICE, pane_id: "a1" } },
    created_at_unix_ms: state.t0 - 134 * MINUTE,
    started_at_unix_ms: state.t0 - 134 * MINUTE,
    decision: state.answered ? null : t("mobile.demo.m1.decision"),
    question: state.answered ? null : { key: 3, why: null, options: [t("mobile.demo.m1.option1"), t("mobile.demo.m1.option2")], recommend: null, node: "sign", asked_at_unix_ms: asked },
    nodes: [
      merged("notes", t("mobile.demo.a4.title"), 2085),
      merged("bump", t("mobile.demo.m1.bump"), 2080),
      step("rehearse", t("mobile.demo.m1.rehearse"), "working", { depends_on: ["bump"], pr: 2093, stage: 1, agent: { device_id: DEMO_DEVICE, pane_id: "a3" } }),
      step("sign", t("mobile.demo.m1.sign"), "working", { depends_on: ["bump"], stage: 0 }),
      // What waits on the two steps at work, so the page shows which step waits on which.
      step("verify", t("mobile.demo.m1.verify"), "planned", { kind: "verify", delivery: null, depends_on: ["rehearse", "sign"], agent_runtime: null }),
      step("publish", t("mobile.demo.m1.publish"), "planned", { depends_on: ["verify", "notes"], agent_runtime: null }),
    ],
  });
  const ui = mission(t, "mission-ui", t("mobile.demo.m2.title"), state, {
    created_at_unix_ms: state.t0 - 50 * MINUTE,
    started_at_unix_ms: state.t0 - 50 * MINUTE,
    nodes: [
      merged("a", t("mobile.demo.m2.tabs"), 2101),
      step("b", t("mobile.demo.m2.missions"), "working", { depends_on: ["a"], pr: 2102, stage: 1, agent: { device_id: DEMO_DEVICE, pane_id: "a4" } }),
    ],
  });
  const finished = mission(t, "mission-guide", t("mobile.demo.a2.title"), state, {
    status: "done",
    created_at_unix_ms: state.t0 - 26 * HOUR,
    started_at_unix_ms: state.t0 - 26 * HOUR,
    closed_at_unix_ms: state.t0 - 24 * HOUR,
    nodes: [merged("a", t("mobile.demo.a2.title"), 2000)],
  });
  return { type: "missions", items: [release, ui, finished], failed: false };
}

const HEAD = "7d3c1f0e9a8b4c2d6e5f10293847a6b5c4d3e2f1";
const SAMPLE_URL = "https://example.com/demo";

function counts(total: number, passed: number, running: number, failed = 0) {
  return { total, passed, failed, running, waiting: 0, skipped: 0, unknown: 0 };
}

function open(number: number, title: string, branch: string, over: Partial<PhonePr>): PhonePr {
  return pr(number, { title, head_branch: branch, url: `${SAMPLE_URL}/pull/${number}`, card: null, ...over });
}

function work(over: Partial<NonNullable<PhonePr["work"]>>): NonNullable<PhonePr["work"]> {
  // The fixture's reviewer is a real login; the sample names a generic one.
  return { ...pr(1).work!, reviewers: [{ login: "reviewer", team: false, state: "approved", at_unix_ms: null }], ...over };
}

/** The pull request that fails a check, which the hand-over sheet opens on. */
export const FAILED_PR_NUMBER = 2088;

/** The `prs` frame; titles, branches and files are the repository's own text, so they are not translated. */
export function prsFrame(t: T, state: DemoState): ServerFrame {
  const feedback: PhonePrFeedback | null = state.feedback
    ? {
        request_id: state.feedback.requestId,
        pr_number: FAILED_PR_NUMBER,
        phase: "ready",
        failed_checks: [{ name: "web-e2e", url: `${SAMPLE_URL}/runs/${FAILED_PR_NUMBER}` }],
        change_requests: [{ author: "reviewer", body: t("mobile.demo.pr.changeRequest") }],
        failure: null,
      }
    : null;
  const items = [
    open(2090, "fix: keep Windows paths on the wire", "fix/win-path-sep", {
      work: work({ check_counts: counts(3, 3, 0) }),
      card: card({
        head: HEAD,
        files: [
          { path: "src/path.rs", additions: 6, deletions: 2, generated: false },
          { path: "src/wire.rs", additions: 71, deletions: 8, generated: false },
          { path: "tests/path.rs", additions: 7, deletions: 2, generated: false },
        ],
        files_total: 3,
      }),
    }),
    open(2085, "docs: draft the 1.2.0 release notes", "docs/release-notes-1-2-0", {
      work: work({ check_counts: counts(3, 3, 0) }),
      card: card({ head: HEAD, files: [{ path: "docs/release-notes-1.2.0.md", additions: 41, deletions: 0, generated: false }], files_total: 1 }),
    }),
    open(2091, "refactor: tidy holder version folders", "refactor/holder-folders", {
      group: "working",
      state: "pending",
      checks: "pending",
      reason: { kind: "working", name: t("mobile.demo.a3.title"), since_unix_ms: state.t0 - 4 * MINUTE },
      agents: [{ device_id: DEMO_DEVICE, pane_id: "a3" }],
      work: work({ merge_state: "blocked", pending_checks: 8, check_counts: counts(22, 14, 8) }),
      card: card({ head: HEAD, files: [{ path: "src/holders.rs", additions: 58, deletions: 19, generated: false }], files_total: 1 }),
    }),
    open(FAILED_PR_NUMBER, "feat: show children on the phone list", "feat/phone-children", {
      group: "blocked",
      state: "failed",
      checks: "failed",
      review: null,
      reason: { kind: "failed", name: "web-e2e", count: 1 },
      work: work({ merge_state: "blocked", failed_checks: [{ name: "web-e2e", url: `${SAMPLE_URL}/runs/${FAILED_PR_NUMBER}` }], check_counts: counts(22, 21, 0, 1), reviewers: [], required_approvals: 1 }),
      card: card({
        head: HEAD,
        files: [{ path: "src/list.tsx", additions: 34, deletions: 6, generated: false }, { path: "tests/list.test.tsx", additions: 18, deletions: 0, generated: false }],
        files_total: 2,
        merge: { methods: ["squash", "merge"], queue: false, can_merge: false },
      }),
    }),
  ].filter((item) => !state.merged.includes(item.number));
  return { type: "prs", projects: [project({ label: "demo/app" })], items, feedback, omitted: 0 };
}

/** What the start sheet lists: the sample checkout, and the agents the sample Mac has. */
export function startCatalog(): StartCatalog {
  return {
    targets: [{ id: "demo-main", device_id: DEMO_DEVICE, place: "demo · main", device_label: null, connected: true }],
    core_platform: "macos aarch64",
    // The sample Mac has two of the agents the app knows, and Claude lists its model aliases.
    kinds: PROVIDER_KINDS.filter((id) => id === "claude" || id === "codex").map((id) => ({ id, models: id === "claude" ? ["opus", "sonnet", "haiku"] : [] })),
    remembered: { kind: null, models: {} },
  };
}
