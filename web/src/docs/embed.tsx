// The parts the user docs draw (docs-user-docs P0): the product's own components over invented values,
// one part per `?part=` so a docs page frames the page it needs. Nothing here reads a daemon; the build is
// `docs-embed.config.ts`, which `hided` never serves.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../index.css";
import { requireInterfaceLanguage } from "../i18n/locale";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { SessionWorkflowScene } from "../gallery/SessionWorkflowScene";
import { sceneParams } from "../gallery/SidebarScene";
import { DocsTaskRow, TASK_ROW_STATES, type TaskRowState } from "./TaskRowPart";

const PARTS = ["task-row", "turn-card"] as const;

function Embed({ params }: { params: URLSearchParams }) {
  useFollowInterfaceLanguage(requireInterfaceLanguage(params.get("lang") ?? "en"));
  const part = params.get("part") ?? "";
  if (part === "task-row") {
    const state = params.get("state");
    if (state !== null && !TASK_ROW_STATES.includes(state as TaskRowState)) throw new Error(`Unknown task-row state ${state}`);
    return <DocsTaskRow params={params} state={state as TaskRowState | null} />;
  }
  if (part === "turn-card") return <SessionWorkflowScene {...sceneParams(params)} scene="pane-header" state="turn-card" />;
  throw new Error(`Unknown docs part "${part}"; known: ${PARTS.join(", ")}`);
}

const container = document.getElementById("root");
if (!container) throw new Error("root missing");
createRoot(container).render(<StrictMode><Embed params={new URLSearchParams(window.location.search)} /></StrictMode>);
