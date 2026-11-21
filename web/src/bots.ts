// How a bot looks (an external tool that ran `hide access request`): one of
// seven owls on one of seven tile colours, both read from its name, so the
// same bot looks the same on the rail, in Settings, on a pane head and on the
// phone without the core storing a choice. Every surface draws it through
// `BotMark` (`components/bot-mark.tsx`), which reads this.

import antenna from "./assets/bots/antenna.png";
import beanie from "./assets/bots/beanie.png";
import headset from "./assets/bots/headset.png";
import sleepy from "./assets/bots/sleepy.png";
import smile from "./assets/bots/smile.png";
import tufts from "./assets/bots/tufts.png";
import wink from "./assets/bots/wink.png";
import type { AccessMode } from "./snapshot";

export const BOT_SHAPES = ["smile", "sleepy", "wink", "headset", "antenna", "beanie", "tufts"] as const;
export const BOT_COLORS = ["slate", "coral", "teal", "lavender", "mustard", "sage", "charcoal"] as const;
export type BotShape = (typeof BOT_SHAPES)[number];
export type BotColor = (typeof BOT_COLORS)[number];
export type BotLook = { shape: BotShape; color: BotColor };

/** Each owl's file, a transparent square in which the tile's colour shows through the pupils and mouth. */
export const BOT_SHAPE_SRC: Record<BotShape, string> = { smile, sleepy, wink, headset, antenna, beanie, tufts };

/** FNV-1a over the name's UTF-16 units: the same name gives the same number in every window and on the phone. */
function nameHash(name: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < name.length; index += 1) {
    hash ^= name.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

/**
 * A bot's owl and tile colour from its name: the shape from the hash's
 * remainder by seven and the colour from the next base-seven digit, so the
 * two vary independently and two bots share both one time in 49.
 */
export function botLook(name: string): BotLook {
  const hash = nameHash(name);
  return { shape: nth(BOT_SHAPES, hash), color: nth(BOT_COLORS, Math.floor(hash / BOT_SHAPES.length)) };
}

function nth<T>(list: readonly T[], value: number): T {
  const item = list[value % list.length];
  if (item === undefined) throw new Error(`no item ${value} in a list of ${list.length}`);
  return item;
}

/** A workspace as a bot's mode names it: its name, and its machine's after it when that is not this window's. */
export function workspaceName(workspace: { label: string; device_id: string }, deviceName: (deviceId: string) => string | null): string {
  const device = deviceName(workspace.device_id);
  return device === null ? workspace.label : `${workspace.label} (${device})`;
}

/**
 * The workspace a bot is held inside, named for the screen, or null for a
 * whole-Hide bot. A workspace the core no longer lists was removed, and the
 * bot reaches nothing there, which `gone` words.
 */
export function botWorkspaceName(mode: AccessMode, workspaces: readonly { id: string; label: string; device_id: string }[], deviceName: (deviceId: string) => string | null, gone: string): string | null {
  if (mode.kind === "all") return null;
  const workspace = workspaces.find((one) => one.id === mode.workspace_id);
  return workspace ? workspaceName(workspace, deviceName) : gone;
}
