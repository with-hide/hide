// Which devices the center already offers 다시 연결 for. The sidebar's
// disconnected block then names the device and its state without a second
// button for the same action; on a screen whose center offers none (Usage,
// History, Settings) the sidebar's button stays the way to try again.

import { useEffect } from "react";
import { create } from "zustand";

export const useCenterReconnect = create<{ shown: Readonly<Record<string, number>> }>(() => ({ shown: {} }));

function count(deviceId: string, by: 1 | -1) {
  useCenterReconnect.setState((state) => {
    const next = (state.shown[deviceId] ?? 0) + by;
    const shown = { ...state.shown };
    if (next > 0) shown[deviceId] = next;
    else delete shown[deviceId];
    return { shown };
  });
}

/** Called by a center surface while it draws a reconnect for `deviceId`; null while it draws none. */
export function useOffersReconnect(deviceId: string | null) {
  useEffect(() => {
    if (!deviceId) return undefined;
    count(deviceId, 1);
    return () => count(deviceId, -1);
  }, [deviceId]);
}
