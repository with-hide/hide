import { describe, expect, it } from "vitest";
import { advanceHint, clearHint, HINT_DELAY_MS, holdModifiers, idleHint, modifiersOf, NO_MODIFIERS, revealedFamily, type Modifiers } from "./hints";
import { REGISTRY, systemRegistry } from "./shortcuts";

const meta: Modifiers = { ...NO_MODIFIERS, meta: true };
const alt: Modifiers = { ...NO_MODIFIERS, alt: true };
const metaShift: Modifiers = { ...NO_MODIFIERS, meta: true, shift: true };

describe("the hold hint state (electron-digit-shortcuts-hints D-03, D-04)", () => {
  it("reveals a hold only once its delay has passed", () => {
    const held = holdModifiers(idleHint(), meta, 1000);
    expect(held).toEqual({ modifiers: meta, deadline: 1000 + HINT_DELAY_MS, revealed: false });
    expect(advanceHint(held, 1000 + HINT_DELAY_MS - 1)).toBe(held);
    const shown = advanceHint(held, 1000 + HINT_DELAY_MS);
    expect(shown).toEqual({ modifiers: meta, deadline: null, revealed: true });
  });

  it("shows nothing for a hold released before the delay (B8: ⌘C never flashes)", () => {
    const held = holdModifiers(idleHint(), meta, 1000);
    // A release arrives as a fresh all-false set off the key event; the state keeps the one idle value by reference, which is how the listener tells an ended hold from none.
    const released = holdModifiers(held, { ...NO_MODIFIERS }, 1050);
    expect(released).toEqual(idleHint());
    expect(released.modifiers).toBe(NO_MODIFIERS);
    expect(idleHint().modifiers).toBe(NO_MODIFIERS);
    expect(clearHint().modifiers).toBe(NO_MODIFIERS);
    expect(advanceHint(released, 2000)).toBe(released);
  });

  it("hides on release, on an added modifier, and on a clear", () => {
    const shown = advanceHint(holdModifiers(idleHint(), meta, 0), HINT_DELAY_MS);
    expect(holdModifiers(shown, NO_MODIFIERS, 500)).toEqual(idleHint());
    // ⇧ added to a held ⌘: still a hold, still "revealed", but the set no longer names a family.
    const widened = holdModifiers(shown, metaShift, 500);
    expect(widened).toEqual({ modifiers: metaShift, deadline: null, revealed: true });
    expect(revealedFamily(widened, REGISTRY, "electron")).toBeNull();
    // Back to ⌘ alone shows again with no second delay, as the native app did.
    expect(revealedFamily(holdModifiers(widened, meta, 600), REGISTRY, "electron")).toBe("tabs");
    expect(clearHint()).toEqual(idleHint());
  });

  it("stays quiet after a clear until the modifiers change again", () => {
    // A key pressed during the hold clears; the same modifiers still down are not a new hold.
    const cleared = clearHint();
    expect(holdModifiers(cleared, NO_MODIFIERS, 10)).toBe(cleared);
    expect(holdModifiers(cleared, meta, 20).deadline).toBe(20 + HINT_DELAY_MS);
  });

  it("names the family the held modifiers select, on the host that has it", () => {
    const ctrl = advanceHint(holdModifiers(idleHint(), { ...NO_MODIFIERS, ctrl: true }, 0), HINT_DELAY_MS);
    const agents = advanceHint(holdModifiers(idleHint(), alt, 0), HINT_DELAY_MS);
    const tabs = advanceHint(holdModifiers(idleHint(), meta, 0), HINT_DELAY_MS);
    expect(revealedFamily(ctrl, REGISTRY, "electron")).toBeNull();
    expect(revealedFamily(agents, REGISTRY, "electron")).toBe("agents");
    expect(revealedFamily(tabs, REGISTRY, "electron")).toBe("tabs");
    expect(revealedFamily(tabs, REGISTRY, "browser")).toBeNull();
    // ⌥ added to a revealed ⌘ turns the numbers to the devices at once, with no second delay.
    expect(revealedFamily(holdModifiers(tabs, { ...meta, alt: true }, 400), REGISTRY, "electron")).toBe("devices");
    expect(revealedFamily(agents, REGISTRY, "browser")).toBeNull();
    expect(revealedFamily(holdModifiers(idleHint(), meta, 0), REGISTRY, "electron")).toBeNull();
  });

  it("reveals the tab numbers on a held Ctrl+Shift and the device numbers on Alt+Shift on Windows and Linux, and nothing on Ctrl alone", () => {
    const pc = systemRegistry("pc");
    const held = (modifiers: Modifiers) => advanceHint(holdModifiers(idleHint(), modifiers, 0), HINT_DELAY_MS);
    expect(revealedFamily(held({ ...NO_MODIFIERS, ctrl: true, shift: true }), pc, "electron")).toBe("tabs");
    expect(revealedFamily(held({ ...NO_MODIFIERS, alt: true, shift: true }), pc, "electron")).toBe("devices");
    expect(revealedFamily(held(alt), pc, "electron")).toBe("agents");
    expect(revealedFamily(held({ ...NO_MODIFIERS, ctrl: true }), pc, "electron")).toBeNull();
    expect(revealedFamily(held(meta), pc, "electron")).toBeNull();
  });

  it("reads the modifiers off a key event", () => {
    expect(modifiersOf({ metaKey: true, altKey: false, shiftKey: false, ctrlKey: false })).toEqual(meta);
    expect(modifiersOf({ metaKey: false, altKey: true, shiftKey: true, ctrlKey: false })).toEqual({ ...alt, shift: true });
  });
});
