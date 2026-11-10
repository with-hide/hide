"""The native root of `test_linux_adopted_children_are_reaped_as_they_exit_*`
(test_agent_live_protection.py): a program run under the guardian that ends
batches of processes whose parent ended first, and after each batch waits for
the state the next one needs, the guardian holding none of them. Exits 1 when
the guardian still holds one after `SECONDS`."""

import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.process_table import snapshot

BATCHES = 12
BATCH = 20
SECONDS = 10


def held_by(guardian):
    """How many processes the guardian holds besides this root, whatever
    their state: a live one has not ended, a zombie has not been reaped, and
    a subject the table could not read is not known to be gone."""
    table = snapshot()
    return len(table.unavailable) + sum(1 for process in table.values()
                                        if process.parent == guardian and process.pid != os.getpid())


def end_a_batch():
    """BATCH middles, each of which starts a leaf and ends at once: the leaf
    is orphaned and adopted by the guardian, which holds it until it reaps it."""
    for _ in range(BATCH):
        if os.fork() == 0:
            if os.fork() == 0:
                os._exit(0)
            os._exit(0)
    while True:
        try:
            os.wait()
        except ChildProcessError:
            return


def main():
    guardian = os.getppid()
    for _ in range(BATCHES):
        end_a_batch()
        end = time.monotonic() + SECONDS
        while held_by(guardian) and time.monotonic() < end:
            time.sleep(0.05)
        if held_by(guardian):
            os._exit(1)


if __name__ == "__main__":
    main()
