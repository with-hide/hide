// Mounts the practice window's page. In the desktop app the window has no
// preload, so the desktop host is stood in for from the address the host
// opened (`host=electron&platform=…`): the shell then answers the chords the
// operator's app answers, and nothing reaches a host.

import { StrictMode } from "react";
import type { Root } from "react-dom/client";
import type { HostBridge } from "../host";
import { clientI18n } from "../i18n/translator";
import { isInterfaceLanguage } from "../i18n/locale";
import { PracticeApp } from "./PracticeApp";

/** The desktop host, standing in: it names its kind and OS, keeps no page and answers nothing. */
export function standInHost(platform: string): HostBridge {
  const quiet = (name: string) => (name.startsWith("on") ? () => () => {} : () => Promise.resolve(null));
  const part = <T extends object>(fixed: Record<string, unknown>): T => new Proxy(fixed, { get: (target, name) => (typeof name !== "string" || name === "then" ? undefined : name in target ? target[name] : quiet(name)) }) as T;
  return part<HostBridge>({ kind: "electron", platform, browser: part({}), updates: part({}) });
}

export function mountPractice(root: Root, params: URLSearchParams) {
  if (params.get("host") === "electron" && !window.hideHost) window.hideHost = standInHost(params.get("platform") ?? "darwin");
  const language = params.get("lang");
  if (isInterfaceLanguage(language)) {
    void clientI18n.changeLanguage(language);
    document.documentElement.lang = language;
  }
  root.render(
    <StrictMode>
      <PracticeApp />
    </StrictMode>,
  );
}
