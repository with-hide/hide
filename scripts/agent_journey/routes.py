"""What the journey's write sandbox lets through to the candidate daemon, and
which of the operator's own routes it refuses to inherit."""

import json
from pathlib import Path

from agent_live_check.protection import ProtectionError, write_private

from . import hook_records


def operator_routing(environment):
    """Inherited operator routes, before the private environment replaces them."""
    routes = []
    for key, suffix in {"HIDE_STATE_DIR": "", "XDG_STATE_HOME": "hide"}.items():
        if value := environment.get(key):
            path = Path(value)
            if not path.is_absolute():
                raise ProtectionError("operator_routing_must_be_absolute")
            routes.append(path / suffix if suffix else path)
    return routes


def allow_hook_records(sandbox, runtime):
    """Let the hooks inside the sandbox keep their records where this candidate reads them.

    A pane's hook records (its counts, its turn, its plan and the record of a
    report that failed) sit beside the state folder of the daemon it asks
    (`hide_agent_hooks::route::records_root`), which for this run's candidate
    is `<state>/agent-hooks` and not the operator's `~/.hide`
    (`scripts/agent_journey/hook_records.py`). The profile hides the whole run
    from the agent, so without this a hook reports its session - which is what
    a spawn waits for - and can keep nothing else: its counts stay absent and
    its turn never moves, so a row never reads idle after the answer, and a
    refused record leaves no file for the run to name as the cause.

    This opens that one folder and nothing else of the controller's state: the
    credentials, the mailbox and the core's own files stay denied, and so does
    the operator's `~/.hide` outside the sandbox.
    """
    records = hook_records.folder(runtime)
    quote = lambda value: json.dumps(str(value), ensure_ascii=True)
    sandbox.rules.extend([
        # The record's parent is this daemon's state folder, which the profile
        # denies whole; its metadata is what a hook lstat's on the way in.
        f"(allow file-read-metadata (literal {quote(records.parent)}))",
        f"(allow file-read* file-write* (subpath {quote(records)}))"])
    replacement = sandbox.profile.with_suffix(".next")
    write_private(replacement, ("\n".join(sandbox.rules) + "\n").encode())
    replacement.replace(sandbox.profile)


def allow_hook_route(sandbox, runtime):
    """Let a hook inside the sandbox find the private daemon the way the shipped hook does.

    The hook reads the bootstrap socket record under the state folder and connects to the
    socket it names (hide-node pane_proof); the sandbox hides the whole run from the
    agent, so this opens exactly that record and the socket's folder, nothing else of
    the controller's state.
    """
    record = runtime.state / "pane-capabilities/bootstrap-socket"
    socket = Path(record.read_text().strip())
    # The daemon binds it in a private folder of its own under /tmp (hide-node pane_proof,
    # bootstrap_parent), named by a random token; the record under this run's own state
    # folder is what ties the folder to this run's daemon and no other.
    if (not socket.is_absolute() or socket.parent.parent != Path("/tmp")
            or not socket.parent.name.startswith("hide-pane") or socket.parent.is_symlink()):
        raise ProtectionError("bootstrap_socket_not_this_daemons")
    quote = lambda value: json.dumps(str(value), ensure_ascii=True)
    sandbox.rules.extend([f"(allow file-read* (literal {quote(record)}))",
                          f"(allow file-read-metadata (literal {quote(runtime.state)}) (literal {quote(record.parent)}))"])
    # The sandbox matches a resolved path, and /tmp is a link to /private/tmp.
    for path in dict.fromkeys((socket, socket.resolve())):
        sandbox.rules.extend([f"(allow file-read-metadata (literal {quote(path.parent)}))",
                              f"(allow file-read* (literal {quote(path)}))",
                              f"(allow network-outbound (remote unix-socket (literal {quote(path)})))"])
    replacement = sandbox.profile.with_suffix(".next")
    write_private(replacement, ("\n".join(sandbox.rules) + "\n").encode())
    replacement.replace(sandbox.profile)
