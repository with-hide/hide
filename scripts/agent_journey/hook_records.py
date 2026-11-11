"""What the candidate's hooks recorded, read where the candidate keeps them.

A hook writes in the records folder of the daemon it asks
(`hide_agent_hooks::route::records_of_state`): a daemon started with a state
folder of its own, as this run's always is (`agent_live_check/runtime.py`
hands it `HIDE_STATE_DIR`), keeps them in `<state>/agent-hooks`, and the
helper that wrote them composes the installation that started its pane into
that folder, so a helper's records sit under `<state>/agent-hooks/installs/
<mark>`. The candidate is an app package (`cli.py::package`), whose panes
name the app installation, so its record is in the folder itself; a run that
used a build placed under a helper root would find it under the mark instead,
and the folder is searched one level further for that case rather than read
as absent.

The run reads them there in local and in CI mode alike, so a hook whose
records folder refuses a write is a sentence in the cell that needed it
instead of a start refused on an identity nobody can explain (engineering
rules 4 and 10).
"""

import hashlib
import json
from pathlib import Path

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

    The file holds the current state, not the history: the next report that
    gets through removes it. A file that cannot be read back is reported as
    an outcome of its own rather than read as "nothing failed". A record the
    app's folder does not hold is looked for under each installation's own
    folder, so a run whose helper wrote under its mark is read rather than
    answered as absent.
    """
    candidates = [folder(runtime, install)]
    if install in (None, "", APP_INSTALL):
        candidates += [path for path in (root(runtime) / "installs").glob("*") if path.is_dir()]
    for directory in candidates:
        answer = _read(directory / "last-report-failure.json")
        if answer is not _ABSENT:
            return answer
    return None


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
