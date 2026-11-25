// The phone's side of `/ws` (PRD mobile-companion D-01, D-04, D-16, D-17,
// D-22): the frames hided sends a phone (hided/src/mobile/phone.rs), the
// ones a phone may send back, how the page address carries a pairing code,
// a credential or a deep link, and the words each refusal and pane state
// shows. Pure, so every rule is tested without a socket or a browser.
// Text is named by catalog key and translated where it renders, so a shown
// line follows a language change.

import type { TFunction } from "i18next";
import type { MessageKey } from "../i18n/catalogs";
import type { PrGroupId, PrReason } from "../agentScope";
import type { WorkerEscalation } from "../missions";
import type { Automation, AutomationRecord, AutomationRun, AutomationSchedule, SubagentRow } from "../snapshot";
import type { AgentSleep, AgentStatusCode, AskSubject, AgentWait, ChangeRequestRef, ViewReopen, GithubStatus, HumanTurnLead, LineMode, MissionAnswer, MissionAnswerFailure, MissionBrief, MissionDelivery, MissionNodeBrief, MissionWaitOutcome, MissionWaitTarget, PrCard, PrState, PrFeedback, PrMergeMethod, PullRequest, RowMark, SleepingSession } from "../snapshot";

export type PhoneAgent = {
  device_id: string;
  pane_id: string;
  root_pane_id: string;
  group: GroupId;
  /** An ask the operator read and has not answered: rests in its own group with a hollow `!`, never announced (docs/status-model.md, A question still waiting). */
  read?: true;
  /** The desktop row's mark (`RowMark`). */
  mark: RowMark;
  /** What an in-progress row waits on, which its line words when it has no sentence. */
  wait: AgentWait | null;
  /** How many of its descendants work, for a row waiting on them. */
  descendants_working: number;
  /** The background tasks a finished turn left running (status-marks D-19); absent when none. */
  background_left?: number;
  /** The descendants still working under a finished turn not read yet, which its line ends with; absent when none. */
  children_left?: number;
  /** The subagents still running under a finished turn not read yet, which its line ends with; absent when none. */
  subagents_left?: number;
  /** The open questions the operator read that wait behind the row's own news or ask, which its line ends with as `외 N건`; absent when none, and from a Mac that predates it. */
  asks_behind?: number;
  /** How many subagents its agent runs inside its own session, which its children toggle counts beside its children; absent for none, for a count Hide cannot read, and from a Mac that predates it. */
  subagents?: number;
  /** The first five of them, oldest first, with what its agent named of each, listed after its children (docs/UI_BEHAVIOR.md, Subagents); absent when it names none. */
  subagent_rows?: SubagentRow[];
  /** How many things its `wait` waits on, which its line counts; absent for running checks and when it waits for nothing. */
  wait_count?: number;
  /** How many of its own subagents its wait waits on too, which a children wait's line adds; absent when none. */
  wait_subagents?: number;
  /** The operator's ★ to revisit it, drawn where the desktop row draws it; absent without one, and from a Mac that predates it. */
  starred?: true;
  /** The external tool whose work the row is, by name, leading its second line as on the desktop's sidebar row (PRD hide-external-control B46); absent on the operator's own rows, and from a Mac that predates it. */
  tool?: string;
  agent_kind: string;
  title: string;
  place: string | null;
  /** The project alone, which the list row's second line names; absent where `place` is and from a Mac that predates it, where the row names `place`. */
  project?: string;
  device_label: string | null;
  /** When the Mac's core last saw this agent change state (epoch ms); the phone counts the elapsed time from it. */
  changed_at_unix_ms: number | null;
  /** The desktop row's second line (`RowLine`), worded the same way. */
  line: { text: string; mode: LineMode; tone: "warning" | "news" | "subtle"; pull_request?: number; pull_reference?: ChangeRequestRef; subject?: AskSubject } | null;
  status_code: AgentStatusCode;
  demand: string;
  /** Present while the agent's process is ended and its pane holds a bare shell: nothing may be sent to it, and the detail says so (`AgentSleepSnapshot`). */
  sleep?: AgentSleep;
  emphasized: boolean;
  holds_notification: boolean;
  /** Its live descendants by state, for a parent's pills; absent when it has none. */
  descendants?: Descendants;
  /** The person's turn the row leads with, its own or a descendant's, which the phone draws with Done (PRD ask-human B19); absent when none stands. */
  human_turn?: HumanTurnLead;
  /** The children listed under it in the sidebar's order (two levels, five to a parent), and the ones behind its "더 보기". */
  children?: PhoneAgent[];
  more_children?: PhoneAgent[];
  /** The core opens its children until this phone chooses otherwise: a descendant waits for the operator. */
  children_open?: true;
};

/** What a parent's descendants are doing: asking the operator, working, finished. */
export type Descendants = { asking: number; working: number; done: number };

export type GroupId = "needs_you" | "done" | "revisit" | "working" | "seen";

/** A conversation intent has no pane address, lineage or filesystem path. */
export type PhoneSleepingSession = {
  sleep_id: string;
  device_id: string;
  agent_kind: string;
  title: string;
  group: GroupId;
  phase: SleepingSession["phase"];
  since_unix_ms: number;
  wake_available: boolean;
  checking: boolean;
  attention: boolean;
};

/** A top row of a group: an agent with its tree, or a sleeping conversation. */
export type GroupRow = ({ kind: "agent" } & PhoneAgent) | ({ kind: "sleeping" } & PhoneSleepingSession);

/**
 * `count` is every top row of the group, the folded ones too; `rows` are the first few in recency order across the Mac and its devices,
 * `more` the rest behind the group's "더 보기", `more_asleep` how many of those sleep.
 */
export type AgentGroup = { group: GroupId; count: number; rows: GroupRow[]; more?: GroupRow[]; more_asleep?: number };

/** The paired Mac (`local`) and each device it reaches, with the counts of their top rows. */
/** Another machine the computer opens that runs work of its own, by its product device id and its name there (B56). */
export type OtherMachine = { machine: string; name: string };
export type PhoneDevice = { device_id: string; label: string | null; local?: true; connected: boolean; needs_you: number; working: number };

