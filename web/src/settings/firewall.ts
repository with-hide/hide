// The Windows firewall rules that let machines on the same network reach
// this PC directly (PRD device-connect-iroh D-38, B32). The desktop host
// reads them and asks Windows for them through its permission prompt
// (`desktop/src/main/firewall.ts`); a browser and the other systems have no
// such rules, and the host answers `not_used` there.

import { useCallback, useEffect, useState } from "react";
import { hostBridge, type FirewallAnswer, type FirewallState } from "../host";

/** Whether this window already asked Windows on its own, so one window asks once. */
let askedOnItsOwn = false;
/** Told when an ask starts and when it ends, so a shown row follows it. */
const asks = new EventTarget();
let askingNow = false;

function asked<T>(ask: Promise<T>): Promise<T> {
  askingNow = true;
  asks.dispatchEvent(new Event("change"));
  return ask.finally(() => {
    askingNow = false;
    asks.dispatchEvent(new Event("change"));
  });
}

/**
 * Asks Windows for the rules when they are missing, once per window: what
 * sharing this PC and allowing a pairing do (B32). Never at login, and never
 * twice on its own; after a no the operator asks from the Devices row.
 */
export async function askForFirewallOnce(): Promise<FirewallAnswer | null> {
  const host = hostBridge();
  if (askedOnItsOwn || !host?.firewallState || !host.allowFirewall) return null;
  askedOnItsOwn = true;
  try {
    // Before the endpoint first opens the folder has no port yet; the ask
    // chooses it (`hide always-on firewall ask-allow`).
    const state = await host.firewallState();
    if (state !== "missing" && state !== "no_port") return null;
    return await asked(host.allowFirewall());
  } catch {
    // The host logged why; the Devices row still offers the ask.
    return "failed";
  }
}

/** For tests: a new window. */
export function forgetFirewallAsk(): void {
  askedOnItsOwn = false;
  askingNow = false;
}

/**
 * The rules' state while `watching`, and the ask the operator presses: the
 * state is read again once Windows answered. `null` while unread or where
 * the host has no rules to read.
 */
export function useFirewall(watching: boolean): { state: FirewallState | null; asking: boolean; allow: () => void } {
  const [state, setState] = useState<FirewallState | null>(null);
  const [asking, setAsking] = useState(askingNow);
  const read = useCallback(() => {
    const host = hostBridge();
    if (!host?.firewallState) return;
    void host.firewallState().then(setState, () => setState("unknown"));
  }, []);
  useEffect(() => {
    if (!watching) return;
    read();
    const follow = () => {
      setAsking(askingNow);
      if (!askingNow) read();
    };
    asks.addEventListener("change", follow);
    return () => asks.removeEventListener("change", follow);
  }, [watching, read]);
  const allow = useCallback(() => {
    const host = hostBridge();
    if (!host?.allowFirewall) return;
    void asked(host.allowFirewall()).catch(() => "failed" as const);
  }, []);
  return { state, asking, allow };
}
