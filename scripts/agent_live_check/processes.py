"""One child-start boundary, with an EOF guardian for external processes.

On Windows each child is owned by a job object instead of a guardian process
(`windows.py`): the same caps, ended and confirmed on every exit path.
"""

import ctypes
import json
import os
from pathlib import Path
import selectors
import secrets
import select
import signal
import stat
import subprocess
import sys
import threading
import time

if __package__:
    from .process_table import ProcessState, ProcessTable, descendants, group_state, identity_state, marked_descendants, require_complete, snapshot
    from .system import WINDOWS
else:
    from process_table import ProcessState, ProcessTable, descendants, group_state, identity_state, marked_descendants, require_complete, snapshot
    from system import WINDOWS

if WINDOWS:
    from . import windows
else:
    import fcntl

MAX_CHILDREN = 16
MAX_DESCENDANTS = 128
MAX_RSS_BYTES = 2 * 1024 * 1024 * 1024
MAX_OUTPUT = 1024 * 1024
MAX_FAILURE_REASON = 512
MAX_COMMANDS = 4096
MAX_OWNER_FAMILIES = 4096
COMMAND_SECONDS = 15
RUN_SECONDS = 30 * 60
POLL_SECONDS = 0.1
RSS_CONSECUTIVE_MISS_LIMIT = 3
# The lines a guardian writes to its child's stderr when it fails or ends the child.
GUARDIAN_FAILURES = ("guardian_failure:", "guardian_cleanup_failure:", "guardian_orphan_scan_failure:",
                     "guardian_signal_failure:")
# How long a child's end may take to confirm: the guardian's exit on Unix,
# the job holding no process on Windows.
END_SECONDS = 4


class ProcessError(RuntimeError):
    pass


class ProcessSafetyError(ProcessError):
    """A guardian or cleanup failure that must end the measurement."""


def signal_reserved_group(group, signum):
    """Signal a group while its caller keeps the group leader unreaped."""
    # Terminal members have no signal obligation. Their presence still has
    # to be resolved by wait/reap and the separate final absence check.
    unavailable = None
    try:
        state = group_state(snapshot(group))
    except Exception as error:
        # The unreaped leader still reserves this group. A metadata failure
        # remains a failure but must not abandon its independent signal path.
        unavailable = error
    else:
        if state is not ProcessState.LIVE:
            return
    try:
        os.killpg(group, signum)
    except ProcessLookupError:
        pass
    except PermissionError as error:
        details = {"group": group, "signal": signum, "errno": error.errno}
        try:
            current = snapshot(group)
            state = group_state(current)
        except Exception as refresh:
            details.update(refresh="unavailable", refresh_error=type(refresh).__name__)
            raise ProcessError("owned_group_signal:" + json.dumps(details, separators=(",", ":"))) from error
        if state is ProcessState.LIVE:
            details.update(refresh="live", live_members=sum(not p.zombie for p in current.values()))
            raise ProcessError("owned_group_signal:" + json.dumps(details, separators=(",", ":"))) from error
        # XNU's explicit-group kill excludes zombies and can return EPERM
        # when none remain signalable. This refresh ends only the signal
        # obligation; the caller still reaps and proves final group absence.
        details.update(refresh="terminal", zombie_members=len(current))
        sys.stderr.write("guardian_signal_terminal:" + json.dumps(details, separators=(",", ":")) + "\n")
    except OSError as error:
        raise ProcessError("owned_group_signal:" + json.dumps(
            {"group": group, "signal": signum, "errno": error.errno},
            separators=(",", ":"))) from error
    if unavailable is not None:
        raise ProcessError("owned_group_metadata_unavailable:" + str(unavailable)[:MAX_FAILURE_REASON]) from unavailable


def signal_identity(process, signum):
    """Signal only the proven birth; one fresh denial check can end its duty.

    A birth check narrows PID reuse races; it does not make kill(pid) atomic.
    """
    def current():
        return identity_state(process.pid, process.birth, uid=process.uid)
    state = current()
    if state is not ProcessState.LIVE:
        return state
    try:
        os.kill(process.pid, signum)
    except ProcessLookupError:
        return ProcessState.VANISHED
    except PermissionError:
        state = current()
        if state is ProcessState.LIVE:
            raise
    return state


def largest(table, count=5):
    """The live processes holding the most memory, by executable name, so a
    crossed memory cap says what held it."""
    live = sorted((p for p in table.values() if p.rss > 0 and not p.zombie), key=lambda p: p.rss, reverse=True)
    return [{"executable": p.name[:32], "rss_bytes": p.rss} for p in live[:count]]


