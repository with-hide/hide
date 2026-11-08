import { emptyScope } from "../test/legacyAgentScope";
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import tokensText from "../../design/tokens.json?raw";
import { initializeInterfaceI18n } from "./i18n/instance";
import {
  ACCENT_CHOICES,
  canRetryDevice,
  deviceSubtitle as deviceSubtitleIn,
  deviceAwayLine as deviceAwayLineIn,
  deviceProblemLine as deviceProblemLineIn,
  retryCanHelp,
  retrySignsIn,
  deviceRemovalLines as deviceRemovalLinesIn,
  deviceLine as deviceLineIn,
  diagnosticsText,
  kitConsentTerms as kitConsentTermsIn,
  kitRemovalLine as kitRemovalLineIn,
  engineLine as engineLineIn,
  hostLine as hostLineIn,
  kitPartLine as kitPartLineIn,
  kitPartNeedsReinstall,
  kitPartText as kitPartTextIn,
  kitProblems,
  shownIn,
  sleepAfterLabel,
  SLEEP_AFTER_CHOICES,
  usableAccent,
  usableFontSize,
  usableTerminalFontFamily,
  usableTerminalFontSize,
  unstoredDeviceDrafts,
} from "./settings";
import { redact } from "./redact";
import type { Device, DeviceHost, DeviceProblem, Kit, KitComponent, RemoteStatus } from "./snapshot";

// The English strings are what the sheet shipped with; the rules read the same under them.
const t = initializeInterfaceI18n("en").getFixedT(null, "translation");
const korean = initializeInterfaceI18n("ko").getFixedT(null, "translation");
const deviceSubtitle = (device: Device) => deviceSubtitleIn(device, t);
const deviceAwayLine = (patch: Partial<Device>, name = "Mac mini") => deviceAwayLineIn(device(patch), name, t);
const deviceProblemLine = (problem: DeviceProblem | null | undefined, alias: string | null, platform?: string) => deviceProblemLineIn(problem, alias, t, platform);
const deviceRemovalLines = (deviceId: string, registrations: Parameters<typeof deviceRemovalLinesIn>[1], tabs: Parameters<typeof deviceRemovalLinesIn>[2], drafts: Parameters<typeof deviceRemovalLinesIn>[3], onlyExported: (tabId: string) => boolean = () => false) =>
  deviceRemovalLinesIn(deviceId, registrations, tabs, drafts, onlyExported, t);
const deviceLine = (device: Device, remote: RemoteStatus | undefined) => deviceLineIn(device, remote, t);
const kitConsentTerms = (helperRoot: string | null, cliDir: string | null) => kitConsentTermsIn(helperRoot, cliDir, t);
const kitRemovalLine = (device: Device) => kitRemovalLineIn(device, t);
const engineLine = (engine: Parameters<typeof engineLineIn>[0]) => engineLineIn(engine, t);
const hostLine = (host: DeviceHost | undefined) => hostLineIn(host, t);
const kitPartLine = (part: KitComponent) => kitPartLineIn(part, t);
const kitPartText = (part: Pick<KitComponent, "state" | "why">) => kitPartTextIn(part, t);

const device = (patch: Partial<Device>): Device => ({ agent_scope: emptyScope(),
  id: "studio",
  label: "Studio",
  kind: "remote",
  state: "unavailable",
  message: null,
  ssh_alias: "studio",
  agent_count: 0,
  test: null,
  ...patch,
});

