#!/bin/bash
# PreToolUse hook for Claude Code, Codex and OpenCode: an agent session reads
# docs/DESIGN_GUIDE.md before its first change to a screen or a token of this
# repository. A read of the guide (the Read tool, or a shell command naming it)
# records the session; an edit of `web/src/**/*.tsx`, `web/src/**/*.css` or
# `design/tokens.json` in any worktree of this repository is refused, with the
# reason returned to the agent, until the session has that record.
#
# Input: the hook JSON on stdin. Exit 2 with the reason on stderr refuses the
# call; any other exit lets it run. Codex registers hooks globally, so the
# script scopes itself to the repository it is checked into. A shell command
# that writes a file is not judged here; `check-ui-parts.mjs` judges what lands.
set -u

input=$(cat)
# Most calls name neither the guide nor a screen file: answer them without
# starting python.
case "$input" in
  *DESIGN_GUIDE.md*|*.tsx*|*.css*|*tokens.json*) ;;
  *) exit 0 ;;
esac
script_dir=$(cd "$(dirname "$0")" && pwd -P)
verdict=$(HOOK_INPUT="$input" GUARDED_ROOT="$(dirname "$(dirname "$script_dir")")" python3 - <<'PY'
import hashlib, json, os, re, subprocess, sys, time

try:
    p = json.loads(os.environ.get("HOOK_INPUT", ""))
except Exception:
    sys.exit(0)
if p.get("hook_event_name") not in (None, "PreToolUse"):
    sys.exit(0)
session = p.get("session_id") or ""
if not session:
    sys.exit(0)
tool = p.get("tool_name") or ""
ti = p.get("tool_input") or {}
cwd = p.get("cwd") or os.getcwd()

READ_TOOLS = {"Read", "read"}
SHELL_TOOLS = {"Bash", "bash", "shell", "exec_command"}
EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch", "edit", "write", "multiedit", "patch"}
PATH_KEYS = ("file_path", "filePath", "path", "notebook_path")
PATCH_KEYS = ("command", "patchText", "input", "patch")
GUIDE = "DESIGN_GUIDE.md"
MARKER_DAYS = 14

def git(*args, cwd):
    try:
        out = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, timeout=5)
        return out.stdout.strip() if out.returncode == 0 else ""
    except Exception:
        return ""

def common_dir(path):
    """The repository's shared git directory and the worktree top for a path,
    which may not exist yet (a new file in a new folder)."""
    d = path if os.path.isdir(path) else os.path.dirname(path)
    while d and not os.path.isdir(d):
        d = os.path.dirname(d)
    if not d:
        return None, None
    top = git("rev-parse", "--show-toplevel", cwd=d)
    common = git("rev-parse", "--git-common-dir", cwd=d)
    if not top or not common:
        return None, None
    return os.path.realpath(os.path.join(d, common)), os.path.realpath(top)

guarded_common, _ = common_dir(os.environ.get("GUARDED_ROOT", ""))
if not guarded_common:
    sys.exit(0)
markers = os.path.join(guarded_common, "hide-design-guide")
marker = os.path.join(markers, hashlib.sha256(session.encode()).hexdigest()[:32])

def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for v in value:
            yield from strings(v)

def record_read():
    os.makedirs(markers, exist_ok=True)
    now = time.time()
    with open(marker, "w") as f:
        f.write(f"{int(now)}\n")
    # A marker outlives its session only by the cap; older ones are dropped.
    for name in os.listdir(markers):
        full = os.path.join(markers, name)
        try:
            if now - os.path.getmtime(full) > MARKER_DAYS * 86400:
                os.remove(full)
        except OSError:
            pass

if tool in READ_TOOLS:
    if any(str(ti.get(k, "")).endswith(GUIDE) for k in PATH_KEYS):
        record_read()
    sys.exit(0)
if tool in SHELL_TOOLS:
    if any(GUIDE in s for s in strings(ti.get("command", ""))):
        record_read()
    sys.exit(0)
if tool not in EDIT_TOOLS or os.path.exists(marker):
    sys.exit(0)

paths = [ti[k] for k in PATH_KEYS if isinstance(ti.get(k), str)]
for key in PATCH_KEYS:
    for text in strings(ti.get(key, "")):
        paths += re.findall(r"^\*\*\* (?:Add|Update) File: (.+?)\s*$", text, re.M)

def gated(rel):
    if rel == "design/tokens.json":
        return True
    if not rel.startswith("web/src/") or rel.endswith(".test.tsx"):
        return False
    return rel.endswith(".tsx") or rel.endswith(".css")

for raw in paths:
    full = os.path.normpath(os.path.join(cwd, os.path.expanduser(raw)))
    common, top = common_dir(full)
    if common != guarded_common:
        continue
    rel = os.path.relpath(os.path.realpath(os.path.dirname(full)), top)
    rel = os.path.normpath(os.path.join(rel, os.path.basename(full))).replace(os.sep, "/")
    if gated(rel):
        print(f"Read docs/{GUIDE} first: this session has not read it, and `{rel}` is a screen or a token.\n"
              f"It names the part to build from and the rules `check-ui-parts.mjs` enforces.\n"
              f"Read it in full (for example `cat {os.path.join(top, 'docs', GUIDE)}`), then retry this edit.")
        sys.exit(2)
PY
)
rc=$?
if [ "$rc" -eq 2 ]; then
  printf '%s\n' "$verdict" >&2
  exit 2
fi
exit 0
