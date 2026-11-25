// The shell's unsaved-draft store (`web/src/buffers.ts`, IndexedDB `hide-shell`
// v4, `drafts_v4`) as a spec reads and shapes it from the page.

import type { Page } from "@playwright/test";

export function draftId(hostId: string, root: string, file: string, device: string): string {
  return [hostId, device, root, file].join("\u0000");
}

export type DraftRow = { id: string; host: string | null; device: string | null; root: string; path: string; contents: string; updated_at: number };

/** Writes one row into the shell's v4 draft store, as a tab or an older build left it. */
export async function seedDraft(page: Page, row: DraftRow): Promise<void> {
  await page.evaluate(async (row) => {
    await new Promise<void>((resolve, reject) => {
      const opened = indexedDB.open("hide-shell", 4);
      opened.onupgradeneeded = () => {
        if (!opened.result.objectStoreNames.contains("drafts_v4")) opened.result.createObjectStore("drafts_v4", { keyPath: "id" });
      };
      opened.onerror = () => reject(opened.error);
      opened.onsuccess = () => {
        const database = opened.result;
        const transaction = database.transaction("drafts_v4", "readwrite");
        transaction.objectStore("drafts_v4").put(row);
        transaction.oncomplete = () => { database.close(); resolve(); };
        transaction.onerror = () => { database.close(); reject(transaction.error); };
      };
    });
  }, row);
}

/** Every row of the shell's v4 draft store. */
export async function storedDrafts(page: Page): Promise<DraftRow[]> {
  return page.evaluate(async () => new Promise<DraftRow[]>((resolve, reject) => {
    const opened = indexedDB.open("hide-shell", 4);
    opened.onerror = () => reject(opened.error);
    opened.onsuccess = () => {
      const database = opened.result;
      if (!database.objectStoreNames.contains("drafts_v4")) { database.close(); resolve([]); return; }
      const read = database.transaction("drafts_v4", "readonly").objectStore("drafts_v4").getAll();
      read.onsuccess = () => { database.close(); resolve(read.result as DraftRow[]); };
      read.onerror = () => { database.close(); reject(read.error); };
    };
  }));
}
