/* Generated from contracts/hided-ws.schema.json. */

export type HidedWs =
  | Handshake
  | ServerFrame
  | ClientEvent
  | WorkspaceQuery
  | WorkspaceAction
  | LinksQuery
  | MemoryHookAsk
  | ProjectOpen
  | WorkspaceResult;
export type LimitCell =
  | {
      state: "unsupported";
    }
  | {
      state: "loading";
    }
  | {
      state: "unavailable";
      cause: UsageCause;
      last_checked_at_ms: number | null;
    }
  | {
      state: "reading";
      value: LimitReading;
      freshness: "fresh" | "stale" | "fallback";
      observed_at_ms: number;
    };
export type UsageCause =
  | "signed_out"
  | "not_installed"
  | "offline"
  | "timed_out"
  | "rate_limited"
  | "format_changed"
  | "capped"
  | "no_records";
export type WindowKind = "five_hour" | "weekly" | "monthly";
export type WindowOutcome =
  | {
      state: "not_started";
    }
  | {
      state: "open";
      used_percent: number;
      resets_at_s: number;
    }
  | {
      state: "expired";
      reset_at_s: number;
    }
  | {
      state: "not_offered";
    }
  | {
      state: "unreadable";
      cause: UsageCause;
    };
export type ConsumptionCell =
  | {
      state: "unsupported";
    }
  | {
      state: "loading";
    }
  | {
      state: "unavailable";
      cause: UsageCause;
      last_checked_at_ms: number | null;
    }
  | {
      state: "reading";
      value: ConsumptionReading;
      freshness: "fresh" | "stale" | "fallback";
      observed_at_ms: number;
    };
export type WorkspaceRequestId = string;
export type WorkspaceCommand =
  | {
      action: "open_file";
      path: string;
      beside: boolean;
    }
  | {
      action: "open_diff";
      path: string;
      beside: boolean;
    }
  | {
      action: "open_browser";
      url: string;
      beside?: boolean;
    }
  | {
      action: "select";
      view_id: string;
    }
  | {
      action: "close";
      view_id: string;
    }
  | {
      action: "split";
      view_id: string;
      area_id: string;
      edge: "left" | "right" | "up" | "down";
    }
  | {
      action: "move";
      view_id: string;
      area_id: string;
      index: number;
    };
export type SnapshotSections =
  | UsageSnapshot1
  | UsagePreferences
  | UsagePinSetPayload
  | UsageHiddenSetPayload
  | UiStateUsageHints
  | FocusCheckoutPayload
  | InterfaceLanguageSetPayload
  | AgentLogoMode
  | AgentLogoModeSetPayload
  | PaneTurnRowMode
  | PaneTurnRowModeSetPayload
  | ColorThemeSetPayload
  | TerminalFrame
  | AgentNotice
  | DesktopNoticesPayload;
/**
 * The core's shared explicit interface language. Null follows each client's primary system language, with English for unsupported languages. Invalid stored values publish en with a diagnostic and remain stored until an explicit edit.
 */
export type InterfaceLanguage = null | "en" | "ko" | "zh-CN" | "ja";
/**
 * Whether the sidebar, the Sessions list and the tab bar draw an agent's provider logo before its title. Hidden is the default and what a store without the field, or with a value this build does not know, publishes.
 */
export type AgentLogoMode = "shown" | "hidden";
/**
 * How an agent pane draws its turn row, the operator's last request and the agent's answer, over the terminal: one_line keeps every turn and ask on one line and is the default, what a store without the field or with a value this build does not know publishes (with the diagnostic ui_state.pane_turn_row_mode_unknown); two_lines gives the request its own line above the answer; off draws no turn row, an ask rides the pane head as a mark, and a pane with live children keeps its children button.
 */
export type PaneTurnRowMode = "one_line" | "two_lines" | "off";
/**
 * The palette every window draws in, `ui_state.color_theme`: hide is the landing page's Owl Field colors and the default (a store without the field opens hide), neutral the grey chrome. Independent of `ui_state.theme`, which picks light, dark or system. A stored value the core does not know opens hide with the diagnostic ui_state.color_theme_unknown.
 */
export type ColorTheme = "hide" | "neutral";

