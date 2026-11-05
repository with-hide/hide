"""What the journey's write sandbox lets through to the candidate daemon, and
which of the operator's own routes it refuses to inherit."""

import json
from pathlib import Path

from agent_live_check.protection import ProtectionError, write_private


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
