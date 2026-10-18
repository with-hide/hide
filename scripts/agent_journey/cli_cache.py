"""One bounded, pin-keyed cache; a run holds its lease until its agents end."""

import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import time

from agent_live_check.protection import ProtectionError, beneath, private_directory, write_private
from agent_live_check.runtime import clean_env


def size(root, cap):
    """Count links themselves, not the trees they point to; bound metadata work too."""
    total = count = 0
    pending = [root]
    while pending:
        path = pending.pop()
        info = path.lstat()
        count += 1
        total += info.st_size
        if total > cap or count > 100_000:
            raise ProtectionError("journey_cli_cache_over_budget: remove unused pins from the cache before retrying")
        if path.is_dir() and not path.is_symlink():
            pending.extend(path.iterdir())
    return total


def pin_key(kind, pin):
    value = json.dumps([kind, pin, platform.system(), platform.machine()], sort_keys=True).encode()
    return kind + "-" + pin["version"] + "-" + hashlib.sha256(value).hexdigest()[:16]


class Cache:
    def __init__(self, root, cap):
        # Do not follow a replaced cache or one of its ancestors into operator files.
        if any(path.is_symlink() for path in (root, *root.parents)):
            raise ProtectionError("journey_cli_cache_link_refused")
        private_directory(root)
        self.root, self.cap = root, cap
        self.lock = None
        self.check()
        import fcntl  # Local fake runs are macOS-only, as the write sandbox is.
        fd = os.open(root / ".lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        self.lock = os.fdopen(fd, "w")
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self.close()
            raise ProtectionError("journey_cli_cache_busy: another local fake run holds the cache") from None

    def close(self):
        if self.lock:
            self.lock.close()
            self.lock = None

    def check(self):
        return size(self.root, self.cap)

    def program(self, kind, pin):
        folder = self.root / pin_key(kind, pin)
        receipt = folder / "receipt.json"
        if not receipt.is_file():
            return None
        data = json.loads(receipt.read_text())
        program = folder / data["program"]
        if (data["pin"] != pin or not beneath(program.resolve(), folder)
                or not program.is_file()
                or hashlib.sha256(program.read_bytes()).hexdigest() != data["sha256"]):
            raise ProtectionError("journey_cli_cache_receipt_mismatch: remove this pin and retry")
        self.check()
        return program

    def install(self, runtime, kind, pin, update_env):
        program = self.program(kind, pin)
        if program:
            return program
        folder = self.root / pin_key(kind, pin)
        if folder.exists():
            raise ProtectionError("journey_cli_cache_incomplete: remove the incomplete pin and retry")
        private_directory(folder)
        for name in ("home", "tmp", "npm-cache"):
            private_directory(folder / name)
        # The installer can write only this pin, not an operator file or another pin.
        quote = lambda path: json.dumps(str(path))
        profile = runtime.run / f"install-{kind}.sb"
        write_private(profile, ("(version 1)\n(allow default)\n(deny file-write*)\n"
                               f"(allow file-write* (subpath {quote(folder)}))\n"
                               '(allow file-write* (literal "/dev/null"))\n'
                               "(deny appleevent-send)\n").encode())
        environment = {**clean_env(), "HOME": str(folder / "home"), "TMPDIR": str(folder / "tmp"),
                       "npm_config_cache": str(folder / "npm-cache"), "npm_config_userconfig": os.devnull,
                       **update_env}
        for name in ("GITHUB_PATH", "GITHUB_OUTPUT", "BASH_ENV", "ENV", "NODE_OPTIONS", "NODE_PATH"):
            environment.pop(name, None)
        import sys
        child = runtime.resident(Path("/usr/bin/sandbox-exec"),
                                 ["-f", str(profile), sys.executable,
                                  str(runtime.checkout / "scripts/install-agent-clis.py"), kind,
                                  "--prefix", str(folder), "--private-fake"], environment, f"install-{kind}")
        deadline = min(runtime.owner.deadline, time.monotonic() + 600)
        while child.poll() is None:
            self.check()
            if runtime.owner.cancelled.is_set() or time.monotonic() >= deadline:
                raise ProtectionError("journey_cli_install_cancelled_or_timed_out")
            runtime.owner.cancelled.wait(0.2)
        if child.returncode:
            raise ProtectionError(f"journey_cli_install_failed: {kind}, see install log")
        self.check()
        data = json.loads((folder / "installed.json").read_text())
        program = Path(data[kind])
        if not program.is_file() or not beneath(program.resolve(), folder):
            raise ProtectionError("journey_cli_install_outside_pin")
        # Download cache and installer HOME are temporary, not one copy per journey.
        for name in ("home", "tmp", "npm-cache"):
            shutil.rmtree(folder / name)
        write_private(folder / "receipt.json", json.dumps({
            "pin": pin, "program": str(program.relative_to(folder)),
            "sha256": hashlib.sha256(program.read_bytes()).hexdigest(),
        }).encode())
        self.check()
        return program
