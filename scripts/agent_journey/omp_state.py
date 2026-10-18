"""What omp writes in the operator's HOME in local mode, and how the journey
takes back its own part of it.

omp 18.7.0 keeps run state under `~/.omp` whatever its agent folder:
`run/daemons/<wyhash of the resolved project folder>` per project,
`run/session-owners/<session id>.lock` per session and `logs/omp.<date>.<pid>.log`
per process, with `logs/http-400-requests/` dumps of refused provider
requests and shared caches under `cache/`. The journey removes only what it
can prove is its own: a daemon folder whose name is the hash of a folder the
run made and a lock named by a session in the run's private agent folder. A
log, a dump or a cache names no session, so what changed there during the
run is listed, never removed.
An omp of the operator's in another folder never shares a run's daemon, as
the name is the folder's hash. omp itself refuses to start beside another
omp (Project Memory m163), so the journey measures it only when none runs.
"""

import shutil
import subprocess

# What a set-up omp 18.7.0 keeps in its agent folder's config.yml: below its
# CURRENT_SETUP_VERSION, 2, omp opens its setup wizard on a terminal, a
# provider sign-in first, and a turn never starts.
SET_UP = "setupVersion: 2\n"

M64 = (1 << 64) - 1
SECRET = (0xa0761d6478bd642f, 0xe7037ed1a0b428db, 0x8ebc6af09c88c6e3, 0x589965cc75374cc3)


def mum(a, b):
    product = a * b
    return product & M64, product >> 64


def mix(a, b):
    a, b = mum(a, b)
    return a ^ b


def read(data, offset, size):
    return int.from_bytes(data[offset:offset + size], "little")


def wyhash(data, seed=0):
    """Zig's std.hash.Wyhash, which Bun.hash.wyhash is and omp names a daemon folder by."""
    state = [seed ^ mix(seed ^ SECRET[0], SECRET[1])] * 3
    length = len(data)
    if length <= 16:
        if length >= 4:
            end, quarter = length - 4, (length >> 3) << 2
            a = (read(data, 0, 4) << 32) | read(data, quarter, 4)
            b = (read(data, end, 4) << 32) | read(data, end - quarter, 4)
        elif length:
            a, b = (data[0] << 16) | (data[length >> 1] << 8) | data[length - 1], 0
        else:
            a = b = 0
    else:
        start = 0
        if length >= 48:
            while start + 48 < length:
                for lane in range(3):
                    offset = start + 16 * lane
                    state[lane] = mix(read(data, offset, 8) ^ SECRET[lane + 1], read(data, offset + 8, 8) ^ state[lane])
                start += 48
            state[0] ^= state[1] ^ state[2]
        rest = data[start:]
        offset = 0
        while offset + 16 < len(rest):
            state[0] = mix(read(rest, offset, 8) ^ SECRET[1], read(rest, offset + 8, 8) ^ state[0])
            offset += 16
        a, b = read(data, length - 16, 8), read(data, length - 8, 8)
    a, b = mum(a ^ SECRET[1], b ^ state[0])
    return mix(a ^ SECRET[0] ^ length, b ^ SECRET[1])


def daemon_folder(operator, project):
    """The folder omp keeps a project's daemons in."""
    return operator / ".omp/run/daemons" / f"{wyhash(str(project.resolve()).encode()):016x}"


def running():
    """The pids of every omp running on this machine."""
    found = subprocess.run(["/usr/bin/pgrep", "-x", "omp"], capture_output=True, text=True, timeout=15)
    return [int(pid) for pid in found.stdout.split()]


def busy_refusal(pids):
    """Why omp is not measured while another omp runs, or None."""
    if not pids:
        return None
    return (f"another omp is running (pid {', '.join(map(str, pids))}), and omp does not start beside one; close it "
            "yourself, then rerun; the journey never ends another session's agent")


def sessions(agent_folder):
    """The omp sessions of the run's private agent folder, read before the
    run's cleanup removes it."""
    return sorted({path.stem.rsplit("_", 1)[-1] for path in agent_folder.glob("sessions/**/*.jsonl")})


def take_back(operator, projects, session_ids, since):
    """Removes this run's omp state from the operator's `~/.omp` once every
    process of the run ended; returns what was removed and the logs left, by path."""
    removed = []
    for project in projects:
        folder = daemon_folder(operator, project)
        if folder.is_dir() and not folder.is_symlink():
            shutil.rmtree(folder)
            removed.append(str(folder))
    for session in session_ids:
        lock = operator / ".omp/run/session-owners" / f"{session}.lock"
        if lock.is_file() and not lock.is_symlink():
            lock.unlink()
            removed.append(str(lock))
    # Its logs, the request dumps it writes under logs/ and its shared caches
    # name no session or project, so what changed during the run is listed.
    left = [str(path) for folder in (".omp/logs", ".omp/cache") for path in sorted((operator / folder).rglob("*"))
            if path.is_file() and not path.is_symlink() and path.stat().st_mtime >= since]
    return {"removed": removed, "logs_left": left}
