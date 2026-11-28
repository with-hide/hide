"""One `hide` command of the journey's controller agent (controller.py): the
spawn provider runs this on each `!`, with the command file as its argument,
so the command is the agent's descendant on every system. Its exit status is
the command's, and its answer is in the response file.

The C provider owns this process and every descendant, including on timeout.
Drain both pipes together with a retained-byte cap; inherited pipes cannot
keep a finished command's driver alive indefinitely. A capture that misses a
stream's end names each stream's state: `ended`, `open` (no end within
PIPE_END_SECONDS of the command's exit, as when a descendant still holds it),
or the reader's error type; and `child_ms` says how long the command ran.
"""

import json
import os
import subprocess
import sys
import threading
import time

OUTPUT_BYTES = 1024 * 1024
PIPE_END_SECONDS = 2


def private_file(path):
    return os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb")


def run(command):
    result = {"id": command["id"], "phase": "spawn", "exit": 1, "os_error": None}
    with private_file(command["response"]) as response, private_file(command["stderr"]) as stderr:
        try:
            # The outer provider's process group/job owns the whole tree.
            child = subprocess.Popen(command["argv"], env={**os.environ, **command["env"]},
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as error:
            result["os_error"] = getattr(error, "winerror", None) or error.errno
            return result
        started = time.monotonic()
        overflow, ended = threading.Event(), {}

        def drain(name, pipe, output):
            retained = 0
            try:
                while data := pipe.read1(8192):
                    keep = data[:max(0, OUTPUT_BYTES - retained)]
                    output.write(keep)
                    # A supervisor kill cannot run this driver's close path.
                    output.flush()
                    retained += len(keep)
                    if len(keep) != len(data):
                        overflow.set()
                        child.kill()
                ended[name] = "ended"
            except (OSError, ValueError) as error:
                ended[name] = type(error).__name__
            finally:
                pipe.close()

        streams = {"stdout": (child.stdout, response), "stderr": (child.stderr, stderr)}
        readers = [threading.Thread(target=drain, args=(name, pipe, output), daemon=True)
                   for name, (pipe, output) in streams.items()]
        for reader in readers:
            reader.start()
        result["exit"] = child.wait()
        result["child_ms"] = round((time.monotonic() - started) * 1000)
        for reader in readers:
            reader.join(PIPE_END_SECONDS)
        # A reader still blocked owns no entry; read once, so the phase and
        # the states describe the same moment.
        result["streams"] = {name: ended.get(name, "open") for name in streams}
        result["phase"] = ("output_limit" if overflow.is_set() else
                           "capture_incomplete" if set(result["streams"].values()) != {"ended"}
                           else "exited")
        if result["phase"] != "exited":
            result["exit"] = 1
    return result


def main():
    with open(sys.argv[1], encoding="utf-8") as stream:
        command = json.load(stream)
    result = run(command)
    with private_file(command["result"]) as stream:
        stream.write(json.dumps(result).encode())
    return result["exit"]


if __name__ == "__main__":
    sys.exit(main())