describe("where settings live and what a device reported", () => {
  it("names a device by its alias and its platform, and only as far as the device reported them", () => {
    expect(deviceSubtitle(device({}))).toBe("studio");
    const host = { state: "ready", platform: "macos aarch64" } as DeviceHost;
    expect(deviceSubtitle(device({ host }))).toBe("studio · macos aarch64");
    expect(deviceSubtitle(device({ host: { ...host, state: "connecting" } }))).toBe("studio");
    expect(deviceSubtitle(device({ kind: "local", host }))).toBe("local, no SSH alias");
    // PRD wsl-device B5: a WSL distribution is named where it is, the core's PC, in place of its Linux platform.
    const wsl = device({ id: "wsl-ubuntu-1a2b3c4d", label: "WSL Ubuntu", ssh_alias: null, wsl_distro: "Ubuntu", host: { ...host, platform: "linux x86_64" } });
    expect(deviceSubtitleIn(wsl, t, "This PC")).toBe("WSL Ubuntu · This PC");
  });

  it("draws every ssh failure as one of three states: sign in, the server key, or cannot connect (operator decision 2026-10-30)", () => {
    const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8")) as { device_problem: DeviceProblem[] };
    const headlines = new Set(contract.device_problem.map((problem) => deviceProblemLine(problem, "studio")?.headline));
    expect(headlines).toEqual(new Set(["Sign-in needed", "Sign-in refused", "Server key changed", "Couldn't connect"]));
    expect(contract.device_problem.filter(retrySignsIn)).toEqual(["sign_in_needed", "sign_in_refused"]);
    expect(deviceProblemLine("host_key_changed", "studio")).toEqual({ headline: "Server key changed", note: null, command: "ssh studio" });
    // ssh's last line is said under it, so the state adds no sentence of its own.
    expect(deviceProblemLine("cannot_connect", "studio")).toEqual({ headline: "Couldn't connect", note: null, command: null });
    expect(deviceProblemLine("openssh_old", "studio")?.note).toContain("8.4");
    expect(deviceProblemLine(null, "studio")).toBeNull();
  });

  it("says a Hide an update replaced connects once it is opened again, and offers no retry from it", () => {
    expect(deviceProblemLine("hide_updated", "studio")).toEqual({
      headline: "Couldn't connect",
      note: "Hide was updated after it started. Quit Hide and open it again to connect.",
      command: null,
    });
    expect(retryCanHelp("hide_updated")).toBe(false);
    expect(retryCanHelp("host_key_changed")).toBe(false);
    expect(retryCanHelp("cannot_connect")).toBe(true);
  });

  it("says how to install ssh on the system the core runs on (B20)", () => {
    expect(deviceProblemLine("ssh_missing", "studio", "windows x86_64")?.note).toContain("Optional features");
    expect(deviceProblemLine("ssh_missing", "studio", "linux x86_64")?.note).toContain("openssh-client");
    expect(deviceProblemLine("ssh_missing", "studio", "macos aarch64")?.note).toBe("This computer has no ssh. Install the OpenSSH client.");
  });
});

describe("what a screen says while a device is away", () => {
  it("names the machine in plain words and keeps the SSH diagnostic out of the sentence", () => {
    const away = deviceAwayLine({ state: "unavailable", message: "remote Herdr socket open failed for ssh:hq3mini: SSH operation timed out" });
    expect(away.text).toBe("Mac mini is not connected. Showing the last state it reported; nothing is sent until it reconnects.");
    expect(away.hint).toBeNull();
    expect(away.text).not.toMatch(/ssh|stage=|timed out/i);
  });

  it("says a refused sign-in or a changed server key by its state, with the terminal check in the hint", () => {
    const away = deviceAwayLine({ state: "unavailable", problem: "sign_in_refused", ssh_alias: "mini", message: "Permission denied (publickey,password)." });
    expect(away).toEqual({ text: "Mac mini is not connected: Sign-in refused. Showing the last state it reported; nothing is sent until it reconnects.", hint: null, toRow: false });
    expect(deviceAwayLine({ state: "unavailable", problem: "host_key_changed", ssh_alias: "mini" }).hint).toBe("Check in a terminal: ssh mini");
    const lost = deviceAwayLine({ state: "unavailable", problem: "cannot_connect", ssh_alias: "mini", message: "ssh: connect to host mini port 22: Operation timed out" });
    expect(lost.text).toBe("Mac mini is not connected: Couldn't connect. Showing the last state it reported; nothing is sent until it reconnects.");
    expect(lost.hint).toBeNull();
  });

  it("leads a device waiting on a sign-in to its row, where it is answered (B15)", () => {
    const asking = deviceAwayLine({ state: "unavailable", ssh_alias: "mini", sign_in: { question: 3, key_command: null, kind: "secret", prompt: "alice@mini's password:" } });
    expect(asking).toEqual({ text: "Mac mini is not connected: waiting for you to sign in. Showing the last state it reported; nothing is sent until it reconnects.", hint: "Answer on its row in Settings › Devices.", toRow: true });
    expect(deviceAwayLine({ state: "unavailable", problem: "sign_in_needed" }).toRow).toBe(true);
  });
});

