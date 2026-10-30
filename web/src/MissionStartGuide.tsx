import { useEffect, useRef, useState } from "react";
import { AgentMark } from "./AgentMark";
import { agentMarkOf } from "./agentLogos";
import { useInterfaceTranslation } from "./i18n/client";
import "./missionStartGuide.css";

// How a mission starts, shown where there is none yet and under the Mission
// tool's `+` (PRD mission-tool D-02, D-10, B3, B4, B8): a moving picture of a
// mission, one line of what it is, three coloured dots, the skill's call in
// Claude Code and in Codex, and where the mission then shows. No field and no
// example prompt: a mission starts in an agent's terminal.

export function MissionStartGuide() {
  const { t } = useInterfaceTranslation();
  return (
    <div className="flex min-w-0 flex-col items-center gap-sm" data-mission-start-guide="true">
      <MissionPicture />
      <p className="break-keep text-center text-subhead font-semibold text-foreground">{t("missionTool.headline")}</p>
      <ul className="flex flex-wrap justify-center gap-x-md gap-y-xxs text-caption text-muted-foreground" data-mission-start-dots="true">
        <Dot tone="bg-mission-working" text={t("missionTool.dot.parallel")} />
        <Dot tone="bg-mission-review" text={t("missionTool.dot.verify")} />
        <Dot tone="bg-mission-hold" text={t("missionTool.dot.decide")} />
      </ul>
      <div className="flex w-full min-w-0 flex-col gap-xxs rounded-md border border-border bg-background px-sm py-xs font-mono text-caption" data-mission-start-calls="true">
        <Call kind="claude" call="/hide-conductor" runtime={t("missionTool.claudeCode")} />
        <Call kind="codex" call="$hide-conductor" runtime={t("missionTool.codex")} />
      </div>
      <p className="break-keep text-center text-caption text-muted-foreground">{t("missionTool.where")}</p>
    </div>
  );
}

function Dot({ tone, text }: { tone: string; text: string }) {
  return (
    <li className="flex items-center gap-xxs">
      <span aria-hidden="true" className={`size-(--size-tab-status-dot) shrink-0 rounded-full ${tone}`} />
      {text}
    </li>
  );
}

function Call({ kind, call, runtime }: { kind: string; call: string; runtime: string }) {
  return (
    <span className="flex min-w-0 items-center gap-xs" data-mission-start-call={call}>
      <AgentMark kind={kind} />
      <span aria-hidden="true" className="text-subtle-foreground">&gt;</span>
      <span className="font-semibold text-primary">{call}</span>
      <span className="min-w-0 truncate text-subtle-foreground">{runtime}</span>
    </span>
  );
}

/**
 * Whether the picture is on screen in a window that shows, so its loop runs
 * only then (B4, PRD risk on CPU): off screen or in a hidden window every
 * animation is paused where it is.
 */
