#!/bin/bash
# PreToolUse hook for Claude Code, Codex and OpenCode: an edit that would leave
# a Markdown file of this repository over the docs cap, and larger than it is
# now, is refused before it lands, with the reason returned to the agent.
#
# The cap and the exception list are LIMIT and EXCEPTIONS in
# scripts/check-docs.py of the worktree the file is in, which CI runs as the
# final gate. A file on the list, an edit that does not grow the file, a file
# git ignores, a file outside this repository, and an edit the hook cannot
# size (its old text is not in the file) pass. A Codex or OpenCode patch is
# sized from its hunk lines.
#
# Input: the hook JSON on stdin. Exit 2 with the reason on stderr refuses the
# call; any other exit lets it run. Codex registers hooks globally, so the
# script scopes itself to the repository it is checked into. A shell command
# that writes a file is not judged here; `check-docs.py` judges what lands.
set -u

input=$(cat)
# Most calls name no Markdown file: answer them without starting python.
case "$input" in
  *.md*) ;;
  *) exit 0 ;;
esac
script_dir=$(cd "$(dirname "$0")" && pwd -P)
verdict=$(printf '%s' "$input" | GUARDED_ROOT="$(dirname "$(dirname "$script_dir")")" python3 -c '
import importlib.util, json, os, re, subprocess, sys

EDIT_TOOLS = {"Edit", "Write", "MultiEdit", "apply_patch", "edit", "write", "multiedit", "patch"}
PATH_KEYS = ("file_path", "filePath", "path")
PATCH_KEYS = ("command", "patchText", "input", "patch")
FILE_LINE = re.compile(r"^\*\*\* (Add|Update|Delete) File: (.+?)\s*$")
MOVE_LINE = re.compile(r"^\*\*\* Move to: (.+?)\s*$")

def git(*args, cwd):
    try:
        out = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, timeout=5)
        return out.returncode, out.stdout.strip()
    except Exception:
        return 1, ""

def common_dir(path):
    """The repository shared git directory and the worktree top for a path,
    which may not exist yet (a new file in a new folder)."""
    d = path if os.path.isdir(path) else os.path.dirname(path)
    while d and not os.path.isdir(d):
        d = os.path.dirname(d)
    if not d:
        return None, None
    rc1, top = git("rev-parse", "--show-toplevel", cwd=d)
    rc2, common = git("rev-parse", "--git-common-dir", cwd=d)
    if rc1 or rc2 or not top or not common:
        return None, None
    return os.path.realpath(os.path.join(d, common)), os.path.realpath(top)

def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for v in value:
            yield from strings(v)

def text_of(full):
    try:
        with open(full, encoding="utf-8", newline="") as f:
            return f.read()
    except FileNotFoundError:
        return ""

def nbytes(text):
    return len(text.encode("utf-8"))

def edited(text, old, new, every):
    """The file after one string replacement, or None when the old text is not there."""
    if "\r\n" in text and "\r\n" not in old:
        old, new = old.replace("\n", "\r\n"), new.replace("\n", "\r\n")
    if old == "":
        return new if text == "" else None
    if old not in text:
        return None
    return text.replace(old, new) if every else text.replace(old, new, 1)

def sizes(tool, ti, cwd):
    """(path, size before, size after the call) for each file the call writes, as far as it can
    be sized; a moved file is compared with the file it was moved from."""
    out = []
    path = next((ti[k] for k in PATH_KEYS if isinstance(ti.get(k), str)), None)
    if path:
        full = os.path.normpath(os.path.join(cwd, os.path.expanduser(path)))
        if isinstance(ti.get("content"), str):
            out.append((full, nbytes(text_of(full)), nbytes(ti["content"])))
        else:
            edits = ti.get("edits") if isinstance(ti.get("edits"), list) else [ti]
            text = before = text_of(full)
            for e in edits:
                old = e.get("old_string", e.get("oldString"))
                new = e.get("new_string", e.get("newString"))
                if not isinstance(old, str) or not isinstance(new, str):
                    text = None
                    break
                text = edited(text, old, new, bool(e.get("replace_all", e.get("replaceAll"))))
                if text is None:
                    break
            if text is not None:
                out.append((full, nbytes(before), nbytes(text)))
    for key in PATCH_KEYS:
        for patch in strings(ti.get(key, "")):
            if "*** Begin Patch" in patch:
                out.extend(patch_sizes(patch, cwd))
    return out

def patch_sizes(patch, cwd):
    out, current = [], None
    def close():
        if current and current["kind"] != "Delete":
            out.append((current["target"], current["before"], current["size"]))
    for line in patch.split("\n"):
        match = FILE_LINE.match(line)
        if match or line.startswith("*** End Patch"):
            close()
            current = None
            if match:
                full = os.path.normpath(os.path.join(cwd, os.path.expanduser(match.group(2))))
                before = 0 if match.group(1) == "Add" else nbytes(text_of(full))
                current = {"kind": match.group(1), "target": full, "before": before, "size": before}
            continue
        if current is None:
            continue
        move = MOVE_LINE.match(line)
        if move:
            current["target"] = os.path.normpath(os.path.join(cwd, os.path.expanduser(move.group(1))))
        elif line.startswith("+"):
            current["size"] += nbytes(line[1:]) + 1
        elif line.startswith("-") and current["kind"] == "Update":
            current["size"] -= nbytes(line[1:]) + 1
    close()
    return out

def rules(top, cache={}):
    """LIMIT and EXCEPTIONS of the worktree check, or None when it has none."""
    if top not in cache:
        script = os.path.join(top, "scripts", "check-docs.py")
        cache[top] = None
        if os.path.isfile(script):
            spec = importlib.util.spec_from_file_location("check_docs", script)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            cache[top] = (module.LIMIT, module.EXCEPTIONS, module.TEMPLATE)
    return cache[top]

def verdict():
    p = json.loads(sys.stdin.read())
    if p.get("hook_event_name") not in (None, "PreToolUse"):
        return None
    if (p.get("tool_name") or "") not in EDIT_TOOLS:
        return None
    ti = p.get("tool_input") or {}
    cwd = p.get("cwd") or os.getcwd()
    guarded, _ = common_dir(os.environ.get("GUARDED_ROOT", ""))
    if not guarded:
        return None
    for full, before, after in sizes(p.get("tool_name"), ti, cwd):
        if not full.endswith(".md"):
            continue
        common, top = common_dir(full)
        if common != guarded:
            continue
        rel = os.path.relpath(os.path.realpath(os.path.dirname(full)), top)
        rel = os.path.normpath(os.path.join(rel, os.path.basename(full))).replace(os.sep, "/")
        if git("check-ignore", "-q", "--", rel, cwd=top)[0] == 0:
            continue
        found = rules(top)
        if not found:
            continue
        limit, exceptions, template = found
        if after > limit and after > before and rel not in exceptions:
            return (f"`{rel}` would be {after} bytes after this edit ({before} now), over the {limit}-byte "
                    f"({limit // 1024} KB) cap of scripts/check-docs.py.\n"
                    f"Edit the sentence that is wrong in place instead of appending a paragraph; history stays in the pull request.\n"
                    f"If the file still does not fit, split it into a table of contents and pieces as {template} describes.")
    return None

try:
    reason = verdict()
except Exception:
    reason = None
if reason:
    print(reason)
    sys.exit(2)
')
rc=$?
if [ "$rc" -eq 2 ]; then
  printf '%s\n' "$verdict" >&2
  exit 2
fi
exit 0