describe("deviceRemovalLines", () => {
  it("counts the device's projects, tabs and drafts and nothing of this machine's", () => {
    const lines = deviceRemovalLines(
      "mini",
      [{ device_id: "mini" }, { device_id: "local" }],
      [
        { id: "t1", device_id: "mini", dirty: true },
        { id: "t2", device_id: "mini", dirty: false },
        { id: "t3", device_id: "local", dirty: true },
      ],
      [{ device: "mini" }, { device: "local" }, { device: null }],
    );
    expect(lines).toEqual([
      "Hide forgets 1 registered project and closes 2 file tabs from it here.",
      "2 unsaved drafts stay in this browser under unsaved drafts, to export or discard.",
    ]);
    expect(deviceRemovalLines("studio", [], [], [])).toEqual([]);
  });

  it("says a draft that was only exported leaves as that file, not as a stored draft (B44)", () => {
    const tabs = [
      { id: "t1", device_id: "mini", dirty: true },
      { id: "t2", device_id: "mini", dirty: true },
    ];
    expect(deviceRemovalLines("mini", [], tabs, [], (tabId) => tabId === "t1")).toEqual([
      "Hide forgets 0 registered projects and closes 2 file tabs from it here.",
      "1 unsaved draft stays in this browser under unsaved drafts, to export or discard.",
      "1 draft could not be stored in this browser and leaves only as the file you exported.",
    ]);
  });
});

