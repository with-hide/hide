// The phone's list, device sheet, missions tab, mission page and the 볼 것
// tab with its viewers from production parts with invented rows (PRD
// mobile-companion-ui B1-B9, B10-B19, B28, B31-B36, doop final board F1, F2,
// F8 and F11). No socket.

import { useLayoutEffect, useState } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ListScreen } from "../mobile/App";
import { Detail } from "../mobile/Detail";
import { MissionDetail } from "../mobile/MissionDetail";
import { MissionsScreen } from "../mobile/MissionsScreen";
import { patch, usePhone } from "../mobile/store";
import "../mobile/mobile.css";
import { RELEASE_MISSION, phoneMissions } from "./mobilePhoneMissionsData";
import { PHONE_LOOK_SCENES, PHONE_MAC, PHONE_ROOM, phoneDevices, phoneGroups, phoneLookDetail, phoneLookGroups, seedPairedMac, type PhoneLookScene } from "./mobilePhoneSceneData";

export type PhoneSceneState = "phone-list" | "phone-devices" | "phone-missions" | "phone-mission" | PhoneLookScene;

/** The scene names this component draws: the list, the device sheet, the missions tab and a mission, and the 볼 것 tab with its viewers. */
export const PHONE_SCENES: readonly string[] = ["phone-list", "phone-devices", "phone-missions", "phone-mission", ...PHONE_LOOK_SCENES];

const noop = () => {};

/** `offline` draws the last content dimmed under the unreachable line (B45). */
export function MobilePhoneScene({ state, language, theme, scale, offline }: { state: PhoneSceneState; language: string; theme: string; scale: number; offline: boolean }) {
  const [ready] = useState(() => { seedPairedMac(); return true; });
  const devicesOpen = state === "phone-devices";
  const look = (PHONE_LOOK_SCENES as readonly string[]).includes(state) ? (state as PhoneLookScene) : null;
  useFollowInterfaceLanguage(language);
  useLayoutEffect(() => {
    const before = usePhone.getState();
    const groups = look ? phoneLookGroups() : phoneGroups();
    patch({
      screen: "app", connected: !offline, unreachable: offline, macName: PHONE_MAC, otherPhones: 0, groups, devices: phoneDevices(groups), childrenOpen: {}, foldsOpen: {},
      detail: look ? phoneLookDetail(look) : null,
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
  }, [state, theme, scale, look, offline]);
  return (
    <TooltipProvider>
      <div className={state === "phone-mission" ? "h-screen w-full bg-background text-foreground" : "min-h-screen w-full bg-background text-foreground"} data-gallery-scene="mobile" data-scene-ready={ready ? "true" : undefined}>
        {look ? (
          <div className="h-screen">
            <Detail onBack={noop} />
          </div>
        ) : state === "phone-missions" ? <MissionsScreen /> : state === "phone-mission" ? <MissionDetail /> : <ListScreen hosted picker={{ room: PHONE_ROOM, open: devicesOpen }} />}
      </div>
    </TooltipProvider>
  );
}
