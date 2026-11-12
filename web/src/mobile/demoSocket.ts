// The sample host (docs/UI_BEHAVIOR.md, Phone demo mode): a `PhoneSocket` that plays hided for a person with no computer running Hide.
// `connection.ts` uses it exactly as it uses the relay's socket, so the store and every screen read the same frames a real host sends.
// It opens no WebSocket, makes no request and touches no storage; it answers each message of the phone with what hided would, a moment
// later, and holds the few things a visit changes (`DemoState`).

import { clientI18n, translate } from "../i18n/translator";
import {
  DEMO_DEVICE, LOOK_HTML, LOOK_IMAGE, START_STATE, agentsFrame, terminalText, base64Of, conversation, looks, lookChart, lookHtml, missionsFrame, prsFrame, startCatalog,
  type DemoState, type Said,
} from "./demoData";
import type { PhoneMessage, ServerFrame } from "./protocol";
import type { PhoneSocket } from "./relaySocket";
import { hostedPhone } from "./relayRuntime";

/** The page address that asks for the demo; only the hosted phone, which has a first screen to enter it from, takes it. */
export function demoRequested(): boolean {
  return hostedPhone() && new URLSearchParams(window.location.search).has("demo");
}

/** How long hided takes to answer; short, so a screen sees a request in flight before its answer. */
const ANSWER_MS = 150;
const REPLY_MS = 900;

const MAX_TITLE_CHARS = 60;
/** What one visit may add: sample rows and sentences are memory the page holds, so crossing either is a refusal the sheet or the reply bar words. */
const MAX_STARTED = 10;
const MAX_SAID = 40;
const DEMO_LIMIT = "demo_limit";

export class DemoSocket implements PhoneSocket {
  readyState: number = WebSocket.CONNECTING;
  onopen: PhoneSocket["onopen"] = null;
  onmessage: PhoneSocket["onmessage"] = null;
  onclose: PhoneSocket["onclose"] = null;
  private state: DemoState = { ...START_STATE, t0: Date.now() };
  private timers = new Set<number>();
  private openPane: string | null = null;
  private counter = 0;
  private followLanguage = () => this.reword();

  constructor() {
    this.later(0, () => {
      this.readyState = WebSocket.OPEN;
      this.onopen?.(new Event("open"));
    });
    clientI18n.on("languageChanged", this.followLanguage);
  }

  send(text: string): void {
    if (this.readyState !== WebSocket.OPEN) throw new Error("demo_socket_closed");
    const message = JSON.parse(text) as PhoneMessage | { client_kind: string };
    if ("client_kind" in message) {
      this.hello();
      return;
    }
    this.answer(message);
  }

  close(): void {
    if (this.readyState === WebSocket.CLOSED) return;
    this.readyState = WebSocket.CLOSED;
    clientI18n.off("languageChanged", this.followLanguage);
    for (const timer of this.timers) window.clearTimeout(timer);
    this.timers.clear();
  }

  private later(ms: number, run: () => void): void {
    const timer = window.setTimeout(() => {
      this.timers.delete(timer);
      if (this.readyState !== WebSocket.CLOSED) run();
    }, ms);
    this.timers.add(timer);
  }

  private emit(frame: ServerFrame): void {
    this.onmessage?.({ data: JSON.stringify(frame) } as MessageEvent);
  }

  /** The frames a Mac sends on connecting: who it is, then the three lists. */
  private hello(): void {
    this.later(0, () => {
      this.emit({ type: "hello", mac_name: translate("mobile.demo.host"), phone_id: "demo", name: null, vapid_public_key: "", notifications: "off" });
      this.emit({ type: "meta", push_mode: "app_closed", other_phones: 0 });
      this.lists();
    });
  }

  private lists(): void {
    this.emit(agentsFrame(translate, this.state));
    this.emit(missionsFrame(translate, this.state));
    this.emit(prsFrame(translate, this.state));
  }

