// The flows after the first: pictures to look things up in, with no screen to
// play. Each has a card on the list (its title and summary), and its detail
// shows a few of its moves drawn one after another (`PracticeFrames.tsx`), one
// plain line of what to know and the chords it uses. The words live in the
// practice catalog under `practice.<id>.*`: `title`, `summary`, `line` and
// `frame1`.. one caption for each frame.

import type { CommandId } from "../shortcuts";
import type { PracticePreviewKind } from "./chapter";

export type PracticeGuide = {
  id: string;
  /** The flow's number as the list shows it. */
  number: number;
  /** The move its card plays on the list. */
  preview: PracticePreviewKind;
  /** The moves its detail draws, in order. */
  frames: PracticePreviewKind[];
  /** The commands whose chords its detail lists. */
  chords: CommandId[];
  /** Whether its detail offers a real mission, as the first flow's last card does. */
  mission?: true;
};

export const GUIDES: readonly PracticeGuide[] = [
  { id: "left", number: 2, preview: "tree", frames: ["tree", "rows", "waits", "asleep"], chords: ["toggle_left_sidebar", "toggle_sidebar_lens", "sidebar_projects"] },
  { id: "agents", number: 3, preview: "split", frames: ["children", "split", "rows", "switch"], chords: ["split_right", "split_down", "recent_area_tab", "close_tab", "reopen_closed_tab"] },
  { id: "mission", number: 4, preview: "mission", frames: ["mission", "children", "rows"], chords: ["start_agent", "open_project_memory"], mission: true },
  { id: "devices", number: 5, preview: "devices", frames: ["devices", "rows", "split"], chords: ["toggle_device_rail", "new_tab"] },
  { id: "day", number: 6, preview: "overview", frames: ["overview", "search", "split"], chords: ["overview", "search"] },
];
