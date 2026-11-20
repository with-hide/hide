import type { TFunction } from "i18next";
import { PlusIcon, XIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Actions } from "../actions";
import { Note } from "../components/settings-rows";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import { Checkbox } from "../components/ui/checkbox";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "../components/ui/dialog";
import { Input } from "../components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../components/ui/select";
import { useInterfaceTranslation } from "../i18n/client";
import { cn } from "../lib/utils";
import type { Device, McpPastedServer, McpReach, McpServer, StoreAdoptConfirmation, StoreAsk, StoreFoundSkill, StoreItem, StoreItemKind, StorePreview, StoreRefusalCause, StoreReport, StoreUpdateDiff } from "../snapshot";
import { agentShortName, addInput, keyNames, previewSource, serverLine, versionValue } from "./skillsMcp";
import { useStoreRequest } from "./useStoreRequest";

/** A refused store change in the operator's words; the kit's sentence stays in the log. */
export function refusalText(cause: StoreRefusalCause | "unavailable", t: TFunction<"translation">): string {
  return t(`store.refused.${cause}`);
}

/** Label and value pairs, as the confirm cards lay them out. */
function Facts({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-md gap-y-sm text-body">{children}</dl>;
}

function Fact({ label, children, mono = false }: { label: string; children: ReactNode; mono?: boolean }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={cn("min-w-0 break-words text-foreground", mono && "break-all font-mono")}>{children}</dd>
    </>
  );
}

/** The files coming in, each executable marked ▲ (B6). */
function FileList({ files, max = 12 }: { files: { path: string; executable: boolean }[]; max?: number }) {
  const { t } = useInterfaceTranslation();
  const shown = files.slice(0, max);
  return (
    <span className="flex flex-col font-mono" data-store-file-list="true">
      {shown.map((file) => (
        <span key={file.path} className="break-all">
          {file.path}
          {file.executable ? <span className="font-sans text-warning" data-store-executable={file.path}> {t("store.detail.executable")}</span> : null}
        </span>
      ))}
      {files.length > max ? <span className="font-sans text-muted-foreground">{t("store.update.more", { count: files.length - max })}</span> : null}
    </span>
  );
}

/** The agents an install or a server reaches, as chips; one that cannot take it is struck out. */
function AgentChips({ agents, left = [] }: { agents: readonly string[]; left?: readonly string[] }) {
  return (
    <span className="flex flex-wrap gap-xs" data-store-agent-chips="true">
      {agents.map((agent) => (
        <Badge key={agent} variant="outline" className={cn(left.includes(agent) && "line-through opacity-(--opacity-disabled)")} data-store-agent-chip={agent}>
          {agentShortName(agent)}
        </Badge>
      ))}
    </span>
  );
}

function StoreDialog({ title, onClose, children, footer, data }: { title: string; onClose: () => void; children: ReactNode; footer: ReactNode; data: string }) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent data-store-dialog={data} aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-md">{children}</DialogBody>
        <DialogFooter>{footer}</DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type Stage = { kind: "source" } | { kind: "pick"; preview: string } | { kind: "confirm"; preview: string; names: string[] } | { kind: "mcp"; preview: string | null; server: McpServer; index: number | null } | { kind: "mcp_confirm"; preview: string | null; server: McpServer; index: number | null; keys: Record<string, string> };

/**
 * + 추가 (scenes 2 and 3): one field takes a GitHub address, a folder or an
 * MCP paste; a source with several skills asks which, then the confirm shows
 * the chosen skill names, descriptions and the agents; a paste fills an MCP
 * server's fields, whose keys are typed here only and hidden once typed.
 */
