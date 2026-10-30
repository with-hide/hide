// A Desk thread connected to the store (PRD session-docs-and-assistant
// B25-B29): the real `ThreadPanel` over the sections the core sends, in the
// wire's own shape: a session thread whose page holds ACP updates, the
// `hide` calls the reader named by tool call id, and where the sessions it
// names run, beside the agent rows, the plan and the mission those resolve
// to. Nothing leaves the page: the panel's open, close and input go nowhere.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { ThreadPanel } from "../components/desk/thread-panel";
import { TooltipProvider } from "../components/ui/tooltip";
import type { DeskJob, DeskMention } from "../desk";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import type { AgentRow, SessionThreadSection, SnapshotRest, ThreadPage } from "../snapshot";
import { useShellStore } from "../store";
import { storyJobs } from "./deskSceneData";
import { missionFixture } from "./MissionScene";
import { planFixture } from "./PlanScene";

export const DESK_THREAD_SCENES = [
  { state: "live", label: "Thread from the store: Hide's parts, a letter each way, a running command (B25, B26, D-75)" },
  { state: "asking", label: "Thread from the store: its question said once, at the foot (D-75)" },
  { state: "failed", label: "Thread from the store: a record it cannot read (B27)" },
  { state: "held", label: "Thread from the store: its pane runs another session (B29)" },
] as const;

type SceneState = (typeof DESK_THREAD_SCENES)[number]["state"];

const noop = () => undefined;

const row = (pane_id: string, identity_label: string, mark: AgentRow["mark"], agent_kind: string) => ({ pane_id, identity_label, mark, agent_kind }) as AgentRow;
const said = (text: string) => ({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
const asked = (text: string) => ({ sessionUpdate: "user_message_chunk", content: { type: "text", text } });
const shell = (toolCallId: string, command: string, status: "completed" | "in_progress", extra: Record<string, unknown> = {}) => ({ sessionUpdate: "tool_call", toolCallId, kind: "execute", title: command, status, rawInput: { command }, ...extra });

/** The Conductor's live page: Hide's parts, a letter each way and a command still running. */
function conductorPage(now: number, mission: string): ThreadPage {
  return {
    updates: [
      asked("Desk 1차를 미션으로 돌려줘"),
      said("계획을 올리고 @1a-desk-room에 첫 층을 맡길게요."),
      shell("p1", "hide plan set --file plan.json", "completed"),
      shell("s1", "hide agent spawn --parent here --name 1b-thread", "completed"),
      shell("l1", "hide request send agent-1a --intent desk:review --body '리뷰 지적 2개 반영해서 다시 올려줘'", "completed"),
      shell("v1", "hide file open shots/thread-dark.png", "completed"),
      shell("m1", `hide mission show --mission ${mission}`, "completed"),
      shell("p2", "hide plan node ui --status done", "completed"),
      { sessionUpdate: "user_message_chunk", content: { type: "text", text: "반영했어요. CI 기다리는 중\n실패하면 다시 알릴게요." }, _meta: { hide: { origin: "letter", from: "1a-desk-room" } } },
      said("CI가 끝나면 lane 노드를 띄울게요. 지금 캡처를 다시 찍어요."),
      shell("r1", "node scripts/capture-gallery.mjs --theme dark", "in_progress", { _meta: { hide: { at_ms: now - 12_000 } } }),
    ],
    parts: {
      p1: { kind: "plan" },
      s1: { kind: "spawn", name: "1b-thread", intent: null, agent_id: "agent-1b" },
      l1: { kind: "letter", to: "agent-1a", body: "리뷰 지적 2개 반영해서 다시 올려줘" },
      v1: { kind: "view", view: "file", target: "shots/thread-dark.png" },
      m1: { kind: "mission", mission },
      p2: { kind: "plan" },
    },
    sessions: {
      "agent-1a": { pane_id: "w1:1a", machine: "mac" },
      "agent-1b": { pane_id: "w1:1b", machine: "mac" },
      "1a-desk-room": { pane_id: "w1:1a", machine: "mac" },
    },
  };
}

function thread(pane: string, page: ThreadPage | null, extra: Partial<SessionThreadSection> = {}): SessionThreadSection {
  return { generation: 1, state: "ready", pane_id: pane, machine: "mac", failure: null, pane: "same", connected: true, has_older: false, older: "idle", tail: page, input: null, ...extra };
}

export function DeskThreadScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const state = (params.get("state") ?? "live") as SceneState;
  if (!DESK_THREAD_SCENES.some((scene) => scene.state === state)) throw new Error(`Unknown Desk thread scene ${state}`);
  const now = useMemo(() => Date.now(), []);
  const jobs = useMemo(() => storyJobs(now), [now]);
  const actions = useMemo(() => createActions(() => true), []);
  const missions = useMemo(() => missionFixture("running", now), [now]);
  const job: DeskJob = state === "asking" ? jobs.gallery! : jobs.conductor!;

  useLayoutEffect(() => {
    const plan = { ...planFixture("running", now), pane_id: job.pane_id! };
    const rows = [row("w1:1a", "1a Desk 방", "done", "codex"), row("w1:1b", "1b 스레드 화면", "going", "codex"), row(job.pane_id!, job.title, job.mark ?? "going", job.agent_kind)];
    const page: Record<SceneState, SessionThreadSection> = {
      live: thread(job.pane_id!, conductorPage(now, missions.mission.id)),
      asking: thread(job.pane_id!, { updates: [asked("gallery 캡처를 라이트·다크로 찍어서 나란히 비교해줘"), shell("c1", "node scripts/capture-gallery.mjs --theme light", "completed"), said("라이트 캡처를 찍었어요.\n다크 모드도 찍을까요?")], parts: {}, sessions: {} }),
      failed: thread(job.pane_id!, null, { state: "failed", failure: "no_record" }),
      held: thread(job.pane_id!, conductorPage(now, missions.mission.id), { pane: "changed" }),
    };
    // The model Hide launched the Conductor with, which the head's second line names (letter-18465).
    const overview_graph = { sessions: [{ device_id: "mac", pane_id: job.pane_id!, session_digest: "d", launch_model: "opus", launch_effort: "high" }] };
    const rest = { ...missions.rest, plans: { items: [plan] }, overview_graph } as unknown as SnapshotRest;
    useShellStore.setState({ rest, agents: rows, connection: "live", sessionThread: page[state], sessionThreadHistory: { generation: 1, pages: [] } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", params.get("scale") ?? "1");
  }, [job, missions, now, state, theme, params]);

  const mentions: DeskMention[] = [{ kind: "session", id: "w1:1a", name: "1a-desk-room", mark: "done", agent_kind: "codex" }];
  return (
    <TooltipProvider>
      <div className="flex h-screen min-w-0 justify-end bg-background text-foreground" data-desk-thread-scene={state}>
        <div className="flex min-h-0 w-full max-w-(--size-lane-ideal) min-w-0 flex-col border-l border-border">
          <ThreadPanel job={job} actions={actions} pinned={false} mentions={mentions} startedByDesk={state === "asking"} onBack={noop} onPin={noop} onClose={noop} onTerminal={noop} onMission={noop} onOutput={noop} onSession={noop} />
        </div>
      </div>
    </TooltipProvider>
  );
}
