"""The stand-in agents of a release-state container, started as `claude` or
`pi` (`agent.c` passes the name it was started as first).

As `claude` it reports its session through the release's own hook program as Claude Code
does (SessionStart, then UserPromptSubmit and Stop around each turn), keeps a
minimal Claude transcript under `~/.claude/projects/`, and draws Claude's
prompt box so the engine reads it as waiting for input.

Started as Hide AI starts Claude Code (`claude auth status`, `claude -p ...`)
it answers at once: signed in, a short model list, and for a request bound
to a JSON schema an answer of that schema, so the release's Memory writer
and labels have something to store. It never reaches a model.

A line typed into it that starts with `!` is run as a shell command with the
pane's environment, the way an agent runs `hide ...`; its result is appended
to `/tmp/release-state-agents/<session>.jsonl` for the scenario to read. Any
other line is a prompt: recorded and answered with an empty turn.

As `pi` it does what Pi with Hide's extension does in a pane (`pi_main`).
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import uuid

HOOKS = [Path.home() / ".hide/host-helper/current/hide-agent-hooks", Path("/opt/hide/hide-agent-hooks")]
PATH_EXTRA = [str(Path.home() / ".local/bin"), "/opt/hide"]
VERSION = "2.1.300 (Claude Code)"


def arguments(argv):
    """(session, resumed) from Claude's own options; any other is ignored."""
    session, resumed = None, False
    for index, word in enumerate(argv):
        if word in ("--session-id", "--resume") and index + 1 < len(argv):
            session, resumed = argv[index + 1], word == "--resume"
    return session or str(uuid.uuid4()), resumed


def instance(shape, text, defs):
    """A value of the JSON schema `shape`: every array holds one item."""
    if "$ref" in shape:
        shape = defs.get(shape["$ref"].rsplit("/", 1)[-1], {})
    if "const" in shape:
        return shape["const"]
    if shape.get("enum"):
        return shape["enum"][0]
    for key in ("anyOf", "oneOf", "allOf"):
        if shape.get(key):
            choices = [choice for choice in shape[key] if choice.get("type") != "null"]
            return instance(choices[0] if choices else shape[key][0], text, defs)
    kind = shape.get("type", "object")
    if isinstance(kind, list):
        kind = next((name for name in kind if name != "null"), "string")
    if kind == "object":
        properties = shape.get("properties") or {}
        return {name: instance(value, text, defs) for name, value in properties.items()}
    if kind == "array":
        count = max(1, shape.get("minItems") or 0) if shape.get("items") else 0
        return [instance(shape["items"], text, defs) for _ in range(min(count, shape.get("maxItems") or count))]
    if kind == "string":
        return text[:shape.get("maxLength") or len(text)]
    if kind == "boolean":
        return False
    if kind in ("integer", "number"):
        return shape.get("minimum", 1)
    return None


def first_card(value):
    """Project Memory's answer as a first card: one new candidate from the
    one event it was given, and nothing related to a card it was not shown
    (every other list of records is empty)."""
    if isinstance(value, dict):
        fixed = {}
        for key, item in value.items():
            if isinstance(item, list) and item and isinstance(item[0], dict) and key != "candidates":
                fixed[key] = []
            else:
                fixed[key] = first_card(item)
        fixed.update({key: replacement for key, replacement in
                      (("relation", "new"), ("target_id", None), ("source_offsets", [0]))
                      if key in fixed})
        return fixed
    if isinstance(value, list):
        return [first_card(item) for item in value]
    return value


def print_mode(argv):
    """Answers a non-interactive start the way Claude Code's frames read."""
    if argv[:2] == ["auth", "status"]:
        print(json.dumps({"loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty"}))
        return 0
    if "--input-format" in argv:
        sys.stdin.readline()
        models = [{"value": "default"}, {"value": "sonnet"}, {"value": "opus"}]
        print(json.dumps({"type": "control_response", "response": {
            "subtype": "success", "request_id": "hide-models", "response": {"models": models}}}))
        return 0
    if "/usage" not in argv:
        sys.stdin.read()
    result = {"type": "result", "subtype": "success", "is_error": False, "result": "{}"}
    if "--json-schema" in argv:
        shape = json.loads(argv[argv.index("--json-schema") + 1])
        defs = {**shape.get("definitions", {}), **shape.get("$defs", {})}
        result["structured_output"] = first_card(
            instance(shape, "The release-state project keeps its notes in notes.md", defs))
    print(json.dumps(result))
    return 0