export function AddDialog({ device, name, report, shown, local, actions, onClose }: { device: Device; name: string; report: StoreReport; shown: ReadonlySet<string>; local: boolean; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const [text, setText] = useState("");
  const [stage, setStage] = useState<Stage>({ kind: "source" });
  const read = useStoreRequest(device.id, actions);
  const apply = useStoreRequest(device.id, actions);
  // The read's own answer names the preview it made; no other read stands in for it.
  const readPreview = read.done ? (report.previews.find((row) => row.id === read.made) ?? null) : null;
  const readPaste = read.done ? (report.mcp_previews?.find((row) => row.id === read.made) ?? null) : null;
  const lost = read.done && !readPreview && !readPaste;

  // A read that answered: on to picking, or straight to the confirm for one skill, or to a pasted server's fields.
  useEffect(() => {
    if (stage.kind !== "source") return;
    if (readPreview) {
      const [only, ...more] = readPreview.skills;
      setStage(only && more.length === 0 && !only.taken ? { kind: "confirm", preview: readPreview.id, names: [only.name] } : { kind: "pick", preview: readPreview.id });
      return;
    }
    const first = readPaste?.servers.find((server) => !server.taken) ?? readPaste?.servers[0] ?? null;
    if (readPaste) setStage({ kind: "mcp", preview: readPaste.id, server: first?.server ?? { name: "", reach: { kind: "local", command: "", args: [], env: [] } }, index: first ? readPaste.servers.indexOf(first) : null });
  }, [readPreview, readPaste, stage.kind]);
  useEffect(() => {
    if (apply.done) onClose();
  }, [apply.done, onClose]);

  const next = () => {
    const input = addInput(text);
    if (!input) return;
    if (input.kind === "mcp") read.send([{ op: "read_mcp", text: input.text }]);
    else read.send([{ op: "inspect", source: input.kind === "git" ? { kind: "git", url: input.url } : { kind: "folder", path: input.path } }]);
  };
  const pickFolder = async () => {
    const folder = await actions.pickFolder();
    if (folder) setText(folder);
  };
  const close = () => {
    // A paste read and not added leaves nothing behind (B9).
    if (stage.kind === "mcp" && stage.preview) actions.changeStore(device.id, `store-cancel-${Date.now().toString(36)}`, [{ op: "cancel_mcp", preview: stage.preview }]);
    onClose();
  };
  const preview = stage.kind === "pick" || stage.kind === "confirm" ? report.previews.find((row) => row.id === stage.preview) ?? null : null;

  if (stage.kind === "pick" && preview) return <PickSkills preview={preview} onClose={close} onNext={(names) => setStage({ kind: "confirm", preview: preview.id, names })} />;
  if (stage.kind === "confirm" && preview) {
    const skills = preview.skills.filter((skill) => stage.names.includes(skill.name));
    return (
      <InstallConfirm
        preview={preview}
        skills={skills}
        shown={shown}
        pending={apply.pending}
        refusal={apply.refusal}
        onClose={close}
        onInstall={() => apply.send([{ op: "install", preview: preview.id, names: stage.names }])}
      />
    );
  }
  if (stage.kind === "mcp") {
    return <McpFields report={report} server={stage.server} pasted={pastedOf(report, stage.preview, stage.index)} onClose={close} onNext={(server, keys) => setStage({ ...stage, kind: "mcp_confirm", server, keys })} />;
  }
  if (stage.kind === "mcp_confirm") {
    return (
      <McpConfirm
        server={stage.server}
        keys={stage.keys}
        pasted={pastedOf(report, stage.preview, stage.index)}
        shown={shown}
        pending={apply.pending}
        refusal={apply.refusal}
        confirmLabel={t("store.mcp.add")}
        onClose={close}
        onConfirm={() => apply.send([{ op: "add_mcp", server: stage.server, keys: stage.keys, ...(stage.preview !== null && stage.index !== null ? { from: { preview: stage.preview, index: stage.index } } : {}) }])}
      />
    );
  }
  return (
    <StoreDialog
      title={t("store.add.title")}
      onClose={close}
      data="add"
      footer={
        <>
          <Button variant="ghost" onClick={close}>{t("common.cancel")}</Button>
          <Button disabled={!addInput(text) || read.pending} aria-busy={read.pending} onClick={next} data-store-add-next="true">
            {read.pending ? t("store.reading") : t("store.add.next")}
          </Button>
        </>
      }
    >
      <Input
        autoFocus
        mono
        value={text}
        placeholder={local ? t("store.add.placeholder") : `${t("store.add.placeholder")} · ${t("store.add.folderPlaceholder", { device: name })}`}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) next();
        }}
        aria-label={t("store.add.placeholder")}
        data-store-add-input="true"
      />
      <span className="flex flex-wrap items-center gap-xs text-body text-muted-foreground">
        {local ? (
          <>
            {t("store.add.or")}
            <Button variant="secondary" size="sm" onClick={() => void pickFolder()} data-store-pick-folder="true">{t("store.add.pickFolder")}</Button>
            ·
          </>
        ) : null}
        {t("store.add.mcpHint")}
      </span>
      {read.refusal ? <Note tone="error" data-store-add-refused={read.refusal}>{refusalText(read.refusal, t)}</Note> : null}
      {lost ? <Note tone="error" data-store-add-refused="not_found">{refusalText("not_found", t)}</Note> : null}
    </StoreDialog>
  );
}

