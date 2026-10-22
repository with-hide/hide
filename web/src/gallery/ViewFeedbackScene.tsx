// View openers and the operator's comments (PRD view-opener-feedback D-20,
// D-24, D-25) in the production Workspace over invented local data: each
// `state` the approved boards draw, from the opener's tooltip to the line
// after Send. No daemon is used; a PDF's bytes are made here.
//
// A browser View's page is drawn by the desktop host, which the gallery does
// not have, so `comment-browser` shows the View's own controls in Comment
// mode and not the page's pins (the page's overlay is judged on the board).

import { useEffect, useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { TooltipProvider } from "../components/ui/tooltip";
import { configureFileBytes, receiveBytes } from "../fileBytes";
import { Sidebar } from "../sidebar";
import type { EditorDocumentSnapshot, EditorTabSnapshot, ViewComment, ViewDisplaySnapshot, ViewFeedback, ViewFeedbackGroup, ViewOpener } from "../snapshot";
import { useShellStore, type Documents } from "../store";
import { TitleRow } from "../TitleRow";
import { useUiStore } from "../ui";
import { browserCommentKey, noteBrowserComment, noteSentFrom, openNote, setRegionMode, useFeedbackStore } from "../viewFeedback";
import { WorkspaceScreen } from "../WorkspaceScreen";
import { REFERENCE_FOLDS, sidebarStatusAxisScene } from "./sceneData";
import { sceneParams, type SceneParams } from "./SidebarScene";

const STATES = ["opener-tip", "views-list", "comment-browser", "comment-diff", "comment-pdf", "send-menu", "sent"] as const;
type FeedbackState = (typeof STATES)[number];

/** Which View is in front for each state. */
const FRONT: Record<FeedbackState, string> = {
  "opener-tip": "d1",
  "views-list": "d1",
  "comment-browser": "d1",
  "comment-diff": "d2",
  "comment-pdf": "d3",
  "send-menu": "d1",
  sent: "d1",
};

const PATCH = [
  "@@ -10,9 +10,12 @@ export function LoginForm() {",
  '   const [email, setEmail] = useState("")',
  '   const [password, setPassword] = useState("")',
  "+  const [error, setError] = useState<string | null>(null)",
  "+",
  "+  const [busy, setBusy] = useState(false)",
  "+  const submit = useSubmit({ onStart: () => setBusy(true) })",
  "+",
  "   return (",
  '     <Card className="login-card">',
  '-      <h3 className="title spaced">Welcome back</h3>',
  "+      <h3>Welcome back</h3>",
  '       <Field label="Email" value={email} onChange={setEmail} />',
  '       <Field label="Password" type="password" value={password} />',
  '-      <Button className="w-full">Continue</Button>',
  '+      <Button size="lg" className="w-full">Continue</Button>',
  "     </Card>",
  "   )",
  "",
].join("\n");

export function viewFeedbackSceneParams(params: URLSearchParams): SceneParams & { state: FeedbackState } {
  const state = params.get("state") ?? "opener-tip";
  if (!(STATES as readonly string[]).includes(state)) throw new Error(`Unknown view feedback state ${state}`);
  return { ...sceneParams(params), state: state as FeedbackState };
}

/** A small PDF of slide-like pages, each a heading and four bars, built with its cross-reference table. */
function slidesPdf(): Uint8Array {
  const pages = ["Where new accounts stop", "Activation rose after the new sign-in", "What we try next"];
  const objects: string[] = [];
  const add = (body: string) => objects.push(body);
  add("<< /Type /Catalog /Pages 2 0 R >>");
  add(`<< /Type /Pages /Kids [${pages.map((_, index) => `${4 + index * 2} 0 R`).join(" ")}] /Count ${pages.length} >>`);
  add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
  pages.forEach((title, index) => {
    const bars = [40, 50, 58, 100].map((height, bar) => `${bar === 3 ? "0.64 0.39 0.03" : "0.83 0.82 0.80"} rg ${40 + bar * 46} 40 34 ${height} re f`).join("\n");
    const stream = `0.99 0.99 0.97 rg 0 0 640 340 re f\n0.64 0.39 0.03 rg BT /F1 9 Tf 40 296 Td (ONBOARDING - WEEK 1) Tj ET\n0.1 0.1 0.1 rg BT /F1 22 Tf 40 266 Td (${title}) Tj ET\n0.35 0.35 0.35 rg BT /F1 11 Tf 40 246 Td (Share of new accounts that wrote a first note within a day.) Tj ET\n${bars}`;
    add(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 640 340] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + index * 2} 0 R >>`);
    add(`<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`);
  });
  let text = "%PDF-1.4\n";
  const offsets = objects.map((body, index) => {
    const at = text.length;
    text += `${index + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = text.length;
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((at) => `${String(at).padStart(10, "0")} 00000 n \n`).join("")}`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(text);
}

/** Answers a `file_bytes` request with one framed reply, as hided would. */
function answerBytes(requestId: string, path: string, bytes: Uint8Array): void {
  const header = new TextEncoder().encode(JSON.stringify({ type: "file_bytes", request_id: requestId, path, offset: 0, total: bytes.byteLength, eof: true }));
  const frame = new Uint8Array(4 + header.byteLength + bytes.byteLength);
  new DataView(frame.buffer).setUint32(0, header.byteLength, false);
  frame.set(header, 4);
  frame.set(bytes, 4 + header.byteLength);
  receiveBytes(frame.buffer);
}

export function ViewFeedbackScene({ theme, scale, content, state }: SceneParams & { state: FeedbackState }) {
  const fixture = useMemo(() => {
    const scene = sidebarStatusAxisScene(content, REFERENCE_FOLDS, Date.now());
    const workspace = scene.rest.navigator!.workspaces!.find((project) => project.id === "hide")!;
    const checkout = workspace.checkouts[0]!;
    const agentTab = checkout.tabs[0]!;
    // The first tab's two agents, named as their rows are, one over the other.
    agentTab.panes = agentTab.panes.map((pane) => ({ ...pane, identity_label: scene.agents.find((row) => row.pane_id === pane.id)?.identity_label ?? pane.id }));
    const root = checkout.path;
    const paneOf = (id: string) => scene.agents.find((row) => row.pane_id === id)!;
    const opener = (pane: string): ViewOpener => ({ name: paneOf(pane).identity_label ?? pane, provider: paneOf(pane).agent_kind ?? "claude", device_id: "local", pane_id: pane, resumable: true });
    const main = opener("a1");
    const other = opener("a2");

    const tab = (id: string, relative: string, kind: EditorTabSnapshot["kind"]): EditorTabSnapshot => ({ id, workspace_id: workspace.id, checkout_id: checkout.id, path: `${root}/${relative}`, label: relative.split("/").pop()!, kind, diff_committed: kind === "diff" ? false : null, markdown_live: true, wrap: false, dirty: false, preview: false });
    const tabs = [tab("t2", "src/LoginForm.tsx", "diff"), tab("t3", "docs/onboarding.pdf", "file"), tab("t4", "README.md", "file")];
    const sent = state === "sent";
    const comments: ViewComment[] = sent
      ? []
      : [
          { id: "c1", display_ids: ["d1"], kind: "browser", path: "", url: "http://localhost:5173/login", anchor: { kind: "element", selector: "button.continue", text: "Continue", rect: { x: 0.12, y: 0.4, width: 0.8, height: 0.08 } }, note: "버튼이 너무 커요. 한 단계 줄여 주세요.", shot: true, changed: false, not_delivered: false, group: "local/a1", number: 1 },
          { id: "c2", display_ids: ["d1"], kind: "browser", path: "", url: "http://localhost:5173/login", anchor: { kind: "area", rect: { x: 0.12, y: 0.1, width: 0.8, height: 0.1 } }, note: "제목과 이메일 칸 사이를 넓혀 주세요.", shot: true, changed: false, not_delivered: false, group: "local/a1", number: 2 },
          { id: "c3", display_ids: ["d2"], kind: "diff", path: tabs[0]!.path, committed: false, anchor: { kind: "lines", start: 19, end: 19, lines: ["      <h3>Welcome back</h3>"], side: "new" }, note: "제목과 부제 사이가 너무 붙어 있어요. 간격을 되살려 주세요.", shot: false, changed: false, not_delivered: false, group: "local/a1", number: 3 },
          { id: "c4", display_ids: ["d3"], kind: "file", path: tabs[1]!.path, anchor: { kind: "region", page: 2, rect: { x: 0.06, y: 0.55, width: 0.31, height: 0.33 } }, note: "마지막 막대 색이 다른 이유를 범례로 적어 주세요.", shot: true, changed: false, not_delivered: false, group: "local/a2", number: 1 },
        ];
    const count = (display: string) => comments.filter((comment) => comment.display_ids.includes(display)).length;
    const display = (id: string, extra: Partial<ViewDisplaySnapshot> & Pick<ViewDisplaySnapshot, "kind" | "label" | "path">): ViewDisplaySnapshot => ({ id, tab_id: null, committed: null, preview: false, state: "open", reason: null, ...(count(id) ? { comments: count(id) } : {}), ...extra });
    const displays = [
      display("d1", { kind: "browser", label: "localhost:5173/login", path: "", url: "http://localhost:5173/login", title: "Owl Notes", load: 1, opener: main }),
      display("d2", { kind: "diff", label: "LoginForm.tsx", path: tabs[0]!.path, tab_id: "t2", committed: false, opener: main }),
      display("d3", { kind: "file", label: "onboarding.pdf", path: tabs[1]!.path, tab_id: "t3", opener: other }),
      display("d4", { kind: "file", label: "README.md", path: tabs[2]!.path, tab_id: "t4" }),
    ];
    const group = (key: string, recipient: ViewOpener, ids: string[]): ViewFeedbackGroup => ({ key, recipient, comments: comments.filter((comment) => comment.group === key).length, views: ids.length, not_delivered: 0, display_ids: ids });
    const feedback: ViewFeedback = {
      comments,
      groups: sent ? [] : [group("local/a1", main, ["d1", "d2"]), group("local/a2", other, ["d3"])],
      opened: [
        { pane_id: "a1", view_id: "d1", kind: "browser", label: "localhost:5173/login", path: "", url: "http://localhost:5173/login", comments: count("d1") },
        { pane_id: "a1", view_id: "d2", kind: "diff", label: "LoginForm.tsx", path: tabs[0]!.path, comments: count("d2") },
        { pane_id: "a2", view_id: "d3", kind: "file", label: "onboarding.pdf", path: tabs[1]!.path, comments: count("d3") },
      ],
      ...(sent ? { send: { request_id: "r1", state: "sent" as const, count: 3, to: main.name, pane_id: "a1", at_unix_ms: Date.now() + 3_600_000 } } : {}),
    };
    const document = (path: string, kind: EditorDocumentSnapshot["document_kind"], contents: string): EditorDocumentSnapshot => ({ path, language: kind === "markdown" ? "markdown" : null, document_kind: kind, contents_utf8: contents, revision: "sha256:0", dirty: false, readonly_reason: null, conflict: null, save: null }) as EditorDocumentSnapshot;
    const documents: Documents = { t3: document(tabs[1]!.path, "pdf", ""), t4: document(tabs[2]!.path, "markdown", "# Owl Notes\n") };
    scene.rest.navigator!.focused_workspace_id = workspace.id;
    scene.rest.navigator!.focused_checkout_id = checkout.id;
    scene.rest.navigator!.changes_root_path = root;
    scene.rest.pane_layouts = [{ workspace_id: workspace.id, tab_id: agentTab.id!, focused_pane_id: "a1", zoomed: false, root: { type: "split", direction: "down", ratio: 0.5, first: { type: "pane", pane_id: "a1" }, second: { type: "pane", pane_id: "a2" } } }];
    scene.rest.workspace_view = {
      agent_layout: { root: { area: { id: "g1", active: agentTab.id, displays: [{ id: agentTab.id! }] } }, active_area: "g1", canvases: {}, limits: { areas: 4, depth: 3, displays: 64 }, display_count: 1 },
      device_id: "local", path: root, open: true, documents: true, tool: null, pinned: true, document_width: 580, detail_width: null, explorer_width: null, panel_width: null, lane_called: 0, lane_calls: 0, lane_called_side: "view",
      layout: { root: { area: { id: "a1", active: FRONT[state], displays } }, active_area: "a1", limits: { areas: 4, depth: 3, displays: 32 }, display_count: displays.length },
      feedback,
    };
    const changes = { root_path: root, entries: [{ path: tabs[0]!.path, relative_path: "src/LoginForm.tsx", status: "modified" }], committed: [], base_branch: "main", selected_path: null, selected_committed: false, diff: null, diffs: [{ path: tabs[0]!.path, committed: false, text: PATCH, notice: null }], unavailable_reason: null };
    return { scene, editor: { tabs, active_tab_id: null }, documents, changes, pdf: slidesPdf() };
  }, [content, state]);
  const actions = useMemo(() => createActions(() => true), []);

  useLayoutEffect(() => {
    configureFileBytes((message) => {
      const { request_id, path } = message.payload as { request_id: string; path: string };
      queueMicrotask(() => answerBytes(request_id, path, fixture.pdf));
      return true;
    });
    useShellStore.setState({ rest: fixture.scene.rest, agents: fixture.scene.agents, connection: "live", editor: fixture.editor as never, documents: fixture.documents, changes: fixture.changes as never });
    useUiStore.setState({ screen: { kind: "workspace" }, laneShown: true, laneExpanded: false });
    useFeedbackStore.setState({ forms: {}, regionMode: {}, browser: {}, sentFrom: {}, deletedFrom: null });
    // A Send this window made shows its line on the View it was made from.
    if (state === "sent") noteSentFrom("r1", "d1");
    if (state === "comment-browser") noteBrowserComment(browserCommentKey(fixture.scene.rest.workspace_view!, "d1"), { on: true, reason: null });
    if (state === "comment-diff") openNote({ displayId: "d2", anchor: { kind: "lines", start: 14, end: 15, lines: ["  const [busy, setBusy] = useState(false)", "  const submit = useSubmit({ onStart: () => setBusy(true) })"], side: "new" }, commentId: null, note: "로딩 중에는 버튼을 막아주세요." });
    if (state === "comment-pdf") {
      setRegionMode("d3", true);
      const c4 = fixture.scene.rest.workspace_view!.feedback!.comments.find((comment) => comment.id === "c4")!;
      openNote({ displayId: "d3", anchor: c4.anchor, commentId: "c4", note: c4.note });
    }
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [fixture, state, theme, scale]);

  // A tooltip or a menu the board draws open is opened the way the operator opens it.
  useEffect(() => {
    const press = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      element?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, pointerType: "mouse" }));
    };
    const target = { "opener-tip": '[data-browser-toolbar] [data-view-opener]', "views-list": '[data-pane-views="a1"]', "send-menu": "[data-view-feedback-send-menu]" }[state as string];
    if (!target) return undefined;
    const open = () => {
      const element = document.querySelector<HTMLElement>(target);
      if (!element) return false;
      if (state === "opener-tip") element.focus();
      else press(target);
      return true;
    };
    if (open()) return undefined;
    const watch = new MutationObserver(() => open() && watch.disconnect());
    watch.observe(document.body, { childList: true, subtree: true });
    return () => watch.disconnect();
  }, [state, fixture]);

  return (
    <TooltipProvider>
      <div className="flex h-full flex-col bg-background text-foreground" data-gallery-scene="view-opener-feedback" data-view-feedback-state={state}>
        <TitleRow actions={actions} />
        <div className="flex min-h-0 flex-1">
          <Sidebar actions={actions} />
          <WorkspaceScreen actions={actions} />
        </div>
      </div>
    </TooltipProvider>
  );
}
