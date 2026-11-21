// One dynamic import per part, so a docs page loads the code of the part it frames and nothing else. A part's file
// default-exports a component of `PartProps`.
import type { ComponentType } from "react";
import type { PartId } from "./parts";

export type PartProps = { state: string | null };
export type PartModule = { default: ComponentType<PartProps> };

export const RENDERERS: Record<PartId, () => Promise<PartModule>> = {
  "task-row": () => import("./parts/TaskRowPart"),
  "status-colors": () => import("./parts/StatusColorsPart"),
  "turn-card": () => import("./parts/TurnCardPart"),
  "pane-header": () => import("./parts/PaneHeaderPart"),
  "pr-card": () => import("./parts/PrCardPart"),
  "view-slot": () => import("./parts/ViewSlotPart"),
  "memory-cards": () => import("./parts/MemoryCardsPart"),
  "mission-map": () => import("./parts/MissionMapPart"),
  letters: () => import("./parts/LettersPart"),
  phone: () => import("./parts/PhonePart"),
};