/** Every agent the list holds, a parent before its children, shown or folded. */
export function listedAgents(groups: readonly AgentGroup[]): PhoneAgent[] {
  const all: PhoneAgent[] = [];
  const add = (agent: PhoneAgent) => {
    all.push(agent);
    for (const child of agent.children ?? []) add(child);
    for (const child of agent.more_children ?? []) add(child);
  };
  for (const group of groups) {
    for (const row of [...group.rows, ...(group.more ?? [])]) if (row.kind === "agent") add(row);
  }
  return all;
}

/**
 * One step of a mission as the phone receives it (`hided/src/mobile/missions.rs`): what the shared mission rules read (`MissionNodeBrief`), the
 * worker's kind and row, and the escalation the core raised for its worker. Nothing else of the core's node reaches the phone.
 */
export type PhoneMissionNode = MissionNodeBrief & {
  delivery: MissionDelivery | null;
  base_pr: number | null;
  /** `base_pr` as its host's unit names it (`!12` on gitlab.com); absent with none and from a Mac before it, which reads as `#n`. */
  base_reference?: ChangeRequestRef | null;
  /** The branch its current pull request merges into, as GitHub last answered; absent until it answered and from a Mac before it. */
  pr_base?: string;
  /** How many pull requests it holds besides its current one, the `+N` beside its pull request's number; absent at zero and from a Mac before it. */
  pr_more?: number;
  /** The pull request an outside step watches; absent for one that watches a branch and from a Mac before it. */
  external_pr?: number;
  /** That pull request as its host's unit names it (`↗!12` on gitlab.com); absent with none and from a Mac before it, which reads as `↗#n`. */
  external_reference?: ChangeRequestRef;
  /** The branch an outside step watches, which its line names; absent otherwise and from a Mac before it. */
  external_branch?: string;
  /** Where an outside step's work goes, its pull request's base or the default branch its branch is judged against; absent until read and from a Mac before it. */
  external_base?: string;
  /** An outside step's pull request state as GitHub last answered; absent for a branch, until read and from a Mac before it. */
  external_state?: PrState;
  /** What a waiting step waits on, when it is due, the progress of a mission it waits on and how it failed (PRD core-wait B22); absent from a Mac before waits. Its pull request is named by `external_reference`. */
  wait_target?: MissionWaitTarget;
  wait_until_unix_ms?: number;
  wait_progress?: { done: number; total: number };
  wait_outcome?: MissionWaitOutcome;
  /** The checks a failed CI wait named; absent for any other wait. */
  wait_checks?: string[];
  /** A waiting step Hide could not read this time; absent while it reads. */
  unread?: boolean;
  stage: number | null;
  /** When the step last went to work, its start or back from a finish; the step list's `작업 중 · N분` counts from it. Absent before it started and from a Mac that predates it, where the line has no time. */
  worked_at_unix_ms?: number | null;
  worker_agent: string | null;
  /** The worker's row in the phone's list; null when the list does not hold it, so there is nothing to open. */
  agent: AgentKey | null;
  escalation: WorkerEscalation | null;
};

/**
 * A step as the frame carries it (PRD node-presets D-11): a general step of the `research` or `verify` preset keeps that preset's
 * name as its kind, so a bundle from before general steps names it as before, and a Hide from before them sends only those names.
 */
export type PhoneMissionNodeOnWire = Omit<PhoneMissionNode, "kind"> & { kind: PhoneMissionNode["kind"] | "research" | "verify" | "external" };
export type PhoneMissionOnWire = Omit<PhoneMission, "nodes"> & { nodes: PhoneMissionNodeOnWire[] };

/**
 * The frame's missions with every step read as this bundle names it: an old kind name is a general step of that preset, and a
 * waiting step, which the frame still names `external` for a bundle from before waits (PRD core-wait D-18), is `wait`.
 */
export function missionsOnWire(items: readonly PhoneMissionOnWire[]): PhoneMission[] {
  return items.map((mission) => ({
    ...mission,
    nodes: mission.nodes.map((node) =>
      node.kind === "research" || node.kind === "verify" ? { ...node, kind: "general", preset: node.kind } : { ...node, kind: node.kind === "external" ? "wait" : node.kind },
    ),
  }));
}

/** A View linked to a mission's open decision (`PhoneDecisionView`). */
export type PhoneDecisionView = { device_id: string; display_id: string; view: string; kind: string; label: string; unseen?: boolean; closed?: boolean; by: "conductor" | "worker"; pane_id?: string; reopens?: ViewReopen[] };

/** A mission as the phone receives it: the shared rules' `MissionBrief`, with what the two screens draw. */
export type PhoneMission = MissionBrief<PhoneMissionNode> & {
  title: string;
  device_id: string;
  /** The branch the mission's results go to, as the Mac's map names it; absent when the Mac cannot read one, and from a Mac before destinations, so the phone names no branch. */
  destination?: string;
  /** The device a mission away from the paired Mac runs on; null on the Mac. */
  device_label: string | null;
  answer: MissionAnswer | null;
  answer_failed: MissionAnswerFailure | null;
  /** The Views linked to the open decision; absent when none. Each opens the 볼 것 of the agent that opened it. */
  decision_views?: PhoneDecisionView[];
  conductor: { present: boolean; takes_answers: boolean; name: string; agent_kind: string; agent: AgentKey | null };
  started_at_unix_ms: number | null;
  stale: boolean;
  /** A finished mission's steps hided left out to keep the frame within one relay message; `nodes` is empty then. */
  nodes_omitted?: number;
};

/** One run as the phone gets it (`hide-core` phone `PhoneRun`): no pane, branch, conversation or answer. */
export type PhoneAutomationRun = Pick<AutomationRun, "number" | "trigger" | "started_at_unix_ms" | "ended_at_unix_ms" | "state" | "attempt" | "reason" | "seen" | "spend">;

export type PhoneAutomationRecord = ({ kind: "run" } & PhoneAutomationRun) | Exclude<AutomationRecord, { kind: "run" }>;

/**
 * One automation as the phone gets it (`hide-core` phone `PhoneAutomation`): no folder, task or agent. `schedule` is the choice it
 * reads as, `records` the newest first with `records_omitted` older ones left on the desktop.
 */