  /** The phone's language changed: the sample content is worded again, as a Mac that changed its language would. */
  private reword(): void {
    this.later(0, () => {
      this.emit({ type: "hello", mac_name: translate("mobile.demo.host"), phone_id: "demo", name: null, vapid_public_key: "", notifications: "off" });
      this.lists();
    });
  }

  private say(pane: string, said: Said): void {
    this.state = { ...this.state, said: { ...this.state.said, [pane]: [...(this.state.said[pane] ?? []), said] } };
  }

  private conversationFrame(pane: string): ServerFrame {
    return { type: "conversation", device_id: DEMO_DEVICE, pane_id: pane, state: "ok", mode: "reset", messages: conversation(translate, this.state, pane), before: null };
  }

  private showDetail(pane: string): void {
    this.emit(this.conversationFrame(pane));
    this.emit({ type: "looks", device_id: DEMO_DEVICE, pane_id: pane, views: looks(this.state, pane), omitted: 0 });
  }

  private answer(message: PhoneMessage): void {
    switch (message.type) {
      case "ping":
        this.later(0, () => this.emit({ type: "pong", id: message.id }));
        return;
      case "open":
        this.openPane = message.pane_id;
        this.later(ANSWER_MS, () => (message.view === "terminal" ? this.terminal(message.pane_id) : this.showDetail(message.pane_id)));
        return;
      case "view":
        if (this.openPane) {
          const pane = this.openPane;
          this.later(ANSWER_MS, () => (message.view === "terminal" ? this.terminal(pane) : this.showDetail(pane)));
        }
        return;
      case "close":
        this.openPane = null;
        return;
      case "input":
        this.input(message);
        return;
      case "look_open":
        this.openLook(message.request_id, message.view_id);
        return;
      case "look_seen":
        if (!this.state.seenLooks.includes(message.view_id)) {
          this.state = { ...this.state, seenLooks: [...this.state.seenLooks, message.view_id] };
          this.later(0, () => {
            this.emit({ type: "looks", device_id: DEMO_DEVICE, pane_id: message.pane_id, views: looks(this.state, message.pane_id), omitted: 0 });
            this.emit(agentsFrame(translate, this.state));
          });
        }
        return;
      case "start_sheet":
        if (message.open) this.later(ANSWER_MS, () => this.emit({ type: "start_catalog", ...startCatalog() }));
        return;
      case "start_agent":
        this.startAgent(message.request_id, message.text);
        return;
      case "delegate_pr":
        this.delegate(message.request_id, message.pr, message.prompt);
        return;
      case "answer_mission":
        this.later(ANSWER_MS, () => {
          this.state = { ...this.state, answered: true };
          this.emit({ type: "mission_result", request_id: message.request_id, mission: message.mission, ok: true, reason: null });
          this.emit(missionsFrame(translate, this.state));
        });
        return;
      case "ask_mission_material":
        this.later(ANSWER_MS, () => this.emit({ type: "mission_result", request_id: message.request_id, mission: message.mission, ok: true, reason: null }));
        return;
      // The sample mission links no View, so none can be opened again.
      case "reopen_mission_view":
        this.later(ANSWER_MS, () => this.emit({ type: "mission_result", request_id: message.request_id, mission: message.mission, ok: false, reason: "gone" }));
        return;
      case "read_pr":
        this.later(ANSWER_MS, () => this.emit({ type: "pr_result", request_id: message.request_id, action: "read_pr", pr: message.pr, ok: true, reason: null }));
        return;
      case "read_pr_feedback":
        this.later(ANSWER_MS, () => {
          this.state = { ...this.state, feedback: { requestId: message.request_id, pr: message.pr } };
          this.emit(prsFrame(translate, this.state));
          this.emit({ type: "pr_result", request_id: message.request_id, action: "read_pr_feedback", pr: message.pr, ok: true, reason: null });
        });
        return;
      case "merge_pr":
        this.later(REPLY_MS / 2, () => {
          this.state = { ...this.state, merged: [...this.state.merged, Number(message.pr.split("#")[1])] };
          this.emit({ type: "pr_result", request_id: message.request_id, action: "merge_pr", pr: message.pr, ok: true, reason: null });
          this.emit(prsFrame(translate, this.state));
        });
        return;
      case "human_turn_done":
        this.later(ANSWER_MS, () => this.emit({ type: "human_turn_result", request_id: message.request_id, ok: true, reason: null }));
        return;
      // The sample host has no sleeping agent, older page, push subscription or further page of rows to send.
      default:
        return;
    }
  }