export interface HideWireTypes {
  frame?: HidedWs;
  sections?: SnapshotSections;
  [k: string]: unknown;
}
export interface Handshake {
  token: string;
  schema_version: 2;
  /**
   * `web` and `desktop` are the shell's windows. `cli` is `hide <path>` on the core's own machine: the connection carries one projectOpen request and its workspace_result, and is never a screen.
   */
  client_kind?: "web" | "desktop" | "cli";
  have_revision?: number;
  /**
   * Explicitly requests independently retained rest fields, with nested navigator, terminal and status omissions. Missing keeps schema-2 complete-rest replacement. Unknown formats close with schema_mismatch (4003).
   */
  snapshot_format?: "semantic-rest-v1";
  /**
   * The `core_instance` of the daemon frame that came before the snapshot `have_revision` was read from. A core started since (a move, a rollback or an update) answers a whole snapshot whatever `have_revision` says, since each core numbers its own revisions.
   */
  have_core?: string;
  /**
   * The `terminal_sequence` of the last terminal frame a reconnecting client applied. With a delta and the `have_terminal_epoch` of the hub that gave it, it resumes every pane from there (a pane the hub's ring no longer reaches is drawn again from a full frame); a client that kept its terminals and names none, or names it without this hub's epoch, has every pane drawn again; a client given a whole snapshot ignores it.
   */
  have_terminal_sequence?: number;
  /**
   * The `terminal_epoch` of the terminal frame `have_terminal_sequence` came from. Each daemon's hub draws its own, so a page that outlived its daemon names an epoch the new hub does not have and is drawn again whole.
   */
  have_terminal_epoch?: string;
}
/**
 * snapshot/delta carry the core state; terminal carries pane output beside them, never through the core (see terminalFrame); error answers a malformed client event; directory_list answers a `file_list`, and path_refused answers any event whose path failed its boundary - the $HOME line for `create_workspace` and `clone_repository`, the checkout-root line for `file_list`, `file_open`, `reveal_path`, `file_save`, `file_create`, `dir_create`, `path_rename`, `path_move`, `path_copy`, `path_import`, `path_trash` and `file_bytes` (see directoryList and pathRefused). A `file_list`, `file_open`, `reveal_path`, `file_save`, `file_create`, `dir_create`, `path_rename`, `path_move`, `path_copy`, `path_import` or `path_trash` that names an SSH device in `device_id` never meets this machine's checkout roots: the core accepts it only for the checkout in front on that device and the device's helper confines every path to that checkout's opened root. file_bytes_error answers a `file_bytes` read the daemon could not serve as bytes (see fileBytes). directory_unavailable answers a `file_list` for a checkout on an SSH device that its helper could not list (see directoryUnavailable). directory_changed announces that a watched folder changed, so the client re-reads that one folder (see directoryChanged). attachment_refused answers an `attachment_stage` or `attachment_commit` that could not be staged (see attachments). file_index_result answers a `file_index` query with the ranked checkout paths (see fileIndex). project_target answers a `project_target` probe (see projectTarget and projectTargetResult). clone_target answers a `clone_target` question about where a clone would land (see cloneTarget). daemon is the first frame after a valid handshake and describes the daemon itself (see daemonInfo). agent_notice reaches only a desktop window, and only while one has asked for it with `desktop_notices` (see agentNotice and desktopNoticesPayload). core_move is the core move as the process's supervisor holds it, which every role of the process sends its screens, a node holding its screen from the moment a move is under way, and the only frame a screen gets while a move holds it (`hided/src/core_move/control.rs` `MoveView`); once the move is under way it carries `alias`, the SSH alias the move reaches its other machine by, which a screen that has not drawn the core names that machine by. core_link is where a node's link to its core stands, sent to a screen the node holds while the core cannot draw it: `phase` `connecting`, `updating`, `waiting`, `stopped` or `disconnected`, `machine`, the core machine's name when its hello gave it, else null, `alias`, the SSH alias the node dials its core by, which a screen that has not drawn the core names instead of an empty machine, and `problem`, the ssh failure's code (`sign_in_needed`, `host_key_changed`, ...) while `stopped`, else null. `stopped` is a failure only the operator can fix: nothing dials again until a window connects (`hide connect`) or a screen's `core_link` event {action: `retry`}. `disconnected` is the operator's own end of the link: a screen's `core_link` event {action: `disconnect`} ends it from that machine's node daemon, which records it so a restart keeps it, and {action: `reconnect`} restores it; the node takes that event, and a `core_move` event, even while it holds the screen, and answers a change it could not make with an error frame whose payload is {kind: `core_link`, reason: `not_changed`}. browser_comment answers a window's `browser_comment` event, which turns Comment mode on a browser View on or off: {workspace, display_id, on, reason} names the View in its checkout, since display ids repeat across checkouts, and on: false says the mode ended, with `reason` null when the operator ended it, or why it could not start or went on (`display_busy`, `comment_limit`, `comment_unavailable_here`, or the page's lease failure); while it is on, a frame with `pick` {id, anchor, at} is a place the operator picked in the page, whose screenshot hided holds, one with `open` {id, at} a pin they pressed, `at` being where it shows as fractions of the page's viewport, and one with `dropped` true says the page navigated, to another document or within the same one, and the held pick is gone; the window writes the note itself and adds it with a `browser_comment` event carrying `add` {pick, note}; only an operator's screen (a desktop or web window whose connection its token proved, never a pane's scoped connection) may send it or `view_comment` and `view_feedback_send`. A `file_bytes` read that succeeds is answered with one or more binary frames on the same socket, not a text frame (see fileBytes). window_call reaches every window of the core's machine after a `hide <path>` brought a project in front (see projectOpen), its payload {device_id, path, checkout_id} naming that project's checkout, `checkout_id` null while a device's new project is not listed yet: a desktop window comes forward, and every window shows that project once the core has it in front.
 */
