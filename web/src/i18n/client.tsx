import { useSyncExternalStore } from "react";
import { readViewer, subscribeViewer } from "../remoteView";
import { useShellStore } from "../store";
import { useFollowInterfaceLanguage } from "./translator";

export { translate, useInterfaceTranslation } from "./translator";

/** Follows confirmed snapshots, including reconnects and another client's edit. */
export function InterfaceLanguageBoundary() {
  // Another machine's screen with no choice of its own reads as the window that shows it (PRD device-connect-iroh B17).
  const viewer = useSyncExternalStore(subscribeViewer, readViewer, () => null);
  useFollowInterfaceLanguage(useShellStore((state) => state.rest?.ui_state?.interface_language), viewer?.language ?? null);
  return null;
}
