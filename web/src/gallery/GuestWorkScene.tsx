// Another machine's work on this computer, over an invented pair of
// machines (PRD device-connect-iroh B10, B49, B68): the sidebar rows of the
// MacBook's panes here, under the project that holds their folder and under
// the MacBook's name, the Windows PC's own Needs You and Done rows with its
// filled chip, and the center showing one of the MacBook's panes with its
// tab strip. `menu=open` draws the pane's menu, its four Workspace entries
// dimmed with the machine to do them on, and `close=ask` the confirmation.
// It proves the rendered parts; the panes and the rows are the daemon's.
import { useLayoutEffect, useMemo } from "react";
import { createActions } from "../actions";
import { EntryPointMenu } from "../components/entry-menu";
import { TooltipProvider } from "../components/ui/tooltip";
import { ConfirmGuestPaneClose, GuestPaneScreen, guestPaneMenu } from "../GuestPaneScreen";
import { GuestMachineGroups, GuestProjectRows, PeerWorkRows } from "../GuestWorkRows";
import { InterfaceLanguageBoundary, useInterfaceTranslation } from "../i18n/client";
import { NOTHING_UNDER_WAY, type Peer } from "../pairing";
import type { Device, GuestPanes, PeerWork } from "../snapshot";
import { useShellStore } from "../store";
import { useUiStore } from "../ui";
import { EMPTY_GALLERY_SCOPE } from "./agentScopes";

const key = (letter: string) => letter.repeat(64);
const PEERS: Peer[] = [
  { key: key("a"), device_id: "macbook", name: "MacBook", opens: null, opened_by: "active", connected: false, path: null },
  { key: key("c"), device_id: "windows-pc", name: "Windows PC", opens: "active", opened_by: null, connected: true, path: "direct" },
];
const PANE = "guest:macbook:pane:w1:p1";
const GUEST_PANES: GuestPanes[] = [{
  device_id: "macbook", key: key("a"),
  panes: [
    { id: PANE, pane: "w1:p1", since_unix_ms: 1, folder: "/work/shop-api", name: "결제 재시도 고치기", branch: "main", mark: "going" },
    { id: "guest:macbook:pane:w1:p2", pane: "w1:p2", since_unix_ms: 2, folder: "/work/shop-api/web", branch: "web-login", mark: "quiet" },
    { id: "guest:macbook:pane:w2:p1", pane: "w2:p1", since_unix_ms: 3, folder: "/Users/me/scratch", mark: "quiet" },
  ],
}];
const PEER_WORK: PeerWork[] = [{
  device_id: "windows-pc", key: key("c"), needs_you: 1, done: 1,
  rows: [
    { id: "w1:p1", group: "needs_you", title: "결제 실패 재현", agent_kind: "claude", place: "shop-api · main", mark: "turn", status_code: "question", detail: "어느 브랜치에서 재현할까요?" },
    { id: "w1:p2", group: "done", title: "배포 스크립트 정리", agent_kind: "codex", place: "infra", mark: "unread", status_code: "done", detail: "스크립트 세 개를 하나로 합쳤어요." },
  ],
}];
const LOCAL: Device = { agent_scope: EMPTY_GALLERY_SCOPE, id: "local", label: null, kind: "local", state: "local", message: null, ssh_alias: null, agent_count: 0, test: null };
const PROJECT = { id: "shop", label: "shop-api", path: "/work/shop-api", device_id: "local", checkouts: [{ id: "shop-main", path: "/work/shop-api", tabs: [] }] };

export function GuestWorkScene({ params }: { params: URLSearchParams }) {
  const theme = params.get("theme") === "light" ? "light" : "dark";
  const lang = params.get("lang") ?? "ko";
  if (!["en", "ko", "ja", "zh-CN"].includes(lang)) throw new Error(`Unknown language ${lang}`);
  const actions = useMemo(() => createActions(() => true), []);
  const { t } = useInterfaceTranslation();
  useLayoutEffect(() => {
    useShellStore.setState({
      connection: "live",
      // This computer's own window: another computer's rows show only there.
      pairing: NOTHING_UNDER_WAY,
      rest: { navigator: { devices: [LOCAL], peers: PEERS, guest_panes: GUEST_PANES, peer_work: PEER_WORK, focused_device_id: "local", workspaces: [PROJECT] }, status: { remote: [] }, ui_state: { interface_language: lang as "en" | "ko" | "ja" | "zh-CN" } },
    } as never);
    useUiStore.setState({ screen: { kind: "guest", paneId: PANE }, guestClose: params.get("close") === "ask" ? PANE : null });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
  }, [theme, lang, params]);
  return (
    <>
      <InterfaceLanguageBoundary />
      <TooltipProvider>
        <div className="flex h-screen w-full bg-background text-foreground" data-gallery-scene="guest-work" data-scene-ready="true">
          <ul className="flex w-(--size-sidebar-ideal) shrink-0 flex-col overflow-auto bg-sidebar px-sm py-sm" data-guest-work-sidebar="true">
            <PeerWorkRows group="needs_you" actions={actions} />
            <PeerWorkRows group="done" actions={actions} />
            <li className="px-sm py-xs text-caption text-foreground">shop-api</li>
            <GuestProjectRows projectId="shop" inset="var(--spacing-sm)" actions={actions} />
            <GuestMachineGroups actions={actions} projects={new Set(["shop"])} inset="var(--spacing-sm)" />
          </ul>
          <div className="relative flex min-w-0 flex-1 flex-col">
            <GuestPaneScreen paneId={PANE} actions={actions} />
            <EntryPointMenu label="MacBook" items={guestPaneMenu("MacBook", t)} onSelect={() => undefined} at={params.get("menu") === "open" ? { x: 520, y: 96 } : null} onClose={() => undefined} data-guest-menu-shown="true" />
          </div>
          <ConfirmGuestPaneClose actions={actions} />
        </div>
      </TooltipProvider>
    </>
  );
}