describe("settings rules", () => {
  it("keeps only accents and sizes the sheet can offer", () => {
    // Every offered accent is a token the design authority defines, and each
    // one's value is an accent the page can store and draw.
    const tokens = (JSON.parse(tokensText) as { tokens: Record<string, { value: string }> }).tokens;
    for (const choice of ACCENT_CHOICES) {
      const value = tokens[choice.token]?.value;
      expect(value, choice.token).toBeDefined();
      expect(usableAccent(value)).toBe(value?.toLowerCase());
    }
    expect(usableAccent("red")).toBeNull();
    expect(usableAccent("#GGGGGG")).toBeNull();
    expect(usableFontSize(13)).toBe(13);
    expect(usableFontSize(10)).toBeNull();
    expect(usableFontSize(18)).toBeNull();
    expect(usableFontSize("13")).toBeNull();
    expect(usableTerminalFontSize(14)).toBe(14);
    expect(usableTerminalFontSize(9)).toBeNull();
    expect(usableTerminalFontSize(25)).toBeNull();
    expect(usableTerminalFontFamily("JetBrains Mono")).toBe("JetBrains Mono");
    expect(usableTerminalFontFamily(" Menlo")).toBe("");
    expect(usableTerminalFontFamily("Mono\nFont")).toBe("");
    expect(usableTerminalFontFamily("M".repeat(129))).toBe("");
    expect(usableTerminalFontFamily(undefined)).toBe("");
  });

  it("tells pending, failed and connected devices apart and offers Retry only when it is a new attempt", () => {
    expect(deviceLine(device({ state: "ready" }), undefined).tone).toBe("ok");
    const waiting = { target_id: "studio", state: "not_connected", message: null };
    expect(deviceLine(device({}), waiting).tone).toBe("pending");
    expect(canRetryDevice(device({}), waiting)).toBe(false);
    const failed = { ...waiting, state: "unreachable" };
    expect(deviceLine(device({}), failed).text).toBe("not connected");
    expect(canRetryDevice(device({}), failed)).toBe(true);
    expect(canRetryDevice(device({ state: "ready" }), undefined)).toBe(false);
    expect(canRetryDevice(device({ kind: "local", state: "local" }), undefined)).toBe(false);
  });

  it("words the terminal engine's connection from the core's state", () => {
    expect(engineLine({ state: "socket_missing" })).toEqual({ text: "The terminal engine is not running.", tone: "warn" });
    expect(engineLine({ state: "connected" }).text).toBe("Connected");
    expect(engineLine(undefined).text).toBe("unavailable");
  });

  it("copies diagnostics without the page token or any secret-shaped value", () => {
    const token = "a".repeat(64);
    const text = diagnosticsText({
      daemon: {
        version: "0.1.0",
        host_id: "host-00000000000000000000000000000000",
        host_name: "studio-host",
        schema_version: 2,
        pid: 42,
        started_at_unix: "1",
        state_dir: "/Users/example/.hide/state",
        core_state_path: "/Users/example/.hide/state/core-state.json",
        herdr_bin_path: null,
        herdr_socket_path: null,
        keep_alive: false,
        idle_secs: 600,
      },
      connection: "live",
      engine: { state: "connected", received_version: "0.9.1" },
      environment: [],
      diagnostics: [{ kind: "ws.url", message: `opened #token=${token} with key=abc123`, occurred_at: 0 }],
      lastError: { kind: "provider.failed", message: 'ghp_Q9zR2wK7mN5pT8vS3uX6yL4gH1jV0qB {"password":"q7Z"} someone@example.invalid' },
      userAgent: "test",
    });
    expect(text).toContain("hided 0.1.0 pid 42");
    expect(text).toContain("engine: connected");
    expect(text).not.toMatch(/herdr/i);
    expect(text).not.toContain(token);
    expect(text).not.toContain("abc123");
    for (const privateText of ["ghp_Q9z", "q7Z", "someone@example.invalid", "/Users/example"]) expect(text).not.toContain(privateText);
    expect(text).toContain("~/.hide/state");
    expect(redact("password=hunter2 ok")).toBe("password=[redacted] ok");
  });

  it("says whether a device's helper may run and never reads a refusal as ready", () => {
    const host = (patch: Partial<DeviceHost>): DeviceHost => ({
      consent: "granted",
      helper_root: "~/.hide/host-helper",
      cli_dir: "~/.local/bin",
      contract: 2,
      bound_identity: null,
      granted_at_unix_ms: 1,
      state: "ready",
      message: null,
      platform: "macos aarch64",
      helper_path: null,
      ...patch,
    });
    expect(hostLine(host({})).tone).toBe("ok");
    expect(hostLine(host({ consent: "none", state: "not_allowed" }))).toEqual({ text: "helper not allowed", tone: "muted" });
    expect(hostLine(host({ consent: "outdated", state: "not_allowed" })).text).toBe("helper needs a new consent");
    expect(hostLine(host({ state: "identity_changed" })).tone).toBe("warn");
    expect(hostLine(host({ state: "unavailable" })).tone).toBe("warn");
    expect(hostLine(host({ consent: "this_machine" })).tone).toBe("local");
  });

  it("names every kit part and where it goes in the one consent", () => {
    const lines = kitConsentTerms("/opt/hide", "/opt/bin");
    const terms = lines.join(" ");
    for (const named of ["/opt/hide", "/opt/bin", "hook helper", "old records are kept", "~/.claude/settings.json", "~/.codex/hooks.json"]) {
      expect(terms).toContain(named);
    }
    // The kit no longer touches Codex's daemon setting (PRD settings-cleanup D-12), so the consent says nothing of it.
    expect(lines).toHaveLength(5);
    expect(terms).not.toMatch(/Herdr/);
    expect(terms).toContain("Files still in use stay");
    expect(terms).not.toMatch(/daemon_auto_start|per pane/);
    expect(terms).toContain("~/hide");
    expect(terms).not.toContain("changes no hook");
  });
});

describe("unstoredDeviceDrafts (S5.5 B26, B44)", () => {
  it("names the device's tabs whose draft is only in the tab", () => {
    const tabs = [
      { id: "t1", device_id: "mac", path: "/r/a.txt" },
      { id: "t2", device_id: "mac", path: "/r/b.txt" },
      { id: "t3", device_id: "local", path: "/r/a.txt" },
    ];
    expect(unstoredDeviceDrafts("mac", tabs, new Set(["t2", "t3"]))).toEqual(["/r/b.txt"]);
    expect(unstoredDeviceDrafts("mac", tabs, new Set())).toEqual([]);
  });
});

