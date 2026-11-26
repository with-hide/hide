import { ArrowUpRightIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { findAnswerPaths, writtenPaths, type AnswerPlace, type FoundAnswerPath } from "../../answerPaths";
import { fileIcon } from "../../fileIcons";
import { revealHost } from "../../host";
import { useInterfaceTranslation } from "../../i18n/client";
import { cn } from "../../lib/utils";
import type { MarkdownChip } from "../../MarkdownText";
import { localDeviceId } from "../../snapshot";
import { useShellStore } from "../../store";
import { FileMark } from "../file-mark";
import { Button } from "../ui/button";
import { Hint } from "../ui/tooltip";
import { PreviewTile } from "./preview-tile";
import { useSessionChipBridge } from "./session-chip";

const LOOK = "inline-flex max-w-full min-w-0 items-center gap-xxs rounded-xs bg-secondary px-xxs align-middle text-foreground hover:bg-accent";

function fileName(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/**
 * A file an answer names (feedback 2 frame M [2]), drawn as the session chip
 * is (its kind's icon and its name). A project's file shows its tile and
 * full path under the pointer and opens in the tab panel; a file outside
 * every project carries a small ↗, says in one line that it shows in the
 * system's file manager (`Finder에서 보기 · <path>`), and a press only
 * reveals it there, never opening or running it.
 */
export function FilePathChip({ file, onOpen }: { file: FoundAnswerPath; onOpen: (file: FoundAnswerPath) => void }) {
  const { t } = useInterfaceTranslation();
  const name = fileName(file.path);
  const face = (
    <Button variant="text" size="inline" className={cn(LOOK)} data-desk-file-chip={file.kind} onClick={() => onOpen(file)}>
      <FileMark icon={fileIcon(name)} />
      <span className="truncate">{name}</span>
      {file.kind === "outside" ? <ArrowUpRightIcon aria-hidden="true" className="size-(--size-icon-sm) shrink-0 text-muted-foreground" /> : null}
    </Button>
  );
  if (file.kind === "outside") {
    const host = revealHost();
    return <Hint label={host ? `${t(host.label)} · ${file.path}` : file.path}>{face}</Hint>;
  }
  return (
    <Hint label={file.path} side="bottom" content={(
      <span className="flex w-(--size-desk-output-thumb) flex-col gap-xxs" data-desk-file-chip-card={file.path}>
        <PreviewTile file={{ id: `file:${file.device}:${file.path}`, label: name, path: file.path, url: null, device: file.device, root: file.root }} named={false} />
        <span className="break-all text-micro text-muted-foreground">{file.path}</span>
      </span>
    )}>
      {face}
    </Hint>
  );
}

/**
 * The chips for the file paths `text` writes, once each is found: a path
 * that names nothing stays words. Nothing is looked up while the words are
 * still `live` (an answer being written), nor for a session no checkout
 * holds (`place` null).
 */
export function useFilePathChips(text: string, place: AnswerPlace | null, live: boolean, onOpen: (file: FoundAnswerPath) => void): MarkdownChip[] {
  const bridge = useSessionChipBridge();
  const latest = useRef(onOpen);
  latest.current = onOpen;
  const written = useMemo(() => (live || !place ? [] : writtenPaths(text)), [text, live, place]);
  const [found, setFound] = useState<Map<string, FoundAnswerPath>>(() => new Map());
  const device = place?.device ?? null;
  const cwd = place?.cwd ?? null;
  useEffect(() => {
    if (written.length === 0 || device === null) return undefined;
    let current = true;
    const rest = useShellStore.getState().rest;
    void findAnswerPaths(written.map((one) => one.written), { device, cwd }, rest, localDeviceId(rest)).then((answer) => {
      if (current) setFound(answer);
    });
    return () => {
      current = false;
    };
  }, [written, device, cwd]);
  return useMemo(() => {
    const chips: MarkdownChip[] = [];
    for (const one of written) {
      const file = found.get(one.written);
      if (!file) continue;
      chips.push({ token: text.slice(one.from, one.to), at: [one.from], node: bridge(<FilePathChip file={file} onOpen={(chosen) => latest.current(chosen)} />) });
    }
    return chips;
  }, [written, found, text, bridge]);
}
