"""What the candidate's hooks recorded, read where the candidate keeps them.

A hook writes in the records folder of the daemon it asks
(`hide_agent_hooks::route::records_of_state`): a daemon started with a state
folder of its own, as this run's always is (`agent_live_check/runtime.py`
hands it `HIDE_STATE_DIR`), keeps them in `<state>/agent-hooks`, and the
helper that wrote them composes the installation that started its pane into
that folder, so a helper's records sit under `<state>/agent-hooks/installs/
<mark>`. The candidate is an app package (`cli.py::package`), whose panes
name the app installation, so its record is in the folder itself. A reader
that asks for another installation reads that installation's own mark and
never another one's: a pane id is reused across installations, so a name
match under some other mark is not this run's cause.

The run reads them there in local and in CI mode alike, so a hook whose
records folder refuses a write is a sentence in the cell that needed it
instead of a start refused on an identity nobody can explain (engineering
rules 4 and 10).
"""

import errno
import hashlib
import json
from pathlib import Path
import re

# `hide_agent_hooks::route::APP_INSTALL`: the desktop package's installation,
# whose panes keep their records in the folder itself, with no mark.
APP_INSTALL = "app"
# `hide_agent_hooks::route::pane_install` reads the pane's installation from
# this variable, and the kit sets it in every pane it starts.
INSTALL_VARIABLE = "HIDE_INSTALL"


def root(runtime):
    """The candidate's records folder before any installation is composed
    into it: the account's own `~/.hide/agent-hooks` while the daemon keeps
    the account's default state folder, and `<state>/agent-hooks` for any
    state folder of its own."""
    state, home = Path(runtime.state), Path(runtime.home)
    if state == home / ".hide" / "state":
        return home / ".hide" / "agent-hooks"
    return state / "agent-hooks"


def folder(runtime, install=None):
    """Where the panes of `install` keep their records under `runtime`'s
    daemon, by the route's own rule (`route::pane_records_at`): the folder
    itself for the app or a pane that names no installation, and
    `installs/<mark>` below it for any other, the mark being the first eight
    bytes of the installation name's sha256 in hex.

    The app's install is the default because the journey starts the candidate
    as a package, whose panes name `app`; a caller that knows another install
    passes it, and a run that composed none reads the folder itself.
    """
    base = root(runtime)
    if install in (None, "", APP_INSTALL):
        return base
    mark = hashlib.sha256(install.encode("utf-8")).hexdigest()[:16]
    return base / "installs" / mark


def last_failure(runtime, install=None):
    """The hook's last report outcome for `install`'s panes, as the hook
    itself recorded it.

    Only that installation's own folder is read, by the same rule the Rust
    route applies (`route::pane_records_at`): the folder itself for the app or
    a pane that names no installation, `installs/<mark>` below it for any
    other. A pane id is reused across installations, so a name match cannot
    tell whose record it is that way, and another installation's failure is
    not this run's cause: the Rust reader answers absence in that case too
    (`records_route::another_installations_failure_record_is_not_this_daemons`).
    Nothing there is absence.

    The file holds the current state, not the history: the next report that
    gets through removes it. A file that cannot be read back is reported as
    an outcome of its own rather than read as "nothing failed".
    """
    answer = _read(folder(runtime, install) / "last-report-failure.json")
    return None if answer is _ABSENT else answer


_ABSENT = object()


def _read(path):
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return _ABSENT
    except (OSError, ValueError) as error:
        return {"pane_id": "", "event": "", "error": f"{path.name} could not be read back: {error}"}
    return value if isinstance(value, dict) else None


def pane_cause(runtime, pane, install=None):
    """The cause the hook recorded for `pane`, when its last outcome is that
    pane's, or None.

    A turn the row never reads as idle is the turn's own failure, and a hook
    that could not keep the record the turn and the counts are read from
    wrote that down for the pane it ran in. Naming it is what makes the cell
    say why rather than only that the row stayed working; another pane's
    outcome explains nothing here and stays unnamed.
    """
    failure = last_failure(runtime, install)
    if failure is None or not pane or failure.get("pane_id") != pane:
        return None
    return cause(failure)


def cause(failure):
    """One named cause for a cell's reason, from the hook's own record.

    `hook_record_refused` when the pane's record could not be written,
    `hook_report_failed` when only the report did not reach the core,
    `hook_record_and_report_failed` when both did (each sentence is kept in
    the record), and `hook_failure_unknown_kind` for a record whose kind an
    earlier build never wrote, with its own sentence kept as it is rather
    than read for words.
    """
    error = str(failure.get("error") or "").strip()
    if not error:
        return None
    kind = failure.get("kind")
    if kind == "record":
        name = "hook_record_refused"
    elif kind == "record_and_report":
        name = "hook_record_and_report_failed"
    elif kind == "report":
        name = "hook_report_failed"
    else:
        name = "hook_failure_unknown_kind"
    pane = failure.get("pane_id") or "no pane"
    event = failure.get("event") or "no event"
    return f"{name} ({pane}, {event}): {error}"


