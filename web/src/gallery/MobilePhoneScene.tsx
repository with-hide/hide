// The phone's list, device sheet, missions tab and mission page from
// production parts with invented rows (PRD mobile-companion-ui B1-B9, B28,
// B31-B36, doop final board F1, F2, F8 and F11). No socket.

import { useLayoutEffect, useState } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ListScreen } from "../mobile/App";
import { MissionDetail } from "../mobile/MissionDetail";
import { MissionsScreen } from "../mobile/MissionsScreen";
import { patch, usePhone } from "../mobile/store";
import "../mobile/mobile.css";
import { RELEASE_MISSION, phoneMissions } from "./mobilePhoneMissionsData";
import { PHONE_MAC, PHONE_ROOM, phoneDevices, phoneGroups, seedPairedMac } from "./mobilePhoneSceneData";

export type PhoneSceneState = "phone-list" | "phone-devices" | "phone-missions" | "phone-mission";

export function MobilePhoneScene({ state, language, theme, scale }: { state: PhoneSceneState; language: string; theme: string; scale: number }) {
  const [ready] = useState(() => { seedPairedMac(); return true; });
  const devicesOpen = state === "phone-devices";
  useFollowInterfaceLanguage(language);
  useLayoutEffect(() => {
    const before = usePhone.getState();
    const groups = phoneGroups();
    patch({
      screen: "app", connected: true, unreachable: false, macName: PHONE_MAC, otherPhones: 0, groups, devices: phoneDevices(groups), childrenOpen: {}, foldsOpen: {},
      tab: state === "phone-missions" || state === "phone-mission" ? "missions" : "agents",
      missions: phoneMissions(), missionsFailed: false, missionOpen: state === "phone-mission" ? RELEASE_MISSION : null,
      missionFolds: state === "phone-mission" ? { [`done:${RELEASE_MISSION}`]: true } : {}, missionDrafts: {}, missionAnswer: null, missionAnswerError: null,
    });
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
      <div className={state === "phone-mission" ? "h-screen w-full bg-background text-foreground" : "min-h-screen w-full bg-background text-foreground"} data-gallery-scene="mobile" data-scene-ready={ready ? "true" : undefined}>
        {state === "phone-missions" ? <MissionsScreen /> : state === "phone-mission" ? <MissionDetail /> : <ListScreen hosted picker={{ room: PHONE_ROOM, open: devicesOpen }} />}
      </div>
    </TooltipProvider>
  );
}