function pastedOf(report: StoreReport, preview: string | null, index: number | null): McpPastedServer | null {
  if (preview === null || index === null) return null;
  return report.mcp_previews?.find((row) => row.id === preview)?.servers[index] ?? null;
}

/** 찾은 스킬 n개: the skills a source holds, the new ones ticked, the ones already in the list greyed. */
function PickSkills({ preview, onClose, onNext }: { preview: StorePreview; onClose: () => void; onNext: (names: string[]) => void }) {
  const { t } = useInterfaceTranslation();
  const [names, setNames] = useState(() => preview.skills.filter((skill) => !skill.taken).map((skill) => skill.name));
  return (
    <StoreDialog
      title={t("store.add.title")}
      onClose={onClose}
      data="pick"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
          <Button disabled={names.length === 0} onClick={() => onNext(names)} data-store-pick-next="true">{t("store.add.next")}</Button>
        </>
      }
    >
      <p className="break-all font-mono text-body text-subtle-foreground">{previewSource(preview)}</p>
      <p className="text-body font-semibold text-subtle-foreground">{t("store.add.found", { count: preview.skills.length })}</p>
      <div className="flex flex-col gap-sm">
        {preview.skills.map((skill) => (
          <label key={skill.name} className={cn("flex items-start gap-sm text-body", skill.taken && "text-muted-foreground")} data-store-pick={skill.name}>
            <Checkbox
              checked={names.includes(skill.name)}
              disabled={skill.taken}
              onCheckedChange={(checked) => setNames((now) => (checked === true ? [...now, skill.name] : now.filter((one) => one !== skill.name)))}
              className="mt-xxs"
            />
            <span className="flex min-w-0 flex-col">
              <span className="font-medium text-foreground">{skill.name}{skill.taken ? <span className="font-normal text-muted-foreground"> · {t("store.add.taken")}</span> : null}</span>
              {skill.description ? <span className="break-words text-caption text-muted-foreground">{skill.description}</span> : null}
            </span>
          </label>
        ))}
      </div>
    </StoreDialog>
  );
}

/** 스킬 n개 설치: where they come from, their names and descriptions, and the agents that get them, every profile included. */
function InstallConfirm({ preview, skills, shown, pending, refusal, onClose, onInstall, confirmLabel, header, title, data = "install" }: { preview: StorePreview; skills: StoreFoundSkill[]; shown: ReadonlySet<string>; pending: boolean; refusal: StoreRefusalCause | "unavailable" | null; onClose: () => void; onInstall: () => void; confirmLabel?: string; header?: ReactNode; title?: string; data?: string }) {
  const { t } = useInterfaceTranslation();
  const agents = preview.agents.filter((agent) => shown.has(agent));
  return (
    <StoreDialog
      title={title ?? t("store.add.install", { count: skills.length })}
      onClose={onClose}
      data={data}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} data-store-cancel="true">{t("common.cancel")}</Button>
          <Button disabled={pending || skills.length === 0} aria-busy={pending} onClick={onInstall} data-store-install="true">
            {confirmLabel ?? t("store.add.install", { count: skills.length })}
          </Button>
        </>
      }
    >
      {header}
      <Facts>
        <Fact label={t("store.detail.source")} mono>{previewSource(preview)}</Fact>
        <Fact label={t("store.section.skill")}>
          <span className="flex flex-col gap-sm" data-store-confirm-skills="true">
            {skills.map((skill) => (
              <span key={skill.name}>
                <span className="block">{skill.name}</span>
                {skill.description ? <span className="block text-caption text-muted-foreground">{skill.description}</span> : null}
              </span>
            ))}
          </span>
        </Fact>
        <Fact label={t("store.detail.agents")}>
          <AgentChips agents={agents} />
          <span className="mt-xs block text-caption text-muted-foreground">{t("store.add.everyProfile")}</span>
        </Fact>
      </Facts>
      {refusal ? <Note tone="error" data-store-install-refused={refusal}>{refusalText(refusal, t)}</Note> : null}
    </StoreDialog>
  );
}

/** A copy an agent holds that Hide is asked to take in: its kind and name, the place it is taken from, where it is on the machine, and an MCP copy's server as that place holds it. */
export type AdoptTarget = { kind: StoreItemKind; name: string; from?: string; path: string | null; server?: McpServer | null };