export interface ServerFrame {
  type:
    | "daemon"
    | "snapshot"
    | "delta"
    | "terminal"
    | "error"
    | "directory_list"
    | "directory_unavailable"
    | "path_refused"
    | "project_target"
    | "file_bytes_error"
    | "directory_changed"
    | "file_index_result"
    | "attachment_refused"
    | "open_external_result"
    | "clone_target"
    | "core_move"
    | "core_link"
    | "agent_notice"
    | "browser_comment"
    | "window_call";
  payload: {
    /**
     * Present only after snapshot_format negotiation. Each rest field replaces one semantic value; omissions retain, explicit null/empty values clear. Navigator, terminal and status merge their independently revisioned fields. Cursor zero/reset carries all fields.
     */
    rest_format?: "semantic-rest-v1";
    usage?: UsageSnapshot;
    [k: string]: unknown;
  };
  message?: string;
}
/**
 * The independently revisioned top-level usage section in a snapshot or delta. Delta omission preserves the previous value; an empty nodes array clears it. A fresh snapshot, including a future-cursor reset, always carries the complete section. Usage-only changes do not resend rest.
 */
export interface UsageSnapshot {
  nodes: NodeUsage[];
}
export interface NodeUsage {
  node: string;
  agents: AgentUsage[];
  history: UsageHistory | null;
  /**
   * Each of the node's projects' last seven days of cost (PRD lane-two-columns B31): absent until the first build succeeds, so a cost not read yet is not drawn as zero (B33); a project that ran no priced call is not listed. Amounts are millionths of a dollar.
   */
  projects?: ProjectCost[];
  /**
   * The latest build of projects failed; the last built list stays. Absent otherwise.
   */
  projects_failed?: boolean;
}
export interface AgentUsage {
  agent: string;
  limits: LimitCell;
  consumption: ConsumptionCell;
}
export interface LimitReading {
  /**
   * @maxItems 16
   */
  windows: WindowReading[];
}
export interface WindowReading {
  kind: WindowKind;
  scope:
    | {
        kind: "all";
      }
    | {
        kind: "model";
        label: string;
      };
  outcome: WindowOutcome;
}
export interface ConsumptionReading {
  popover: UsageRange;
  year: UsageRange;
  retained: {
    since: string | null;
    amount: UsageAmount;
    /**
     * @maxItems 32
     */
    providers: string[];
  } | null;
}
export interface UsageRange {
  /**
   * Inclusive civil-date calendar bound owned by the core.
   */
  calendar_start: string;
  /**
   * Inclusive civil-date calendar bound owned by the core.
   */
  calendar_end: string;
  /**
   * Sparse recorded days, including calls with zero new tokens. The shell fills omitted dates inside the inclusive calendar bounds.
   *
   * @maxItems 371
   */
  days: UsageDay[];
  amount: UsageAmount;
  since: string | null;
  active_days: number;
  observed_days: number;
  peak: {
    date: string;
    new_tokens: number;
  } | null;
  current_streak: number;
  longest_streak: number;
  reading: boolean;
  limited: boolean;
  files_done: number;
  files_total: number;
  /**
   * The first retained day Hide AI was counted on; no earlier day is. Only the all-agents range carries it.
   */
  hide_ai_since: string | null;
}
export interface UsageDay {
  date: string;
  week: number;
  weekday: number;
  today: boolean;
  recent_week: boolean;
  level: number;
  pending: boolean;
  amount: UsageAmount;
  /**
   * @maxItems 3
   */
  top_agents: UsageContribution[];
  /**
   * @maxItems 6
   */
  agents: UsageContribution[];
  other_agents: number;
  /**
   * Only the all-agents range carries it, on a day Hide AI was counted; an agent's own range is that agent's sessions.
   */
  hide_ai: UsageHideAiDay | null;
}
export interface UsageAmount {
  calls: number;
  new_tokens: number;
  tokens: UsageTokens;
  cost: UsageCost;
}
export interface UsageTokens {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  cache_write_1h: number;
  reasoning: number | null;
}
export interface UsageCost {
  micro_usd: number;
  partial: boolean;
  waiting: boolean;
  stale: boolean;
  incomplete: boolean;
  /**
   * @maxItems 8
   */
  unpriced_models: string[];
  other_unpriced_models: number;
}
export interface UsageContribution {
  agent: string;
  new_tokens: number;
  share_basis_points: number;
}
/**
 * Hide's own background AI on one day. Its known part is already inside the day's amount.
 */