function useOnScreen<T extends Element>(): [React.RefObject<T | null>, boolean] {
  const ref = useRef<T>(null);
  const [seen, setSeen] = useState(false);
  const [shown, setShown] = useState(() => typeof document === "undefined" || document.visibilityState !== "hidden");
  useEffect(() => {
    const node = ref.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => setSeen(entries.some((entry) => entry.isIntersecting)));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const update = () => setShown(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return [ref, seen && shown];
}

/** Workers' boxes: where each stands, its agent and what it makes. */
const WORKERS = [
  { n: 1, x: 6, kind: "claude", label: "missionTool.node.screen" },
  { n: 2, x: 104, kind: "codex", label: "missionTool.node.store" },
  { n: 3, x: 202, kind: "claude", label: "missionTool.node.test" },
] as const;
const WORKER_W = 64;
const WORKER_Y = 62;
const BOX_H = 32;

/** The satellite checks: beside each worker, then two beside the demo check, each by another company's agent than the worker's. */
const CHECKS = [
  { n: 1, cx: 81, cy: 78, from: 70, kind: "codex" },
  { n: 2, cx: 179, cy: 78, from: 168, kind: "claude" },
  { n: 3, cx: 277, cy: 78, from: 266, kind: "codex" },
  { n: 4, cx: 195, cy: 140, from: 184, kind: "codex" },
  { n: 5, cx: 195, cy: 160, from: 184, kind: "claude" },
] as const;
const CHECK_R = 8;

/**
 * The picture itself (B3, B4): an SVG in its own coordinates, so it scales
 * with the tool's width; the colours, the stroke width and the type size are
 * tokens (`missionStartGuide.css`). For a reader it is one image with one
 * sentence of what it shows.
 */
function MissionPicture() {
  const { t } = useInterfaceTranslation();
  const [ref, playing] = useOnScreen<SVGSVGElement>();
  return (
    <svg ref={ref} viewBox="0 0 290 232" role="img" aria-label={t("missionTool.picture")} className="msg-picture w-full max-w-(--size-mission-popover)" data-playing={playing ? "true" : "false"} data-mission-start-picture="true">
      <path className="msg-edge msg-edge-top" d="M145 32 C145 48 38 46 38 62" />
      <path className="msg-edge msg-edge-top" d="M145 32 L136 62" />
      <path className="msg-edge msg-edge-top" d="M145 32 C145 48 234 46 234 62" />
      <path className="msg-edge msg-edge-done msg-e4" d="M38 94 C38 116 140 112 140 134" />
      <path className="msg-edge msg-edge-done msg-e5" d="M136 94 L140 134" />
      <path className="msg-edge msg-edge-done msg-e6" d="M234 94 C234 116 140 112 140 134" />
      <path className="msg-edge msg-edge-done msg-e7" d="M140 166 L145 198" />
      <g>
        <rect className="msg-box msg-conductor" x="89" y="6" width="112" height="26" rx="13" />
        <Mark kind="claude" x={103} y={13} />
        <text className="msg-label" x="119" y="23">{t("missionTool.node.conductor")}</text>
      </g>
      {WORKERS.map((worker) => (
        <Worker key={worker.n} n={worker.n} x={worker.x} y={WORKER_Y} width={WORKER_W} kind={worker.kind} label={t(worker.label)} />
      ))}
      <Worker n={4} x={98} y={134} width={84} kind="claude" label={t("missionTool.node.demo")} />
      <g className="msg-ask" aria-hidden="true">
        <circle cx="203" cy="62" r="7" />
        <text x="203" y="66" textAnchor="middle">!</text>
      </g>
      {CHECKS.map((check) => (
        <g key={check.n}>
          <line className={`msg-tether msg-l${check.n}`} x1={check.from} y1={check.cy} x2={check.cx - CHECK_R} y2={check.cy} />
          <circle className={`msg-ring msg-r${check.n}`} cx={check.cx} cy={check.cy} r={CHECK_R} />
          <Mark kind={check.kind} x={check.cx - 5} y={check.cy - 5} size={10} />
          <text className={`msg-tick msg-t${check.n}`} x={check.cx + 5} y={check.cy + 11}>✓</text>
        </g>
      ))}
      <g>
        <rect className="msg-box msg-main" x="110" y="198" width="70" height="24" rx="12" />
        <text className="msg-label msg-main-label" x="145" y="214" textAnchor="middle">⎇ {t("missionTool.node.main")}</text>
      </g>
    </svg>
  );
}

function Worker({ n, x, y, width, kind, label }: { n: number; x: number; y: number; width: number; kind: string; label: string }) {
  return (
    <g>
      <rect className={`msg-box msg-worker msg-w${n}`} x={x} y={y} width={width} height={BOX_H} rx="7" />
      <Mark kind={kind} x={x + 6} y={y + 6} />
      <text className="msg-label" x={x + 21} y={y + 16}>{label}</text>
      <rect className="msg-track" x={x + 6} y={y + 24} width={width - 12} height="3" rx="1.5" />
      <rect className={`msg-fill msg-f${n}`} x={x + 6} y={y + 24} width={width - 12} height="3" rx="1.5" />
    </g>
  );
}

/** An agent's logo inside the picture; none for an agent without one. */
function Mark({ kind, x, y, size = 12 }: { kind: string; x: number; y: number; size?: number }) {
  const logo = agentMarkOf(kind)?.logo;
  return logo ? <image href={logo} x={x} y={y} width={size} height={size} aria-hidden="true" /> : null;
}