describe("the install kit rows (PRD device-parity B7, B8, B27)", () => {
  const part = (id: KitComponent["id"], state: KitComponent["state"]): KitComponent => ({ id, label: id, state, why: null, location: null });
  const kit = (components: KitComponent[], unavailable: Kit["unavailable"] = null) => ({ unavailable, busy: false, components, agents: [], offers_reinstall: false, shares_account_with: null });

  it("offers Reinstall only for a part a reinstall would change", () => {
    const offered = (["installed", "outdated", "not_installed", "removed", "failed", "absent", "off"] as const).filter((state) => kitPartNeedsReinstall(part("cli", state)));
    expect(offered).toEqual(["outdated", "not_installed", "removed", "failed"]);
    expect(kitPartLine(part("coordination_retirement", "removed"))).toEqual({ text: "Removed", tone: "warn" });
    expect(kitPartLine(part("coordination_retirement", "absent")).tone).toBe("muted");
  });

  it("mentions only the parts that need the operator, and a part that is not on the machine says so once", () => {
    const withReason = (id: KitComponent["id"], state: KitComponent["state"], cause: string): KitComponent => ({ ...part(id, state), why: { cause } });
    const healthy = kit([part("cli", "installed"), withReason("codex_hook", "absent", "agent_absent"), part("claude_code_hook", "off")]);
    expect(kitProblems(healthy)).toEqual([]);
    const broken = kit([part("cli", "installed"), withReason("codex_hook", "failed", "unreadable"), part("claude_code_hook", "removed")]);
    expect(kitProblems(broken).map((problem) => problem.id)).toEqual(["codex_hook", "claude_code_hook"]);
    expect(kitProblems(kit([part("cli", "failed")], "standalone"))).toEqual([]);
    expect(kitPartText(withReason("codex_hook", "absent", "agent_absent"))).toBe("Not on this machine");
    expect(kitPartText(withReason("codex_hook", "failed", "unreadable"))).toBe("Failed: Hide could not read what is there.");
    expect(kitPartText({ state: "outdated", why: { cause: "other_build", value: "/opt/hide/bin/hide-agent-hooks" } })).toBe("Outdated: It runs another Hide build: /opt/hide/bin/hide-agent-hooks");
    // A cause a newer helper named, or one sent without the value its sentence needs, leaves the state alone.
    expect(kitPartText({ state: "failed", why: { cause: "from_a_newer_helper" } })).toBe("Failed");
    expect(kitPartText({ state: "outdated", why: { cause: "other_build" } })).toBe("Outdated");
  });

  it("reads an off part as neutral", () => {
    expect(kitPartLine(part("cli", "off"))).toEqual({ text: "Off", tone: "muted" });
  });

  it("says in one line what removing a device takes off it and what stays (B22, B24)", () => {
    const connected = device({ host: { state: "ready", helper_root: "~/.hide/host-helper" } as DeviceHost });
    expect(kitRemovalLine(connected)).toMatch(/removes its hook entries, its hide link and its helper folder \(~\/\.hide\/host-helper\); the records in ~\/\.hide stay/);
    const offline = device({ host: { state: "unavailable" } as DeviceHost });
    expect(kitRemovalLine(offline)).toMatch(/not connected .* its kit stays there/);
    const shared = device({ host: { state: "ready" } as DeviceHost, kit: { ...kit([]), shares_account_with: "Studio, second Herdr" } });
    expect(kitRemovalLine(shared)).toMatch(/^Studio, second Herdr reaches the same account on .*, so Hide's kit stays there for it\.$/);
  });
});

describe("in Korean", () => {
  it("words the removal summary, the sleep choices and a missing value in the interface language", () => {
    const lines = deviceRemovalLinesIn("mini", [{ device_id: "mini" }], [{ id: "t1", device_id: "mini", dirty: true }], [{ device: "mini" }], () => false, korean);
    expect(lines).toEqual([
      "여기서 프로젝트 1개의 등록을 지우고, 열린 파일 탭 1개도 닫아요.",
      "저장하지 않은 초안 2개는 이 브라우저에 남아요. 저장하지 않은 초안 목록에서 내보내거나 버릴 수 있어요.",
    ]);
    expect(SLEEP_AFTER_CHOICES.map((choice) => sleepAfterLabel(choice, korean))).toEqual(["잠들지 않음", "12시간", "24시간", "2일", "3일"]);
    expect(shownIn(undefined, korean)).toBe("사용할 수 없음");
    expect(shownIn("/state", korean)).toBe("/state");
    expect(kitPartLineIn({ state: "installed" }, korean).text).toBe("설치됨");
  });
});