export interface UsageHideAiDay {
  amount: UsageAmount1;
  /**
   * The known Hide AI new tokens over the day's new tokens.
   */
  share_basis_points: number;
  /**
   * @maxItems 5
   */
  features: UsageHideAiFeature[];
  /**
   * Attempts that were submitted and reported no usage. A count of calls, never tokens, so it is in no amount.
   */
  unknown_calls: number;
}
/**
 * The sum of features. A lower bound (cost.partial) when unknown_calls is not zero.
 */
export interface UsageAmount1 {
  calls: number;
  new_tokens: number;
  tokens: UsageTokens;
  cost: UsageCost;
}
export interface UsageHideAiFeature {
  /**
   * The closed group a router request's feature id falls into; an id outside the list is other.
   */
  feature: "labels" | "memory" | "factory" | "issue" | "other";
  amount: UsageAmount;
  /**
   * Provider ids of the models that answered, for the row's marks.
   *
   * @maxItems 8
   */
  providers: string[];
}
export interface UsageHistory {
  /**
   * Newest recorded model/day contributions inside the year calendar. Absent on older daemons; unknown model names are null. No model is inferred from aggregate totals.
   *
   * @maxItems 4096
   */
  models?: UsageModelDay[];
  /**
   * Model detail exceeded 4096 rows. Listed contributions are partial; the independent daily and period totals are unchanged.
   */
  models_limited?: boolean;
  popover: UsageHistorySurface;
  year: UsageHistorySurface;
}
/**
 * One recorded day/agent/model contribution on the core node. Hide AI is already in all-agent totals. Per-model call counts are reports, not backend attempts.
 */