export type PhoneAutomation = Pick<Automation, "id" | "name" | "next_run_at_unix_ms" | "running" | "unseen" | "paused"> & {
  schedule: AutomationSchedule;
  records: PhoneAutomationRecord[];
  records_omitted?: number;
};

/** The phone's own presses on an automation (PRD automations B34); making, editing and removing stay with an agent. */
export type AutomationAction = "run" | "pause" | "resume";

/** One project the PR tab lists pull requests of, with how its GitHub reads stand (`hided/src/mobile/prs.rs`, `PhoneProject`). */
export type PhonePrProject = {
  id: string;
  label: string;
  device_id: string;
  /** The device's name for a project away from the paired Mac; null on the Mac. */
  device_label: string | null;
  github: GithubStatus[];
};

/** What a card read beyond the list, as the phone gets it: no body and no threads. */
export type PhonePrCard = Pick<PrCard, "host" | "reading" | "head" | "files" | "files_total" | "merge" | "failed" | "write">;

/**
 * One listed pull request: the desktop list's value (`PullRequest`) with the board's group and reason, the agents it names on rows
 * the phone lists, whether a checkout of its branch exists, and the card the core read. `key` is `<project id>#<number>`.
 */
export type PhonePr = PullRequest & {
  key: string;
  project: string;
  group: PrGroupId;
  reason: PrReason;
  agents: AgentKey[];
  checkout: boolean;
  card: PhonePrCard | null;
};

/** The one feedback read the core holds, which a hand-over's prompt is built from. */
export type PhonePrFeedback = Pick<PrFeedback, "request_id" | "pr_number" | "phase" | "failed_checks" | "change_requests" | "failure">;

export type Notifications = "on" | "off" | "unasked";

export type RowsState = "ok" | "gone" | "device_unreachable" | "unavailable";

/** Which half of a detail hided reads for the phone. */
export type DetailView = "conversation" | "terminal";

/**
 * What a detail shows: one of the two halves, or the agent's Views. 볼 것 is the phone's own choice: hided sends the Views with
 * the detail and is never told which tab is up (PRD mobile-companion-ui D-06).
 */
export type DetailTab = DetailView | "looks";

/** What a View is on the phone: hided names it by what the phone can show of it (hided/src/mobile/looks.rs `Shown`). */
/** `html` is an HTML file of a checkout, which the phone draws itself in a sandboxed frame. */
export type LookKind = "changes" | "page" | "html" | "doc" | "image" | "other";

/** One row of the 볼 것 list: a View the agent opened that is open now. It carries no path; the phone asks for its content by `id` alone (D-16). */
export type LookRow = {
  id: string;
  kind: LookKind;
  name: string;
  unseen: boolean;
  /** When the agent opened it while unseen, else when it was last looked at (epoch ms). */
  at: number;
  /** A page's address. */
  url?: string;
};

export type LookFileStatus = "modified" | "added" | "deleted" | "untracked" | "renamed" | "conflict";

/**
 * One changed file: its path under the checkout and its line counts. The counts are null for a file git cannot count
 * (`binary`) and for a file Git does not track yet, which the desktop's list shows no number for either.
 */
export type LookFile = { path: string; status: LookFileStatus; added: number | null; removed: number | null; binary: boolean };

/** The answer states of `look_result`: everything that is not content. */
/** `not_found` and `outside_roots` say why a file cannot be read (gone since its View opened, outside every checkout), and
 * `grant_expired`, `file_replaced` and `no_window` why a file outside every checkout cannot (the desktop app no longer holds its
 * opening, it became a link or a folder, no desktop app is open to read it); a bundle from before them shows each as a failed read. */
export type LookResultState =
  | "page" | "other" | "too_large" | "closed" | "failed" | "gone" | "refused" | "unavailable" | "device_unreachable"
  | "not_found" | "outside_roots" | "grant_expired" | "file_replaced" | "no_window";

/** Why a page's screen cannot be taken, when the operator can act on it (no desktop app, the window on another computer, a page
 * the app has not drawn); absent for any other reason, and a bundle from before it ignores it and says the screen cannot be taken now. */
export type LookPageReason = "no_window" | "window_elsewhere" | "not_drawn";

/**
 * What a text, an image or a page's screenshot sends first (hided/src/mobile/looks.rs `chunks`). A page's (`mode: "page"`,
 * an image of the desktop app's page now) also names the address, when it was taken (ms) and the device that took it.
 * An HTML file's (`mode: "html"`) is one document with the files of its checkout it names already folded in.
 */
export type LookHead = {
  type: "look_head";
  request_id: string;
  view_id: string;
  mode: "text" | "markdown" | "image" | "page" | "html";
  size: number;
  total: number;
  truncated: boolean;
  chunks: number;
  url?: string;
  taken_at?: number;
  taken_by?: string;
};

/** One message of an agent's conversation (hided/src/mobile/conversation.rs). */
export type ConversationMessage = {
  /** The byte offset of its transcript record: unique and in order. */
  id: number;
  who: "you" | "agent" | "stopped";
  text: string;
  truncated: boolean;
  at_ms: number;
  /** The images an operator's message carries as the agent recorded them (a photo it took in from its path); absent at none. */
  images?: number;
  /** The files the agent recorded beside an operator's message by their paths, the last part of each; absent at none. */
  files?: string[];
};

/** The agent kinds a phone may start (hided/src/mobile/start.rs KINDS). */
export type StartKind = string;

/** One place a start can go; the folder it names never reaches the phone. */
export type StartTarget = {
  /** The checkout's id. */
  id: string;
  device_id: string;
  /** The place on the device: `project · branch`, or a folder's name; from a Mac that predates `project`, the sheet lists each target as a project of its own named by it. */
  place: string;
  /** The project's name alone, the sheet's project word; absent from a Mac that predates it. */
  project?: string;
  /** The project's id, which gathers its checkouts under one project; absent from a Mac that predates it. */
  project_id?: string;
  /** The checkout's branch, the sheet's branch word; null for a folder, and absent from a Mac that predates it. */
  branch?: string | null;
  /** The project's main checkout, where choosing the project starts; absent from a Mac that predates it. */
  main?: boolean;
  /** The device's name; null is the core's own machine, named from the catalog's `core_platform`. */
  device_label: string | null;
  connected: boolean;
};

