// The phone's list, device sheet, missions tab, mission page, PR tab, pull
// request page and the merge and hand-over sheets from production parts with
// invented rows (PRD mobile-companion-ui B1-B9, B28, B31-B44, doop final board
// F1, F2, F8, F9, F10 and F11). No socket.

import { useLayoutEffect, useState } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ListScreen } from "../mobile/App";
import { MissionDetail } from "../mobile/MissionDetail";
import { MissionsScreen } from "../mobile/MissionsScreen";
import { PrDetail } from "../mobile/PrDetail";
import { PrsScreen } from "../mobile/PrsScreen";
import { confirmOf } from "../mobile/prModel";
import { patch, usePhone } from "../mobile/store";
import "../mobile/mobile.css";
import { RELEASE_MISSION, phoneMissions } from "./mobilePhoneMissionsData";
import { AGENT_PR, READY_PR, phoneFeedback, phonePrs } from "./mobilePhonePrsData";
import { PHONE_MAC, PHONE_ROOM, phoneDevices, phoneGroups, seedPairedMac } from "./mobilePhoneSceneData";

export type PhoneSceneState = "phone-list" | "phone-devices" | "phone-missions" | "phone-mission" | "phone-prs" | "phone-pr" | "phone-pr-agent" | "phone-pr-merge" | "phone-pr-delegate";

/** The pull request a page state opens: the failing one for the hand-over, the one an agent works on for its row, else the one that can merge. */
const PR_PAGE: Partial<Record<PhoneSceneState, string>> = { "phone-pr": READY_PR, "phone-pr-agent": AGENT_PR, "phone-pr-merge": READY_PR, "phone-pr-delegate": "p1#2088" };

/** The core's own start catalog for a hand-over: the kind and model the desktop remembered. */
const CATALOG = {
  targets: [],
  core_platform: "macos aarch64",
  kinds: [{ id: "claude", models: ["opus", "sonnet"] }, { id: "codex", models: ["gpt-5"] }],
  remembered: { kind: "claude", models: { claude: "opus" } },
};

export function MobilePhoneScene({ state, language, theme, scale }: { state: PhoneSceneState; language: string; theme: string; scale: number }) {
  const [ready] = useState(() => { seedPairedMac(); return true; });
  const devicesOpen = state === "phone-devices";
  useFollowInterfaceLanguage(language);
  useLayoutEffect(() => {
    const before = usePhone.getState();
    const groups = phoneGroups();
    const prs = phonePrs();
    const prKey = PR_PAGE[state] ?? null;
    const page = prKey ? prs.items.find((candidate) => candidate.key === prKey) ?? null : null;
    patch({
      screen: "app", connected: true, unreachable: false, macName: PHONE_MAC, otherPhones: 0, groups, devices: phoneDevices(groups), childrenOpen: {}, foldsOpen: {},
      tab: state === "phone-missions" || state === "phone-mission" ? "missions" : state.startsWith("phone-pr") ? "prs" : "agents",
      missions: phoneMissions(), missionsFailed: false, missionOpen: state === "phone-mission" ? RELEASE_MISSION : null,
      missionFolds: state === "phone-mission" ? { [`done:${RELEASE_MISSION}`]: true } : {}, missionDrafts: {}, missionAnswer: null, missionAnswerError: null,
      prs, prOpen: prKey, prPending: null, prNotice: null, prDelegated: null,
      prConfirm: state === "phone-pr-merge" && page ? confirmOf(page) : null,
      startCatalog: state === "phone-pr-delegate" ? CATALOG : null,
      prDelegate: state === "phone-pr-delegate" && page
        ? { key: page.key, prompt: "", edited: false, choice: { target: null, kind: null, model: undefined }, feedbackId: "gallery-feedback", pending: null, error: null }
        : null,
    });
    // The feedback the core read for the hand-over, as the frame carries it.
    if (state === "phone-pr-delegate") patch({ prs: { ...prs, feedback: phoneFeedback("gallery-feedback") } });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => {
      usePhone.setState(before, true);
      document.documentElement.style.removeProperty("--interface-scale");
    };
  }, [state, theme, scale]);
  return (
    <TooltipProvider>
      <div className={state === "phone-mission" || state === "phone-prs" || state in PR_PAGE ? "h-screen w-full bg-background text-foreground" : "min-h-screen w-full bg-background text-foreground"} data-gallery-scene="mobile" data-scene-ready={ready ? "true" : undefined}>
        {state === "phone-missions" ? <MissionsScreen /> : state === "phone-mission" ? <MissionDetail /> : state === "phone-prs" ? <PrsScreen /> : state in PR_PAGE ? <PrDetail /> : <ListScreen hosted picker={{ room: PHONE_ROOM, open: devicesOpen }} />}
      </div>
    </TooltipProvider>
  );
}
