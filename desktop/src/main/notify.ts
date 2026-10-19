/**
 * The shell's system notifications (PRD first-run-onboarding D-10):
 * a root agent that newly
 * needs the operator or finished; the host shows them while the app runs.
 * Clicking one brings the window forward and hands its id back to the shell,
 * which opens what it names. Herdr's own notification cannot carry where a
 * click goes, which is why the host shows these and the daemon does not.
 * The app icon's badge counts the agents that need the operator, which the
 * shell reports.
 */

/** What the shell may ask for; longer text is refused rather than cut by the system. */
export const NOTIFY_LIMITS = { id: 200, title: 120, body: 400 } as const;

/** Notifications kept for their click; an older one still shows but its click is lost. */
export const NOTIFY_KEPT = 16;

/** Ids one withdrawal may name; an agent judgment clears far fewer. */
export const WITHDRAW_LIMIT = 64;

/**
 * `unlessFocused`: an agent's notification (PRD first-run-onboarding D-10)
 * is not shown while a Hide window has the keyboard, since the operator is
 * already looking; the test notification always shows.
 */
export type NotifyRequest = { id: string; title: string; body: string; unlessFocused: boolean };

/** The shell's request when it is well formed, or why it is refused. */
export function notifyRequest(value: unknown): NotifyRequest | { refused: "shape" | "length" } {
  if (typeof value !== "object" || value === null) return { refused: "shape" };
  const { id, title, body, unlessFocused = false } = value as Record<string, unknown>;
  if (typeof id !== "string" || typeof title !== "string" || typeof body !== "string" || typeof unlessFocused !== "boolean") return { refused: "shape" };
  if (id.length === 0 || title.trim().length === 0) return { refused: "shape" };
  if (id.length > NOTIFY_LIMITS.id || title.length > NOTIFY_LIMITS.title || body.length > NOTIFY_LIMITS.body) return { refused: "length" };
  return { id, title, body, unlessFocused };
}

/** The notification ids the shell withdraws (an agent the operator has seen, B35), or why the request is refused. */
export function withdrawRequest(value: unknown): string[] | { refused: "shape" | "length" } {
  if (!Array.isArray(value) || !value.every((id) => typeof id === "string" && id.length > 0)) return { refused: "shape" };
  if (value.length > WITHDRAW_LIMIT || value.some((id: string) => id.length > NOTIFY_LIMITS.id)) return { refused: "length" };
  return value as string[];
}

/** The largest count the shell may report for the app icon, far above any desktop's agents; a larger one is refused. */
export const BADGE_LIMIT = 9999;

/** The count the shell reports for the app icon, or why it is refused. */
export function badgeRequest(value: unknown): number | { refused: "shape" | "length" } {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) return { refused: "shape" };
  if (value > BADGE_LIMIT) return { refused: "length" };
  return value;
}

/** What a Windows toast's `launch` carries: the notification's id, handed back by a click that started Hide (`launchedId`). */
const LAUNCH_PREFIX = "hide-open=";

/**
 * A notification as a Windows toast, written here rather than by Electron
 * only to set its `launch`: Electron 44's own toast has none, so a click on a
 * toast in Action Center after Hide quit reaches `Notification.handleActivation`
 * with empty `arguments` and nothing says which notification it was
 * (`shell/browser/notifications/win/windows_toast_notification.cc`,
 * `GetToastXml`). The text is Electron's own form: the title and the body as
 * two lines of the generic template, the body left out when empty.
 */
export function toastXml(request: NotifyRequest): string {
  const lines = [request.title, request.body].filter((line) => line.length > 0);
  const text = lines.map((line) => `<text>${escapeXml(line)}</text>`).join("");
  return `<toast launch="${escapeXml(LAUNCH_PREFIX + encodeURIComponent(request.id))}"><visual><binding template="ToastGeneric">${text}</binding></visual></toast>`;
}