export type StartKindEntry = { id: StartKind; models: string[] };

/** What the start sheet lists, as hided sends it while the sheet is open. */
export type StartCatalog = {
  targets: StartTarget[];
  /** The core's machine's system as its device row says it (`macos aarch64`), from which the phone says This Mac, This PC or This computer. */
  core_platform: string | null;
  kinds: StartKindEntry[];
  /** The kind and each kind's model the last start chose, shared with the desktop. */
  remembered: { kind: StartKind | null; models: Partial<Record<StartKind, string>> };
  /** How many places past hided's cap were left out, the ones used longest ago; absent from an older Mac. */
  omitted?: number;
};

export type ServerFrame =
  | { type: "paired"; credential: string; phone_id: string; name: string }
  /** `device_connect`: the computer offers connections between two Hides, and with them the gathered view (`gather.ts`); absent from every computer before 1.2.12. */
  /** `machine`: the computer's product device id, which another computer's `machines` names it by; sent with `device_connect`. */
  /**
   * `answer`: the computer takes a reply of several lines up to one phone message (`replyFits`), sends the open detail's `ask` and the
   * conversation's `running`, and takes an `input` that picks a choice; absent from every computer before it, which keeps the one-line
   * reply of `MAX_REPLY_CHARS`.
   */
  /** `attachments`: the computer takes files attached to a reply (`uploads.ts`); absent from every computer before it, which shows no `+`. */
  | { type: "hello"; mac_name: string | null; phone_id: string; name: string | null; vapid_public_key: string; notifications: Notifications; device_connect?: boolean; machine?: string; answer?: boolean; attachments?: boolean }
  // Older computers still send off; new phone settings do not use this global mode.
  /** `machines`: the other machines this computer opens that run work of their own (PRD device-connect-iroh B56); absent when none. */
  | { type: "meta"; push_mode: "off" | "app_closed" | "always"; other_phones: number; machines?: OtherMachine[] }
  /**
   * `interface_language` is the core's explicit choice as stored (en, ko, zh-CN, ja), or null while the phone follows its own language.
   * `color_theme` is the operator's palette (`hide`, `neutral`); the phone keeps its own light or dark.
   */
  /** `omitted` counts the agents hided left out to keep the frame within one relay message. */
  | { type: "agents"; groups: AgentGroup[]; omitted?: number; devices: PhoneDevice[]; interface_language: string | null; color_theme: string | null }
  /** The missions this Mac holds, newest first in each state; `failed` when the record could not be read; `omitted` counts the ones past the frame's caps. */
  /** `asks`: the listed missions that are their own 내 차례 item, oldest first, as the computer counts them in Needs You; absent from a Mac before it. */
  | { type: "missions"; items: PhoneMissionOnWire[]; failed: boolean; omitted?: number; asks?: string[] }
  /** hided's answer to `answer_mission`, `ask_mission_material` and `reopen_mission_view`; what the core does with it comes back on the mission itself. */
  | { type: "mission_result"; request_id?: string; mission?: string | null; ok: boolean; reason: string | null }
  /** The open pull requests this Mac lists, the cards it read and its one feedback read; `omitted` counts the ones past the frame's cap. */
  | { type: "prs"; projects: PhonePrProject[]; items: PhonePr[]; feedback: PhonePrFeedback | null; omitted?: number }
  /** The automations this computer's core runs, in its order; `omitted` counts the ones past the frame's caps; `zone` is the IANA zone their times are read in. */
  | { type: "automations"; items: PhoneAutomation[]; omitted?: number; zone?: string | null; asks?: string[]; unreadable?: boolean }
  /** hided's answer to an `automation` press: whether it sent the core's event; what the core did comes back on the automation. */
  | { type: "automation_result"; request_id?: string; action: AutomationAction | null; id?: string | null; ok: boolean; reason: string | null }
  /** hided's answer to a read or a merge: whether it sent the core's request; what GitHub did comes back on the card. */
  | { type: "pr_result"; request_id?: string; action: "read_pr" | "merge_pr" | "read_pr_feedback"; pr?: string | null; ok: boolean; reason: string | null }
  /** The holder watch's last screen mode; absent on older computers means false. */
  | { type: "rows"; device_id: string; pane_id: string; state: RowsState; text?: string; lines?: number; more?: boolean; alternate_screen?: boolean }
  | {
      type: "conversation";
      device_id: string;
      pane_id: string;
      /** `none`: the pane has no conversation to show, so its terminal stands alone. */
      state: "ok" | "none";
      mode?: "reset" | "append" | "older";
      messages?: ConversationMessage[];
      /** The cursor for the page before the oldest message; null at the start. */
      before?: number | null;
      /** What the agent runs now, on a reset or an append from a computer that answers (its hello says `answer`); null while nothing does. */
      running?: Running | null;
    }
  /** What the agent asks on its own screen, sent while its detail is open by a computer that answers; null once it asks nothing. */
  | { type: "ask"; device_id: string; pane_id: string; ask: PhoneAsk | null }
  /** The agent's open Views, newest first; sent again only when they change. `omitted` counts the ones past hided's cap. */
  | { type: "looks"; device_id: string; pane_id: string; views: LookRow[]; omitted: number }
  | { type: "look_result"; request_id: string; view_id: string; state: LookResultState; url?: string; reason?: LookPageReason }
  | { type: "look_changes"; request_id: string; view_id: string; files: LookFile[]; count: number; added: number; removed: number }
  | LookHead
  | { type: "look_chunk"; request_id: string; seq: number; data: string }
  | { type: "look_end"; request_id: string; view_id: string }
  | { type: "input_result"; request_id?: string; ok: boolean; reason: string | null }
  /** An upload frame taken (hided/src/mobile/uploads.rs): a chunk on disk (`seq`), every byte in (`done`), or refused with why (`failed`). */
  | { type: "attach_ack"; stage: string; seq?: number; begun?: boolean; done?: boolean; cancelled?: boolean; failed?: string }
  | { type: "human_turn_result"; request_id?: string; ok: boolean; reason: string | null }
  /** Accepted for dispatch; progress and refusal remain on the core-owned row. A pane's wake answers with `sleepTarget` of its key as `sleep_id`. */
  | { type: "sleep_result"; request_id?: string; sleep_id: string; ok: boolean; reason: string | null }
  | ({ type: "start_catalog" } & StartCatalog)
  | { type: "start_result"; request_id?: string; ok: boolean; reason: string | null; device_id?: string; pane_id?: string }
  | { type: "push_state"; notifications: "on" | "refused" }
  /** The answer to `ping`: hided is there. */
  | { type: "pong"; id: number }
  | { type: "refused"; reason: string }
  | { type: "refused_request"; request: string; reason: string };