/**
 * [모두에 적용] or [가져오기]: what is taken in, shown before anything moves
 * (B20). A skill's copy is read like a folder added: its source, name,
 * description and the agents it goes to; a server's, as MCP 추가's
 * confirm shows one (B9): the command and arguments or the address, its
 * keys' names and the agents, read from the copy's own place. A server Hide
 * cannot read offers no Apply. Only Apply takes it in.
 */
export function AdoptDialog({ device, target, report, shown, actions, onClose }: { device: Device; target: AdoptTarget; report: StoreReport; shown: ReadonlySet<string>; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const read = useStoreRequest(device.id, actions);
  const apply = useStoreRequest(device.id, actions);
  const asked = useRef(false);
  const reads = target.kind === "skill" && target.path !== null;
  useEffect(() => {
    if (!reads || asked.current || target.path === null) return;
    asked.current = true;
    read.send([{ op: "inspect", source: { kind: "folder", path: target.path } }]);
  }, [reads, read, target.path]);
  useEffect(() => {
    if (apply.done) onClose();
  }, [apply.done, onClose]);
  const adopt = (confirmed: StoreAdoptConfirmation) => apply.send([{ op: "adopt", kind: target.kind, name: target.name, ...(target.from ? { from: target.from } : {}), confirmed }]);
  const title = t("store.adopt.title", { name: target.name });
  const header = <p className="text-body text-muted-foreground">{t("store.adopt.line")}</p>;
  // The preview this dialog's own read made, as its answer names it: another pane's read never replaces it.
  const preview = reads && read.done ? (report.previews.find((row) => row.id === read.made) ?? null) : null;
  const skill = preview?.skills.find((skill) => skill.name === target.name);
  const lost = reads && read.done && (!preview || !skill);
  if (preview && skill) {
    return <InstallConfirm title={title} data="adopt" preview={preview} skills={[skill]} shown={shown} pending={apply.pending} refusal={apply.refusal} onClose={onClose} onInstall={() => adopt({ kind: "skill", digest: skill.digest })} confirmLabel={t("store.adopt.apply")} header={header} />;
  }
  const agents = (report.columns[target.kind] ?? []).flatMap((column) => column.agents).filter((agent, index, all) => shown.has(agent) && all.indexOf(agent) === index);
  if (target.kind === "mcp" && target.server) {
    const names = keyNames(target.server);
    return (
      <McpConfirm
        data="adopt"
        title={title}
        header={header}
        server={target.server}
        keys={{}}
        pasted={null}
        shown={new Set(agents)}
        pending={apply.pending}
        refusal={apply.refusal}
        confirmLabel={t("store.adopt.apply")}
        onClose={onClose}
        onConfirm={() => adopt({ kind: "mcp", server: target.server! })}
        keyEntry={<span className="flex flex-col font-mono">{names.map((key) => <span key={key}>{key}</span>)}</span>}
      />
    );
  }
  const unreadable = target.kind === "mcp";
  const reading = reads && !read.refusal && !lost;
  return (
    <StoreDialog
      title={title}
      onClose={onClose}
      data="adopt"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} data-store-cancel="true">{t("common.cancel")}</Button>
        </>
      }
    >
      {header}
      {reading ? (
        <span className="text-body text-muted-foreground">{t("store.reading")}</span>
      ) : (
        <Facts>
          {target.path !== null ? <Fact label={t("store.detail.source")} mono>{target.path}</Fact> : null}
          <Fact label={t("store.detail.agents")}><AgentChips agents={agents} /></Fact>
        </Facts>
      )}
      {read.refusal || lost ? <Note tone="error" data-store-adopt-refused={read.refusal ?? "not_found"}>{refusalText(read.refusal ?? "not_found", t)}</Note> : null}
      {unreadable ? <Note tone="error" data-store-adopt-unreadable="true">{t("store.adopt.unreadable")}</Note> : null}
      {!reads && !unreadable ? <Note tone="error">{refusalText("not_found", t)}</Note> : null}
      {apply.refusal ? <Note tone="error" data-store-install-refused={apply.refusal}>{refusalText(apply.refusal, t)}</Note> : null}
    </StoreDialog>
  );
}