/** The notification id a toast click that started Hide names (`ActivationArguments.arguments`), or null when it names none Hide wrote. */
export function launchedId(argumentsText: string): string | null {
  if (!argumentsText.startsWith(LAUNCH_PREFIX)) return null;
  let id: string;
  try {
    id = decodeURIComponent(argumentsText.slice(LAUNCH_PREFIX.length));
  } catch {
    return null;
  }
  return id.length > 0 && id.length <= NOTIFY_LIMITS.id ? id : null;
}

/** Text as XML 1.0 takes it: the markup escaped, and the control characters it refuses (all below U+0020 but tab, newline and return) left out. */
function escapeXml(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "").replace(/[<>&"']/g, (char) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&apos;" })[char]!);
}

/** The Windows taskbar overlay's size in pixels; Electron resizes any other to it (`shell/browser/ui/win/taskbar_host.cc`). */
export const OVERLAY_SIZE = 16;

/**
 * The taskbar overlay's mark while agents need the operator: a filled dot in
 * `color` (`#rrggbb`), the amber Hide draws its Needs You marks in, as the
 * premultiplied BGRA pixels `nativeImage.createFromBitmap` takes, its edge
 * softened over one pixel.
 */
export function overlayDot(color: string): Buffer {
  const rgb = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color);
  if (!rgb) throw new Error(`overlay color is not #rrggbb: ${color}`);
  const [red, green, blue] = rgb.slice(1).map((hex) => Number.parseInt(hex, 16));
  const pixels = Buffer.alloc(OVERLAY_SIZE * OVERLAY_SIZE * 4);
  const center = OVERLAY_SIZE / 2;
  for (let y = 0; y < OVERLAY_SIZE; y += 1) {
    for (let x = 0; x < OVERLAY_SIZE; x += 1) {
      const alpha = Math.min(1, Math.max(0, center - 0.5 - Math.hypot(x + 0.5 - center, y + 0.5 - center)));
      pixels.set([blue!, green!, red!, 255].map((channel) => Math.round(channel * alpha)), (y * OVERLAY_SIZE + x) * 4);
    }
  }
  return pixels;
}

/**
 * Where a notification click goes (`openNotification` in host.ts): to the
 * shell at once while it holds a live snapshot, else held, the latest click
 * winning, until the shell reports its count, which it does once live.
 */
export class ClickGate {
  private live = false;
  private held: string | null = null;

  /** A click: the id to hand the shell now, or null while it waits. */
  click(id: string): string | null {
    if (this.live) return id;
    this.held = id;
    return null;
  }

  /** The shell reported its count: the held id to hand it now, if any. */
  shellLive(): string | null {
    this.live = true;
    const held = this.held;
    this.held = null;
    return held;
  }

  /** The window left the shell's page, or the shell lost its daemon. */
  shellGone(): void {
    this.live = false;
  }
}

/** The shown notifications a click can still reach, newest last, at most `NOTIFY_KEPT`. */
export class KeptNotifications<T> {
  private readonly kept: { id: string; value: T }[] = [];

  /** Keeps `value` for `id`, replacing an earlier one for the same id; answers the one that dropped out, if any. */
  keep(id: string, value: T): T | null {
    const earlier = this.kept.findIndex((entry) => entry.id === id);
    const replaced = earlier >= 0 ? this.kept.splice(earlier, 1)[0]!.value : null;
    this.kept.push({ id, value });
    if (replaced !== null) return replaced;
    return this.kept.length > NOTIFY_KEPT ? this.kept.shift()!.value : null;
  }

  /** Takes `id` out, answering what was kept for it: a newer notification for the same id replaces it on screen (B35). */
  take(id: string): T | null {
    const index = this.kept.findIndex((entry) => entry.id === id);
    return index >= 0 ? this.kept.splice(index, 1)[0]!.value : null;
  }

  /** Forgets `id` once its notification was clicked or closed. */
  forget(id: string): void {
    const index = this.kept.findIndex((entry) => entry.id === id);
    if (index >= 0) this.kept.splice(index, 1);
  }

  get size(): number {
    return this.kept.length;
  }
}