/** The only messages a phone sends after its first frame. */
export type PhoneMessage =
  | { type: "open"; device_id: string; pane_id: string; view: DetailView }
  | { type: "view"; view: DetailView }
  | { type: "older"; before: number }
  | { type: "more" }
  | { type: "close" }
  /** Content of one View by its id, only while it is open; hided refuses anything else and logs it (D-16). */
  | { type: "look_open"; request_id: string; device_id: string; pane_id: string; view_id: string }
  /** The chunks up to `seq` arrived; hided sends a window of four ahead of the last one confirmed. */
  | { type: "look_ack"; request_id: string; seq: number }
  /** The View's content was shown: it is seen, here and on the desktop (D-15). */
  | { type: "look_seen"; device_id: string; pane_id: string; view_id: string }
  | { type: "wake_sleeping_session" | "check_sleeping_session"; request_id: string; sleep_id: string }
  | { type: "wake_agent"; request_id: string; device_id: string; pane_id: string; fresh?: boolean }
  | { type: "human_turn_done"; request_id: string; device_id: string; pane_id: string; declaration: string }
  /** A reply's `stages` are the files uploaded for it (`uploads.ts`), typed as their paths before its words. */
  | ({ type: "input"; request_id: string; device_id: string; pane_id: string } & ({ text: string; stages?: string[] } | { key: QuickKey } | { turn: string; kind: PhoneAsk["kind"]; choice: string }))
  /** A file attached to the reply being written, uploaded one phone message at a time; only to a computer whose hello says `attachments`. */
  | { type: "attach_begin"; stage: string; name: string; size: number }
  | { type: "attach_chunk"; stage: string; seq: number; data: string }
  | { type: "attach_end"; stage: string }
  | { type: "attach_cancel"; stage: string }
  /** `option` counts from 1; one of `option`, `text` or `resend` (the desktop banner's own three). */
  | { type: "answer_mission"; request_id: string; mission: string; key: number; option?: number; text?: string; resend?: boolean }
  /** The decision card's `자료 요청` for the question `key` names; the core asks the Conductor once per question. */
  | { type: "ask_mission_material"; request_id: string; mission: string; key: number }
  /** A closed View of the decision opened again on the Mac first; the row's `reopens` says what became of it. */
  | { type: "reopen_mission_view"; request_id: string; mission: string; key: number; view: string }
  /** The PR tab's own requests: `pr` is the list's key (`PhonePr.key`), `head` the card's head commit the merge was confirmed for. */
  | { type: "read_pr"; request_id: string; pr: string; look_again?: boolean }
  | { type: "merge_pr"; request_id: string; pr: string; head: string; method?: PrMergeMethod }
  | { type: "read_pr_feedback"; request_id: string; pr: string }
  /** [지금 실행], [멈춤] or [다시 켜기] on a listed automation, answered by an `automation_result`. */
  | { type: "automation"; request_id: string; action: AutomationAction; id: string }
  /** Answered by a `start_result`, like `start_agent`; the agent starts where the desktop would start it and the desktop's screen stays. */
  | { type: "delegate_pr"; request_id: string; pr: string; prompt: string; kind: StartKind; model?: string }
  | { type: "start_sheet"; open: boolean }
  | { type: "start_agent"; request_id: string; text: string; target: string; kind: StartKind; model?: string }
  | { type: "push_subscription"; subscription: { endpoint: string; keys: { p256dh: string; auth: string } } }
  | { type: "push_permission"; state: "denied" | "default" }
  /** The iOS app's notifications: Apple's token for this install and the key this computer encrypts each notice with. */
  | { type: "apns_registration"; token: string; environment: "sandbox" | "production"; key: { p256dh: string; auth: string } }
  /** The Android app's notifications: Google's FCM token for this install and the same per-computer key; FCM has one host, so no environment. */
  | { type: "fcm_registration"; token: string; key: { p256dh: string; auth: string } }
  /** Asks whether hided still answers; the phone gives up the socket when nothing comes back. */
  | { type: "ping"; id: number };

export type AgentKey = { device_id: string; pane_id: string };

/** The target a pane's wake is answered under (`AgentKey::tag` in hided): one id per pane, as a SleepId is per conversation. */
export function sleepTarget(key: AgentKey): string {
  return `${key.device_id}|${key.pane_id}`;
}

export function sameKey(a: AgentKey | null, b: AgentKey | null): boolean {
  return !!a && !!b && a.device_id === b.device_id && a.pane_id === b.pane_id;
}

/** The sentences that carry a count limit, named by `{{limit}}`. */
type LimitKey = "mobile.input.tooLong" | "mobile.start.tooLong" | "mobile.refusal.phoneLimit";

/** The other input, refusal, row and start sentences carry no value. */
type PlainKey = Exclude<Extract<MessageKey, `mobile.input.${string}` | `mobile.refusal.${string}` | `mobile.rows.${string}` | `mobile.start.${string}` | `mobile.mission.answer.${string}` | `mobile.mission.material.${string}` | `mobile.mission.reopen.${string}` | `mobile.prs.refusal.${string}` | `mobile.automations.refusal.${string}`>, LimitKey>;

/** A sentence the phone shows, kept as its catalog key so it follows a language change. */
export type Notice = { key: PlainKey } | { key: LimitKey; limit: number };

