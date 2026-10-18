"""Bounded detection of this run's traces; no matched bytes leave this module."""

import json
import os
from pathlib import Path
import re
import stat
from urllib.parse import urlsplit

from agent_live_check.protection import ProtectionError

MAX_SCAN_BYTES = 1024 ** 3


def path_aliases(path):
    names = {str(path.absolute()), str(path.resolve())}
    for public in ('/var/', '/tmp/'):
        private = '/private' + public
        for name in tuple(names):
            if name.startswith(private):
                names.add(public + name[len(private):])
            elif name.startswith(public):
                names.add(private + name[len(public):])
    return names


def escaped(name):
    slashes = name.replace('/', r'\/')
    return {name, json.dumps(name)[1:-1], slashes, json.dumps(slashes)[1:-1]}


class Markers:
    def __init__(self, run=None, values=(), home=None):
        self.run = run.resolve() if run is not None else None
        literals, bounded = set(), set()
        for value in values:
            if value.startswith('http://'):
                address = urlsplit(value)
                hosts = {address.hostname}
                if address.hostname in {'127.0.0.1', 'localhost'}:
                    hosts.update(('127.0.0.1', 'localhost'))
                for host in hosts:
                    authority = f'{host}:{address.port}'
                    bounded.update((authority, f'http://{authority}'))
            else:
                literals.add(value)
        if run is not None:
            bounded.update(path_aliases(run))
            if home is not None:
                for name in tuple(bounded):
                    for parent in path_aliases(home):
                        if name.startswith(parent.rstrip('/') + '/'):
                            bounded.add('~/' + name[len(parent.rstrip('/')) + 1:])
        paths = {value.encode() for name in bounded if name for value in escaped(name)}
        self.literals = tuple(value.encode() for name in literals if name for value in escaped(name))
        # A sibling name or longer port is not this run. Delimiters also cover
        # JSON strings and escaped path separators without decoding private data.
        self.pattern = (re.compile(b'(?:' + b'|'.join(re.escape(value) for value in sorted(paths, reverse=True))
                                   + rb''')(?=$|[/\\\s"'\)\]\},;])''') if paths else None)
        self.values = (*self.literals, *paths)

    def contains(self, content, *, final=True):
        return (any(value in content for value in self.literals) or
                bool(self.pattern and any(final or match.end() < len(content)
                                          for match in self.pattern.finditer(content))))

    def link(self, path):
        return (self.run is not None and path.resolve().is_relative_to(self.run))

    def tree(self, path, budget):
        """Inspect only new top-level entries, with one bounded traversal."""
        pending, found = [path], False
        overlap = max(map(len, self.values), default=1)
        while pending:
            current = pending.pop()
            budget[0] -= 1
            if budget[0] < 0:
                raise ProtectionError('operator_watch_entry_budget_exceeded')
            try:
                info = current.lstat()
            except FileNotFoundError:
                continue
            if not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)
                    or stat.S_ISLNK(info.st_mode)):
                continue
            named = self.contains(os.fsencode(current.name))
            if stat.S_ISLNK(info.st_mode):
                try:
                    target = os.readlink(current)
                    found |= named or self.link(current) or self.contains(os.fsencode(target))
                except FileNotFoundError:
                    continue
            elif stat.S_ISDIR(info.st_mode):
                try:
                    with os.scandir(current) as entries:
                        for entry in entries:
                            if len(pending) >= budget[0]:
                                raise ProtectionError('operator_watch_entry_budget_exceeded')
                            pending.append(Path(entry.path))
                    found |= named
                except FileNotFoundError:
                    continue
            elif stat.S_ISREG(info.st_mode):
                try:
                    fd = os.open(current, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
                except FileNotFoundError:
                    continue
                with os.fdopen(fd, 'rb') as stream:
                    opened = os.fstat(stream.fileno())
                    if not stat.S_ISREG(opened.st_mode):
                        continue
                    if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
                        raise ProtectionError('operator_watch_entry_changed_during_read')
                    found |= named
                    tail = b''
                    while chunk := stream.read(1024 ** 2):
                        budget[1] -= len(chunk)
                        if budget[1] < 0:
                            raise ProtectionError('operator_watch_marker_bytes_over_budget')
                        data = tail + chunk
                        found |= self.contains(data, final=False)
                        tail = data[-overlap:] if overlap else b''
                    found |= self.contains(tail)
        return found
