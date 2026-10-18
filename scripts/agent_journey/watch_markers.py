"""Bounded detection of this run's traces; no matched bytes leave this module."""

import json
import os
from pathlib import Path
import stat
from urllib.parse import urlsplit

from agent_live_check.protection import MAX_CONFIG_FILES, ProtectionError

MAX_SCAN_BYTES = 1024 ** 3


class Markers:
    def __init__(self, run=None, values=()):
        self.run = run.resolve() if run is not None else None
        names = set(values)
        names.update(urlsplit(value).netloc for value in values if value.startswith('http://'))
        if run is not None:
            names.update((str(run.absolute()), str(self.run)))
            if str(self.run).startswith('/private/var/'):
                names.add(str(self.run).removeprefix('/private'))
        self.values = tuple(value.encode() for name in names if name
                            for value in (name, json.dumps(name)[1:-1], name.replace('/', r'\/')))

    def contains(self, content):
        return any(value in content for value in self.values)

    def link(self, path):
        return (self.run is not None and path.resolve().is_relative_to(self.run))

    def tree(self, path, budget):
        """Inspect only new top-level entries, with one bounded traversal."""
        pending, found = [path], False
        overlap = max(map(len, self.values), default=1) - 1
        while pending:
            current = pending.pop()
            budget[0] -= 1
            if budget[0] < 0:
                raise ProtectionError('operator_watch_entry_budget_exceeded')
            info = current.lstat()
            found |= self.contains(os.fsencode(current.name))
            if stat.S_ISLNK(info.st_mode):
                found |= self.link(current) or self.contains(os.fsencode(os.readlink(current)))
            elif stat.S_ISDIR(info.st_mode):
                with os.scandir(current) as entries:
                    for entry in entries:
                        if len(pending) >= budget[0]:
                            raise ProtectionError('operator_watch_entry_budget_exceeded')
                        pending.append(Path(entry.path))
            elif stat.S_ISREG(info.st_mode):
                fd = os.open(current, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
                with os.fdopen(fd, 'rb') as stream:
                    opened = os.fstat(stream.fileno())
                    if (not stat.S_ISREG(opened.st_mode) or
                            (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino)):
                        raise ProtectionError('operator_watch_entry_changed_during_read')
                    tail = b''
                    while chunk := stream.read(1024 ** 2):
                        budget[1] -= len(chunk)
                        if budget[1] < 0:
                            raise ProtectionError('operator_watch_marker_bytes_over_budget')
                        data = tail + chunk
                        found |= self.contains(data)
                        tail = data[-overlap:] if overlap else b''
            else:
                raise ProtectionError('operator_watch_entry_unreadable')
        return found