/** `words` fills a notice about one pull request with what its host calls it (`noun`). */
export function noticeText(t: TFunction<"translation">, notice: Notice, words?: { noun: string }): string {
  return "limit" in notice ? t(notice.key, { limit: notice.limit }) : t(notice.key, words);
}

export const GROUP_TITLE: Record<GroupId, MessageKey> = {
  needs_you: "mobile.group.needs_you",
  done: "mobile.group.done",
  revisit: "mobile.group.revisit",
  working: "mobile.group.working",
  seen: "mobile.group.seen",
};


/** The five quick keys (B26, B38): what hided sends the engine, the keycap, and the accessible name. */
export type QuickKey = "enter" | "escape" | "up" | "down" | "ctrl_c";

export const QUICK_KEYS: readonly { key: QuickKey; label: string; name: MessageKey }[] = [
  { key: "enter", label: "Enter", name: "mobile.key.enter" },
  { key: "escape", label: "Esc", name: "mobile.key.escape" },
  { key: "up", label: "↑", name: "mobile.key.up" },
  { key: "down", label: "↓", name: "mobile.key.down" },
  { key: "ctrl_c", label: "Ctrl-C", name: "mobile.key.ctrlC" },
];

/** The longest reply a computer before `answer` writes in one input, on one line. */
export const MAX_REPLY_CHARS = 2000;

/** The most bytes one message from the phone may hold; hided drops a longer one unread (hided/src/mobile/phone.rs MAX_FRAME). */
/**
 * What the agent asks on its own screen (hided/src/mobile/ask.rs, Doop 03, 04): its question with each choice's note, its plan
 * waiting for approval, or a tool call waiting for permission with the first of the command it runs. `turn` names the ask an
 * answer is written for; `answerable` says the computer types a picked choice as the keys the agent's screen takes, and when it is
 * false the choices are shown and answered in the terminal. A question's choice is its place (`1` for the first); a plan's is
 * `approve` or `keep_planning`; a permission's is `allow`, `allow_always` or `deny`.
 */
export type PhoneAsk = {
  kind: "question" | "plan_approval" | "permission";
  turn: string;
  since_ms: number | null;
  text?: string;
  truncated?: boolean;
  command?: string;
  command_truncated?: boolean;
  /** The call the computer tied the ask to, when it could: the same call under a new turn is the same ask. */
  call?: string;
  choices: { id: string; label?: string; note?: string }[];
  answerable: boolean;
};

/**
 * Which ask an answer belongs to: the agent's call when the computer names one, else the turn and what it asks, so a new
 * question or approval with the same words is a new ask and an answered one that stays open under a new turn is not.
 */
export function askIdentity(key: AgentKey, ask: PhoneAsk): string {
  const what = ask.call !== undefined ? `call ${ask.call}` : `turn ${ask.turn} ${JSON.stringify([ask.text ?? null, ask.command ?? null, ask.choices])}`;
  return `${key.device_id}|${key.pane_id}|${ask.kind}|${what}`;
}

/** What `identity` (`askIdentity`) was made on when it is one of `key`'s asks: its call, or its turn; null for another agent's. */
export function askOf(identity: string, key: AgentKey): { call: string } | { turn: number } | null {
  const agent = `${key.device_id}|${key.pane_id}|`;
  if (!identity.startsWith(agent)) return null;
  const rest = identity.slice(agent.length);
  const what = rest.slice(rest.indexOf("|") + 1);
  if (what.startsWith("call ")) return { call: what.slice("call ".length) };
  return { turn: Number(what.slice("turn ".length, what.indexOf(" ", "turn ".length))) };
}

/**
 * What runs now under the agent's turn, by kind and since when, never by what it runs (Doop 02); `unknown` when the computer
 * kept fewer of the agent's open calls than it made, so it cannot say.
 */
export type Running = { kind: string; since_ms: number | null };

/** The input that picks `choice` on the ask the phone showed: typed by the computer as keys, never as text. */
export function choiceMessage(key: AgentKey, ask: Pick<PhoneAsk, "turn" | "kind">, choice: string, requestId: string): Extract<PhoneMessage, { type: "input" }> {
  return { type: "input", request_id: requestId, device_id: key.device_id, pane_id: key.pane_id, turn: ask.turn, kind: ask.kind, choice };
}

export const MAX_FRAME_BYTES = 16 * 1024;

/**
 * More than the request id `newRequestId` makes for the next decades (`<time in base 36>-<counter>-<16 hex>`), so a reply measured
 * while it is written, before its id exists, is measured at least as large as the message that leaves.
 */
export const REQUEST_ID_BYTES = 40;

/** The `input` a reply leaves in, with the stages of the files it attaches, the one shape `sendReply` sends and `replyFits` measures. */
export function replyMessage(key: AgentKey, text: string, requestId: string, stages: readonly string[] = []): Extract<PhoneMessage, { type: "input" }> {
  return { type: "input", request_id: requestId, device_id: key.device_id, pane_id: key.pane_id, text, ...(stages.length > 0 ? { stages: [...stages] } : {}) };
}

/** A stage's length (a UUID), for measuring a reply whose files are still uploading. */
const STAGE_CHARS = 36;

/** Whether a reply fits one phone message as JSON, its escapes and its files' stages counted (a new line, a quote and a tab take two bytes each). */
export function replyFits(key: AgentKey, text: string, requestId = "x".repeat(REQUEST_ID_BYTES), files = 0): boolean {
  const stages = Array.from({ length: files }, () => "x".repeat(STAGE_CHARS));
  return new TextEncoder().encode(JSON.stringify(replyMessage(key, text, requestId, stages))).length <= MAX_FRAME_BYTES;
}

/**
 * Why a reply cannot be sent yet, checked before it leaves the phone (B26, B27). A computer with `answer` (`lines`) takes new lines
 * and tabs and any length one message holds, the rule of the desktop conversation; one before it takes one line of 2,000 characters.
 */
export function replyProblem(text: string, lines: boolean, key?: AgentKey, files = 0): string | null {
  if (text.trim().length === 0 && files === 0) return "empty";
  if (lines) {
    // eslint-disable-next-line no-control-regex
    if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(text)) return "control_characters";
    return key && !replyFits(key, text, undefined, files) ? "too_long" : null;
  }
  if ([...text].length > MAX_REPLY_CHARS) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f-\u009f]/.test(text)) return "control_characters";
  return null;
}