/** A server's fields and its keys' values, as one row each: the name, the value typed and then hidden (B33). */
function KeyFields({ names, keys, onChange, kept = false }: { names: string[]; keys: Record<string, string>; onChange: (keys: Record<string, string>) => void; kept?: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <span className="flex flex-col gap-xs">
      {names.map((key) => (
        <span key={key} className="flex items-center gap-sm">
          <span className="w-2/5 min-w-0 truncate font-mono text-subtle-foreground">{key}</span>
          <Input
            type="password"
            autoComplete="off"
            value={keys[key] ?? ""}
            placeholder={kept ? t("store.mcp.keyKept") : t("store.mcp.keyValue")}
            onChange={(event) => onChange({ ...keys, [key]: event.target.value })}
            aria-label={`${key} ${t("store.mcp.keyValue")}`}
            className="flex-1"
            data-store-key={key}
          />
        </span>
      ))}
    </span>
  );
}

/**
 * MCP 추가: the paste read as a name, a kind, a command and its arguments or
 * an address, and its keys (scene 3, left). The command and each argument
 * are fields of their own, as the paste parser read them, so an argument
 * with a space in it (a folder path, `C:\\Program Files`) stays one.
 */
export function McpFields({ report, server, pasted, onClose, onNext }: { report: StoreReport; server: McpServer; pasted: McpPastedServer | null; onClose: () => void; onNext: (server: McpServer, keys: Record<string, string>) => void }) {
  const { t } = useInterfaceTranslation();
  const [title, setTitle] = useState(server.name);
  const [reach, setReach] = useState<McpReach>(server.reach);
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [extra, setExtra] = useState("");
  // Which argument rows the operator added here: one left empty is no
  // argument, while an empty argument the server was read with stays one.
  const [added, setAdded] = useState<boolean[]>([]);
  const names = keyNames({ name: title, reach });
  const taken = report.items.some((item) => item.kind === "mcp" && item.name === title.trim());
  const args = reach.kind === "local" ? (reach.args ?? []) : [];
  const fresh = args.map((_, at) => added[at] === true);
  const setArgs = (next: string[], nextFresh: boolean[]) => {
    setReach((now) => (now.kind === "local" ? { ...now, args: next } : now));
    setAdded(nextFresh);
  };
  const reached = reach.kind === "local" ? reach.command.trim() : reach.url.trim();
  const built = (): McpServer => {
    if (reach.kind === "remote") return { name: title.trim(), reach: { ...reach, url: reach.url.trim() } };
    return { name: title.trim(), reach: { ...reach, command: reach.command.trim(), args: args.filter((arg, at) => !(fresh[at] && arg === "")) } };
  };
  const addKey = () => {
    const key = extra.trim();
    if (!key || names.includes(key)) return;
    setReach((now) => (now.kind === "local" ? { ...now, env: [...(now.env ?? []), key] } : { ...now, headers: [...(now.headers ?? []), key] }));
    setExtra("");
  };
  return (
    <StoreDialog
      title={t("store.mcp.title")}
      onClose={onClose}
      data="mcp"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
          <Button disabled={!title.trim() || !reached || taken} onClick={() => onNext(built(), Object.fromEntries(Object.entries(keys).filter(([key, value]) => value && names.includes(key))))} data-store-mcp-next="true">
            {t("store.add.next")}
          </Button>
        </>
      }
    >
      <Facts>
        <Fact label={t("store.mcp.name")}>
          <Input mono value={title} onChange={(event) => setTitle(event.target.value)} aria-label={t("store.mcp.name")} data-store-mcp-name="true" />
          {taken ? <span className="mt-xs block text-caption text-destructive">{t("store.refused.name_taken")}</span> : null}
        </Fact>
        <Fact label={t("store.mcp.kind")}>
          <Select value={reach.kind === "local" ? "local" : reach.transport} onValueChange={(value) => setReach(value === "local" ? { kind: "local", command: "", args: [], env: reach.kind === "local" ? reach.env : [] } : { kind: "remote", transport: value as "http" | "sse", url: "", headers: reach.kind === "remote" ? reach.headers : [] })}>
            <SelectTrigger size="sm" aria-label={t("store.mcp.kind")} data-store-mcp-kind="true"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="local">{t("store.mcp.local")}</SelectItem>
              <SelectItem value="http">{t("store.mcp.remote")}</SelectItem>
              <SelectItem value="sse">{`${t("store.mcp.remote")} · ${t("store.mcp.sse")}`}</SelectItem>
            </SelectContent>
          </Select>
        </Fact>
        {reach.kind === "local" ? (
          <>
            <Fact label={t("store.mcp.command")}>
              <Input mono value={reach.command} onChange={(event) => setReach({ ...reach, command: event.target.value })} aria-label={t("store.mcp.command")} data-store-mcp-command="true" />
            </Fact>
            <Fact label={t("store.mcp.args")}>
              <span className="flex flex-col gap-xs">
                {args.map((arg, index) => (
                  <span key={index} className="flex items-center gap-xs">
                    <Input mono value={arg} onChange={(event) => setArgs(args.map((kept, at) => (at === index ? event.target.value : kept)), fresh)} aria-label={t("store.mcp.argNumbered", { n: index + 1 })} data-store-mcp-arg={index} />
                    <Button variant="row-action" size="icon-xs" aria-label={t("store.mcp.removeArg", { n: index + 1 })} onClick={() => setArgs(args.filter((_, at) => at !== index), fresh.filter((_, at) => at !== index))} data-store-mcp-arg-remove={index}>
                      <XIcon aria-hidden="true" />
                    </Button>
                  </span>
                ))}
                <span>
                  <Button variant="secondary" size="sm" onClick={() => setArgs([...args, ""], [...fresh, true])} data-store-mcp-arg-add="true">
                    <PlusIcon aria-hidden="true" />
                    {t("store.mcp.addArg")}
                  </Button>
                </span>
              </span>
            </Fact>
          </>
        ) : (
          <Fact label={t("store.mcp.url")}>
            <Input mono value={reach.url} onChange={(event) => setReach({ ...reach, url: event.target.value })} aria-label={t("store.mcp.url")} data-store-mcp-url="true" />
          </Fact>
        )}
        <Fact label={t(reach.kind === "local" ? "store.mcp.env" : "store.mcp.headers")}>
          <KeyFields names={names} keys={keys} onChange={setKeys} />
          <span className="mt-xs flex items-center gap-sm">
            <Input mono value={extra} placeholder={t("store.mcp.keyName")} onChange={(event) => setExtra(event.target.value)} aria-label={t("store.mcp.keyName")} className="w-2/5" data-store-mcp-extra="true" />
            <Button variant="secondary" size="sm" disabled={!extra.trim()} onClick={addKey}>{t("store.mcp.addKey")}</Button>
          </span>
        </Fact>
      </Facts>
      {pasted?.taken ? <Note tone="error">{t("store.refused.name_taken")}</Note> : null}
    </StoreDialog>
  );
}

