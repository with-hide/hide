// A command's title in the operator's language. Kept apart from
// `shortcutLabels.ts`, which reads the shell's store, so the desktop host's
// main process can name the app menu's commands without that module graph.

import type { TFunction } from "i18next";
import { isNumberedCommand, numberedCommand, type CommandId, type NumberedFamily } from "./shortcuts";

/** The title key of each numbered family, read with its number or its range. */
export const FAMILY_TITLE: Readonly<Record<NumberedFamily, "commands.select_tab" | "commands.select_agent" | "commands.select_device">> = {
  tabs: "commands.select_tab",
  agents: "commands.select_agent",
  devices: "commands.select_device",
};

/** The registry's `title` stays the English the e2e specs and the sheet's search read; this is what a screen or the menu draws. */
export function commandTitle(id: CommandId, t: TFunction<"translation">): string {
  if (isNumberedCommand(id)) {
    const { family, number } = numberedCommand(id)!;
    return t(FAMILY_TITLE[family], { number });
  }
  return t(`commands.${id}`);
}