/** The line under the reply bar for a refused or failed input; `lines` words a reply too long for one message, which has no count. */
export function inputFailure(reason: string | null, lines = false): Notice {
  switch (reason) {
    case "uncertain":
      return { key: "mobile.input.uncertain" };
    case "in_flight":
      return { key: "mobile.input.inFlight" };
    case "empty":
      return { key: "mobile.input.empty" };
    case "too_long":
      return lines ? { key: "mobile.input.tooLongMessage" } : { key: "mobile.input.tooLong", limit: MAX_REPLY_CHARS };
    case "control_characters":
      return lines ? { key: "mobile.input.controlCharacter" } : { key: "mobile.input.controlCharacters" };
    case "gone":
      return { key: "mobile.input.gone" };
    case "stale":
    case "unknown_choice":
      return { key: "mobile.input.stale" };
    case "screen_differs":
      return { key: "mobile.input.screenDiffers" };
    case "ask_not_shown":
      return { key: "mobile.input.askNotShown" };
    case "answers_full":
      return { key: "mobile.input.answersFull" };
    case "asleep":
      return { key: "mobile.input.asleep" };
    case "device_unreachable":
      return { key: "mobile.input.deviceUnreachable" };
    case "attachment_lost":
      return { key: "mobile.input.attachmentLost" };
    case "attachment_failed":
      return { key: "mobile.input.attachmentFailed" };
    case "offline":
      return { key: "mobile.input.offline" };
    default:
      return { key: "mobile.input.failed" };
  }
}

/** The line under a mission's answer for one hided refused or the connection lost (B35); a changed question, a missing Conductor and a lost device come back on the mission, in the desktop's words. */
/** Why a `자료 요청` did not reach the core, in the phone's words. */
export function missionMaterialFailure(reason: string | null): Notice {
  switch (reason) {
    case "gone":
      return { key: "mobile.mission.material.gone" };
    case "offline":
      return { key: "mobile.mission.material.offline" };
    default:
      return { key: "mobile.mission.material.failed" };
  }
}

/** Why a closed decision View did not open again, in the phone's words: the question moved on, the Mac was out of reach, or it opened nothing. */
export function missionReopenFailure(reason: string | null): Notice {
  switch (reason) {
    case "gone":
      return { key: "mobile.mission.reopen.gone" };
    case "offline":
      return { key: "mobile.mission.reopen.offline" };
    default:
      return { key: "mobile.mission.reopen.failed" };
  }
}

export function missionAnswerFailure(reason: string | null): Notice {
  switch (reason) {
    case "in_flight":
      return { key: "mobile.mission.answer.inFlight" };
    case "gone":
      return { key: "mobile.mission.answer.gone" };
    case "offline":
      return { key: "mobile.mission.answer.offline" };
    default:
      return { key: "mobile.mission.answer.failed" };
  }
}

/** The line under a pull request's buttons for a read or merge hided refused or the connection lost (B42, B43). */
export function prFailure(reason: string | null): Notice {
  switch (reason) {
    case "head_moved":
      return { key: "mobile.prs.refusal.headMoved" };
    case "gone":
    case "settled":
      return { key: "mobile.prs.refusal.gone" };
    case "unread":
      return { key: "mobile.prs.refusal.unread" };
    case "in_flight":
      return { key: "mobile.prs.refusal.inFlight" };
    case "offline":
      return { key: "mobile.prs.refusal.offline" };
    default:
      return { key: "mobile.prs.refusal.failed" };
  }
}

/** The line under an automation's buttons for a press hided refused or the connection lost. */
export function automationFailure(reason: string | null): Notice {
  switch (reason) {
    case "running":
      return { key: "mobile.automations.refusal.running" };
    case "gone":
      return { key: "mobile.automations.refusal.gone" };
    case "in_flight":
      return { key: "mobile.automations.refusal.inFlight" };
    case "offline":
      return { key: "mobile.automations.refusal.offline" };
    default:
      return { key: "mobile.automations.refusal.failed" };
  }
}

/** A refused connection's screen (B13, B14, B16). */
export type Refusal = "code_expired" | "phone_limit" | "revoked" | "mobile_off" | "no_credential";

export function refusalOf(reason: string): Refusal {
  switch (reason) {
    case "code_expired":
    case "phone_limit":
    case "revoked":
    case "mobile_off":
      return reason;
    default:
      return "revoked";
  }
}

/** The phones hided pairs at once (hided/src/mobile/phones.rs MAX_PHONES). */
export const MAX_PHONES = 4;

export const REFUSAL_NOTICE: Record<Refusal, Notice> = {
  code_expired: { key: "mobile.refusal.codeExpired" },
  phone_limit: { key: "mobile.refusal.phoneLimit", limit: MAX_PHONES },
  revoked: { key: "mobile.refusal.revoked" },
  mobile_off: { key: "mobile.refusal.offline" },
  no_credential: { key: "mobile.refusal.noCredential" },
};

export const UNREACHABLE_NOTICE = REFUSAL_NOTICE.mobile_off;

/** The detail's line for a pane hided could not read (B28). */
export function rowsProblem(state: RowsState): Notice | null {
  switch (state) {
    case "ok":
      return null;
    case "gone":
      return { key: "mobile.input.gone" };
    case "device_unreachable":
      return { key: "mobile.rows.deviceUnreachable" };
    default:
      return { key: "mobile.rows.unavailable" };
  }
}

/**
 * A row drawn only with box-drawing characters, such as the rule a TUI draws
 * across the desktop pane: wrapped at a phone's width it becomes a stack of
 * rules, so the detail clips it to one line instead.
 */
export function boxDrawingRow(text: string): boolean {
  return /^[\s\u2500-\u257f]*[\u2500-\u257f][\s\u2500-\u257f]*$/.test(text);
}

/** The most messages a detail holds; at it, no older page is asked for. */
export const MAX_MESSAGES = 300;

export type Conversation = { messages: ConversationMessage[]; before: number | null };