/** MCP 추가: name — how it is reached, its keys hidden, the agents it reaches with those that cannot struck out (scene 3, right). */
function McpConfirm({ server, keys, pasted, shown, pending, refusal, confirmLabel, title, onClose, onConfirm, header, keyEntry, data = "mcp-confirm" }: { server: McpServer; keys: Record<string, string>; pasted: McpPastedServer | null; shown: ReadonlySet<string>; pending: boolean; refusal: StoreRefusalCause | "unavailable" | null; confirmLabel: string; title?: string; onClose: () => void; onConfirm: () => void; header?: ReactNode; keyEntry?: ReactNode; data?: string }) {
  const { t } = useInterfaceTranslation();
  const every = [...shown];
  const reaching = pasted?.agents?.filter((agent) => shown.has(agent));
  const left = reaching ? every.filter((agent) => !reaching.includes(agent)) : [];
  const names = keyNames(server);
  return (
    <StoreDialog
      title={title ?? t("store.mcp.titleNamed", { name: server.name })}
      onClose={onClose}
      data={data}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} data-store-cancel="true">{t("common.cancel")}</Button>
          <Button disabled={pending} aria-busy={pending} onClick={onConfirm} data-store-mcp-add="true">{confirmLabel}</Button>
        </>
      }
    >
      {header}
      <Facts>
        <Fact label={t(server.reach.kind === "local" ? "store.detail.command" : "store.detail.address")} mono>{serverLine(server)}</Fact>
        {names.length > 0 ? (
          <Fact label={t("store.detail.keys")}>
            {keyEntry ?? (
              <span className="flex flex-col font-mono">
                {names.map((key) => <span key={key}>{key} {keys[key] ? "••••••" : <span className="font-sans text-muted-foreground">·</span>}</span>)}
              </span>
            )}
          </Fact>
        ) : null}
        <Fact label={t("store.detail.agents")}>
          <AgentChips agents={every} left={left} />
          {left.length > 0 ? <span className="mt-xs block text-caption text-muted-foreground">{t("store.mcp.unsupported")}</span> : null}
        </Fact>
      </Facts>
      {refusal ? <Note tone="error" data-store-mcp-refused={refusal}>{refusalText(refusal, t)}</Note> : null}
    </StoreDialog>
  );
}