  private terminal(pane: string): void {
    const text = terminalText(translate, this.state, pane);
    this.emit({ type: "rows", device_id: DEMO_DEVICE, pane_id: pane, state: "ok", text, lines: text.split("\n").length - 1, more: false });
    this.emit(this.conversationFrame(pane));
  }

  /** A reply or a key: accepted, then (for a reply) answered with a sample sentence. */
  private input(message: Extract<PhoneMessage, { type: "input" }>): void {
    this.later(ANSWER_MS, () => {
      if ("text" in message && (this.state.said[message.pane_id]?.length ?? 0) >= MAX_SAID) {
        this.emit({ type: "input_result", request_id: message.request_id, ok: false, reason: DEMO_LIMIT });
        return;
      }
      if ("text" in message) {
        this.say(message.pane_id, { who: "you", text: message.text });
        this.emit({ type: "input_result", request_id: message.request_id, ok: true, reason: null });
        this.emit(this.conversationFrame(message.pane_id));
        this.later(REPLY_MS, () => {
          this.say(message.pane_id, { who: "agent", key: "mobile.demo.reply" });
          this.emit(this.conversationFrame(message.pane_id));
        });
        return;
      }
      this.emit({ type: "input_result", request_id: message.request_id, ok: true, reason: null });
    });
  }

  private openLook(requestId: string, viewId: string): void {
    const html = viewId === LOOK_HTML;
    if (!html && viewId !== LOOK_IMAGE) {
      this.later(ANSWER_MS, () => this.emit({ type: "look_result", request_id: requestId, view_id: viewId, state: "closed" }));
      return;
    }
    const { data, size } = base64Of(html ? lookHtml(translate) : lookChart(translate));
    this.later(ANSWER_MS, () => {
      this.emit({ type: "look_head", request_id: requestId, view_id: viewId, mode: html ? "html" : "image", size, total: size, truncated: false, chunks: 1 });
      this.emit({ type: "look_chunk", request_id: requestId, seq: 0, data });
      this.emit({ type: "look_end", request_id: requestId, view_id: viewId });
    });
  }

  /** A started agent joins the list as a sample row; its answer comes after the list has it, so its detail opens as a real one does. */
  private addAgent(title: string, prompt: string): string {
    this.counter += 1;
    const pane = `s${this.counter}`;
    this.state = { ...this.state, started: [{ pane, title, prompt }, ...this.state.started] };
    return pane;
  }

  private startAgent(requestId: string, text: string): void {
    this.later(ANSWER_MS, () => {
      if (this.state.started.length >= MAX_STARTED) {
        this.emit({ type: "start_result", request_id: requestId, ok: false, reason: DEMO_LIMIT });
        return;
      }
      const pane = this.addAgent([...text.trim().split("\n")[0]!].slice(0, MAX_TITLE_CHARS).join(""), text);
      this.emit(agentsFrame(translate, this.state));
      this.emit({ type: "start_result", request_id: requestId, ok: true, reason: null, device_id: DEMO_DEVICE, pane_id: pane });
    });
  }

  private delegate(requestId: string, pr: string, prompt: string): void {
    this.later(ANSWER_MS, () => {
      if (this.state.started.length >= MAX_STARTED) {
        this.emit({ type: "start_result", request_id: requestId, ok: false, reason: DEMO_LIMIT });
        return;
      }
      const pane = this.addAgent(`#${pr.split("#")[1]}`, prompt);
      this.state = { ...this.state, feedback: null };
      this.emit(agentsFrame(translate, this.state));
      this.emit({ type: "start_result", request_id: requestId, ok: true, reason: null, device_id: DEMO_DEVICE, pane_id: pane });
      this.emit(prsFrame(translate, this.state));
    });
  }
}
