import { describe, expect, it } from "vitest";
import {
  DIGITS,
  AREA_COMMANDS,
  EDITABLE_PANE_COMMANDS,
  REGISTRY,
  bindingProblem,
  chordEquals,
  displayChord,
  displayCommand,
  effectiveRegistry,
  familyModifiers,
  hostRegistry,
  isChromeReserved,
  isNumberedCommand,
  keySystemOf,
  modifierLabel,
  macChord,
  modChord,
  parseStoredChord,
  serializeStoredChord,
  systemRegistry,
  matchHost,
  numberedCommand,
  sheetRows,
  parseChord,
  parseMacosChord,
  serializeChord,
  serializeMacosChord,
  storedKey,
} from "./shortcuts";

describe("shortcut registry", () => {
  it("claims no chord Chrome keeps for itself", () => {
    for (const command of REGISTRY) {
      expect(command.browser && isChromeReserved(command.browser, "mac"), command.id).toBeFalsy();
    }
  });

  it("marks exactly the ten moved chords", () => {
    const moved = REGISTRY.filter((command) => command.moved).map((command) => command.id).sort();
    // Chrome keeps ⇧⌘R (reload) and ⇧⌘B (bookmarks bar), so PRs and the sidebar lens move to ⌥⇧ in a tab (PRD sidebar-task-lens D-32).
    expect(moved).toEqual(["add_backlog", "close_pane", "close_tab", "new_tab", "previous_recent_area_tab", "recent_area_tab", "reopen_closed_tab", "settings", "toggle_lane_prs", "toggle_sidebar_lens"].sort());
  });

  it("adds to the backlog with ⇧⌘M in the desktop app and ⌥⇧M in a browser, where Chrome keeps ⇧⌘M (PRD overview-device-tasks B37)", () => {
    expect(displayCommand("add_backlog", "electron", REGISTRY, "mac")).toBe("⇧⌘M");
    expect(displayCommand("add_backlog", "browser", REGISTRY, "mac")).toBe("⌥⇧M");
    expect(isChromeReserved({ code: "KeyM", meta: true, shift: true }, "mac")).toBe(true);
    const pc = systemRegistry("pc");
    for (const host of ["browser", "electron"] as const) expect(displayCommand("add_backlog", host, pc, "pc"), host).toBe("Alt+Shift+M");
  });

  it("offers Add project only in the desktop app, which has the folder picker", () => {
    expect(displayCommand("new_workspace", "browser", REGISTRY, "mac")).toBe("");
    expect(displayCommand("new_workspace", "electron", REGISTRY, "mac")).toBe("⇧⌘N");
  });

  it("offers Start agent only in the desktop app, where ⌘N is not the browser's", () => {
    expect(displayCommand("start_agent", "browser", REGISTRY, "mac")).toBe("");
    expect(displayCommand("start_agent", "electron", REGISTRY, "mac")).toBe("⌘N");
  });

  it("gives the desktop app the macOS chords", () => {
    // The macOS chord set as the operator reads it; Keyboard shortcuts has no
    // macOS chord and keeps the browser's.
    const macosSet: Record<string, string> = {
      new_tab: "⌘T",
      close_tab: "⌘W",
      reopen_closed_tab: "⇧⌘T",
      recent_area_tab: "⌃⇥",
      recent_panel: "",
      previous_recent_panel: "",
      previous_recent_area_tab: "⌃⇧⇥",
      new_workspace: "⇧⌘N",
      start_agent: "⌘N",
      recent_project: "⌥⇥",
      previous_recent_project: "⌥⇧⇥",
      search: "⌘K",
      open_file: "⌘P",
      toggle_left_sidebar: "⌘B",
      toggle_sidebar_lens: "⇧⌘B",
      // ⌘E shows the Explorer in both hosts (issue 170); the macOS set keeps
      // it on the sidebar switch, which has no chord here until one is bound.
      overview: "⇧⌘O",
      add_backlog: "⇧⌘M",
      sidebar_projects: "⇧⌘P",
      toggle_device_rail: "",
      toggle_lane: "⌘E",
      toggle_lane_memory: "⇧⌘L",
      toggle_lane_prs: "⇧⌘R",
      toggle_lane_explorer: "⇧⌘E",
      toggle_lane_changes: "⇧⌘G",
      toggle_lane_documents: "",
      find_in_pane: "⌘F",
      save_file: "⌘S",
      keep_open: "⇧⌘K",
      split_right: "⌘D",
      split_down: "⇧⌘D",
      toggle_zoom: "⇧⌘↩",
      toggle_task_list: "⇧⌘U",
      close_pane: "⇧⌘W",
      text_larger: "⌘=",
      text_smaller: "⌘-",
      text_reset: "⌘0",
      move_to_trash: "⌘⌫",
      copy_path: "⌥⌘C",
      copy_relative_path: "⌥⇧⌘C",
      explorer_cut: "⌘X",
      explorer_copy: "⌘C",
      explorer_paste: "⌘V",
      settings: "⌘,",
      shortcuts: "⌘/",
      // The area commands have no chord until the operator binds one (PRD cmdk-navigation D-05).
      ...Object.fromEntries(AREA_COMMANDS.map((id) => [id, ""])),
      // ⌥n selects the nth Agents row and ⌘n the nth device (electron-digit-shortcuts-hints D-02).
      ...Object.fromEntries(DIGITS.map((digit) => [`select_agent_${digit}`, `⌥${digit}`])),
      ...Object.fromEntries(DIGITS.map((digit) => [`select_device_${digit}`, `⌘${digit}`])),
    };
    expect(Object.fromEntries(REGISTRY.map((command) => [command.id, displayCommand(command.id, "electron", REGISTRY, "mac")]))).toEqual(macosSet);
  });

  describe("numbered commands (electron-digit-shortcuts-hints D-02, B3, B4)", () => {
    it("holds nine of each family, on the desktop host only", () => {
      const numbered = REGISTRY.filter((command) => numberedCommand(command.id));
      expect(numbered).toHaveLength(18);
      for (const command of numbered) expect(command.browser, command.id).toBeNull();
      expect(numberedCommand("select_agent_9")).toEqual({ family: "agents", number: 9 });
      expect(numberedCommand("select_device_1")).toEqual({ family: "devices", number: 1 });
      expect(numberedCommand("new_tab")).toBeNull();
      expect(isNumberedCommand("select_device_1")).toBe(true);
      expect(isNumberedCommand("text_reset")).toBe(false);
    });

    it("answers ⌥n and ⌘n in the desktop app, leaves ⌃n to the terminal, and answers nothing in a browser", () => {
      expect(matchHost(press("Digit2", { ctrl: true }), REGISTRY, "electron")).toBeNull();
      expect(matchHost(press("Digit7", { alt: true }), REGISTRY, "electron")?.id).toBe("select_agent_7");
      expect(matchHost(press("Digit2", { meta: true }), REGISTRY, "electron")?.id).toBe("select_device_2");
      expect(matchHost(press("Digit2", { meta: true, shift: true }), REGISTRY, "electron")).toBeNull();
      expect(matchHost(press("Digit2", { meta: true }), REGISTRY, "browser")).toBeNull();
      expect(matchHost(press("Digit2", { ctrl: true }), REGISTRY, "browser")).toBeNull();
      expect(matchHost(press("Digit2", { alt: true }), REGISTRY, "browser")).toBeNull();
    });

    it("names each family's shared modifiers per host, for the hold hint", () => {
      expect(familyModifiers("agents", REGISTRY, "electron")).toEqual({ meta: false, alt: true, shift: false, ctrl: false });
      expect(familyModifiers("devices", REGISTRY, "electron")).toEqual({ meta: true, alt: false, shift: false, ctrl: false });
      expect(familyModifiers("devices", REGISTRY, "browser")).toBeNull();
      expect(familyModifiers("agents", REGISTRY, "browser")).toBeNull();
    });

    it("writes the held modifiers as each system presses them", () => {
      const agents = familyModifiers("agents", REGISTRY, "electron")!;
      expect(modifierLabel(agents, "mac")).toBe("⌥");
      expect(modifierLabel(familyModifiers("devices", REGISTRY, "electron")!, "mac")).toBe("⌘");
      const pcRegistry = systemRegistry("pc");
      expect(modifierLabel(familyModifiers("devices", pcRegistry, "electron")!, "pc")).toBe("Ctrl+Shift");
      expect(modifierLabel(familyModifiers("agents", pcRegistry, "electron")!, "pc")).toBe("Alt");
    });

    it("refuses a pane chord bound onto a numbered one by that command's name", () => {
      expect(bindingProblem("split_right", { code: "Digit3", meta: true }, REGISTRY, "electron", "mac")).toEqual({ code: "conflict", chord: "⌘3", commandId: "select_device_3" });
      expect(bindingProblem("split_right", { code: "Digit5", alt: true, meta: true }, REGISTRY, "electron", "mac")).toBeNull();
      // ⌥⌘1-4 are free since the lane tools took letters (PRD sidebar-task-lens B24).
      expect(bindingProblem("split_right", { code: "Digit3", alt: true, meta: true }, REGISTRY, "electron", "mac")).toBeNull();
      expect(effectiveRegistry({ split_right: "command+3" }, "electron", "mac").diagnostic).toContain("conflict select_device_3");
    });

    it("folds each family into one sheet row with its range, absent in a browser", () => {
      expect(sheetRows("Tabs", REGISTRY, "electron", "mac").map((row) => row.id)).toEqual(["new_tab", "close_tab", "reopen_closed_tab"]);
      expect(sheetRows("Navigate", REGISTRY, "electron", "mac").find((row) => row.id === "select_agent_1")).toMatchObject({ title: "Select agent 1-9", chord: "⌥1 … ⌥9" });
      expect(sheetRows("Navigate", REGISTRY, "electron", "mac").find((row) => row.id === "select_device_1")).toMatchObject({ title: "Select device 1-9", chord: "⌘1 … ⌘9" });
      expect(sheetRows("Navigate", REGISTRY, "browser", "mac").find((row) => row.id === "select_device_1")).toMatchObject({ chord: null });
      expect(sheetRows("Panes", REGISTRY, "browser", "mac").find((row) => row.id === "close_pane")).toMatchObject({ chord: "⌥⇧W", moved: true, movedFrom: "⇧⌘W" });
    });
  });

  it("binds every electron chord once", () => {
    const shown = REGISTRY.map((command) => displayCommand(command.id, "electron", REGISTRY, "mac")).filter(Boolean);
    expect(new Set(shown).size).toBe(shown.length);
  });

  it("answers the native chords in the desktop app and leaves the browser's alone", () => {
    const press = (code: string, mods: { meta?: boolean; alt?: boolean; shift?: boolean; ctrl?: boolean } = {}) => ({
      code,
      metaKey: !!mods.meta,
      altKey: !!mods.alt,
      shiftKey: !!mods.shift,
      ctrlKey: !!mods.ctrl,
    });
    expect(matchHost(press("KeyT", { meta: true }), REGISTRY, "electron")?.id).toBe("new_tab");
    expect(matchHost(press("KeyW", { meta: true }), REGISTRY, "electron")?.id).toBe("close_tab");
    expect(matchHost(press("KeyT", { meta: true, shift: true }), REGISTRY, "electron")?.id).toBe("reopen_closed_tab");
    expect(matchHost(press("Tab", { ctrl: true }), REGISTRY, "electron")?.id).toBe("recent_area_tab");
    expect(matchHost(press("KeyT", { alt: true }), REGISTRY, "electron")).toBeNull();
    expect(matchHost(press("Backspace", { meta: true }), REGISTRY, "electron")).toBeNull();
    // B12: the browser host still runs its own column.
    expect(matchHost(press("KeyT", { meta: true }), REGISTRY, "browser")).toBeNull();
    expect(matchHost(press("KeyT", { alt: true }), REGISTRY, "browser")?.id).toBe("new_tab");
  });

  it("runs each host's own stored set and never the other's", () => {
    const uiState = { browser_shortcut_bindings: { split_right: "alt+KeyR" }, shortcut_bindings: { toggle_zoom: "command+option+return" } };
    const browser = hostRegistry(uiState, "browser", "mac").registry;
    const desktop = hostRegistry(uiState, "electron", "mac").registry;
    expect(matchHost({ code: "KeyR", metaKey: false, altKey: true, shiftKey: false, ctrlKey: false }, browser, "browser")?.id).toBe("split_right");
    expect(matchHost({ code: "KeyR", metaKey: false, altKey: true, shiftKey: false, ctrlKey: false }, desktop, "electron")).toBeNull();
    expect(matchHost({ code: "KeyD", metaKey: true, altKey: false, shiftKey: false, ctrlKey: false }, desktop, "electron")?.id).toBe("split_right");
    expect(matchHost({ code: "Enter", metaKey: true, altKey: true, shiftKey: false, ctrlKey: false }, desktop, "electron")?.id).toBe("toggle_zoom");
    expect(matchHost({ code: "Enter", metaKey: true, altKey: true, shiftKey: false, ctrlKey: false }, browser, "browser")).toBeNull();
    expect(hostRegistry({}, "electron", "mac").registry).toBe(REGISTRY);
  });

  it("binds every browser chord once", () => {
    const seen: string[] = [];
    for (const command of REGISTRY) {
      if (!command.browser) continue;
      const shown = displayChord(command.browser, "mac");
      expect(seen, `${command.id} reuses ${shown}`).not.toContain(shown);
      seen.push(shown);
    }
  });

  it("matches on the physical key and every modifier", () => {
    expect(matchHost({ code: "KeyT", metaKey: false, altKey: true, shiftKey: false, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("new_tab");
    expect(matchHost({ code: "KeyT", metaKey: false, altKey: true, shiftKey: true, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("reopen_closed_tab");
    expect(matchHost({ code: "KeyT", metaKey: true, altKey: false, shiftKey: false, ctrlKey: false }, REGISTRY, "browser")).toBeNull();
    expect(matchHost({ code: "Backquote", metaKey: false, altKey: true, shiftKey: false, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("recent_area_tab");
    expect(matchHost({ code: "Slash", metaKey: true, altKey: false, shiftKey: false, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("shortcuts");
    expect(matchHost({ code: "Backspace", metaKey: true, altKey: false, shiftKey: false, ctrlKey: false }, REGISTRY, "browser")).toBeNull();
    expect(chordEquals({ code: "KeyA" }, { code: "KeyA", meta: false })).toBe(true);
  });

  it("renders chords in macOS modifier order", () => {
    expect(displayChord({ code: "Enter", meta: true, alt: true }, "mac")).toBe("⌥⌘↩");
    expect(displayChord({ code: "Backquote", alt: true, shift: true }, "mac")).toBe("⌥⇧`");
    expect(displayChord({ code: "Digit0", meta: true }, "mac")).toBe("⌘0");
    expect(displayChord({ code: "Tab", ctrl: true, shift: true }, "mac")).toBe("⌃⇧⇥");
  });
});

const press = (code: string, modifiers: { meta?: boolean; alt?: boolean; shift?: boolean; ctrl?: boolean } = {}) => ({
  code,
  metaKey: !!modifiers.meta,
  altKey: !!modifiers.alt,
  shiftKey: !!modifiers.shift,
  ctrlKey: !!modifiers.ctrl,
});

describe("browser pane chord overrides (S5 B9, B10)", () => {
  it("round-trips a chord through its stored form", () => {
    const chord = { code: "KeyR", alt: true, shift: true };
    expect(serializeChord(chord)).toBe("alt+shift+KeyR");
    expect(parseChord("alt+shift+KeyR")).toEqual(chord);
    expect(parseChord("alt+alt+KeyR")).toBeNull();
    expect(parseChord("hyper+KeyR")).toBeNull();
    expect(parseChord("alt+Process")).toBeNull();
  });

  it("runs and lists a stored override in place of the default", () => {
    const { registry, diagnostic } = effectiveRegistry({ split_right: "alt+KeyR" }, "browser", "mac");
    expect(diagnostic).toBeNull();
    expect(matchHost(press("KeyR", { alt: true }), registry, "browser")?.id).toBe("split_right");
    expect(matchHost(press("KeyD", { meta: true }), registry, "browser")).toBeNull();
    expect(matchHost(press("KeyD", { meta: true, shift: true }), registry, "browser")?.id).toBe("split_down");
  });

  it("refuses a chord with no command modifier, a Chrome chord, and another command's chord", () => {
    expect(bindingProblem("split_right", { code: "KeyR" }, REGISTRY, "browser", "mac")).toEqual({ code: "modifier", modifier: "browser_mac" });
    expect(bindingProblem("split_right", { code: "KeyR", shift: true }, REGISTRY, "browser", "mac")).toEqual({ code: "modifier", modifier: "browser_mac" });
    expect(bindingProblem("split_right", { code: "KeyW", meta: true }, REGISTRY, "browser", "mac")).toMatchObject({ code: "browser_reserved" });
    expect(bindingProblem("split_right", { code: "KeyT", alt: true }, REGISTRY, "browser", "mac")).toMatchObject({ code: "conflict", commandId: "new_tab" });
    expect(bindingProblem("split_right", { code: "KeyD", meta: true }, REGISTRY, "browser", "mac")).toBeNull();
  });

  it("falls back to the defaults as a whole when a stored map cannot run", () => {
    const unusable: Record<string, string>[] = [{ split_right: "KeyR" }, { split_right: "meta+KeyW" }, { split_right: "alt+KeyT" }, { toggle_conversation: "alt+KeyC" }];
    for (const stored of unusable) {
      const resolved = effectiveRegistry(stored, "browser", "mac");
      expect(resolved.registry, JSON.stringify(stored)).toBe(REGISTRY);
      expect(resolved.diagnostic).toMatch(/defaults are in use/);
    }
    // Two overrides that collide with each other are refused together.
    expect(effectiveRegistry({ split_right: "alt+KeyR", split_down: "alt+KeyR" }, "browser", "mac").registry).toBe(REGISTRY);
  });

  it("edits the pane commands, the cycle commands, the sidebar switch, the device rail toggle, the four lane tool toggles and the eight area commands", () => {
    expect([...EDITABLE_PANE_COMMANDS].sort()).toEqual(
      [
        "recent_area_tab", "previous_recent_area_tab", "recent_panel", "previous_recent_panel",
        "close_pane", "split_down", "split_right", "text_larger", "text_reset", "text_smaller", "toggle_device_rail", "overview", "sidebar_projects", "toggle_zoom", "toggle_task_list",
        "toggle_sidebar_lens", "toggle_lane_memory", "toggle_lane_prs", "toggle_lane_explorer", "toggle_lane_changes",
        "focus_next_agent_area", "focus_previous_agent_area", "grow_agent_area", "shrink_agent_area",
        "focus_next_view_area", "focus_previous_view_area", "grow_view_area", "shrink_view_area",
        "copy_path", "copy_relative_path",
      ].sort(),
    );
    for (const id of EDITABLE_PANE_COMMANDS) expect(REGISTRY.some((command) => command.id === id)).toBe(true);
  });

  it("leaves the area commands unbound on every host until the operator binds one (PRD cmdk-navigation B24)", () => {
    for (const id of AREA_COMMANDS) {
      for (const host of ["browser", "electron"] as const) expect(displayCommand(id, host, REGISTRY, "mac"), `${id} ${host}`).toBe("");
    }
    const bound = effectiveRegistry({ grow_view_area: "meta+alt+KeyG", focus_next_agent_area: "alt+KeyJ" }, "browser", "mac");
    expect(bound.diagnostic).toBeNull();
    expect(displayCommand("grow_view_area", "browser", bound.registry, "mac")).toBe("⌥⌘G");
    const desktop = effectiveRegistry({ shrink_agent_area: "command+option+h" }, "electron", "mac");
    expect(desktop.diagnostic).toBeNull();
    expect(displayCommand("shrink_agent_area", "electron", desktop.registry, "mac")).toBe("⌥⌘H");
  });

  it("ignores a saved Agents overview chord, a command that no longer exists, without losing the other saved chords", () => {
    for (const host of ["browser", "electron"] as const) {
      const resolved = effectiveRegistry({ sidebar_agents: host === "browser" ? "meta+shift+KeyA" : "command+shift+a", split_right: host === "browser" ? "meta+alt+KeyR" : "command+option+r" }, host, "mac");
      expect(resolved.diagnostic).toMatch(/retired/);
      expect(displayCommand("split_right", host, resolved.registry, "mac")).toBe("⌥⌘R");
      expect(displayCommand("add_backlog", host, resolved.registry, "mac")).toBe(host === "browser" ? "⌥⇧M" : "⇧⌘M");
    }
  });

  it("ignores the retired sidebar toggle without losing the other saved chords", () => {
    for (const host of ["browser", "electron"] as const) {
      const resolved = effectiveRegistry({ toggle_sidebar_view: "invalid", split_right: host === "browser" ? "meta+alt+KeyR" : "command+option+r" }, host, "mac");
      expect(resolved.diagnostic).toMatch(/retired/);
      expect(displayCommand("split_right", host, resolved.registry, "mac")).toBe("⌥⌘R");
      expect(displayCommand("sidebar_projects", host, resolved.registry, "mac")).toBe("⇧⌘P");
      expect(resolved.registry.some((command) => command.id as string === "toggle_sidebar_view")).toBe(false);
    }
  });
});

describe("the desktop app's macOS pane chords (user decision 2026-09-26)", () => {
  // The operator's native app state.json on the day of the decision.
  const operatorSet = {
    close_pane: "command+shift+w",
    split_down: "command+shift+d",
    split_right: "command+d",
    toggle_zoom: "command+shift+return",
    increase_text_size: "command+=",
    decrease_text_size: "command+-",
    reset_text_size: "command+0",
  };

  it("reads the macOS set's chord text leniently", () => {
    expect(parseMacosChord("command+shift+return")).toEqual({ code: "Enter", meta: true, shift: true });
    expect(parseMacosChord("Cmd + Opt + Enter")).toEqual({ code: "Enter", meta: true, alt: true });
    expect(parseMacosChord("command+=")).toEqual({ code: "Equal", meta: true });
    expect(parseMacosChord("command+command+d")).toBeNull();
    expect(parseMacosChord("d")).toBeNull();
    expect(parseMacosChord("control+tab")).toEqual({ code: "Tab", ctrl: true });
    expect(bindingProblem("recent_area_tab", { code: "Tab", meta: true }, REGISTRY, "electron", "mac")).toMatchObject({ code: "mac_reserved" });
    expect(serializeMacosChord({ code: "Enter", meta: true, shift: true, alt: true, ctrl: true })).toBe("command+control+option+shift+return");
    expect(serializeMacosChord({ code: "ArrowUp", meta: true })).toBeNull();
    expect(storedKey("text_larger", "electron")).toBe("increase_text_size");
    expect(storedKey("text_larger", "browser")).toBe("text_larger");
  });

  it("runs the operator's set: ⇧⌘↩ zooms and ⌥⌘↩ no longer does", () => {
    const { registry, diagnostic } = effectiveRegistry(operatorSet, "electron", "mac");
    expect(diagnostic).toBeNull();
    expect(displayCommand("toggle_zoom", "electron", registry, "mac")).toBe("⇧⌘↩");
    expect(matchHost(press("Enter", { meta: true, shift: true }), registry, "electron")?.id).toBe("toggle_zoom");
    expect(matchHost(press("Enter", { meta: true, alt: true }), registry, "electron")).toBeNull();
    expect(displayCommand("text_larger", "electron", registry, "mac")).toBe("⌘=");
    // The browser column is untouched by the macOS set.
    expect(registry.find((command) => command.id === "toggle_zoom")?.browser).toEqual({ code: "Enter", meta: true, shift: true });
  });

  it("keeps the surfaceless Toggle Conversation in the set without running it", () => {
    const { registry, diagnostic } = effectiveRegistry({ toggle_conversation: "command+option+c", split_right: "command+option+r" }, "electron", "mac");
    expect(diagnostic).toBeNull();
    expect(matchHost(press("KeyR", { meta: true, alt: true }), registry, "electron")?.id).toBe("split_right");
  });

  it("lets two commands trade chords in one set", () => {
    const { diagnostic } = effectiveRegistry({ split_right: "command+shift+d", split_down: "command+d" }, "electron", "mac");
    expect(diagnostic).toBeNull();
  });

  it("refuses a chord without ⌘ or a reserved one, and falls back to the defaults as a whole", () => {
    expect(bindingProblem("split_right", { code: "KeyR", alt: true }, REGISTRY, "electron", "mac")).toEqual({ code: "modifier", modifier: "desktop_mac" });
    expect(bindingProblem("split_right", { code: "KeyQ", meta: true }, REGISTRY, "electron", "mac")).toMatchObject({ code: "mac_reserved" });
    expect(bindingProblem("split_right", { code: "ArrowUp", meta: true }, REGISTRY, "electron", "mac")).toEqual({ code: "native_key" });
    expect(bindingProblem("split_right", { code: "KeyT", meta: true }, REGISTRY, "electron", "mac")).toMatchObject({ code: "conflict", commandId: "new_tab" });
    // A chord Chrome keeps is the desktop app's to use.
    expect(bindingProblem("close_pane", { code: "KeyW", meta: true, shift: true }, REGISTRY, "electron", "mac")).toBeNull();
    const unusable: Record<string, string>[] = [{ split_right: "command+t" }, { split_right: "option+r" }, { zoom: "command+z" }, { split_right: "command+x", split_down: "command+x" }];
    for (const stored of unusable) {
      const resolved = effectiveRegistry(stored, "electron", "mac");
      expect(resolved.registry, JSON.stringify(stored)).toBe(REGISTRY);
      expect(resolved.diagnostic).toMatch(/defaults are in use/);
    }
  });
});

describe("Windows and Linux (operator decision 2026-10-03)", () => {
  const pc = systemRegistry("pc");
  const pcPress = (code: string, modifiers: { alt?: boolean; shift?: boolean; ctrl?: boolean; meta?: boolean } = {}) => press(code, modifiers);

  it("tells macOS from Windows and Linux by every platform name a host reports", () => {
    for (const name of ["darwin", "MacIntel", "macOS", "iPhone", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"]) expect(keySystemOf(name), name).toBe("mac");
    for (const name of ["win32", "Win32", "Windows", "linux", "Linux x86_64", "Mozilla/5.0 (X11; Linux x86_64)"]) expect(keySystemOf(name), name).toBe("pc");
  });

  it("gives the desktop app ⌘ as Ctrl+Shift, ⇧⌘ and ⌥⌘ as Alt+Shift, and the listed exceptions", () => {
    // The table docs/UI_BEHAVIOR.md (Keyboard shortcuts per system) prints.
    const pcSet: Record<string, string> = {
      new_tab: "Ctrl+Shift+T",
      close_tab: "Ctrl+Shift+W",
      reopen_closed_tab: "Alt+Shift+T",
      recent_area_tab: "Ctrl+Tab",
      previous_recent_area_tab: "Ctrl+Shift+Tab",
      recent_panel: "",
      previous_recent_panel: "",
      new_workspace: "Alt+Shift+N",
      start_agent: "Ctrl+Shift+N",
      recent_project: "Ctrl+Shift+`",
      previous_recent_project: "Ctrl+Alt+Shift+`",
      search: "Ctrl+Shift+K",
      open_file: "Ctrl+Shift+P",
      toggle_left_sidebar: "Ctrl+Shift+B",
      toggle_sidebar_lens: "Alt+Shift+B",
      overview: "Alt+Shift+O",
      add_backlog: "Alt+Shift+M",
      sidebar_projects: "Alt+Shift+P",
      toggle_device_rail: "",
      toggle_lane: "Ctrl+Shift+E",
      toggle_lane_memory: "Alt+Shift+L",
      toggle_lane_prs: "Alt+Shift+R",
      toggle_lane_explorer: "Alt+Shift+E",
      toggle_lane_changes: "Alt+Shift+G",
      toggle_lane_documents: "",
      find_in_pane: "Ctrl+Shift+F",
      save_file: "Ctrl+Shift+S",
      keep_open: "Alt+Shift+K",
      split_right: "Ctrl+Shift+D",
      split_down: "Alt+Shift+D",
      toggle_zoom: "Alt+Shift+Enter",
      toggle_task_list: "Alt+Shift+U",
      close_pane: "Alt+Shift+W",
      text_larger: "Ctrl+=",
      text_smaller: "Ctrl+-",
      text_reset: "Ctrl+0",
      move_to_trash: "Delete",
      copy_path: "Alt+Shift+C",
      copy_relative_path: "Alt+Shift+H",
      explorer_cut: "Ctrl+X",
      explorer_copy: "Ctrl+C",
      explorer_paste: "Ctrl+V",
      settings: "Ctrl+Shift+,",
      shortcuts: "Ctrl+Shift+/",
      ...Object.fromEntries(AREA_COMMANDS.map((id) => [id, ""])),
      ...Object.fromEntries(DIGITS.map((digit) => [`select_agent_${digit}`, `Alt+${digit}`])),
      ...Object.fromEntries(DIGITS.map((digit) => [`select_device_${digit}`, `Ctrl+Shift+${digit}`])),
    };
    expect(Object.fromEntries(pc.map((command) => [command.id, displayCommand(command.id, "electron", pc, "pc")]))).toEqual(pcSet);
  });

  it("leaves every plain Ctrl key to the terminal but text size, Ctrl+Tab and the tree's own clipboard", () => {
    for (const host of ["browser", "electron"] as const) {
      // A command with a passthrough is answered by the Explorer alone while it holds the keyboard, so a terminal still gets its Ctrl+C, X and V.
      const plainCtrl = pc
        .filter((command) => !command.passthrough)
        .map((command) => (host === "electron" ? command.electron : command.browser))
        .filter((chord) => chord && chord.ctrl && !chord.shift && !chord.alt && !chord.meta)
        .map((chord) => chord!.code);
      expect(new Set(plainCtrl), host).toEqual(new Set(host === "electron" ? ["Equal", "Minus", "Digit0", "Tab"] : ["Equal", "Minus", "Digit0"]));
    }
    expect(pc.some((command) => command.electron?.meta || command.browser?.meta)).toBe(false);
  });

  it("presses no chord with more than three keys but the ones that Chrome or a held cycle forces (operator decision 2026-10-03)", () => {
    const heavy = pc.flatMap((command) =>
      (["browser", "electron"] as const)
        .filter((host) => { const chord = host === "electron" ? command.electron : command.browser; return chord && [chord.ctrl, chord.alt, chord.shift].filter(Boolean).length === 3; })
        .map((host) => `${command.id} ${host}`),
    );
    expect(heavy.sort()).toEqual(["previous_recent_project browser", "previous_recent_project electron", "reopen_closed_tab browser", "toggle_sidebar_lens browser"]);
  });

  it("binds every chord once on each host, and claims none that Chrome or the system keeps", () => {
    for (const host of ["browser", "electron"] as const) {
      const shown = pc.map((command) => displayCommand(command.id, host, pc, "pc")).filter(Boolean);
      expect(new Set(shown).size, host).toBe(shown.length);
    }
    for (const command of pc) expect(command.browser && isChromeReserved(command.browser, "pc"), command.id).toBeFalsy();
    expect(isChromeReserved({ code: "KeyT", ctrl: true, shift: true }, "pc")).toBe(true);
    expect(isChromeReserved({ code: "Tab", alt: true }, "pc")).toBe(true);
    expect(isChromeReserved({ code: "KeyT", alt: true, shift: true }, "pc")).toBe(true);
  });

  it("notes a browser chord as moved only where Chrome keeps the desktop chord on this system", () => {
    expect(pc.filter((command) => command.moved).map((command) => command.id).sort()).toEqual(["close_tab", "new_tab", "previous_recent_area_tab", "recent_area_tab", "reopen_closed_tab", "toggle_sidebar_lens"]);
    expect(sheetRows("Tabs", pc, "browser", "pc").find((row) => row.id === "new_tab")).toMatchObject({ chord: "Alt+T", moved: true, movedFrom: "Ctrl+Shift+T" });
    // Chrome keeps Alt+Shift+T for its toolbar, and the macOS ⌥⇧T is those keys.
    expect(sheetRows("Tabs", pc, "browser", "pc").find((row) => row.id === "reopen_closed_tab")).toMatchObject({ chord: "Ctrl+Alt+Shift+T", moved: true, movedFrom: "Alt+Shift+T" });
    // Chrome keeps Alt+Shift+B for its bookmarks bar, and the macOS ⌥⇧B is those keys (PRD sidebar-task-lens B25).
    expect(sheetRows("Panels", pc, "browser", "pc").find((row) => row.id === "toggle_sidebar_lens")).toMatchObject({ chord: "Ctrl+Alt+Shift+B", moved: true, movedFrom: "Alt+Shift+B" });
    expect(matchHost(pcPress("KeyB", { alt: true, shift: true }), pc, "browser")).toBeNull();
    expect(matchHost(pcPress("KeyB", { ctrl: true, alt: true, shift: true }), pc, "browser")?.id).toBe("toggle_sidebar_lens");
    expect(matchHost(pcPress("KeyB", { alt: true, shift: true }), pc, "electron")?.id).toBe("toggle_sidebar_lens");
    // Every other browser chord is the desktop chord.
    const differs = pc.filter((command) => command.browser && command.electron && !command.moved && displayChord(command.browser, "pc") !== displayChord(command.electron, "pc"));
    expect(differs.map((command) => command.id)).toEqual([]);
    expect(sheetRows("Navigate", pc, "electron", "pc").find((row) => row.id === "select_device_1")).toMatchObject({ chord: "Ctrl+Shift+1 … Ctrl+Shift+9" });
  });

  it("answers the Ctrl+Shift chords and lets Ctrl, the Windows key and AltGr-free chords through", () => {
    expect(matchHost(pcPress("KeyT", { ctrl: true, shift: true }), pc, "electron")?.id).toBe("new_tab");
    expect(matchHost(pcPress("KeyT", { alt: true, shift: true }), pc, "electron")?.id).toBe("reopen_closed_tab");
    expect(matchHost(pcPress("KeyT", { ctrl: true, shift: true, alt: true }), pc, "electron")).toBeNull();
    expect(matchHost(pcPress("KeyT", { ctrl: true, shift: true, alt: true }), pc, "browser")?.id).toBe("reopen_closed_tab");
    expect(matchHost(pcPress("Backquote", { ctrl: true, shift: true, alt: true }), pc, "electron")?.id).toBe("previous_recent_project");
    expect(matchHost(pcPress("Digit3", { ctrl: true, shift: true }), pc, "electron")?.id).toBe("select_device_3");
    expect(matchHost(pcPress("Digit3", { ctrl: true }), pc, "electron")).toBeNull();
    expect(matchHost(pcPress("Backquote", { ctrl: true, shift: true }), pc, "electron")?.id).toBe("recent_project");
    expect(matchHost(pcPress("KeyK", { ctrl: true, shift: true }), pc, "browser")?.id).toBe("search");
    for (const code of ["KeyC", "KeyR", "KeyW", "KeyK", "KeyT", "KeyD"]) expect(matchHost(pcPress(code, { ctrl: true }), pc, "electron"), code).toBeNull();
    expect(matchHost(pcPress("KeyT", { meta: true }), pc, "electron")).toBeNull();
    expect(matchHost(pcPress("Delete"), pc, "electron")).toBeNull();
  });

  it("opens Memory, PRs and Changes with ⇧⌘L/R/G and turns the sidebar lens with ⇧⌘B on a Mac, Alt+Shift elsewhere, ⌥⇧R and ⌥⇧B in a Mac browser tab, and leaves ⌥⌘1-4 unbound (PRD sidebar-task-lens B24, B25)", () => {
    const letters = { toggle_lane_memory: "KeyL", toggle_lane_prs: "KeyR", toggle_lane_changes: "KeyG", toggle_sidebar_lens: "KeyB" } as const;
    for (const [id, code] of Object.entries(letters)) {
      expect(matchHost({ code, metaKey: true, altKey: false, shiftKey: true, ctrlKey: false }, REGISTRY, "electron")?.id, `electron mac ${id}`).toBe(id);
      for (const host of ["electron", "browser"] as const) {
        // Chrome keeps Alt+Shift+B in a Windows or Linux browser tab; that one is Ctrl+Alt+Shift+B (below).
        if (host === "browser" && id === "toggle_sidebar_lens") continue;
        expect(matchHost(pcPress(code, { alt: true, shift: true }), pc, host)?.id, `${host} pc ${id}`).toBe(id);
      }
    }
    expect(matchHost({ code: "KeyR", metaKey: false, altKey: true, shiftKey: true, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("toggle_lane_prs");
    expect(matchHost({ code: "KeyB", metaKey: false, altKey: true, shiftKey: true, ctrlKey: false }, REGISTRY, "browser")?.id).toBe("toggle_sidebar_lens");
    for (const command of pc.filter((row) => row.id in letters)) expect(command.browser && isChromeReserved(command.browser, "pc"), command.id).toBeFalsy();
    for (const host of ["electron", "browser"] as const) {
      for (const index of [1, 2, 3, 4]) expect(matchHost({ code: `Digit${index}`, metaKey: true, altKey: true, shiftKey: false, ctrlKey: false }, REGISTRY, host), `${host} ⌥⌘${index}`).toBeNull();
    }
    // Alt+n still selects the nth agent, and Ctrl+Shift+n the nth device.
    expect(matchHost(pcPress("Digit1", { alt: true }), pc, "electron")?.id).toBe("select_agent_1");
    expect(matchHost(pcPress("Digit1", { ctrl: true, shift: true }), pc, "electron")?.id).toBe("select_device_1");
  });

  it("reads a stored lane tool chord on both systems, drops a stored Work tool chord without refusing the set, and refuses a chord a lane tool holds", () => {
    const stored = { toggle_lane_memory: "command+option+9" };
    expect(displayCommand("toggle_lane_memory", "electron", effectiveRegistry(stored, "electron", "mac").registry, "mac")).toBe("⌥⌘9");
    expect(displayCommand("toggle_lane_memory", "electron", effectiveRegistry(stored, "electron", "pc").registry, "pc")).toBe("Alt+Shift+9");
    const retired = effectiveRegistry({ toggle_lane_work: "command+option+1", split_right: "command+option+1" }, "electron", "mac");
    expect(retired.registry.find((command) => command.id === "split_right")?.electron).toEqual({ code: "Digit1", meta: true, alt: true });
    expect(effectiveRegistry({ split_right: "command+shift+l" }, "electron", "mac").diagnostic).toContain("conflict toggle_lane_memory");
  });

  it("stores a chord as the macOS chord it stands for, so one set reads the same on both systems", () => {
    const pressed = { code: "KeyR", alt: true, shift: true };
    expect(macChord(pressed, "pc")).toEqual({ code: "KeyR", meta: true, shift: true });
    expect(serializeStoredChord(pressed, "electron", "pc")).toBe("command+shift+r");
    expect(serializeStoredChord(pressed, "browser", "pc")).toBe("shift+meta+KeyR");
    expect(parseStoredChord("command+shift+r", "electron", "pc")).toEqual(pressed);
    expect(parseStoredChord("command+shift+r", "electron", "mac")).toEqual({ code: "KeyR", meta: true, shift: true });
    expect(serializeStoredChord({ code: "Tab", ctrl: true }, "electron", "pc")).toBe("control+tab");
    expect(serializeStoredChord({ code: "KeyR", meta: true, ctrl: true }, "electron", "pc")).toBeNull();
    expect(modChord({ code: "Enter", meta: true, alt: true }, "pc")).toEqual({ code: "Enter", alt: true, shift: true });
    expect(macChord({ code: "KeyR", ctrl: true, shift: true }, "pc")).toEqual({ code: "KeyR", meta: true });
    expect(macChord({ code: "Backquote", ctrl: true, alt: true, shift: true }, "pc")).toEqual({ code: "Backquote", ctrl: true, alt: true, shift: true });
    expect(modChord({ code: "Tab", alt: true }, "pc")).toEqual({ code: "Tab", alt: true });
    // The operator's macOS set (2026-09-26) runs on Windows as the same keys through the rule.
    const { registry, diagnostic } = effectiveRegistry({ toggle_zoom: "command+shift+return", split_right: "command+d" }, "electron", "pc");
    expect(diagnostic).toBeNull();
    expect(displayCommand("toggle_zoom", "electron", registry, "pc")).toBe("Alt+Shift+Enter");
    expect(matchHost(pcPress("Enter", { alt: true, shift: true }), registry, "electron")?.id).toBe("toggle_zoom");
  });

  it("refuses a desktop chord without Ctrl+Shift or Alt+Shift, one the system keeps, the Windows key, and a taken one", () => {
    expect(bindingProblem("split_right", { code: "KeyR", ctrl: true }, pc, "electron", "pc")).toEqual({ code: "modifier", modifier: "desktop_pc" });
    expect(bindingProblem("split_right", { code: "KeyR", alt: true }, pc, "electron", "pc")).toMatchObject({ code: "modifier" });
    expect(bindingProblem("split_right", { code: "KeyY", alt: true, shift: true }, pc, "electron", "pc")).toBeNull();
    expect(bindingProblem("split_right", { code: "KeyR", alt: true, shift: true }, pc, "electron", "pc")).toEqual({ code: "conflict", chord: "Alt+Shift+R", commandId: "toggle_lane_prs" });
    // Ctrl+Alt+Shift stands for no macOS chord, so a set holding it would be refused on a Mac.
    expect(bindingProblem("split_right", { code: "KeyR", ctrl: true, alt: true, shift: true }, pc, "electron", "pc")).toMatchObject({ code: "modifier" });
    expect(bindingProblem("split_right", { code: "KeyD", alt: true, shift: true }, pc, "electron", "pc")).toEqual({ code: "conflict", chord: "Alt+Shift+D", commandId: "split_down" });
    expect(bindingProblem("split_right", { code: "KeyT", alt: true, shift: true }, pc, "browser", "pc")).toEqual({ code: "browser_reserved", chord: "Alt+Shift+T", macos: false });
    expect(bindingProblem("recent_area_tab", { code: "Tab", alt: true }, pc, "electron", "pc")).toEqual({ code: "system_reserved", chord: "Alt+Tab" });
    expect(bindingProblem("split_right", { code: "KeyR", meta: true }, pc, "electron", "pc")).toEqual({ code: "system_key" });
    expect(bindingProblem("split_right", { code: "KeyT", ctrl: true, shift: true }, pc, "electron", "pc")).toEqual({ code: "conflict", chord: "Ctrl+Shift+T", commandId: "new_tab" });
    expect(bindingProblem("split_right", { code: "KeyR", ctrl: true, shift: true }, pc, "electron", "pc")).toBeNull();
    expect(bindingProblem("split_right", { code: "KeyC", ctrl: true, shift: true }, pc, "electron", "pc")).toEqual({ code: "terminal_copy", chord: "Ctrl+Shift+C" });
    expect(bindingProblem("split_right", { code: "KeyV", ctrl: true, shift: true }, pc, "browser", "pc")).toEqual({ code: "terminal_paste", chord: "Ctrl+Shift+V" });
    expect(bindingProblem("recent_area_tab", { code: "KeyJ", ctrl: true }, pc, "electron", "pc")).toBeNull();
    expect(bindingProblem("split_right", { code: "KeyT", ctrl: true }, pc, "browser", "pc")).toEqual({ code: "browser_reserved", chord: "Ctrl+T", macos: false });
    expect(bindingProblem("split_right", { code: "KeyR", shift: true }, pc, "browser", "pc")).toEqual({ code: "modifier", modifier: "browser_pc" });
  });

  it("writes chords in words joined with +, in Windows' order Ctrl, Alt, Shift", () => {
    expect(displayChord({ code: "Enter", meta: true, alt: true }, "mac")).toBe("⌥⌘↩");
    expect(displayChord({ code: "Enter", ctrl: true, shift: true, alt: true }, "pc")).toBe("Ctrl+Alt+Shift+Enter");
    expect(displayChord({ code: "KeyD", shift: true, alt: true }, "pc")).toBe("Alt+Shift+D");
    expect(displayChord({ code: "ArrowUp", alt: true }, "pc")).toBe("Alt+Up");
    expect(displayChord({ code: "Slash", ctrl: true, shift: true }, "pc")).toBe("Ctrl+Shift+/");
    expect(displayChord({ code: "Delete" }, "pc")).toBe("Delete");
  });
});


// Unbinding a navigation key preserves the rest of the host registry.
describe("cleared navigation keys", () => {
  it("keeps unrelated overrides while removing every new navigation chord", () => {
    for (const host of ["browser", "electron"] as const) {
      const result = effectiveRegistry({ overview: "none", sidebar_projects: "none", split_right: host === "browser" ? "meta+alt+KeyR" : "command+option+r" }, host, "mac");
      expect(result.diagnostic).toBeNull();
      for (const id of ["overview", "sidebar_projects"]) expect(result.registry.find(row => row.id === id)?.[host]).toBeNull();
      expect(result.registry.find(row => row.id === "split_right")?.[host]?.code).toBe("KeyR");
    }
  });
});
