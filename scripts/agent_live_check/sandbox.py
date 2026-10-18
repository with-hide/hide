"""macOS protection of routing, declared configuration and probe controls.

Native account/session state remains writable under letters 2702/2766.
This is not universal confinement of writes to the disposable probe.
A failed protection self-test prevents any provider launch.
"""

import json
import os
from pathlib import Path
import socket
import stat
import sys

from .processes import OwnedProcesses
from .protection import ProtectionError, beneath, private_directory, write_private


def spellings(paths: list[Path]) -> list[Path]:
    """Protect every alias traversed, its endpoint and each ancestor chain."""
    pending, seen = list(paths), {}
    while pending:
        path = pending.pop().absolute()
        if path in seen:
            continue
        if len(seen) >= 256:
            raise ProtectionError("control_path_aliases_over_budget")
        seen[path] = None
        resolved = path.resolve()
        if resolved != path:
            pending.append(resolved)
        for node in (path, *path.parents):
            try:
                info = node.lstat()
            except (FileNotFoundError, NotADirectoryError):
                continue
            if stat.S_ISLNK(info.st_mode):
                target = node.parent / os.readlink(node)
                pending.append(target / path.relative_to(node))
    return list(seen)


class WriteSandbox:
    def __init__(self, run: Path, sockets: Path, operator_home: Path,
                 state: Path | None = None, *, checkout: Path,
                 protected: list[Path] | None = None, routing: list[Path] | None = None,
                 executables: list[Path] | None = None, fake_private: bool = False):
        if sys.platform != "darwin" or not Path("/usr/bin/sandbox-exec").is_file():
            raise ProtectionError("authenticated_probes_require_macos_write_sandbox")
        self.run = run.resolve()
        self.sockets = sockets.resolve()
        self.probe = self.run / "probe"
        self.state = (state or self.run / "state").resolve()
        if not beneath(self.state, self.run):
            raise ProtectionError("sandbox_state_outside_run")
        self.temp = self.run / "agent-tmp"
        self.declared_proof = self.temp / "guard-declared-config"
        protected = protected or []
        for path in (self.probe, self.temp):
            private_directory(path)
        if any(not beneath(path, operator_home) or path.is_symlink() for path in protected):
            raise ProtectionError("protected_configuration_outside_operator_home_or_link")
        quote = lambda value: json.dumps(str(value), ensure_ascii=True)
        controls = [checkout.resolve(), self.run, operator_home / ".hide",
                    operator_home / ".local/state/hide", operator_home / ".local/share/hide",
                    operator_home / ".config/herdr", Path("/Applications/hide.app"),
                    *(routing or []), *(executables or [])]
        # A linked worktree's index and shared object/ref store live outside
        # the checkout. Protect both rather than just its .git pointer file.
        marker = checkout / ".git"
        if marker.is_file():
            value = marker.read_text().strip()
            if not value.startswith("gitdir: ") or "\n" in value:
                raise ProtectionError("candidate_git_control_path_invalid")
            directory = checkout / value.removeprefix("gitdir: ")
            controls.append(directory)
            common = directory / "commondir"
            if common.is_file():
                value = common.read_text().strip()
                if not value or "\n" in value:
                    raise ProtectionError("candidate_git_common_path_invalid")
                controls.append(directory / value)
        controls = spellings(controls)
        allowed = [self.probe, self.temp, self.sockets]
        # The remote-unix grammar follows the installed system sandbox profiles.
        rules = ["(version 1)", "(allow default)",
                 "(deny signal (target others))",
                 "(allow signal (target same-sandbox))",
                 "(deny appleevent-send hid-control mach-task*)",
                 "(deny process-info*)",
                 "(allow process-info* (target self))",
                 '(deny sysctl-read (sysctl-name-prefix "kern.proc"))',
                 "(deny file-write* " + " ".join(f"(subpath {quote(p)})" for p in controls) + ")",
                 "(allow file-write* " + " ".join(f"(subpath {quote(p)})" for p in allowed) + ")",
                 '(deny network-outbound (remote unix-socket))',
                 # macOS resolves every host name through mDNSResponder's
                 # socket; without it a native agent cannot reach its own API.
                 '(allow network-outbound (remote unix-socket (path "/private/var/run/mDNSResponder")))',
                 f"(allow network-outbound (remote unix-socket (subpath {quote(self.sockets)})))",
                 f"(allow network-outbound (remote unix-socket (subpath {quote(self.run)})))",
                 f"(deny file-read* (subpath {quote(self.run)}))",
                 # Settings readers lstat every ancestor, and Bun's realpath
                 # opens it (OpenCode resolves its project folder that way,
                 # and falls back to HOME when it cannot). Permit only this
                 # directory itself, its metadata and its names, not its
                 # children.
                 f"(allow file-read-metadata file-read-data (literal {quote(self.run)}))",
                 "(allow file-read* " + " ".join(f"(subpath {quote(p)})" for p in
                                                   [*allowed, self.run / "bin"]) + ")"]
        leaves = spellings([*protected, self.declared_proof])
        for path in dict.fromkeys(leaves):
            rules.append(f"(deny file-write* (literal {quote(path)}))")
        # A leaf denial alone cannot prevent renaming its containing tree.
        ancestors = {parent for path in [*controls, *leaves, self.sockets]
                     for parent in (path, *path.parents) if parent != Path("/")}
        rules.append("(deny file-write-unlink " + " ".join(
            f"(literal {quote(p)})" for p in sorted(ancestors)) + ")")
        # Installed hook executables may live in .hide/kit. They remain
        # readable; only operator routing/credentials are concealed.
        for path in spellings([operator_home / ".hide/state", operator_home / ".hide/hcoord",
                     operator_home / ".local/state/hide", operator_home / ".config/herdr",
                     *(routing or [])]):
            rules.append(f"(deny file-read* (subpath {quote(path)}))")
        if fake_private:
            # Only keyless pinned CLIs enter this stricter mode. Operator native
            # state is neither an auth source nor a destination, even if a CLI
            # ignores HOME. No remote provider can receive a request.
            rules.extend(["(deny file-write*)",
                          "(allow file-write* " + " ".join(f"(subpath {quote(p)})" for p in allowed) + ")",
                          '(allow file-write* (literal "/dev/null"))',
                          "(deny network-outbound)",
                          '(allow network-outbound (remote ip "127.0.0.1:*"))',
                          f"(allow network-outbound (remote unix-socket (subpath {quote(self.sockets)})))",
                          f"(allow network-outbound (remote unix-socket (subpath {quote(self.run)})))"])
            for name in (".claude", ".claude.json", ".codex", ".gemini", ".grok", ".omp", ".pi",
                         ".config/opencode", ".local/share/opencode", "Library/Keychains"):
                rules.append(f"(deny file-read* file-write* (subpath {quote(operator_home / name)}))")
        self.profile = self.run / ("fake-write-guard.sb" if fake_private else "native-write-guard.sb")
        self.rules = rules
        self.references = set()
        write_private(self.profile, ("\n".join(rules) + "\n").encode())

    def allow_reference(self, reference: Path) -> None:
        from .protection import stamp
        if (reference.parent != self.state / "pane-capabilities" or reference.is_symlink()
                or stamp(reference) is None or len(self.references) >= 256):
            raise ProtectionError("candidate_capability_reference_refused")
        self.references.add(reference)
        rules = list(self.rules)
        for path in sorted(self.references):
            rules.append(f"(allow file-read* (literal {json.dumps(str(path))}))")
            rules.append(f"(allow file-read* file-write* (literal {json.dumps(str(path.with_suffix('.claimed')))}))")
        replacement = self.profile.with_suffix(".next")
        write_private(replacement, ("\n".join(rules) + "\n").encode())
        replacement.replace(self.profile)

    def command(self, argv: list[str]) -> list[str]:
        return ["/usr/bin/sandbox-exec", "-f", str(self.profile), *argv]

    def verify_private(self, owner, env, outside):
        """Fail before a CLI starts if the fake-only write boundary is ineffective.

        The attempted forbidden write is a disposable stand-in, never an operator file.
        """
        forbidden, allowed = outside / "forbidden", self.probe / "private-guard-proof"
        write_private(forbidden, b"unchanged")
        script = ("import pathlib,sys\n"
                  f"pathlib.Path({str(allowed)!r}).write_bytes(b'allowed')\n"
                  f"try: pathlib.Path({str(forbidden)!r}).write_bytes(b'changed')\n"
                  "except PermissionError: pass\n"
                  "else: sys.exit(31)\n")
        code, _, _ = owner.run(self.command([sys.executable, "-c", script]), env=env, check=False)
        if code or forbidden.read_bytes() != b"unchanged" or not allowed.is_file():
            raise ProtectionError("private_home_write_guard_not_enforced")
        allowed.unlink()

    def verify(self, owner: OwnedProcesses, env: dict, outside: Path) -> None:
        """Exercise declared-file/control protection and Unix socket isolation.

        Both the forbidden file and listening socket belong to this self-test,
        not the operator. No operator endpoint is contacted even if the profile
        is broken. Native-state stand-ins are writable without a history list.
        """
        forbidden = self.declared_proof
        native = outside / "allowed-native-state"
        endpoint = outside / "operator-stand-in.sock"
        inside = self.probe / "guard-write-proof"
        private = self.run / "guard-controller-read"
        # The run root itself may be listed (realpath opens it); its folders may not.
        hidden = self.run / "guard-controller-folder"
        if beneath(outside, self.run) or beneath(outside, self.sockets):
            raise ProtectionError("guard_selftest_outside_aliases_allowance")
        if any(path.exists() for path in (forbidden, endpoint, inside, native, private, hidden)):
            raise ProtectionError("guard_selftest_paths_already_exist")
        listener = socket.socket(socket.AF_UNIX)
        try:
            write_private(forbidden, b"unchanged")
            write_private(private, b"controller-only")
            hidden.mkdir(mode=0o700)
            listener.bind(str(endpoint))
            listener.listen(1)
            program = (
                "import ctypes,errno,os,pathlib,socket,sys; "
                f"pathlib.Path({str(inside)!r}).write_bytes(b'proof'); "
                f"pathlib.Path({str(native)!r}).write_bytes(b'native-state'); "
                f"p=pathlib.Path({str(forbidden)!r}); "
                f"\nfor parent in pathlib.Path({str(inside)!r}).parents: parent.lstat()"
                f"\ntry: os.listdir({str(hidden)!r})"
                "\nexcept PermissionError: pass"
                "\nelse: sys.exit(35)"
                f"\ntry: pathlib.Path({str(private)!r}).read_bytes()"
                "\nexcept PermissionError: pass"
                "\nelse: sys.exit(36)"
                "\ntry: p.write_bytes(b'forbidden')"
                "\nexcept PermissionError: pass"
                "\nelse: sys.exit(31)"
                "\ns=socket.socket(socket.AF_UNIX)"
                f"\ntry: s.connect({str(endpoint)!r})"
                "\nexcept PermissionError: pass"
                "\nelse: sys.exit(32)"
                f"\ntry: os.kill({os.getpid()}, 0)"
                "\nexcept PermissionError: pass"
                "\nelse: sys.exit(33)"
                "\nlib=ctypes.CDLL(None,use_errno=True)"
                f"\nmib=(ctypes.c_int*3)(1,49,{os.getpid()})"
                "\nsize=ctypes.c_size_t(1024*1024); data=ctypes.create_string_buffer(size.value)"
                "\nif lib.sysctl(mib,3,data,ctypes.byref(size),None,0) != -1 or ctypes.get_errno() != errno.EPERM: sys.exit(34)"
            )
            code, _, _ = owner.run(self.command([sys.executable, "-c", program]),
                                   env=env, check=False)
            if (code or not inside.exists() or forbidden.read_bytes() != b"unchanged"
                    or native.read_bytes() != b"native-state"):
                raise ProtectionError("native_protection_guard_not_enforced")
        finally:
            listener.close()
            for path in (forbidden, endpoint, inside, native, private):
                path.unlink(missing_ok=True)
            if hidden.is_dir():
                hidden.rmdir()
