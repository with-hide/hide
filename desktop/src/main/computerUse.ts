// System observation belongs to the host; hided owns whether phones rest.
export const COMPUTER_AWAY_SECONDS = 10 * 60;
/** Bounds detection of new input after idle, without watching individual inputs. */
export const COMPUTER_USE_POLL_MS = 1_000;
/** One resumed-input report per pause, using the existing poll on every OS. */
export const COMPUTER_INPUT_PAUSE_SECONDS = 5;

type PowerEvent = "lock-screen" | "unlock-screen" | "suspend" | "resume";
type Power = {
  getSystemIdleState(seconds: number): string;
  getSystemIdleTime(): number;
  on(event: PowerEvent, listener: () => void): unknown;
  removeListener(event: PowerEvent, listener: () => void): unknown;
};

/** One reader, one timer, four subscriptions; all end with their host. */
export class ComputerUse {
  private locked = false;
  private suspended = false;
  private timer: ReturnType<typeof setInterval> | null = null;
  private value: boolean | undefined;
  private idle: number | undefined;
  private readonly events: Record<PowerEvent, () => void> = {
    "lock-screen": () => { this.locked = true; this.read(); },
    "unlock-screen": () => { this.locked = false; this.read(); },
    suspend: () => { this.suspended = true; this.read(); },
    resume: () => { this.suspended = false; this.read(); },
  };

  constructor(private readonly power: Power, private readonly changed: (inUse: boolean) => void) {}

  start(): void {
    if (this.timer) return;
    for (const [event, listener] of Object.entries(this.events)) this.power.on(event as PowerEvent, listener);
    this.read();
    this.timer = setInterval(() => this.read(), COMPUTER_USE_POLL_MS);
  }

  current(): boolean { return this.value ?? true; }

  private read(): void {
    const state = this.power.getSystemIdleState(COMPUTER_AWAY_SECONDS);
    // An unknown idle source (including some Wayland sessions) deliberately
    // retains in-use behavior. A lock/suspend event still takes precedence.
    const idle = this.power.getSystemIdleTime();
    const resumed = this.idle !== undefined && this.idle >= COMPUTER_INPUT_PAUSE_SECONDS && idle < this.idle;
    this.idle = idle;
    const inUse = !this.locked && !this.suspended && state !== "locked"
      && (state === "unknown" || idle < COMPUTER_AWAY_SECONDS);
    if (inUse === this.value && !(inUse && resumed)) return;
    this.value = inUse;
    this.changed(inUse);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    for (const [event, listener] of Object.entries(this.events)) this.power.removeListener(event as PowerEvent, listener);
  }
}
