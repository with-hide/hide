import type { TFunction } from "i18next";
import type { StoreAsk } from "./snapshot";
import type { Notice } from "./ui";

/** One line saying what an agent asked to install, update or add with `hide skill` / `hide mcp`. */
export function storeAskTitle(ask: StoreAsk, t: TFunction<"translation">): string {
  switch (ask.what) {
    case "install":
      return t("store.ask.install", { names: ask.names.join(", ") });
    case "update":
      return t("store.ask.update", { name: ask.item.split(":").slice(1).join(":") });
    case "add_mcp":
      return t("store.ask.addMcp", { name: ask.server.name });
    case "set_mcp":
      return t("store.ask.setMcp", { name: ask.server.name });
  }
}

const KEY_PREFIX = "store_ask:";

/** The corner notice's key for one ask, so a notice and the ask it stands for are found by the ask's id. */
export function storeAskNoticeKey(id: string): string {
  return `${KEY_PREFIX}${id}`;
}

/**
 * What the corner has to do when the asks waiting on the operator change:
 * a notice for each ask that was not there before, and the ids of the asks
 * that were answered or gone, whose notices leave with them (the operator's
 * answer is the cause that clears an important notice).
 */
export function storeAskNoticeChanges(current: readonly StoreAsk[] | undefined, previous: readonly StoreAsk[] | undefined): { raise: StoreAsk[]; gone: string[] } {
  const now = current ?? [];
  const before = previous ?? [];
  const had = new Set(before.map((ask) => ask.id));
  const has = new Set(now.map((ask) => ask.id));
  return { raise: now.filter((ask) => !had.has(ask.id)), gone: before.filter((ask) => !has.has(ask.id)).map((ask) => ask.id) };
}

/** The important notice for one ask: its line, and the press that opens its confirm. */
export function storeAskNotice(ask: StoreAsk, t: TFunction<"translation">): Notice {
  return { level: "important", key: storeAskNoticeKey(ask.id), text: storeAskTitle(ask, t), refreshable: false, storeAsk: ask.id };
}
