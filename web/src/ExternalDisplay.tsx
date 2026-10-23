// An outside file's tab body (PRD outside-path-in-hide): the file the desktop
// host granted this window, drawn read-only in the viewer its kind opens in,
// the same image, PDF and video viewers a checkout file uses, a page for HTML
// and a read-only text view for everything else. Another window, which holds
// no grant, draws the file's name and nothing of it (B19).

import { highlightSpecialChars, drawSelection, EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { Compartment, EditorState } from "@codemirror/state";
import { defaultKeymap } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { useEffect, useMemo, useRef } from "react";
import type { Actions } from "./actions";
import { AreaEmpty } from "./AreaEmpty";
import { ExternalPage } from "./BrowserDisplay";
import { Button } from "./components/ui/button";
import { lineOffset, onLineRequest, takeLine } from "./editor/lineRequest";
import { languageLoader } from "./editor/languages";
import { editorPhrases } from "./editor/phrases";
import { baseTheme, scaleTheme } from "./editor/theme";
import { externalLineKey, useExternalBytes, useExternalGrant, type ExternalBytes } from "./externalFiles";
import { useInterfaceTranslation } from "./i18n/client";
import type { ViewDisplaySnapshot } from "./snapshot";
import { useShellStore } from "./store";
import type { ViewWorkspace } from "./viewLayout";
import { ImageBody, PdfBody, VideoBody, ViewerNotice } from "./viewers/FileViewer";
import type { FileBytesState } from "./viewers/useFileBytes";

export function ExternalDisplay({ display, workspace, actions }: { display: ViewDisplaySnapshot; workspace: ViewWorkspace; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const file = display.external;
  const held = useExternalGrant(file?.grant ?? "");
  if (!file || held === undefined) return <AreaEmpty state="external-opening" text={t("documents.view.openingNamed", { label: display.label })} />;
  if (held === null) return <AreaEmpty state="external-elsewhere" text={t("documents.external.elsewhere", { name: display.label })} />;
  if (file.kind === "html") return <ExternalPage display={display} workspace={workspace} actions={actions} />;
  return <ExternalFile display={display} grant={file.grant} kind={file.kind} actions={actions} />;
}

function ExternalFile({ display, grant, kind, actions }: { display: ViewDisplaySnapshot; grant: string; kind: "text" | "image" | "pdf" | "video"; actions: Actions }) {
  const { t } = useInterfaceTranslation();
  const read = useExternalBytes(grant, display.load ?? 0);
  if (read.status === "failed") {
    // The reason is in the host's log (B16); the tab says it cannot open.
    return (
      <AreaEmpty state="view-unavailable" text={t("documents.view.unavailable", { path: display.label })}>
        <Button variant="secondary" data-keyboard-stays="true" onPointerDown={(event) => { event.preventDefault(); event.stopPropagation(); }} onClick={() => actions.closeView(display.id)} data-close-unavailable={display.id}>
          {t("documents.view.closeView")}
        </Button>
      </AreaEmpty>
    );
  }
  if (read.status === "preview_only") return <ViewerNotice state="external-too-large" reason={t("documents.tooLarge")} />;
  const state = bytesState(read);
  // A name stands for the path the viewers name their file by: its type is in it, and nothing else is.
  if (kind === "image") return <ImageBody key={`${grant}:${display.load ?? 0}`} path={display.label} state={state} displayId={display.id} />;
  if (kind === "pdf") return <PdfBody key={`${grant}:${display.load ?? 0}`} path={display.label} state={state} displayId={display.id} />;
  if (kind === "video") return <VideoBody path={display.label} state={state} />;
  if (read.status !== "ready") return <AreaEmpty state="external-opening" text={t("documents.view.openingNamed", { label: display.label })} />;
  return <ExternalText key={`${grant}:${display.load ?? 0}`} grant={grant} name={display.label} bytes={read.bytes} />;
}

/** The language a file's name says, the way the core names a checkout document's (`hide-host` `language_for`). */
export function languageOfName(name: string): string | null {
  const lower = name.toLowerCase();
  const whole: Record<string, string> = { makefile: "makefile", gnumakefile: "makefile", dockerfile: "dockerfile", gemfile: "ruby", rakefile: "ruby", ".env": "ini", ".editorconfig": "ini" };
  if (whole[lower]) return whole[lower];
  const dot = lower.lastIndexOf(".");
  if (dot <= 0 || dot === lower.length - 1) return null;
  const extension = lower.slice(dot + 1);
  const named: Record<string, string> = {
    rs: "rust", js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript", ts: "typescript", tsx: "typescript",
    json: "json", jsonc: "json", jsonl: "json", md: "markdown", markdown: "markdown", mdown: "markdown",
    sh: "bash", bash: "bash", zsh: "bash", fish: "bash", toml: "ini", ini: "ini", cfg: "ini", py: "python", pyw: "python",
    htm: "html", scss: "css", sass: "css", less: "css",
  };
  return named[extension] ?? extension;
}

const language = new Compartment();

function bytesState(read: Exclude<ExternalBytes, { status: "preview_only" }>): FileBytesState {
  return read;
}

/**
 * An outside text file, read-only: line numbers, its language's colours and
 * Find, and a terminal link's line (`lineRequest.ts`), taken when it mounts or
 * when a second click brings it forward (B2). It holds no draft and saves
 * nothing, since the file is not the checkout's (D-06).
 */
function ExternalText({ grant, name, bytes }: { grant: string; name: string; bytes: Uint8Array }) {
  const { t } = useInterfaceTranslation();
  const host = useRef<HTMLDivElement>(null);
  const scale = useShellStore((s) => s.rest?.ui_state?.editor_text_scale ?? 1);
  const text = useMemo(() => new TextDecoder().decode(bytes), [bytes]);
  useEffect(() => {
    if (!host.current) return undefined;
    const view = new EditorView({
      state: EditorState.create({
        doc: text,
        extensions: [
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          highlightSpecialChars(),
          drawSelection(),
          highlightActiveLine(),
          lineNumbers(),
          highlightActiveLineGutter(),
          search({ top: true }),
          keymap.of([...defaultKeymap, ...searchKeymap]),
          baseTheme(),
          editorPhrases(t),
          scaleTheme(scale),
          language.of([]),
        ],
      }),
      parent: host.current,
    });
    let live = true;
    const loader = languageLoader(languageOfName(name), name);
    if (loader) {
      void loader().then((pack) => {
        if (live) view.dispatch({ effects: language.reconfigure(pack) });
      }, () => undefined);
    }
    const goToLine = () => {
      const asked = takeLine(externalLineKey(grant));
      if (!asked) return;
      const offset = lineOffset(view.state.doc, asked.line, asked.column);
      view.dispatch({ selection: { anchor: offset }, userEvent: "select", effects: EditorView.scrollIntoView(offset, { y: "center" }) });
    };
    goToLine();
    const stop = onLineRequest(goToLine);
    return () => {
      live = false;
      stop();
      view.destroy();
    };
    // One view per read: a second click remounts it by its key.
  }, [grant, name, text, scale, t]);
  return <div ref={host} className="min-h-0 min-w-0 flex-1 overflow-hidden [&_.cm-editor]:h-full" data-external-text={grant} />;
}
