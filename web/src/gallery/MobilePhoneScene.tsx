// The phone's list and device sheet from production parts with invented rows
// (PRD mobile-companion-ui B1-B9, doop final board F1 and F2). No socket.

import { useLayoutEffect, useState } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ListScreen } from "../mobile/App";
import { Detail } from "../mobile/Detail";
import { patch, usePhone } from "../mobile/store";
import "../mobile/mobile.css";
import { PHONE_LOOK_SCENES, PHONE_MAC, PHONE_ROOM, phoneDevices, phoneGroups, phoneLookDetail, phoneLookGroups, seedPairedMac, type PhoneLookScene } from "./mobilePhoneSceneData";

/** The scene names this component draws: the list, the device sheet, and the 볼 것 tab with its viewers. */
export const PHONE_SCENES: readonly string[] = ["phone-list", "phone-devices", ...PHONE_LOOK_SCENES];

const noop = () => {};

/** `offline` draws the last content dimmed under the unreachable line (B45). */
export function MobilePhoneScene({ state, language, theme, scale, offline }: { state: string; language: string; theme: string; scale: number; offline: boolean }) {
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
    });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => {
      usePhone.setState(before, true);
      document.documentElement.style.removeProperty("--interface-scale");
    };
  }, [theme, scale, look, offline]);
  return (
    <TooltipProvider>
      <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="mobile" data-scene-ready={ready ? "true" : undefined}>
        {look ? (
          <div className="h-screen">
            <Detail onBack={noop} />
          </div>
        ) : (
          <ListScreen hosted picker={{ room: PHONE_ROOM, open: devicesOpen }} />
        )}
      </div>
    </TooltipProvider>
  );
}