def main():
    persona, argv = sys.argv[1], sys.argv[2:]
    if persona == "pi":
        return pi_main(argv)
    if "--version" in argv or "-v" in argv:
        print(VERSION)
        return 0
    if argv[:1] == ["auth"] or "-p" in argv or "--print" in argv:
        return print_mode(argv)
    session, resumed = arguments(argv)
    hooks = next((str(path) for path in HOOKS if path.exists()), None)
    if hooks is None:
        print("no hook program", file=sys.stderr)
        return 1
    cwd = os.getcwd()
    transcript = Path.home() / ".claude/projects" / re.sub(r"[^A-Za-z0-9]", "-", cwd) / f"{session}.jsonl"
    transcript.parent.mkdir(parents=True, exist_ok=True)
    results = Path("/tmp/release-state-agents") / f"{session}.jsonl"
    results.parent.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "PATH": ":".join(PATH_EXTRA + [os.environ.get("PATH", "/usr/bin:/bin")])}

    def hook(event, **extra):
        payload = dict(session_id=session, transcript_path=str(transcript), hook_event_name=event, cwd=cwd, **extra)
        subprocess.run([hooks, "hook", "--runtime", "claude-code", "--event", event],
                       input=json.dumps(payload), text=True, capture_output=True, timeout=60, env=env)

    def record(kind, text):
        stamp = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
        common = dict(sessionId=session, cwd=cwd, timestamp=stamp, uuid=str(uuid.uuid4()), parentUuid=None,
                      isSidechain=False, userType="external", entrypoint="cli", version=VERSION.split()[0])
        if kind == "user":
            line = dict(type="user", message=dict(role="user", content=text), promptSource="user", **common)
        else:
            line = dict(type="assistant", message=dict(role="assistant", model="claude-model", type="message",
                                                       content=[dict(type="text", text=text)], stop_reason="end_turn"),
                        **common)
        with transcript.open("a") as file:
            file.write(json.dumps(line) + "\n")

    def prompt_box():
        print("─" * 40 + "\n❯ \n" + "─" * 40, flush=True)

    hook("SessionStart", source="resume" if resumed else "startup")
    prompt_box()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        hook("UserPromptSubmit", prompt=line)
        record("user", line)
        if line.startswith("!"):
            command = line[1:].strip()
            done = subprocess.run(["bash", "-c", command], text=True, capture_output=True, env=env, cwd=cwd)
            with results.open("a") as file:
                file.write(json.dumps(dict(command=command, rc=done.returncode, out=done.stdout, err=done.stderr)) + "\n")
            print(f"$ {command}\n{done.stdout}{done.stderr}[rc={done.returncode}]", flush=True)
            record("assistant", f"ran it: rc={done.returncode}")
        else:
            record("assistant", "noted")
        hook("Stop", stop_hook_active=False)
        prompt_box()
    hook("SessionEnd", reason="other")
    return 0


# The extension's version, which the released helper answers only when it is
# its own: `hide_agent_hooks::pi_extension::VERSION` in 1.2.3 and 1.2.4.
PI_EXTENSION = 15
PI_VERSION = "1.0.4"


def pi_main(argv):
    """A stand-in Pi 1.0.4 with Hide's extension, for the sleep that closes
    the pane (Pi's dormant sleep): Pi's session file in its default folder
    for the checkout (`~/.pi/agent/sessions/--<cwd>--/<time>_<id>.jsonl`,
    a version 3 header naming the id and cwd, then the turns as messages),
    which the release's own reader proves the session by; the session and
    each turn told to the release's helper as Hide's extension tells it
    (`hide-agent-hooks pi session|turn`, the file's path as the session);
    and Pi's idle screen. `--session <id>` resumes that session's file.
    A line typed into it is a prompt, a line starting with `!` run as
    `agent.py` runs it."""
    if "--version" in argv or "-v" in argv:
        print(PI_VERSION)
        return 0
    hooks = next((str(path) for path in HOOKS if path.exists()), None)
    if hooks is None:
        print("no hook program", file=sys.stderr)
        return 1
    cwd = os.getcwd()
    folder = Path.home() / ".pi/agent/sessions" / ("--" + cwd.lstrip("/").replace("/", "-") + "--")
    folder.mkdir(parents=True, exist_ok=True)
    resumed = argv[argv.index("--session") + 1] if "--session" in argv[:-1] else None
    session = resumed or str(uuid.uuid4())
    found = sorted(folder.glob(f"*_{session}.jsonl")) if resumed else []
    if found:
        transcript = found[0]
    else:
        transcript = folder / (time.strftime("%Y-%m-%dT%H-%M-%S-000Z", time.gmtime()) + f"_{session}.jsonl")
        with transcript.open("w") as file:
            file.write(json.dumps(dict(type="session", version=3, id=session, timestamp=stamp(), cwd=cwd)) + "\n")
    results = Path("/tmp/release-state-agents") / f"{session}.jsonl"
    results.parent.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "PATH": ":".join(PATH_EXTRA + [os.environ.get("PATH", "/usr/bin:/bin")])}
    parent = [None]

    def helper(operation, **fields):
        subprocess.run([hooks, "pi", operation], input=json.dumps(dict(version=PI_EXTENSION, **fields)),
                       text=True, capture_output=True, timeout=30, env=env)

    def record(role, text):
        entry = uuid.uuid4().hex[:8]
        message = dict(role=role, content=[dict(type="text", text=text)], timestamp=int(time.time() * 1000))
        if role == "assistant":
            message.update(provider="stand-in", model="stand-in", stopReason="stop")
        with transcript.open("a") as file:
            file.write(json.dumps(dict(type="message", id=entry, parentId=parent[0], timestamp=stamp(),
                                       message=message)) + "\n")
        parent[0] = entry

    def idle_screen():
        rule = "─" * 40
        print(f"{rule}\n\n{rule}\n~/{Path(cwd).name} (main)\n$0.000 0.0%/200k (auto)    stand-in", flush=True)

    helper("session", session_id=str(transcript))
    idle_screen()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        helper("turn", session_id=str(transcript), turn="working")
        record("user", line)
        if line.startswith("!"):
            command = line[1:].strip()
            done = subprocess.run(["bash", "-c", command], text=True, capture_output=True, env=env, cwd=cwd)
            with results.open("a") as file:
                file.write(json.dumps(dict(command=command, rc=done.returncode, out=done.stdout, err=done.stderr)) + "\n")
            print(f"$ {command}\n{done.stdout}{done.stderr}[rc={done.returncode}]", flush=True)
            record("assistant", f"ran it: rc={done.returncode}")
        else:
            record("assistant", "noted")
        helper("turn", session_id=str(transcript), turn="done")
        idle_screen()
    return 0


def stamp():
    return time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())


if __name__ == "__main__":
    sys.exit(main())