class RssSamples:
    """RSS availability is a sampled budget, independent of ownership proof."""

    def __init__(self):
        self.consecutive = {}
        self.missed = 0
        self.max_consecutive = 0

    def summary(self):
        return {"missed": self.missed, "max_consecutive_misses": self.max_consecutive,
                "consecutive_miss_limit": RSS_CONSECUTIVE_MISS_LIMIT}

    def measure(self, table):
        # snapshot refreshes identity after a failed RSS read. Only a still
        # live birth accrues a miss; success/exit/replacement resets its streak.
        missing = {(p.pid, p.birth) for p in table.values() if p.rss < 0 and not p.zombie}
        self.consecutive = {key: self.consecutive.get(key, 0) + 1 for key in missing}
        self.missed += len(missing)
        self.max_consecutive = max(self.max_consecutive, max(self.consecutive.values(), default=0))
        exhausted = [key for key, count in self.consecutive.items()
                     if count >= RSS_CONSECUTIVE_MISS_LIMIT]
        if exhausted:
            raise ProcessError("rss_samples_unavailable:" + json.dumps(
                {"count": len(exhausted), "identities": [
                    {"pid": pid, "birth": birth} for pid, birth in sorted(exhausted)[:16]],
                 **self.summary()}, separators=(",", ":")))
        return {"rss_bytes": sum(p.rss for p in table.values() if p.rss >= 0 and not p.zombie),
                "rss_complete": not missing, "rss_samples_missed": self.missed}


def owner_registry():
    directory = Path(__file__).resolve().parents[2] / "agents/runs/process-owner-families"
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ProcessError("owner_registry_not_private")
    return directory


def issued_family(family):
    if len(family) != 64 or any(character not in "0123456789abcdef" for character in family):
        return False
    try:
        info = (owner_registry() / family).lstat()
    except FileNotFoundError:
        return False
    return stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and not info.st_mode & 0o077


def control_plane(table, root):
    excluded = {}
    ancestor = table.get(root)
    if ancestor is None:
        raise ProcessError("control_plane_identity_unavailable")
    while ancestor.pid not in excluded:
        excluded[ancestor.pid] = ancestor.birth
        if len(excluded) > MAX_DESCENDANTS:
            raise ProcessError("control_plane_ancestry_over_budget")
        if ancestor.parent == 0:
            return excluded
        parent = table.get(ancestor.parent)
        if parent is None:
            # Darwin can positively identify its foreign-UID system init
            # without full metadata. A same-UID namespace PID 1 is instead
            # a real controller whose birth must be excluded like any other.
            if ancestor.parent == 1 and 1 in getattr(table, "foreign_uid_pids", ()):
                return excluded
            raise ProcessError("control_plane_ancestry_unavailable")
        ancestor = parent
    raise ProcessError("control_plane_ancestry_cycle")


def linux_children_remain():
    # After Popen's direct child has been waited, only owned adopted children
    # remain. __WALL includes clone children with a non-SIGCHLD exit signal.
    # ECHILD is the kernel's no-child proof; an empty sampled table is not.
    for _ in range(MAX_DESCENDANTS + 1):
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG | 0x40000000)
        except ChildProcessError:
            return False
        if pid == 0:
            return True
    raise ProcessError("adopted_child_reap_over_budget")


