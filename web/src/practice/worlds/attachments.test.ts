import { describe, expect, it } from "vitest";
import "../chapters";
import chapter from "../chapters/attachments";
import type { CoreEvent } from "../world";
import { ATTACHMENTS_WORLD, MEASURING, type AttachmentsModel } from "./attachments";

const doneAt = (id: string, model: AttachmentsModel) => chapter.steps.find((step) => step.id === id)!.done!(model);
const apply = (model: AttachmentsModel, event: CoreEvent) => ATTACHMENTS_WORLD.apply(model, event);
const press = (kind: string, payload: Record<string, unknown> = {}): CoreEvent => ({ kind, payload });
const row = (model: AttachmentsModel) => ATTACHMENTS_WORLD.snapshot(model, Date.now()).agents.find((agent) => agent.pane_id === MEASURING)!;
const phone = (model: AttachmentsModel) => ATTACHMENTS_WORLD.phone!(model);
const beat = (model: AttachmentsModel) => apply(model, ATTACHMENTS_WORLD.later!(model)!.event)!;

describe("the attachments flow's world (UI_BEHAVIOR, Practice window)", () => {
  it("attaches a photo and a log, leaves out the large trace, and sends them with the words once the log is in", () => {
    const start = ATTACHMENTS_WORLD.initial();
    expect(phone(start).reply).toMatchObject({ files: [], notice: null, sendable: false });
    expect(apply(start, press("practice_phone_send"))).toBeNull();

    const chosen = apply(start, press("practice_phone_attach"))!;
    expect(doneAt("attach", chosen)).toBe(true);
    expect(phone(chosen).reply!.files.map((file) => [file.name, file.state])).toEqual([["IMG_2041.jpg", "ready"], ["holder.log", "uploading"]]);
    expect(phone(chosen).reply!.notice).toMatchObject({ kind: "too_large", name: "trace.json" });
    // Send waits for the log's last bytes.
    expect(phone(chosen).reply!.sendable).toBe(false);
    expect(apply(chosen, press("practice_phone_send"))).toBeNull();

    const ready = beat(chosen);
    expect(doneAt("files", ready)).toBe(true);
    expect(phone(ready).reply!.sendable).toBe(true);

    const gone = apply(ready, press("practice_phone_send"))!;
    expect(doneAt("send", gone)).toBe(true);
    expect(phone(gone).reply).toMatchObject({ text: "", files: [], notice: null, sendable: false });
    expect(phone(gone).attached?.[1]?.map((file) => file.name)).toEqual(["IMG_2041.jpg", "holder.log"]);
    expect(row(gone)).toMatchObject({ group: "working" });
    // The paths come before the words, as one message.
    const typed = ATTACHMENTS_WORLD.screen(gone, MEASURING, 120);
    expect(typed.indexOf("holder.log")).toBeLessThan(typed.indexOf("Yesterday's screen"));

    const read = beat(gone);
    expect(doneAt("typed", read)).toBe(true);
    expect(phone(read).said.map((said) => said.who)).toEqual(["agent", "you", "agent"]);
    expect(ATTACHMENTS_WORLD.screen(read, MEASURING, 120)).toContain("Read");
    expect(ATTACHMENTS_WORLD.later!(read)).toBeNull();
  });

  it("takes one file off with its ×, and sends only what is left", () => {
    const chosen = apply(ATTACHMENTS_WORLD.initial(), press("practice_phone_attach"))!;
    const one = apply(chosen, press("practice_phone_remove", { id: "log" }))!;
    expect(phone(one).reply!.files.map((file) => file.name)).toEqual(["IMG_2041.jpg"]);
    expect(apply(one, press("practice_phone_remove", { id: "log" }))).toBeNull();

    const gone = apply(beat(one), press("practice_phone_send"))!;
    expect(phone(gone).attached?.[1]?.map((file) => file.name)).toEqual(["IMG_2041.jpg"]);
    expect(phone(beat(gone)).said.at(-1)!.text).toContain("send the holder log");

    // Taking off the last file empties the bar, and `+` picks again.
    const none = apply(one, press("practice_phone_remove", { id: "photo" }))!;
    expect(doneAt("attach", none)).toBe(false);
    expect(apply(none, press("practice_phone_attach"))).not.toBeNull();
  });
});