export interface UsageModelDay {
  date: string;
  agent: string;
  model: string | null;
  hide_ai: boolean;
  amount: UsageAmount;
}
export interface UsageHistorySurface {
  range: UsageRange;
  /**
   * @maxItems 6
   */
  shares: UsageShare[];
}
export interface UsageShare {
  contribution: UsageContribution;
  since: string | null;
  lifetime: UsageAmount;
  /**
   * @maxItems 32
   */
  providers: string[];
}
export interface ProjectCost {
  workspace_id: string;
  micro_usd: number;
  /**
   * @maxItems 7
   */
  days: {
    date: string;
    micro_usd: number;
  }[];
  /**
   * @maxItems 16
   */
  models: {
    agent: string;
    model: string | null;
    micro_usd: number;
  }[];
  /**
   * @maxItems 3
   */
  sessions: {
    agent: string;
    session_id: string;
    micro_usd: number;
  }[];
  agents: {
    agent: string;
    micro_usd: number;
  }[];
  unpriced_agents: string[];
  partial: boolean;
  waiting: boolean;
  stale: boolean;
  limited: boolean;
}
export interface ClientEvent {
  schema_version: 2;
  kind: string;
  payload: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
/**
 * A pane-scoped query on a capability-authenticated WebSocket. The daemon derives device, pane, and Workspace from the credential and current core projection.
 */
export interface WorkspaceQuery {
  type: "workspace_query";
  request_id: WorkspaceRequestId;
  query: "info" | "view_list";
}
/**
 * One pane-scoped document or View transition. Repeating the same request ID and command returns its recorded result within the ten-minute retry window.
 */
export interface WorkspaceAction {
  type: "workspace_action";
  request_id: WorkspaceRequestId;
  command: WorkspaceCommand;
}
/**
 * `hide links`: a read of the link record on a capability-authenticated WebSocket, answered as a workspace_result without a shell. The caller checkout's Project is the scope unless all_projects is set; a target only another Project holds is refused with other_project, and one the record never saw with not_found.
 */
export interface LinksQuery {
  type: "links";
  request_id: WorkspaceRequestId;
  query: {
    all_projects?: boolean;
    target:
      | {
          kind: "pr";
          number: number;
        }
      | {
          kind: "issue";
          number: number;
        }
      | {
          kind: "branch";
          name: string;
        }
      | {
          kind: "session";
          id: string;
        };
  };
}
/**
 * `hide workspace memory`: an agent hook's Project Memory, answered by the core from the one store it owns whichever machine the agent runs on, as a workspace_result without a shell. The Project is the one holding the caller pane's attested checkout, read by that checkout's own node; the request never names it. The core's own node and any registered linked device have Memory through that device's node, regardless of connection direction; a link that is down or a helper without a Memory reader refuses immediately with a diagnostic reason, and a disconnected node refuses its callers with hide_unavailable. No store is copied to the device. result is {context, outcome, count?}: context is the Memory text or null, a prompt's cards with their signed receipt and a session start's card index without one, and outcome one of provided (with count), empty, disabled, unavailable, deadline, project_unresolved. The prompt is at most its first 8 KiB, and the core spends at most 1 s on the answer.
 */
export interface MemoryHookAsk {
  type: "memory_hook";
  request_id: WorkspaceRequestId;
  event: "SessionStart" | "UserPromptSubmit";
  runtime: "claude" | "codex" | "opencode" | "pi" | "omp" | "antigravity";
  session: string;
  prompt?: string;
}
/**
 * `hide <path>`: a folder of the caller's machine opened as a project, answered as a workspace_result with {label, path, added, window}: the checkout that came forward, whether this request registered it, and whether a window of the core's machine was open to be called forward (a `window_call` frame). Sent on a `cli` connection with the daemon's token, the folder is the core's machine's; sent with a pane capability, it is the capability's machine's, never a field of the request. A folder no registered checkout holds is registered by the same `create_workspace` Add a project sends, judged on hided's $HOME line or by the device's helper, with no confirmation, so a folder outside home is refused needs_confirmation; a refusal's reason is that line's code or the core's error kind with `_` for `.`.
 */
export interface ProjectOpen {
  type: "project_open";
  request_id: WorkspaceRequestId;
  path: string;
}
/**
 * A pane-scoped request outcome. A successful action result includes context, request_id, changed, view_id, and optionally area_id; a refusal names reason and next_action.
 */
export interface WorkspaceResult {
  type: "workspace_result";
  request_id?: WorkspaceRequestId;
  ok: boolean;
  result?: {
    [k: string]: unknown;
  };
  reason?: string;
  next_action?: string;
  [k: string]: unknown;
}
export interface UsageSnapshot1 {
  nodes: NodeUsage[];
}
export interface UsagePreferences {
  /**
   * @maxItems 3
   */
  pinned: {
    agent: string;
    window: WindowKind;
  }[];
  /**
   * @maxItems 7
   */
  hidden: string[];
}
export interface UsagePinSetPayload {
  agent: string;
  window: WindowKind;
  enabled: boolean;
}
export interface UsageHiddenSetPayload {
  agent: string;
  hidden: boolean;
}
/**
 * Two fields a `ui_state_update` payload may carry beside the UI state it replaces; the core never persists or echoes them, and an absent field keeps the core's value. usage_window_visible: a shell window is on screen, so the core reads each provider every five minutes. usage_popover_open: the Weekly Usage popover is open; its change from false to true reads again any provider last read more than a minute ago.
 */
export interface UiStateUsageHints {
  usage_window_visible?: boolean;
  usage_popover_open?: boolean;
  [k: string]: unknown;
}
/**
 * focus_checkout: one admitted focus and optional sidebar disclosure transition. Unknown workspace/checkout refuses the disclosure intent before either value changes.
 */
export interface FocusCheckoutPayload {
  workspace_id: string;
  checkout_id: string;
  focus_device?: boolean;
  display_id?: string;
  expanded?: boolean;
  project_expanded?: boolean;
}
/**
 * Payload of interface_language_set. This event alone edits the core preference; ui_state_update never changes it.
 */
export interface InterfaceLanguageSetPayload {
  language: InterfaceLanguage;
}
/**
 * Payload of agent_logo_mode_set. This event alone edits the core preference; ui_state_update never changes it, and a mode the event does not name is refused and changes nothing.
 */
export interface AgentLogoModeSetPayload {
  mode: AgentLogoMode;
}
/**
 * Payload of pane_turn_row_mode_set. This event alone edits the core preference; ui_state_update never changes it, a mode outside paneTurnRowMode is refused and changes nothing, and the current mode again saves nothing.
 */
export interface PaneTurnRowModeSetPayload {
  mode: PaneTurnRowMode;
}
/**
 * Payload of color_theme_set. This event alone edits the color theme; ui_state_update never changes it, a value outside colorTheme is refused, and the current value again saves nothing.
 */
export interface ColorThemeSetPayload {
  color_theme: ColorTheme;
}
/**
 * Payload of a terminal frame: pane output from the screen-side hub (PRD core-host-node-terminal D-11, D-22), in the order the panes' nodes produced it. One sequence numbers every pane's chunks; terminal_sequence is the client's cursor after this frame, which it names as have_terminal_sequence, with terminal_epoch as have_terminal_epoch, when it reconnects; each daemon's hub has its own epoch, so a cursor resumes only on the hub that gave it. After a whole snapshot the client resets every terminal and is sent a terminal frame with no chunks that gives its new cursor. A pane whose output a client left unsent past 1 MiB, or that a resume cannot reach, sends that client nothing more until a full frame redraws it. Keys go the other way as the `key` event {pane_id or pending_request, bytes_base64}, and a view's grid as `terminal_viewport` {pane_id, cols, rows, new_view}; both reach the pane's node directly and never the core.
 */
export interface TerminalFrame {
  chunks: {
    pane_id: string;
    bytes_base64: string;
  }[];
  terminal_epoch: string;
  terminal_sequence: number;
}
/**
 * Payload of an agent_notice frame (PRD first-run-onboarding D-09): the daemon judged the agents' latest change the way the phone's push does. Each notice is a root agent that newly needs the operator or has finished; clear lists the tags of notifications to withdraw because the operator has seen that agent. Whether a notification shows, and its words, is the window's and the host's to decide.
 */
export interface AgentNotice {
  notices: {
    tag: string;
    device_id: string;
    pane_id: string;
    title: string;
    state: "needs_you" | "done";
    place: string;
    /**
     * The root row's status word code. Optional for older frames.
     */
    status_code?:
      | "error"
      | "question"
      | "approval"
      | "working"
      | "done"
      | "idle"
      | "unknown"
      | "starting"
      | "waiting"
      | "unfinished"
      | "stopped"
      | "attached"
      | "sleeping"
      | "waking"
      | "sleep_failed"
      | null;
    /**
     * The core's chosen row detail, untranslated; absent or null means the original state-and-project body.
     */
    detail?: string | null;
    /**
     * A question the core asks on the row for the agent, which the window words in the interface language in place of detail: `plan_review` is a proposed plan of `steps` steps waiting for the operator's review, `mission_decision` is the decision `line` the Conductor's mission waits on (ask_subject_cause in snapshot-wire-enums.json). A window that does not know the cause draws detail.
     */
    subject?:
      | null
      | {
          cause: "plan_review";
          steps: number;
        }
      | {
          cause: "mission_decision";
          line: string;
        };
  }[];
  clear: string[];
}
/**
 * Payload of the `desktop_notices` client event, accepted only from a desktop window and never sent to the core: wanted says whether this window shows agent notifications. The daemon judges agent changes while any window wants them, and a window's closing withdraws its wish; turning on announces nothing that was already waiting.
 */
export interface DesktopNoticesPayload {
  wanted: boolean;
}