/** 편집 on a server's keys: each key's value typed again; one left empty is kept as it is. */
export function KeysDialog({ device, item, report, actions, onClose }: { device: Device; item: StoreItem; report: StoreReport; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const apply = useStoreRequest(device.id, actions);
  const [keys, setKeys] = useState<Record<string, string>>({});
  useEffect(() => {
    if (apply.done) onClose();
  }, [apply.done, onClose]);
  const server = item.server ?? report.items.find((row) => row.id === item.id)?.server;
  if (!server) return null;
  const typed = Object.fromEntries(Object.entries(keys).filter(([, value]) => value));
  return (
    <StoreDialog
      title={t("store.mcp.editTitle", { name: item.name })}
      onClose={onClose}
      data="keys"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
          <Button disabled={apply.pending || Object.keys(typed).length === 0} aria-busy={apply.pending} onClick={() => apply.send([{ op: "set_mcp", item: item.id, server, keys: typed }])} data-store-keys-save="true">
            {t("store.mcp.save")}
          </Button>
        </>
      }
    >
      <Facts>
        <Fact label={t(server.reach.kind === "local" ? "store.detail.command" : "store.detail.address")} mono>{serverLine(server)}</Fact>
        <Fact label={t("store.detail.keys")}><KeyFields names={keyNames(server)} keys={keys} onChange={setKeys} kept /></Fact>
      </Facts>
      {apply.refusal ? <Note tone="error">{refusalText(apply.refusal, t)}</Note> : null}
    </StoreDialog>
  );
}

/** 파일 보기: an item's files, each executable marked. */
export function FilesDialog({ item, onClose }: { item: StoreItem; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <StoreDialog title={item.name} onClose={onClose} data="files" footer={<Button variant="secondary" onClick={onClose}>{t("common.close")}</Button>}>
      <FileList files={item.files ?? []} max={200} />
    </StoreDialog>
  );
}

/** An update's two versions and commit subjects, as confirmed on 2026-11-20. */
function DiffFacts({ diff }: { diff: StoreUpdateDiff }) {
  const { t } = useInterfaceTranslation();
  const summary = diff.summary.slice(0, 3);
  return (
    <Facts>
      <Fact label={t("store.update.version")} mono>{`${versionValue(diff.from)} → ${versionValue(diff.to)}`}</Fact>
      {summary.length > 0 ? (
        <Fact label={t("store.update.summary")}>
          <span className="flex flex-col">
            {summary.map((line, index) => <span key={index} className="break-words">{line}</span>)}
            {diff.summary.length > summary.length ? <span className="text-caption text-muted-foreground">{t("store.update.more", { count: diff.summary.length - summary.length })}</span> : null}
          </span>
        </Fact>
      ) : null}
    </Facts>
  );
}

/**
 * 업데이트 (scene 2-1): the items with a newer version, one at a time, each
 * read first and shown as what changes before it is taken.
 */
