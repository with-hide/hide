// The real close sheet of an agent with many children over invented rows:
// 33 children whose agents ended, one still working and one asking (PRD
// close-agent-subtree; quiet rows fold). `state=open` opens the fold.
import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import { createActions } from "../actions";
import type { Subtree, SubtreeRow } from "../close";
import { AlertDialog, AlertDialogContent } from "../components/ui/alert-dialog";
import { TooltipProvider } from "../components/ui/tooltip";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { SubtreeClose } from "../Overlays";
import { galleryAgentState } from "./agentStates";
import { agent } from "./sceneData";

/** Gallery panes whose fixed wire state is quiet, working and asking. */
const PRESET = { quiet: "a1c7", working: "a1c1", waiting: "a1g1" } as const;

function row(pane: string, label: string, state: keyof typeof PRESET, depth: number, target = false): SubtreeRow {
  const mark = state === "quiet" ? "quiet" : state === "working" ? "going" : "turn";
  const status = state === "quiet" ? "idle" : state === "working" ? "working" : "question";
  return {
    agent: agent({ pane_id: pane, identity_label: label, group: "working", mark, status_code: status, elapsed: "5m", state: galleryAgentState(PRESET[state], null, Date.now()) }),
    depth,
    target,
    state: target ? "quiet" : state,
  };
}

export function CloseTreeScene({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "ko"));
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const subtree = useMemo((): Subtree => {
    const children = Array.from({ length: 33 }, (_, index) =>
      index === 4 ? row(`c${index}`, "온보딩 4장 연습 모드", "working", 1) : index === 11 ? row(`c${index}`, "릴리스 노트 확인", "waiting", 1) : row(`c${index}`, `PR 리뷰 ${index + 1}`, "quiet", 1),
    );
    return {
      ids: children.map((child) => child.agent.pane_id),
      rows: [row("parent", "머지 순서 관리", "quiet", 0, true), ...children],
      counts: { working: 1, waiting: 1, unread: 0, unknown: 0 },
      unknown: false,
      targetUnknown: false,
    };
  }, []);
  const actions = useMemo(() => createActions(() => true), []);
  const footer = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme]);
  useEffect(() => {
    if (params.get("state") !== "open") return;
    const timer = window.setTimeout(() => document.querySelector<HTMLButtonElement>("[data-subtree-fold]")?.click(), 50);
    return () => window.clearTimeout(timer);
  }, [params]);
  return (
    <TooltipProvider>
      <div className="h-full bg-background" data-gallery-scene="close-tree">
        <AlertDialog open>
          <AlertDialogContent data-confirm-subtree="true" aria-describedby={undefined}>
            <SubtreeClose actions={actions} kind="pane" targetId="parent" subtree={subtree} footer={footer} />
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </TooltipProvider>
  );
}
