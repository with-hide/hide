// The phone's list and device sheet from production parts with invented rows
// (PRD mobile-companion-ui B1-B9, doop final board F1 and F2). No socket.

import { useLayoutEffect, useState } from "react";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { ListScreen } from "../mobile/App";
import { patch, usePhone } from "../mobile/store";
import "../mobile/mobile.css";
import { PHONE_DEVICES, PHONE_MAC, PHONE_ROOM, phoneGroups, seedPairedMac } from "./mobilePhoneSceneData";

export function MobilePhoneScene({ state, language, theme, scale }: { state: "phone-list" | "phone-devices"; language: string; theme: string; scale: number }) {
  const [ready] = useState(() => { seedPairedMac(); return true; });
  const devicesOpen = state === "phone-devices";
  useFollowInterfaceLanguage(language);
  useLayoutEffect(() => {
    const before = usePhone.getState();
    patch({ screen: "app", connected: true, unreachable: false, macName: PHONE_MAC, otherPhones: 0, groups: phoneGroups(), devices: PHONE_DEVICES, childrenOpen: {}, foldsOpen: {} });
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
    return () => {
      usePhone.setState(before, true);
      document.documentElement.style.removeProperty("--interface-scale");
    };
  }, [theme, scale]);
  return (
    <TooltipProvider>
      <div className="min-h-screen w-full bg-background text-foreground" data-gallery-scene="mobile" data-scene-ready={ready ? "true" : undefined}>
        <ListScreen hosted picker={{ room: PHONE_ROOM, open: devicesOpen }} />
      </div>
    </TooltipProvider>
  );
}