class OwnedProcesses:
    """Guardians end children on pipe EOF, including abrupt owner death.

    The driver still owns the corresponding protocol close. A guardian is the
    final containment boundary, not a substitute for stopping a server.
    """

    def __init__(self, cancelled: threading.Event | None = None, diagnostics: Path | None = None,
                 run_seconds: float = RUN_SECONDS):
        """`run_seconds` caps the whole run, every child's guardian included."""
        self.children = {}
        self.diagnostics = diagnostics
        self.receipts = {}
        self.family = None
        self.sequence = 0
        self.rss_samples = RssSamples()
        self.guardian_rss_samples = None
        if diagnostics is not None:
            diagnostics.mkdir(mode=0o700)
        self.cancelled = cancelled or threading.Event()
        self.deadline = time.monotonic() + run_seconds
        # Windows: the port every child's job reports a refused limit to, and
        # each end that was not confirmed, which no later close erases.
        self.port = None
        self.jobs = 0
        self.unconfirmed = []

    def spawn(self, argv, *, env, cwd=None, stdin=subprocess.DEVNULL,
              stdout=subprocess.PIPE, stderr=subprocess.PIPE, _guarded=True, deadline=None):
        launch_deadline = min(self.deadline, self.deadline if deadline is None else deadline)

        def admit():
            now = time.monotonic()
            if self.cancelled.is_set() or now >= self.deadline:
                raise ProcessError("run_cancelled_or_timed_out")
            if now >= launch_deadline:
                raise ProcessError("command_timeout")

        admit()
        for child in list(self.children):
            if child.poll() is not None:
                self.end(child)
                admit()
        if len(self.children) >= MAX_CHILDREN:
            raise ProcessError("child_count_over_budget")
        admit()
        if WINDOWS:
            return self.spawn_in_job(argv, env=env, cwd=cwd, stdin=stdin, stdout=stdout, stderr=stderr,
                                     admit=admit)
        reader = writer = None
        diagnostic = ""
        command = argv
        if _guarded:
            if self.family is None:
                directory = owner_registry()
                with open(directory / ".lock", "a", opener=lambda path, flags: os.open(path, flags, 0o600)) as lock:
                    try:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError as error:
                        raise ProcessError("owner_registry_busy") from error
                    if sum(1 for path in directory.iterdir() if path.name != ".lock") >= MAX_OWNER_FAMILIES:
                        raise ProcessError("owner_family_count_over_budget")
                    self.family = secrets.token_hex(32)
                    descriptor = os.open(directory / self.family, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
                    os.close(descriptor)
            reader, writer = os.pipe()
            if self.diagnostics is not None:
                if self.sequence >= MAX_COMMANDS:
                    os.close(reader)
                    os.close(writer)
                    raise ProcessError("command_count_over_budget")
                self.sequence += 1
                diagnostic = str(self.diagnostics / (str(self.sequence) + ".json"))
            command = [sys.executable, str(Path(__file__).resolve()),
                       "--guard", str(reader), diagnostic, self.family, str(launch_deadline), str(self.deadline),
                       "--", *map(str, argv)]
        try:
            # The only raw spawn. Only the guardian uses the unguarded branch.
            launch_env = {key: value for key, value in env.items() if key != "HIDE_LIVE_CHECK_OWNER"} if _guarded else env
            admit()
            child = subprocess.Popen(command, env=launch_env, cwd=cwd, stdin=stdin,
                                     stdout=stdout, stderr=stderr,
                                     start_new_session=True,
                                     pass_fds=(() if reader is None else (reader,)))
        except BaseException:
            if diagnostic:
                self.sequence -= 1
            if writer is not None:
                os.close(writer)
            raise
        finally:
            if reader is not None:
                os.close(reader)
        self.children[child] = writer
        if _guarded and diagnostic:
            self.receipts[child] = Path(diagnostic)
        return child

    def spawn_in_job(self, argv, *, env, cwd, stdin, stdout, stderr, admit):
        """Windows: the child starts suspended, joins a job of its own, and
        only then runs, so everything it starts is in the job. The job refuses
        the process and the memory past the caps, and closing its handle,
        which no child inherits, ends whatever it still holds."""
        if self.port is None:
            self.port = windows.JobPort({
                windows.JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT: "owned_processes_over_budget:" + json.dumps(
                    {"descendants": MAX_DESCENDANTS}, separators=(",", ":")),
                windows.JOB_OBJECT_MSG_JOB_MEMORY_LIMIT: "owned_processes_over_budget:" + json.dumps(
                    {"memory_bytes": MAX_RSS_BYTES}, separators=(",", ":"))})
        self.jobs += 1
        job = windows.Job(self.port.handle, self.jobs, processes=MAX_DESCENDANTS, memory=MAX_RSS_BYTES)
        self.port.register(job)
        try:
            admit()
            child = subprocess.Popen(argv, env={key: value for key, value in env.items()
                                                if key != "HIDE_LIVE_CHECK_OWNER"},
                                     cwd=cwd, stdin=stdin, stdout=stdout, stderr=stderr,
                                     creationflags=windows.CREATE_SUSPENDED | windows.console_flags())
        except BaseException:
            self.port.forget(job)
            job.close()
            raise
        try:
            job.adopt(child.pid)
        except BaseException as error:
            # Suspended, it has run nothing; a job that took it ends it on close.
            child.kill()
            try:
                child.wait(timeout=END_SECONDS)
            finally:
                for stream in (child.stdin, child.stdout, child.stderr):
                    if stream is not None:
                        stream.close()
                self.port.forget(job)
                job.close()
            raise ProcessError("job_start_failed:" + str(error)[:MAX_FAILURE_REASON]) from error
        self.children[child] = job
        return child

    def read_output(self, child, end, output, threads):
        """Windows: both pipes drained by threads into `output`, as Windows
        waits on no pipe in a selector; the same cancellation, deadline and
        byte cap. The caller joins `threads` once the child has ended."""
        lock, finished, over = threading.Lock(), threading.Event(), threading.Event()
        open_streams = [2]

        def drain(stream, index):
            try:
                while data := os.read(stream.fileno(), 65536):
                    with lock:
                        if sum(map(len, output)) + len(data) > MAX_OUTPUT:
                            over.set()
                            return
                        output[index].extend(data)
            finally:
                with lock:
                    open_streams[0] -= 1
                    if not open_streams[0]:
                        finished.set()
        for index, stream in enumerate((child.stdout, child.stderr)):
            threads.append(threading.Thread(target=drain, args=(stream, index), daemon=True))
            threads[-1].start()
        while not finished.wait(POLL_SECONDS):
            if over.is_set():
                raise ProcessError("command_output_over_budget")
            if self.cancelled.is_set():
                raise ProcessError("run_cancelled")
            if time.monotonic() >= end:
                raise ProcessError("command_timeout")
        if over.is_set():
            raise ProcessError("command_output_over_budget")

    def run(self, argv, *, env, cwd=None, seconds=COMMAND_SECONDS, check=True, deadline=None,
            stdin=subprocess.DEVNULL):
        if seconds > COMMAND_SECONDS:
            # A command never runs past the cap; a longer wait is not shortened behind the caller's back.
            raise ValueError(f"a command runs at most {COMMAND_SECONDS} s, not {seconds} s")
        end = min(self.deadline, time.monotonic() + seconds,
                  self.deadline if deadline is None else deadline)
        child = self.spawn(argv, env=env, cwd=cwd, deadline=end, stdin=stdin)
        output = [bytearray(), bytearray()]
        threads = []
        try:
            if WINDOWS:
                self.read_output(child, end, output, threads)
            else:
                with selectors.DefaultSelector() as selector:
                    for index, stream in enumerate((child.stdout, child.stderr)):
                        os.set_blocking(stream.fileno(), False)
                        selector.register(stream, selectors.EVENT_READ, index)
                    while selector.get_map():
                        if self.cancelled.is_set():
                            raise ProcessError("run_cancelled")
                        if time.monotonic() >= end:
                            raise ProcessError("command_timeout")
                        for key, _ in selector.select(POLL_SECONDS):
                            data = os.read(key.fileobj.fileno(), 65536)
                            if not data:
                                selector.unregister(key.fileobj)
                            else:
                                output[key.data].extend(data)
                                if sum(map(len, output)) > MAX_OUTPUT:
                                    raise ProcessError("command_output_over_budget")
            code = child.wait(timeout=max(0.01, end - time.monotonic()))
            if check and code:
                raise ProcessError(f"command_exit_{code}")
            return code, *(bytes(value).decode("utf-8", errors="replace") for value in output)
        finally:
            try:
                try:
                    self.end(child)
                except ProcessError as error:
                    reasons = [line[:MAX_FAILURE_REASON] for line in output[1].decode("utf-8", errors="replace").splitlines()
                               if line.startswith(GUARDIAN_FAILURES)]
                    raise type(error)(str(error) + (": " + "; ".join(reasons[:4]) if reasons else "")) from error
            finally:
                # The ended tree closed the pipes' other ends, so the readers stop.
                for thread in threads:
                    thread.join(timeout=2)
                child.stdout.close()
                child.stderr.close()

    def end(self, child):
        if WINDOWS:
            return self.end_job(child)
        writer = self.children.pop(child, None)
        if writer is not None:
            os.close(writer)
        if child.poll() is None:
            if writer is None:
                child.send_signal(signal.SIGCONT)
                child.terminate()
            try:
                child.wait(timeout=END_SECONDS)
            except subprocess.TimeoutExpired as error:
                # Killing the guardian here would discard its ownership proof.
                self.children[child] = None
                raise ProcessSafetyError("cleanup_unconfirmed") from error
        if writer is not None and (child.returncode == 125 or child.returncode < 0):
            raise ProcessSafetyError("guardian_cleanup_or_resource_failure")
        receipt = self.receipts.pop(child, None)
        if receipt is not None:
            try:
                with receipt.open("rb") as stream:
                    data = stream.read(MAX_OUTPUT + 1)
                if len(data) > MAX_OUTPUT or json.loads(data).get("confirmed") is not True:
                    raise ProcessSafetyError("guardian_cleanup_receipt_unconfirmed")
            except (OSError, ValueError) as error:
                raise ProcessSafetyError("guardian_cleanup_receipt_missing_or_invalid") from error

    def end_job(self, child):
        """Windows: ends everything the child's job holds and confirms the job
        is empty; a job that is not stays owned, and its failure stays on
        record for attribution_report."""
        job = self.children.pop(child, None)
        if job is None:
            return
        try:
            job.terminate()
            end = time.monotonic() + END_SECONDS
            while job.active():
                if time.monotonic() >= end:
                    raise ProcessSafetyError("cleanup_unconfirmed")
                time.sleep(0.01)
            child.wait(timeout=END_SECONDS)
            self.port.settle(job, END_SECONDS)
        except (OSError, subprocess.TimeoutExpired, ProcessSafetyError) as error:
            # Its handle stays open: closing it is still the last resort.
            self.children[child] = job
            reason = str(error)[:MAX_FAILURE_REASON]
            self.unconfirmed.append(reason)
            if isinstance(error, ProcessSafetyError):
                raise
            raise ProcessSafetyError("cleanup_unconfirmed:" + reason) from error
        self.port.forget(job)
        job.close()
        if job.failure:
            raise ProcessSafetyError(job.failure)

    def usage(self):
        if WINDOWS:
            # Only the bell check reads it, and it runs on macOS and Linux.
            raise ProcessError("usage_not_built_for_windows")
        table = snapshot()
        # Group guardians enforce caps; a host-wide partial view is only a
        # sampled resource summary, never an ownership or cleanup verdict.
        current = {}
        for child in self.children:
            if child.poll() is None:
                current.update(descendants(table, child.pid))
        if len(current) > MAX_CHILDREN * (MAX_DESCENDANTS + 1):
            raise ProcessError("owned_processes_over_budget")
        measured = {"descendants": len(current), **self.rss_samples.measure(current),
                    "in_flight": len(self.children), "sampled": True}
        if measured["rss_bytes"] > MAX_CHILDREN * MAX_RSS_BYTES:
            raise ProcessError("owned_processes_over_budget")
        return measured

    def close(self):
        failures = []
        for child in list(self.children)[::-1]:
            try:
                self.end(child)
            except (OSError, ProcessError) as error:
                failures.append(type(error).__name__)
        if self.port is not None and not self.children:
            self.port.close()
            self.port = None
        if failures:
            raise ProcessError("cleanup_unconfirmed: " + ",".join(failures))

    def rss_report(self):
        controller, guardians = self.rss_samples.summary(), self.guardian_rss_samples
        return {"controller": controller, "guardians": guardians,
                "missed": None if guardians is None else controller["missed"] + guardians["missed"],
                "max_consecutive_misses": None if guardians is None else max(
                    controller["max_consecutive_misses"], guardians["max_consecutive_misses"]),
                "consecutive_miss_limit": RSS_CONSECUTIVE_MISS_LIMIT}

    def attribution_report(self):
        # A failed reread cannot reuse earlier complete guardian accounting.
        self.guardian_rss_samples = None
        if WINDOWS:
            if self.unconfirmed:
                raise ProcessError("job_cleanup_unconfirmed:" + json.dumps(self.unconfirmed[:4]))
            return {"status": "job", "processes": [], "limit": MAX_DESCENDANTS,
                    "additional_records_omitted": False, "rss_samples": None,
                    "limitation": "Each child and everything it starts is held by its own job object; "
                                  "a program Windows starts on its behalf (a service, a COM server, "
                                  "a scheduled task) is outside it."}
        records = {}
        omitted = False
        rss_missed, rss_max_consecutive = 0, 0
        if self.diagnostics is not None:
            count = 0
            for path in self.diagnostics.iterdir():
                count += 1
                with path.open("rb") as stream:
                    data = stream.read(MAX_OUTPUT + 1)
                if len(data) > MAX_OUTPUT:
                    raise ProcessError("process_diagnostic_over_budget")
                record = json.loads(data)
                if not isinstance(record, dict) or record.get("confirmed") is not True:
                    raise ProcessError("guardian_cleanup_receipt_unconfirmed")
                entries = record.get("unattributed")
                if (type(record.get("additional_records_omitted")) is not bool
                        or not isinstance(entries, list) or len(entries) > MAX_DESCENDANTS
                        or any(not isinstance(item, dict)
                               or type(item.get("pid")) is not int or item["pid"] <= 0
                               or type(item.get("birth")) is not int or item["birth"] < 0
                               or not isinstance(item.get("executable"), str) or len(item["executable"]) > 32
                               for item in entries)):
                    raise ProcessError("guardian_cleanup_receipt_invalid")
                samples = record.get("rss_samples")
                if (not isinstance(samples, dict) or any(type(samples.get(key)) is not int
                        for key in ("missed", "max_consecutive_misses", "consecutive_miss_limit"))
                        or samples["consecutive_miss_limit"] != RSS_CONSECUTIVE_MISS_LIMIT
                        or not 0 <= samples["max_consecutive_misses"] <= RSS_CONSECUTIVE_MISS_LIMIT
                        or samples["missed"] < samples["max_consecutive_misses"]):
                    raise ProcessError("guardian_rss_receipt_invalid")
                rss_missed += samples["missed"]
                rss_max_consecutive = max(rss_max_consecutive, samples["max_consecutive_misses"])
                omitted |= record["additional_records_omitted"]
                for item in record["unattributed"]:
                    key = (item["pid"], item["birth"])
                    if key not in records and len(records) >= MAX_DESCENDANTS:
                        omitted = True
                    else:
                        records[key] = item
            if count != self.sequence:
                raise ProcessError("guardian_cleanup_receipt_missing")
            self.guardian_rss_samples = {"missed": rss_missed, "max_consecutive_misses": rss_max_consecutive,
                                        "consecutive_miss_limit": RSS_CONSECUTIVE_MISS_LIMIT}
        return {"status": "출처 확인 못 함", "processes": list(records.values()),
                "limit": MAX_DESCENDANTS, "additional_records_omitted": omitted,
                "rss_samples": self.rss_report(),
                "limitation": "An unseen double-fork descendant that clears its marker and leaves the owned group may escape attribution."}

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def group_exists(group):
    try:
        os.killpg(group, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        state = group_state(snapshot(group))
        if state is ProcessState.LIVE:
            raise
        # A terminal group can still hold zombies awaiting another parent's
        # reap. Presence is not signalability and is never cleanup success.
        return state is ProcessState.ZOMBIE


class ExitWatch:
    """Wakes as soon as an unreaped child exits, and never reaps it: the root's
    zombie keeps its process group reserved until the guardian waits for it.

    Kqueue's NOTE_EXIT on macOS and a pidfd on Linux report the exit, so a
    command that ends at once is not held for a whole poll (issue 828: one
    fixed 100 ms wait per command, about 110 commands a two-scene run).
    Elsewhere the wait is the poll it always was. The poll still bounds every
    wait, so cancellation and the descendant budgets keep their cadence.
    """

    def __init__(self, pid):
        self.exited = False
        self.kqueue = None
        self.pidfd = None
        try:
            if hasattr(select, "kqueue"):
                self.kqueue = select.kqueue()
                self.kqueue.control([select.kevent(
                    pid, filter=select.KQ_FILTER_PROC,
                    flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT,
                    fflags=select.KQ_NOTE_EXIT)], 0, 0)
            elif hasattr(os, "pidfd_open"):
                self.pidfd = os.pidfd_open(pid)
        except ProcessLookupError:
            # Only an exited child refuses: its unreaped zombie holds the pid.
            self.exited = True
        except BaseException:
            self.close()
            raise

    def wait(self, seconds, cancelled):
        """Waits at most `seconds` for the exit; true once it has happened."""
        if self.exited:
            cancelled.wait(seconds)
        elif self.kqueue is not None:
            self.exited = bool(self.kqueue.control(None, 1, seconds))
        elif self.pidfd is not None:
            self.exited = bool(select.select([self.pidfd], [], [], seconds)[0])
        else:
            cancelled.wait(seconds)
        return self.exited

    def close(self):
        if self.kqueue is not None:
            self.kqueue.close()
        if self.pidfd is not None:
            os.close(self.pidfd)


def guard(reader: int, argv: list[str], diagnostic: str = "", family: str = "",
          launch_deadline: float | None = None, run_deadline: float | None = None) -> int:
    """`run_deadline` the owner's, on the monotonic clock every process of
    the machine shares, so a child started first ends with the run, not
    `RUN_SECONDS` after its own start."""
    cancelled = threading.Event()
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, lambda *_: cancelled.set())
    # Keep the native root unreaped until all group signals are finished. Its
    # reserved PID also reserves the PGID, including after the root exits.
    signal.signal(signal.SIGCHLD, signal.SIG_DFL)
    if sys.platform.startswith("linux"):
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(36, 1, 0, 0, 0) != 0:
            return 125

    def watch_owner():
        try:
            os.read(reader, 1)
        finally:
            os.close(reader)
            cancelled.set()

    threading.Thread(target=watch_owner, daemon=True).start()
    owner = OwnedProcesses(cancelled)
    child = None
    group = None
    observed = {}
    unattributed = {}
    omitted = False
    failed = False
    launch_timed_out = False
    confirmed = False
    root_exited = False
    exit_watch = None
    rss_samples = RssSamples()
    initial = snapshot()
    identity = initial[os.getpid()]
    # Neither the controller nor its ancestors were started by this guardian.
    # An inherited prior token must never enlist the control plane itself.
    excluded = control_plane(initial, identity.pid)
    if not issued_family(family):
        raise ProcessError("owner_family_not_issued")
    marker = f"{family}:{identity.pid}:{identity.birth}:{secrets.token_hex(32)}"
    deadline = time.monotonic() + RUN_SECONDS if run_deadline is None else run_deadline

    def unknown(process):
        nonlocal omitted
        if process.parent != 1 and not process.traced:
            return
        key = (process.pid, process.birth)
        if key not in unattributed and len(unattributed) < MAX_DESCENDANTS:
            unattributed[key] = {"pid": process.pid, "birth": process.birth,
                                 "executable": process.name[:32]}
        elif key not in unattributed:
            omitted = True

    def collect(include_group=True):
        table = snapshot(group) if include_group else ProcessTable()
        require_complete(table)
        # Never infer ownership from an unreadable host orphan. A readable
        # token from a dead guardian in this checkout proves an earlier run;
        # another live guardian's token belongs to concurrent work.
        if sys.platform == "darwin" or sys.platform.startswith("linux"):
            all_processes = snapshot()
            retained = {}
            for pid, previous in observed.items():
                state = identity_state(pid, previous.birth, uid=previous.uid, table=all_processes)
                if state is not ProcessState.VANISHED:
                    retained[pid] = all_processes[pid]

            def matches(entry):
                prefix = b"HIDE_LIVE_CHECK_OWNER="
                if not entry.startswith(prefix):
                    return False
                value = entry.split(b"=", 1)[1].decode("ascii", errors="replace")
                if value == marker:
                    return True
                parts = value.split(":")
                if (len(parts) != 4 or not issued_family(parts[0]) or
                        not parts[1].isdecimal() or not parts[2].isdecimal() or
                        len(parts[3]) != 64 or any(c not in "0123456789abcdef" for c in parts[3])):
                    return False
                guardian_pid, birth = int(parts[1]), int(parts[2])
                try:
                    state = identity_state(guardian_pid, birth, table=all_processes)
                except RuntimeError:
                    # Unknown prior supervision grants no adoption authority.
                    return False
                return state is not ProcessState.LIVE

            candidates = {pid: process for pid, process in all_processes.items()
                          if excluded.get(pid) != process.birth}
            extras = marked_descendants(candidates, matches, known=observed,
                                        unknown=unknown)
            table.update(extras)
            # Retain an earlier token proof while that exact birth remains.
            table.update(retained)
        observed.clear()
        observed.update(table)
        return table

    def signal_proven(table, signum, *, skip_group=True):
        errors = []
        # Every retained identity already has group/token proof. Recheck each
        # birth independently so an unavailable peer cannot abandon the rest.
        for process in table.values():
            if (skip_group and process.group == group) or process.zombie:
                continue
            try:
                signal_identity(process, signum)
            except Exception as error:
                errors.append({"pid": process.pid, "type": type(error).__name__,
                               "reason": str(error)[:MAX_FAILURE_REASON]})
        return errors

    def signal_owned(signum):
        errors = []
        try:
            table = collect()
            needs_group = any(process.group == group and not process.zombie for process in table.values())
            skip_group = True
        except Exception as error:
            # Metadata failure still ends the reserved group and remains a
            # failure. Retained positive identities get independent attempts.
            table = dict(observed)
            needs_group, skip_group = True, False
            errors.append({"type": type(error).__name__, "reason": str(error)[:MAX_FAILURE_REASON]})
        if needs_group:
            try:
                signal_reserved_group(group, signum)
            except Exception as error:
                errors.append({"type": type(error).__name__, "reason": str(error)[:MAX_FAILURE_REASON]})
        # Group signals are atomic with respect to membership. Escaped marked
        # helpers require a fresh birth check before each individual signal.
        errors.extend(signal_proven(table, signum, skip_group=skip_group))
        if errors:
            raise ProcessError("owned_signal_failures:" + json.dumps(errors, separators=(",", ":")))

    try:
        child = owner.spawn(argv, env={**os.environ, "HIDE_LIVE_CHECK_OWNER": marker},
                            stdin=None, stdout=None, stderr=None, _guarded=False, deadline=launch_deadline)
        group = child.pid
        # Popen returns only after start_new_session/setsid succeeded. Darwin
        # getpgid returns ESRCH for a fast, unreaped zombie; the group snapshot
        # below remains the authoritative live/terminal membership evidence.
        if group <= 1 or group == os.getpgrp():
            raise ProcessError("owned_group_unconfirmed")
        exit_watch = ExitWatch(child.pid)
        while not cancelled.is_set():
            table = collect()
            if len(table) > MAX_DESCENDANTS:
                raise ProcessError("owned_processes_over_budget:" + json.dumps(
                    {"descendants": len(table)}, separators=(",", ":")))
            rss = rss_samples.measure(table)["rss_bytes"]
            if rss > MAX_RSS_BYTES:
                raise ProcessError("owned_processes_over_budget:" + json.dumps(
                    {"descendants": len(table), "rss_bytes": rss, "largest": largest(table)}, separators=(",", ":")))
            root = table.get(child.pid)
            if root is None:
                raise ProcessError("unreaped_owned_root_missing")
            if root.group != group:
                raise ProcessError("owned_group_unconfirmed")
            if root.zombie:
                root_exited = True
                break
            if time.monotonic() >= deadline:
                raise ProcessError("guardian_run_timeout")
            exit_watch.wait(POLL_SECONDS, cancelled)
    except BaseException as error:
        if (child is None and launch_deadline is not None and time.monotonic() >= launch_deadline
                and isinstance(error, ProcessError) and str(error) in ("command_timeout", "run_cancelled_or_timed_out")):
            # Nothing launched; this is the controller's expired operation,
            # not a failed resource guardian. The empty cleanup is receipted.
            launch_timed_out = True
            sys.stderr.write("guardian_launch_timeout\n")
        else:
            sys.stderr.write("guardian_failure:" + type(error).__name__ + ":" + str(error) + "\n")
            failed = True
    finally:
        if exit_watch is not None:
            exit_watch.close()
        try:
            if child is not None:
                try:
                    # The unreaped root still reserves its group. Refresh
                    # that cheap group view before wait, retaining the last
                    # marker proofs without repeating the whole-host scan.
                    if root_exited:
                        current = snapshot(group)
                        require_complete(current)
                        observed.update(current)
                        current = dict(observed)
                    else:
                        current = collect()
                    active = any(not process.zombie for process in current.values())
                except BaseException:
                    active = True
                    failed = True
                if active:
                    # A table error must not prevent ending the reserved group.
                    for signum in (signal.SIGCONT, signal.SIGTERM):
                        try:
                            signal_owned(signum)
                        except BaseException as error:
                            failed = True
                            sys.stderr.write("guardian_signal_failure:" + type(error).__name__ + ":" + str(error) + "\n")
                    end = time.monotonic() + 0.5
                    while time.monotonic() < end:
                        try:
                            if not any(not p.zombie for p in collect().values()):
                                break
                        except BaseException:
                            failed = True
                            break
                        time.sleep(POLL_SECONDS)
                    try:
                        signal_owned(signal.SIGKILL)
                    except BaseException as error:
                        failed = True
                        sys.stderr.write("guardian_signal_failure:" + type(error).__name__ + ":" + str(error) + "\n")
                child.wait(timeout=1)
                # No group signal after wait: a future reused PGID is not ours.
                end = time.monotonic() + 2
                term_deadlines = {}
                while True:
                    if sys.platform.startswith("linux"):
                        linux_children_remain()
                    try:
                        remaining = collect(include_group=False)
                    except Exception as error:
                        # Missing metadata remains failure, but cannot skip a
                        # readable peer's TERM/grace/KILL stage. Retained
                        # identities still need independent fresh birth checks.
                        failed = True
                        sys.stderr.write("guardian_cleanup_failure:" + type(error).__name__ +
                                         ":" + str(error)[:MAX_FAILURE_REASON] + "\n")
                        remaining = dict(observed)
                    live_extras = [p for p in remaining.values() if not p.zombie]
                    identities = {(p.pid, p.birth) for p in live_extras}
                    term_deadlines = {key: value for key, value in term_deadlines.items() if key in identities}
                    new = {p.pid: p for p in live_extras if (p.pid, p.birth) not in term_deadlines}
                    errors = []
                    for signum in (signal.SIGCONT, signal.SIGTERM):
                        errors.extend(signal_proven(new, signum, skip_group=False))
                    for p in new.values():
                        term_deadlines[p.pid, p.birth] = min(end, time.monotonic() + 0.5)
                    # The released PGID grants no signal authority. Retained
                    # group proofs and readable tokens still authorize each
                    # independently rechecked birth, including same-group peers.
                    expired = {p.pid: p for p in live_extras
                               if time.monotonic() >= term_deadlines[p.pid, p.birth]}
                    errors.extend(signal_proven(expired, signal.SIGKILL, skip_group=False))
                    if errors:
                        # Preserve the failure while allowing other proven
                        # peers their grace/KILL and absence confirmation.
                        failed = True
                        sys.stderr.write("guardian_signal_failure:owned_signal_failures:" +
                                         json.dumps(errors, separators=(",", ":")) + "\n")
                    if not group_exists(group) and not remaining:
                        confirmed = True
                        break
                    if time.monotonic() >= end:
                        raise ProcessError("owned_processes_remain")
                    time.sleep(POLL_SECONDS)
            else:
                confirmed = True
        except BaseException as error:
            sys.stderr.write("guardian_cleanup_failure:" + type(error).__name__ + ":" + str(error) + "\n")
            failed = True
        if diagnostic:
            try:
                with open(diagnostic, "x", encoding="utf-8", opener=lambda path, flags: os.open(path, flags, 0o600)) as stream:
                    json.dump({"confirmed": confirmed, "unattributed": list(unattributed.values()),
                               "launch_timed_out": launch_timed_out,
                               "rss_samples": rss_samples.summary(),
                               "unattributed_limit": MAX_DESCENDANTS,
                               "additional_records_omitted": omitted}, stream)
            except BaseException as error:
                failed = True
                sys.stderr.write("guardian_cleanup_failure:diagnostic_write:" + type(error).__name__ + "\n")
    return 125 if failed or not confirmed else 124 if launch_timed_out else (child.returncode if child is not None else 125)


if __name__ == "__main__":
    if len(sys.argv) < 9 or sys.argv[1] != "--guard" or sys.argv[7] != "--":
        raise SystemExit(2)
    try:
        code = guard(int(sys.argv[2]), sys.argv[8:], sys.argv[3], sys.argv[4], float(sys.argv[5]),
                     float(sys.argv[6]))
    except BaseException as error:
        sys.stderr.write("guardian_failure:" + type(error).__name__ + "\n")
        code = 125
    raise SystemExit(code)
