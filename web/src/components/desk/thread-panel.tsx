import { useEffect, useMemo, useRef, useState } from "react";
import type { Actions } from "../../actions";
import { refusalText, submitThreadInput } from "../../attachments";
import type { DeskJob, DeskMention } from "../../desk";
import { useInterfaceTranslation } from "../../i18n/client";
import type { MessageKey } from "../../i18n/catalogs";
import { allAgents } from "../../paneAgents";
import { useShellStore } from "../../store";
import { Input } from "../ui/input";
import { threadInputHeld, threadLaunchModel, threadScreen } from "../../threadView";
import type { ComposerAttachment } from "./desk-composer";
import { ThreadHead, ThreadView, type ThreadConductor, type ThreadPartActions } from "./thread-view";

type Attached = ComposerAttachment & { bytes: Uint8Array };

/** The refusals a thread's input words itself; any other is an attachment's, worded as a terminal attachment's is. */
const INPUT_WORDS = new Set(["pane_changed", "pane_gone", "closed", "asleep", "uncertain", "unavailable", "device_unreachable", "too_long", "control_characters", "attachment_failed", "input_refused"]);

/**
 * A thread on screen, connected to the core (D-61, B25-B29): it opens the
 * session thread of `job`'s pane while it is shown and closes it after, draws
 * the record as the core reads it on, reads the page before when the
 * conversation is scrolled to its top, and types the input box's words and
 * files into that pane. Where the lane puts it, and where a chip or `터미널로`
 * goes, are the lane's.
 */
export function ThreadPanel({ job, actions, conductor = null, pinned, mentions = [], startedByDesk = false, selectedOutput = null, focused = false, onBack, onPin, onClose, onTerminal, ...parts }: {
  job: DeskJob;
  actions: Pick<Actions, "openThread" | "closeThread" | "readOlderThread">;
  conductor?: ThreadConductor | null;
  pinned: boolean;
  mentions?: readonly DeskMention[];
  startedByDesk?: boolean;
  selectedOutput?: string | null;
  focused?: boolean;
  onBack: () => void;
  onPin: () => void;
  onClose: () => void;
  onTerminal: () => void;
} & ThreadPartActions) {
  const { t } = useInterfaceTranslation();
  const paneId = job.pane_id;
  const latest = useRef(actions);
  latest.current = actions;
  useEffect(() => {
    if (!paneId) return;
    latest.current.openThread(paneId);
    return () => latest.current.closeThread();
  }, [paneId]);

  // The core holds one thread; until it holds this pane's, this one is still being read.
  const thread = useShellStore((s) => (paneId && s.sessionThread?.pane_id === paneId ? s.sessionThread : null));
  const history = useShellStore((s) => s.sessionThreadHistory);
  const plans = useShellStore((s) => s.rest?.plans);
  const missions = useShellStore((s) => s.rest?.missions);
  const launches = useShellStore((s) => s.rest);
  const rows = useShellStore((s) => allAgents(s.agents, s.rest?.status?.remote));
  const refusal = useShellStore((s) => s.threadInputRefusal);
  const screen = useMemo(() => threadScreen(thread, history, { plans, missions }, rows), [thread, history, plans, missions, rows]);
  const model = threadLaunchModel(launches, thread);

  const [text, setText] = useState("");
  const [files, setFiles] = useState<Attached[]>([]);
  const [sent, setSent] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const words = (reason: string) => (INPUT_WORDS.has(reason) ? t(`desk.thread.input.${reason}` as MessageKey) : refusalText(reason, t));

  // The words stay in the box until the pane took them, so a refused send loses nothing.
  // The answer names its request, so it is read from whichever thread the core
  // holds: an input that arrived after another pane's thread opened is
  // answered there, as closed.
  const input = useShellStore((s) => s.sessionThread?.input ?? null);
  useEffect(() => {
    if (!sent || input?.request_id !== sent || input.state === "sending") return;
    setSent(null);
    if (input.state === "sent") {
      setText("");
      setFiles([]);
      setNotice(null);
    } else {
      setNotice(input.reason ? words(input.reason) : null);
    }
  }, [input, sent]);
  useEffect(() => {
    if (!sent || refusal?.request_id !== sent) return;
    setSent(null);
    setNotice(words(refusal.reason));
  }, [refusal, sent]);

  const held = threadInputHeld(thread);
  const send = () => {
    if (!thread?.pane_id || held !== null || sent) return;
    const request = submitThreadInput(thread.generation, thread.pane_id, text, files.map(({ name, bytes }) => ({ name, bytes })));
    if (request) {
      setSent(request);
      setNotice(null);
    }
  };
  const attach = async (picked: FileList | null) => {
    const read = await Promise.all([...(picked ?? [])].map(async (file, index) => ({ id: `${Date.now()}-${index}-${file.name}`, name: file.name, image: file.type.startsWith("image/"), bytes: new Uint8Array(await file.arrayBuffer()) })));
    setFiles((current) => [...current, ...read]);
  };
  // One ask per state the core shows: scroll events at the top come faster
  // than its answer, and a read that failed is asked again only after the
  // core moved.
  const askedOlder = useRef(false);
  useEffect(() => {
    askedOlder.current = false;
  }, [thread?.generation, thread?.older]);
  const reachTop = () => {
    if (askedOlder.current || !thread?.has_older || (thread.older !== "idle" && thread.older !== "failed")) return;
    askedOlder.current = true;
    actions.readOlderThread(thread.generation);
  };

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-desk-thread-panel={paneId ?? ""}>
      <ThreadHead job={job} model={model} conductor={conductor} pinned={pinned} onBack={onBack} onTerminal={onTerminal} onPin={onPin} onClose={onClose} />
      <ThreadView
        job={job}
        lines={screen.lines}
        reading={thread === null || screen.reading}
        failure={screen.failure}
        mentions={mentions}
        startedByDesk={startedByDesk}
        outputs={job.outputs}
        selectedOutput={selectedOutput}
        composer={{
          value: text,
          attachments: files,
          focused,
          held: held !== null || sent !== null,
          notice: held === "pane_changed" || held === "pane_gone" ? words(held) : notice,
          onChange: setText,
          onSend: send,
          onAttach: () => picker.current?.click(),
          onRemoveAttachment: (id) => setFiles((current) => current.filter((file) => file.id !== id)),
        }}
        onTerminal={onTerminal}
        onReachTop={reachTop}
        {...parts}
      />
      <Input
        ref={picker}
        type="file"
        multiple
        hidden
        data-desk-thread-picker="true"
        onChange={(event) => {
          void attach(event.target.files);
          event.target.value = "";
        }}
      />
    </div>
  );
}
