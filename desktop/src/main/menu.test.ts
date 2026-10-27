import type { MenuItemConstructorOptions } from "electron";
import { describe, expect, it } from "vitest";
import { initializeInterfaceI18n } from "../../../web/src/i18n/instance";
import { EDITABLE_PANE_COMMANDS, REGISTRY } from "../../../web/src/shortcuts";
import { accelerator, BINDINGS_CAP, KEYBOARD_ONLY, MENU_LAYOUT, menuBindings, menuTemplate } from "./menu";

const english = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");

describe("the app menu (B9)", () => {
  it("writes registry chords as Electron accelerators", () => {
    expect(accelerator({ code: "KeyT", meta: true })).toBe("Command+T");
    expect(accelerator({ code: "KeyT", meta: true, shift: true })).toBe("Shift+Command+T");
    expect(accelerator({ code: "Enter", meta: true, alt: true })).toBe("Alt+Command+Return");
    expect(accelerator({ code: "Digit0", meta: true })).toBe("Command+0");
    expect(accelerator({ code: "Comma", meta: true })).toBe("Command+,");
    expect(accelerator({ code: "BracketRight", meta: true, shift: true })).toBe("Shift+Command+]");
    expect(accelerator({ code: "PageDown", ctrl: true })).toBe("Control+PageDown");
    expect(() => accelerator({ code: "IntlRo", meta: true })).toThrow();
  });

  it("places every clickable command exactly once, including the immediate navigation commands", () => {
    const placed = Object.values(MENU_LAYOUT).flat().filter((id) => id !== null);
    expect(new Set(placed).size).toBe(placed.length);
    const expected = REGISTRY.map((command) => command.id).filter((id) => !KEYBOARD_ONLY.includes(id));
    expect([...placed].sort()).toEqual([...expected].sort());
  });

  it("holds every editable command in the reported set, so binding them all is never refused", () => {
    expect(BINDINGS_CAP).toBeGreaterThanOrEqual(EDITABLE_PANE_COMMANDS.length);
  });

  it("lists the Agent and View area commands in the Pane menu, with no chord until one is set", () => {
    const template = menuTemplate({ appName: "hide", send: () => undefined, developer: false, system: "mac", t: english });
    const pane = template.find((menu) => menu.label === "Pane");
    const items = Array.isArray(pane?.submenu) ? pane.submenu : [];
    for (const id of ["focus_next_agent_area", "shrink_agent_area", "focus_next_view_area", "grow_view_area"]) {
      expect(items.find((item) => item.id === id)?.accelerator).toBeUndefined();
      expect(items.some((item) => item.id === id)).toBe(true);
    }
  });

  it("shows each item with its electron chord and sends its id when clicked", () => {
    const sent: string[] = [];
    const template = menuTemplate({ appName: "hide", send: (id) => sent.push(id), developer: false, system: "mac", t: english });
    const items = template.flatMap((menu) => (Array.isArray(menu.submenu) ? menu.submenu : []));
    const newTab = items.find((item) => item.id === "new_tab");
    expect(newTab?.accelerator).toBe("Command+T");
    const closeTab = items.find((item) => item.id === "close_tab");
    expect(closeTab?.accelerator).toBe("Command+W");
    const cycle = items.find((item) => item.id === "recent_area_tab");
    expect(cycle?.accelerator).toBeUndefined();
    (cycle?.click as () => void)();
    expect(sent).toEqual(["recent_area_tab"]);
    sent.length = 0;
    (newTab?.click as () => void)();
    expect(sent).toEqual(["new_tab"]);
    // No standard item claims a chord the registry owns.
    const roles = items.filter((item) => item.role).map((item) => item.role);
    expect(roles).not.toContain("close");
    expect(roles).not.toContain("reload");
  });

  it("shows the operator's macOS pane chords once the shell reports them", () => {
    const resolved = menuBindings({ toggle_zoom: "command+shift+return", toggle_conversation: "command+option+c" }, "mac");
    if ("refused" in resolved) throw new Error(resolved.refused);
    expect(resolved.diagnostic).toBeNull();
    const template = menuTemplate({ appName: "hide", send: () => undefined, developer: false, system: "mac", t: english, registry: resolved.registry });
    const items = template.flatMap((menu) => (Array.isArray(menu.submenu) ? menu.submenu : []));
    expect(items.find((item) => item.id === "toggle_zoom")?.accelerator).toBe("Shift+Command+Return");
    expect(items.find((item) => item.id === "split_right")?.accelerator).toBe("Command+D");
  });

  it("refuses a report that is not a short string map, and runs the defaults for an unusable set", () => {
    expect(menuBindings(null, "mac")).toEqual({ refused: "not a map" });
    expect(menuBindings(["command+d"], "mac")).toEqual({ refused: "not a map" });
    expect(menuBindings({ split_right: 4 }, "mac")).toEqual({ refused: "not short strings" });
    expect(menuBindings(Object.fromEntries(Array.from({ length: BINDINGS_CAP + 1 }, (_, index) => [`k${index}`, "command+d"])), "mac")).toEqual({ refused: "too many entries" });
    const unusable = menuBindings({ split_right: "command+t" }, "mac");
    expect("refused" in unusable ? null : unusable.registry).toBe(REGISTRY);
  });

  it("shows Windows and Linux the Ctrl+Shift and Alt+Shift chords the window answers there, and none of macOS's own items", () => {
    const template = menuTemplate({ appName: "hide", send: () => undefined, developer: false, system: "pc", t: english });
    const items = template.flatMap((menu) => (Array.isArray(menu.submenu) ? menu.submenu : []));
    const shortcut = (id: string) => items.find((item) => item.id === id)?.accelerator;
    expect(shortcut("new_tab")).toBe("Control+Shift+T");
    expect(shortcut("reopen_closed_tab")).toBe("Alt+Shift+T");
    expect(shortcut("toggle_zoom")).toBe("Alt+Shift+Return");
    expect(shortcut("text_larger")).toBe("Control+=");
    expect(shortcut("settings")).toBe("Control+Shift+,");
    expect(items.some((item) => item.accelerator?.includes("Command"))).toBe(false);
    const roles = items.filter((item) => item.role).map((item) => item.role);
    for (const role of ["services", "hide", "hideOthers", "unhide"]) expect(roles).not.toContain(role);
    expect(roles).toContain("quit");
    // A stored macOS chord reads as the same keys through the rule.
    const resolved = menuBindings({ toggle_zoom: "command+shift+return" }, "pc");
    if ("refused" in resolved) throw new Error(resolved.refused);
    expect(resolved.diagnostic).toBeNull();
    expect(menuTemplate({ appName: "hide", send: () => undefined, developer: false, system: "pc", t: english, registry: resolved.registry }).flatMap((menu) => (Array.isArray(menu.submenu) ? menu.submenu : [])).find((item) => item.id === "toggle_zoom")?.accelerator).toBe("Alt+Shift+Return");
  });

  it("leaves plain Ctrl keys on Windows and Linux to the edit roles, text size and the tab steps", () => {
    // Electron's own role keys on Linux, the stricter of the two (Windows
    // gives quit none), for every role that does not name its accelerator.
    const ROLE_KEYS: Record<string, string> = {
      undo: "Control+Z",
      redo: "Shift+Control+Z",
      cut: "Control+X",
      copy: "Control+C",
      paste: "Control+V",
      pasteAndMatchStyle: "Shift+Control+V",
      selectAll: "Control+A",
      quit: "Control+Q",
      reload: "Control+R",
      toggleDevTools: "Control+Shift+I",
      togglefullscreen: "F11",
      close: "Control+W",
      minimize: "Control+M",
    };
    const template = menuTemplate({ appName: "hide", send: () => undefined, developer: true, system: "pc", t: english });
    const flat = (menu: MenuItemConstructorOptions[]): MenuItemConstructorOptions[] =>
      menu.flatMap((item) => [item, ...(Array.isArray(item.submenu) ? flat(item.submenu) : [])]);
    const plainCtrl = flat(template)
      .map((item) => ({ name: item.role ?? item.id, keys: item.accelerator ?? (item.role === "windowMenu" ? "Control+W" : ROLE_KEYS[item.role ?? ""]) }))
      .filter(({ keys }) => keys?.includes("Control") && !keys.includes("Shift"))
      .map(({ name }) => name);
    expect(plainCtrl.sort()).toEqual(["copy", "cut", "next_tab", "paste", "previous_tab", "selectAll", "text_larger", "text_reset", "text_smaller", "undo"]);
  });

  it("opens Help with Set up Hide on every system, which sends the host's own command", () => {
    for (const system of ["mac", "pc"] as const) {
      const sent: string[] = [];
      const help = menuTemplate({ appName: "hide", send: (id) => sent.push(id), developer: false, system, t: english }).find((menu) => menu.role === "help");
      const first = Array.isArray(help?.submenu) ? help.submenu[0] : undefined;
      expect(first?.label).toBe("Set up Hide");
      (first?.click as () => void)();
      expect(sent).toEqual(["setup_hide"]);
    }
  });

  it("puts Check for updates… in the app menu on macOS and in Help elsewhere, sending the host's own command (D-25)", () => {
    const find = (menu: MenuItemConstructorOptions | undefined) => (Array.isArray(menu?.submenu) ? menu.submenu.find((item) => item.id === "check_updates") : undefined);
    for (const [system, home] of [["mac", 0], ["pc", -1]] as const) {
      const sent: string[] = [];
      const template = menuTemplate({ appName: "hide", send: (id) => sent.push(id), developer: false, system, t: english });
      const holders = template.filter((menu) => find(menu));
      expect(holders).toEqual([home === 0 ? template[0] : template.at(-1)]);
      const item = find(holders[0]);
      expect(item?.label).toBe("Check for updates…");
      (item?.click as () => void)();
      expect(sent).toEqual(["check_updates"]);
    }
  });

  it("draws every label, the system's role items included, in the language it is given", () => {
    const labels = (menu: MenuItemConstructorOptions[]): string[] =>
      menu.flatMap((item) => [...(item.label ? [item.label] : []), ...(Array.isArray(item.submenu) ? labels(item.submenu) : [])]);
    const template = menuTemplate({ appName: "hide", send: () => undefined, developer: true, system: "mac", t: korean });
    expect(template.map((menu) => menu.label)).toEqual(["hide", "파일", "편집", "보기", "터미널", "윈도우", "도움말"]);
    const all = labels(template);
    expect(all).toEqual(expect.arrayContaining(["hide 정보", "hide 종료", "실행 취소", "전체 선택", "개발자 도구 열기/닫기", "전체 화면 전환", "모두 앞으로 가져오기"]));
    for (const english of ["Undo", "Select All", "Services", "Show All", "Bring All to Front", "Toggle Full Screen"]) expect(all).not.toContain(english);
    const pc = menuTemplate({ appName: "hide", send: () => undefined, developer: false, system: "pc", t: korean });
    expect(pc.map((menu) => menu.label)).toEqual(["hide", "파일", "편집", "보기", "터미널", "도움말"]);
  });
});