/** What a detail holds after one conversation frame: a fresh page, appended messages, or an older page. */
export function mergeConversation(
  current: Conversation | null,
  frame: { mode?: "reset" | "append" | "older"; messages?: ConversationMessage[]; before?: number | null },
): Conversation {
  const incoming = frame.messages ?? [];
  if (!current || frame.mode === "reset" || !frame.mode) return { messages: incoming, before: frame.before ?? null };
  if (frame.mode === "older") {
    const first = current.messages[0]?.id ?? Number.POSITIVE_INFINITY;
    return { messages: [...incoming.filter((message) => message.id < first), ...current.messages], before: frame.before ?? null };
  }
  const last = current.messages.at(-1)?.id ?? -1;
  const messages = [...current.messages, ...incoming.filter((message) => message.id > last)];
  if (messages.length <= MAX_MESSAGES) return { messages, before: current.before };
  // Past the cap the oldest go, and the page before the new oldest is still there to read.
  const kept = messages.slice(-MAX_MESSAGES);
  return { messages: kept, before: kept[0]?.id ?? null };
}

/** A message's time: the hour and minute today, with the day before today. */
export function messageTime(atMs: number, now: Date = new Date()): string {
  const at = new Date(atMs);
  const time = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
  const sameDay = at.getFullYear() === now.getFullYear() && at.getMonth() === now.getMonth() && at.getDate() === now.getDate();
  return sameDay ? time : `${at.getMonth() + 1}/${at.getDate()} ${time}`;
}

/** The QR's payload (D-22): `{v:1, endpoint, code}` as base64url JSON. */
export type PairPayload = { v: 1; endpoint: string; code: string };

function fromBase64Url(text: string): string {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  return new TextDecoder().decode(Uint8Array.from(binary, (char) => char.charCodeAt(0)));
}

export function toBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function parsePair(value: string): PairPayload | null {
  try {
    const parsed: unknown = JSON.parse(fromBase64Url(value));
    if (typeof parsed !== "object" || parsed === null) return null;
    const { v, endpoint, code } = parsed as Record<string, unknown>;
    if (v !== 1 || typeof endpoint !== "string" || typeof code !== "string" || code.length === 0) return null;
    return { v: 1, endpoint, code };
  } catch {
    return null;
  }
}

/** What the page address carries after `#` (D-22): a pairing code, a credential, a deep link. */
export type Fragment = { pair: PairPayload | null; credential: string | null; open: AgentKey | null };

export function parseFragment(hash: string): Fragment {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const pair = params.get("pair");
  const credential = params.get("k");
  const open = params.get("open");
  return {
    pair: pair ? parsePair(pair) : null,
    credential: credential && /^[0-9a-f]{64}$/.test(credential) ? credential : null,
    open: open ? openKey(open) : null,
  };
}

/** A deep link names one agent as `device|pane`, the notification tag's form (D-17). */
export function openKey(value: string): AgentKey | null {
  const bar = value.indexOf("|");
  if (bar <= 0 || bar === value.length - 1) return null;
  return { device_id: value.slice(0, bar), pane_id: value.slice(bar + 1) };
}

export function keyTag(key: AgentKey): string {
  return `${key.device_id}|${key.pane_id}`;
}

/**
 * The page address a paired phone keeps, so "Add to Home Screen" carries the
 * credential into the installed app, whose storage iOS may keep apart from
 * Safari's (PRD Risks).
 */
export function credentialHash(credential: string): string {
  return `#k=${credential}`;
}

/** The Mac's name before pairing: the first label of the ts.net address; empty when the address names none. */
export function macNameOf(endpoint: string): string {
  try {
    return new URL(endpoint).hostname.split(".")[0] ?? "";
  } catch {
    return "";
  }
}

/** The Mac's name with the names of the devices it reaches that are connected: `MacBook Pro + Mac mini` (PRD mobile-companion-ui B8). */
/** The name the phone shows for the device `id`: the paired Mac's own name for this Mac, the label of a connected device otherwise. */
export function deviceNameOf(devices: readonly PhoneDevice[], id: string, macName: string): string {
  const device = devices.find((candidate) => candidate.device_id === id);
  return device && !device.local && device.label ? device.label : macName;
}

export function machinesName(macName: string, devices: readonly PhoneDevice[]): string {
  const names = devices.filter((device) => !device.local && device.connected && device.label).map((device) => device.label);
  return names.length > 0 ? `${macName} + ${names.join(", ")}` : macName;
}

/** The list header's second line (B19). */
export function headerLine(t: TFunction<"translation">, macName: string, otherPhones: number, devices: readonly PhoneDevice[] = []): string {
  const name = machinesName(macName, devices);
  return otherPhones > 0 ? t("mobile.otherPhones", { name, count: otherPhones }) : name;
}

/** Tags whose notifications the open app closes (D-21): agents no longer waiting or done. */
export function staleTags(tags: readonly string[], groups: readonly AgentGroup[]): string[] {
  const live = new Set<string>();
  for (const agent of listedAgents(groups)) {
    if (agent.holds_notification) live.add(keyTag({ device_id: agent.device_id, pane_id: agent.root_pane_id }));
  }
  return tags.filter((tag) => !live.has(tag));
}

/** The unseen Views first and then the seen ones, each in hided's order (newest first): what the 볼 것 list draws (B12). */
export function splitLooks(rows: readonly LookRow[]): { unseen: LookRow[]; seen: LookRow[] } {
  return { unseen: rows.filter((row) => row.unseen), seen: rows.filter((row) => !row.unseen) };
}

/** The most characters of a View's name a reply carries in front of it. */
const ABOUT_CHARS = 60;

/**
 * A reply sent while looking at a View begins with its name in brackets, so the agent knows which result the words are about (B30).
 * A control character in a name would make the reply refused, so it becomes a space.
 */
export function replyAbout(name: string, text: string): string {
  // eslint-disable-next-line no-control-regex
  const clean = name.replace(/[\u0000-\u001f\u007f-\u009f]+/g, " ").replace(/[[\]]/g, "").trim();
  const shown = [...clean].slice(0, ABOUT_CHARS).join("");
  return shown ? `[${shown}] ${text}` : text;
}
