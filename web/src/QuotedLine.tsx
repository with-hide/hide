// A line another program printed (a pane's shell, Git), quoted beside the
// screen's own sentence as that program wrote it: it is not translated, and
// it is the operator's own content, so no log keeps it.

export function QuotedLine({ line }: { line: string }) {
  return <span className="block break-all font-mono text-caption text-muted-foreground" data-quoted-line="true">{line}</span>;
}