def identity_cause(runtime, pane, install=None):
    """Why a start was refused on the agent's session, from the hook records.

    A run whose spawn answered `native_identity_unavailable` asks the
    candidate's own records which pane's hook outcome explains it: the
    record the hook could not write, the report that never reached the core,
    or none at all. Each is named, so the refusal reads as its cause rather
    than as the same unclear sentence in every later cell.
    """
    failure = last_failure(runtime, install)
    if failure is None:
        return "no_hook_outcome: the hook left no report record in " + str(folder(runtime, install))
    named = cause(failure)
    if named is None:
        return "no_hook_outcome: the hook's record carries no cause"
    if pane and failure.get("pane_id") not in ("", pane):
        return f"no_hook_outcome: the hook's last record names pane {failure.get('pane_id')}, not {pane}"
    return named


# `hide_agent_hooks::hook_log`: the hooks' bounded log beside the records,
# one JSON line per refusal, failed report or event kept from the pane.
LOG_NAME = "spawn-guard.log"
# The fields a kept log line may carry: ids, kinds and times. A free-form
# field (a cause's sentence) can name a path, and stays behind.
LOG_FIELDS = ("component", "kind", "failure", "pane", "agent", "runtime", "event", "conversation", "at_unix_ms")
# How much of the log's end is read: it rotates at 256 KiB, and a failed
# cell keeps the pane's newest lines of it.
LOG_TAIL_BYTES = 32 * 1024
# How much of the pane's record is read: `hide_agent_hooks::route` keeps a
# record of a few sessions, well under this; a longer one is named
# `too_large`, not read in part.
RECORD_BYTES = 64 * 1024


def pane_file(records, pane):
    """`hide_agent_hooks::route::pane_file`: the pane's record under `panes/`."""
    return records / "panes" / (re.sub(r"[^A-Za-z0-9_-]", "_", pane) + ".json")


def unread(error):
    """An error reading the records as a code, never its message, which
    names the file's path."""
    return errno.errorcode.get(error.errno, type(error).__name__) if isinstance(error, OSError) else type(error).__name__


def pane_record(runtime, pane, cap, install=None):
    """What the hooks kept of `pane`, for a failed cell to keep beside it, as
    JSON text of at most `cap` bytes, its own `cut` mark and formatting
    included, or None when they kept nothing.

    The sessions the pane's record holds, oldest first, by the agent and the
    session its hook named and the turn it last reported, and the pane's
    lines of the end of the hooks' log ([`LOG_TAIL_BYTES`]); when they do not
    all fit, the oldest log lines go first, then the oldest sessions, and
    `cut` says so. A start that names another agent's session then shows
    whether the pane's hook named it (with-hide/hide#3124). Only ids, kinds,
    turns and times are copied, and a file that cannot be read is named by
    its error code: no prompt, task, command, sentence or path.
    """
    if not pane:
        return None
    records = folder(runtime, install)
    kept = {}
    try:
        with pane_file(records, pane).open("rb") as file:
            text = file.read(RECORD_BYTES + 1)
        if len(text) > RECORD_BYTES:
            kept["record_unread"] = "too_large"
            written = None
        else:
            written = json.loads(text)
    except FileNotFoundError:
        written = None
    except (OSError, ValueError) as error:
        kept["record_unread"] = unread(error)
        written = None
    if isinstance(written, dict):
        sessions = written.get("sessions") if isinstance(written.get("sessions"), list) else [written]
        kept["sessions"] = [{key: session.get(key) for key in ("agent", "session", "turn")
                             if isinstance(session.get(key), (str, int, type(None)))}
                            for session in sessions if isinstance(session, dict)]
    try:
        with (records / LOG_NAME).open("rb") as file:
            file.seek(max(0, file.seek(0, 2) - LOG_TAIL_BYTES))
            tail = file.read(LOG_TAIL_BYTES).decode("utf-8", errors="replace").splitlines()
    except FileNotFoundError:
        tail = []
    except OSError as error:
        kept["log_unread"] = unread(error)
        tail = []
    logged = []
    for line in tail:
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        if isinstance(entry, dict) and entry.get("pane") == pane:
            logged.append({key: entry[key] for key in LOG_FIELDS
                           if isinstance(entry.get(key), (str, int)) and not isinstance(entry.get(key), bool)})
    if not kept and not logged:
        return None
    kept = {"pane": pane, **kept}
    sessions = kept.pop("sessions", None)
    cut = False
    while True:
        shown = {**kept, **({"sessions": sessions} if sessions is not None else {}),
                 **({"log": logged} if logged else {}), **({"cut": True} if cut else {})}
        text = json.dumps(shown, indent=2) + "\n"
        if len(text.encode()) <= cap:
            return text
        # The oldest log lines go first, then the oldest sessions.
        if logged:
            logged = logged[max(1, len(logged) // 4):]
        elif sessions:
            sessions = sessions[max(1, len(sessions) // 4):]
        else:
            return None
        cut = True