export function UpdateDialog({ device, queue, report, actions, onClose }: { device: Device; queue: string[]; report: StoreReport; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const [at, setAt] = useState(0);
  const read = useStoreRequest(device.id, actions);
  const apply = useStoreRequest(device.id, actions);
  const id = queue[at];
  const item = report.items.find((row) => row.id === id) ?? null;
  // The diff this item's own read made, as its answer names it.
  const diff = read.done ? (report.diffs.find((row) => row.id === read.made && row.item === id) ?? null) : null;
  const lost = read.done && !diff;
  const { send: readSend } = read;
  useEffect(() => {
    if (!id) return;
    readSend([{ op: "preview_update", item: id }]);
    // Each item is read once when it comes up.
  }, [id, readSend]);
  const { done: applied, reset } = apply;
  useEffect(() => {
    if (!applied) return;
    reset();
    if (at + 1 < queue.length) setAt(at + 1);
    else onClose();
  }, [applied, reset, at, queue.length, onClose]);
  if (!id || !item) return null;
  const left = queue.length - at - 1;
  return (
    <StoreDialog
      title={t("store.update.title", { name: item.name })}
      onClose={onClose}
      data="update"
      footer={
        <>
          {left > 0 ? <span className="mr-auto text-caption text-muted-foreground">{t("store.update.left", { count: left })}</span> : null}
          <Button variant="ghost" onClick={() => (left > 0 ? setAt(at + 1) : onClose())} data-store-cancel="true">{t("common.cancel")}</Button>
          <Button disabled={!diff || apply.pending} aria-busy={apply.pending} onClick={() => diff && apply.send([{ op: "update", item: id, commit: diff.commit }])} data-store-update-confirm="true">
            {t("store.update.confirm")}
          </Button>
        </>
      }
    >
      {diff ? <DiffFacts diff={diff} /> : read.refusal || lost ? null : <p className="text-body text-muted-foreground">{t("store.reading")}</p>}
      {read.refusal || apply.refusal || lost ? <Note tone="error">{refusalText(read.refusal ?? apply.refusal ?? "not_found", t)}</Note> : null}
    </StoreDialog>
  );
}

/**
 * The confirm an agent's `hide skill add` / `update` or `hide mcp add` waits
 * on (B32, scene 4): what it would install or change, with [설치] and
 * [취소]; for an MCP the operator types the keys here, never the agent.
 */
export function AskDialog({ ask, report, shown, actions, onClose }: { ask: StoreAsk; report: StoreReport; shown: ReadonlySet<string>; actions: Actions; onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  const [keys, setKeys] = useState<Record<string, string>>({});
  const answer = (confirm: boolean, detail: { names?: string[]; keys?: Record<string, string> } = {}) => {
    actions.answerStoreAsk(ask.id, confirm, detail);
    onClose();
  };
  const decline = () => answer(false);
  if (ask.what === "install") {
    const preview = report.previews.find((row) => row.id === ask.preview);
    if (!preview) return <Gone onClose={onClose} />;
    return <AskInstall preview={preview} names={ask.names} shown={shown} onDecline={decline} onConfirm={(names) => answer(true, { names })} />;
  }
  if (ask.what === "update") {
    const diff = report.diffs.find((row) => row.id === ask.diff);
    const item = report.items.find((row) => row.id === ask.item);
    if (!diff) return <Gone onClose={onClose} />;
    return (
      <StoreDialog
        title={t("store.ask.update", { name: item?.name ?? ask.item })}
        onClose={onClose}
        data="ask-update"
        footer={
          <>
            <Button variant="ghost" onClick={decline} data-store-ask-decline={ask.id}>{t("common.cancel")}</Button>
            <Button onClick={() => answer(true)} data-store-ask-confirm={ask.id}>{t("store.ask.confirmUpdate")}</Button>
          </>
        }
      >
        <DiffFacts diff={diff} />
      </StoreDialog>
    );
  }
  const server = ask.server;
  return (
    <McpConfirm
      server={server}
      keys={keys}
      pasted={null}
      shown={shown}
      pending={false}
      refusal={null}
      title={t(ask.what === "add_mcp" ? "store.ask.addMcp" : "store.ask.setMcp", { name: server.name })}
      confirmLabel={ask.what === "add_mcp" ? t("store.ask.confirmMcp") : t("store.mcp.save")}
      onClose={decline}
      onConfirm={() => answer(true, { keys: Object.fromEntries(Object.entries(keys).filter(([, value]) => value)) })}
      keyEntry={<KeyFields names={keyNames(server)} keys={keys} onChange={setKeys} kept={ask.what === "set_mcp"} />}
    />
  );
}

function AskInstall({ preview, names, shown, onDecline, onConfirm }: { preview: StorePreview; names: string[]; shown: ReadonlySet<string>; onDecline: () => void; onConfirm: (names: string[]) => void }) {
  const { t } = useInterfaceTranslation();
  const [ticked, setTicked] = useState(names);
  const skills = preview.skills.filter((skill) => ticked.includes(skill.name));
  return (
    <InstallConfirm
      preview={preview}
      skills={skills}
      shown={shown}
      pending={false}
      refusal={null}
      onClose={onDecline}
      onInstall={() => onConfirm(ticked)}
      confirmLabel={t("store.ask.confirmInstall")}
      header={
        names.length > 1 ? (
          <div className="flex flex-col gap-xs">
            {names.map((name) => (
              <label key={name} className="flex items-center gap-sm text-body" data-store-ask-pick={name}>
                <Checkbox checked={ticked.includes(name)} onCheckedChange={(checked) => setTicked((now) => (checked === true ? [...now, name] : now.filter((one) => one !== name)))} />
                {name}
              </label>
            ))}
          </div>
        ) : null
      }
    />
  );
}

function Gone({ onClose }: { onClose: () => void }) {
  const { t } = useInterfaceTranslation();
  return (
    <StoreDialog title={t("store.ask.gone")} onClose={onClose} data="ask-gone" footer={<Button variant="secondary" onClick={onClose}>{t("common.close")}</Button>}>
      <p className="text-body text-muted-foreground">{t("store.ask.gone")}</p>
    </StoreDialog>
  );
}
