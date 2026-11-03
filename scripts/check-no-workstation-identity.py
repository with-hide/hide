#!/usr/bin/env python3
"""Check tracked bytes without printing matched values.

Default scope is tracked checkout files, including unstaged edits, not the staged
blobs. Use --scope index to inspect exactly the staged blobs, including new files.
Neither scope inspects untracked files, history, releases, image metadata or
pixels. Home/workstation and email shapes cannot prove arbitrary names, account
aliases, encoded content or credentials absent. Keep those separate audit tasks.
"""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import threading


# One tracked file is read whole, so each read is bounded.
MAX_BYTES = 16 * 1024 * 1024
# A tracked source that outgrows that bound for a reason, with its own: the
# shared screens file the screen builders regenerate, which passed 16 MiB once
# the Factory AI screens joined it and 32 MiB once the View feedback boards
# joined Screen / Workspace. Every other file keeps MAX_BYTES.
LARGE_SOURCES = {"design/hide-screens.pen": 40 * 1024 * 1024}
MAX_DIAGNOSTICS = 200
MAX_MANIFEST_BYTES = 8 * 1024 * 1024
MAX_TRACKED_FILES = 100000
MAX_GIT_HEADER_BYTES = 1024
MAX_ROOT_BYTES = 64 * 1024
READ_CHUNK_BYTES = 64 * 1024
GIT_READ_SECONDS = 10
GIT_CLEANUP_SECONDS = 2
# Audited current fixtures: generic example; alice/al test component prefixes;
# me, remote and u stand for local/remote test users. Exact occurrences only.
NEUTRAL_ACCOUNTS = frozenset({"example", "alice", "al", "me", "remote", "u"})
BOUNDARY = r"(?:(?<![\w./\\])|(?<=file://))"
HOME = re.compile(BOUNDARY + r"(?:"
                  r"(?P<macos_home>/Users" r"/)|(?P<linux_home>/home" r"/)|"
                  r"(?P<windows_home>(?i:[A-Za-z]:[\\/]+Users[\\/]+)))")
WORKSTATION = re.compile(
    r"(?<![\w.-])(?P<account>[\w.]+(?:-[\w.]+)*)-"
    r"(?:MacBook(?:-Pro|-Air)?|Mac-mini|Mac-Studio|Mac-Pro|iMac|mbp)"
    r"(?:\.local)?(?![\w-])", re.I)
DYNAMIC_ACCOUNT = re.compile(
    r"\.?(?:\$[A-Za-z_][A-Za-z_0-9]*|\$\{[A-Za-z_][A-Za-z_0-9]*\})")
EVIDENCE = re.compile(
    r"^(?:docs/(?:verification|screenshots)(?:/|$)|spikes/[^/]+/evidence(?:/|$))")
PROFILE = re.compile(
    r"(?:^|/)(?:Cookies|History|Login Data|Web Data)(?:-(?:journal|wal|shm))?$"
    r"|(?:^|/)(?:Local State|Preferences|Secure Preferences)$"
    r"|\.pma$|(?:^|/)(?:Session_|Tabs_)[0-9]+$"
    r"|(?:^|/)(?:browser-profile|chromium-profile|chrome-profile)(?:/|$)", re.I)

EMAIL = re.compile(r"(?<![\w.+-])[A-Za-z0-9][A-Za-z0-9._%+-]*@"
                   r"(?P<domain>(?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,})(?![\w.-])")


# The project's published contact address, shown on the public site on purpose. It is an
# exact address, never its domain: withhide.dev is a Workspace domain whose other accounts
# belong to people, and their addresses stay refused.
PUBLIC_CONTACTS = frozenset({"hello@withhide.dev"})

# Exact, code-owned systemd namespace identifiers, not contact addresses.
# Do not exempt arbitrary unit suffixes: .service can also be an email domain.
SYSTEMD_IDENTIFIERS = frozenset({
    "systemd-journald@hide-relay.service", "journald@hide-relay.conf",
})


def email_findings(text):
    for match in EMAIL.finditer(text):
        if match.group().lower() in PUBLIC_CONTACTS or match.group() in SYSTEMD_IDENTIFIERS:
            continue
        domain = match.group("domain").lower()
        if domain in {"example.com", "example.net", "example.org"} or domain.endswith(
                (".example", ".invalid", ".test")):
            continue
        # A URL authority or scp-style remote is not a contact address.
        token_start = max(text.rfind(char, 0, match.start()) for char in " \t\r\n\"'<>") + 1
        prefix = text[token_start:match.start()]
        scheme = prefix.rfind("://")
        authority = scheme >= 0 and "/" not in prefix[scheme + 3:]
        remote = text[match.end():].startswith(":") and match.group().startswith("git@")
        if authority or remote:
            continue
        yield match


class ScanError(Exception):
    def __init__(self, classification, path="."):
        self.classification = classification
        self.path = path


def closing_quote(text, start, quote, depth):
    """Locate a delimiter at the same escaping level as its opening quote."""
    backslashes = 0
    for pos in range(start, len(text)):
        char = text[pos]
        if char in "\r\n\x00":
            break
        if char == quote:
            matches = (backslashes == depth if depth else
                       quote == "'" or backslashes % 2 == 0)
            if matches:
                return pos - depth, pos + 1
        backslashes = backslashes + 1 if char == "\\" else 0
    return None


def home_components(text):
    """Whitespace and punctuation belong to a name until its boundary is proven.

    A slash proves a component boundary. A terminal component needs either the
    end of an unquoted token or an unescaped, matching closing quote. Ambiguous
    quotes never establish an exemption. This is a lexical check, not a shell or
    programming-language evaluator.
    """
    quote = None
    quote_boundary = None
    escaped = False
    cursor = 0
    for match in HOME.finditer(text):
        for char in text[cursor:match.start()]:
            if char in "\r\n\x00":
                quote, quote_boundary, escaped = None, None, False
            elif escaped:
                escaped = False
            elif char == "\\" and quote != "'":
                escaped = True
            elif quote:
                if char == quote:
                    quote = None
                    quote_boundary = None
            elif char in "\"'`":
                quote = char
                quote_boundary = None
        cursor = match.start()
        start = end = match.end()
        component_quote = quote
        boundary = None
        before = match.start()
        if before and text[before - 1] in "\"'`":
            # An inner shell/JSON quote can be inside a source string. Pair it at
            # its own escaping level rather than treating punctuation as a name.
            component_quote = text[before - 1]
            depth = 0
            before -= 2
            while before >= 0 and text[before] == "\\":
                depth += 1
                before -= 1
            boundary = closing_quote(text, start, component_quote, depth)
        elif quote:
            if quote_boundary is None:
                # Cache one lookahead per outer token, including unmatched quotes.
                quote_boundary = closing_quote(text, start, quote, 0) or False
            boundary = quote_boundary or None
        separators = "/\\" if match.lastgroup == "windows_home" else "/"
        proven = component_quote is None
        while end < len(text):
            char = text[end]
            if boundary and end == boundary[0]:
                after = text[boundary[1]:boundary[1] + 1]
                proven = (not after or after.isspace() or after in ",;:)}]"
                          or component_quote == "`" and after == ".")
                break
            if char in separators:
                proven = component_quote is None or boundary is not None
                break
            if char in "\r\n\x00":
                proven = component_quote is None
                break
            end += 1
        if end > start:
            yield match, end, proven


def safe_path(path):
    pieces = []
    cursor = 0
    for match, end, _ in home_components(path):
        if match.end() < cursor:
            continue
        pieces.extend((path[cursor:match.end()], "[redacted]"))
        cursor = end
    pieces.append(path[cursor:])
    return EMAIL.sub("[redacted-email]", WORKSTATION.sub("[redacted-workstation]", "".join(pieces)))


def diagnostic(path, line, classification, scope):
    print(json.dumps({"path": safe_path(path), "line": line,
                      "class": classification, "scope": scope}), file=sys.stderr)


class GitReader:
    """At most one pipe reader, with a deadline portable to Windows pipes."""

    def __init__(self, process):
        self.process = process
        self.reader = None

    def read(self, operation):
        if GIT_READ_SECONDS <= 0:
            raise ScanError("git_read_timeout")
        done = threading.Event()
        outcome = []

        def run():
            try:
                outcome.append((operation(), None))
            except ScanError as error:
                outcome.append((None, error))
            except Exception:
                outcome.append((None, ScanError("git_read_failed")))
            finally:
                done.set()

        self.reader = threading.Thread(target=run, daemon=True)
        self.reader.start()
        if not done.wait(GIT_READ_SECONDS):
            raise ScanError("git_read_timeout")
        self.reader.join()
        self.reader = None
        value, error = outcome[0]
        if error:
            raise error
        return value

    def close(self, failed):
        process = self.process
        try:
            if failed and process.poll() is None:
                process.kill()
            if process.stdin:
                try:
                    process.stdin.close()
                except BrokenPipeError:
                    pass
            try:
                process.wait(timeout=GIT_CLEANUP_SECONDS)
            except subprocess.TimeoutExpired:
                process.kill()
                try:
                    process.wait(timeout=GIT_CLEANUP_SECONDS)
                except subprocess.TimeoutExpired:
                    raise ScanError("git_cleanup_failed") from None
                if not failed:
                    raise ScanError("git_read_timeout")
        finally:
            if self.reader:
                self.reader.join(timeout=GIT_CLEANUP_SECONDS)
                if self.reader.is_alive():
                    raise ScanError("git_cleanup_failed")
            process.stdout.close()


@contextmanager
def owned_git_process(root, *args, input_pipe=False):
    """Only spawn point. EOF/broken output ends Git if the scanner is killed."""
    process = subprocess.Popen(["git", "--no-replace-objects", "-C", str(root), *args],
                               stdin=subprocess.PIPE if input_pipe else subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    helper = GitReader(process)
    failed = False
    try:
        yield helper
    except BaseException:
        failed = True
        raise
    finally:
        helper.close(failed)


def git_root():
    def read_root():
        data = helper.process.stdout.read(MAX_ROOT_BYTES + 1)
        if len(data) > MAX_ROOT_BYTES:
            raise ScanError("git_root_too_large")
        return data

    with owned_git_process(Path.cwd(), "rev-parse", "--show-toplevel") as helper:
        data = helper.read(read_root)
    if helper.process.returncode or not data:
        raise ScanError("git_read_failed")
    return Path(os.fsdecode(data).rstrip("\r\n")).resolve()


def tracked_entries(root):
    def read_manifest():
        entries = []
        pending = bytearray()
        total = 0
        while True:
            chunk = helper.process.stdout.read(min(READ_CHUNK_BYTES,
                                                  MAX_MANIFEST_BYTES - total + 1))
            total += len(chunk)
            if total > MAX_MANIFEST_BYTES:
                raise ScanError("git_manifest_byte_limit_exceeded")
            if not chunk:
                break
            pending.extend(chunk)
            start = 0
            while (end := pending.find(b"\0", start)) >= 0:
                if len(entries) >= MAX_TRACKED_FILES:
                    raise ScanError("tracked_file_limit_exceeded")
                row = pending[start:end]
                start = end + 1
                try:
                    header, raw_path = row.split(b"\t", 1)
                    mode, oid, stage = header.decode("ascii").split()
                except (ValueError, UnicodeError):
                    raise ScanError("git_manifest_invalid") from None
                if not raw_path or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", oid):
                    raise ScanError("git_manifest_invalid")
                path = os.fsdecode(bytes(raw_path))
                if stage != "0":
                    raise ScanError("unmerged_index", path)
                if mode not in {"100644", "100755", "120000"}:
                    raise ScanError("unsupported_index_entry", path)
                entries.append((path, oid))
            del pending[:start]
        if pending:
            raise ScanError("git_manifest_invalid")
        return entries

    with owned_git_process(root, "ls-files", "--stage", "-z") as helper:
        entries = helper.read(read_manifest)
    if helper.process.returncode:
        raise ScanError("git_read_failed")
    if not entries:
        raise ScanError("no_tracked_files")
    return entries


def read_limit(path):
    return LARGE_SOURCES.get(path, MAX_BYTES)


def checkout_bytes(root, path):
    target = root / path
    try:
        if not target.parent.resolve().is_relative_to(root):
            raise ScanError("checkout_path_outside_repository", path)
        mode = target.lstat().st_mode
        if stat.S_ISLNK(mode):
            return os.fsencode(os.readlink(target))
        if not stat.S_ISREG(mode):
            raise ScanError("unsupported_checkout_entry", path)
        limit = read_limit(path)
        if target.stat().st_size > limit:
            raise ScanError("tracked_blob_too_large", path)
        with target.open("rb") as source:
            data = source.read(limit + 1)
        if len(data) > limit:
            raise ScanError("tracked_blob_too_large", path)
        return data
    except OSError:
        raise ScanError("checkout_read_failed", path) from None


# A package's deprecation notice, as the npm registry wrote it into the
# lockfile pnpm generates: the package author's words, often with their own
# contact, and nothing about this workstation. Every other line of the
# lockfile, and that key in any other file, is still checked.
LOCKFILE = "pnpm-lock.yaml"
REGISTRY_NOTICE = re.compile(r"^[ \t]*deprecated: ", re.MULTILINE)


def registry_notice(text, start):
    line_start = text.rfind("\n", 0, start) + 1
    return REGISTRY_NOTICE.match(text, line_start) is not None


def content_findings(data, path=""):
    # Windows text files may carry UTF-16; a BOM makes that interpretation exact.
    encoding = "utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8"
    text = data.decode(encoding, errors="replace")
    for match, end, proven in home_components(text):
        account = text[match.end():end]
        if proven and (account in NEUTRAL_ACCOUNTS or DYNAMIC_ACCOUNT.fullmatch(account)):
            continue
        yield text.count("\n", 0, match.start()) + 1, match.lastgroup
    for match in WORKSTATION.finditer(text):
        if match.group("account") not in NEUTRAL_ACCOUNTS:
            yield text.count("\n", 0, match.start()) + 1, "named_workstation"

    # Contact addresses are checked in text, avoiding random binary byte matches.
    if "\x00" not in text:
        lockfile = path == LOCKFILE
        for match in email_findings(text):
            if lockfile and registry_notice(text, match.start()):
                continue
            yield text.count("\n", 0, match.start()) + 1, "contact_email"


def index_bytes(batch, path, oid):
    def read_blob():
        process = batch.process
        process.stdin.write(oid.encode("ascii") + b"\n")
        process.stdin.flush()
        raw_header = process.stdout.readline(MAX_GIT_HEADER_BYTES + 1)
        if len(raw_header) > MAX_GIT_HEADER_BYTES or not raw_header.endswith(b"\n"):
            raise ScanError("index_blob_read_failed", path)
        header = raw_header.split()
        if len(header) != 3 or header[0] != oid.encode("ascii") or header[1] != b"blob":
            raise ScanError("index_blob_read_failed", path)
        try:
            size = int(header[2])
        except ValueError:
            raise ScanError("index_blob_read_failed", path) from None
        if size < 0 or size > read_limit(path):
            raise ScanError("tracked_blob_too_large", path)
        data = process.stdout.read(size)
        if len(data) != size or process.stdout.read(1) != b"\n":
            raise ScanError("index_blob_read_failed", path)
        return data

    return batch.read(read_blob)


@contextmanager
def owned_git_batch(root, scope):
    """One local helper; stdin EOF ends it even if the scanner is killed."""
    if scope == "index":
        with owned_git_process(root, "cat-file", "--batch", input_pipe=True) as batch:
            yield batch
    else:
        batch = None
        yield batch


def scan(root, scope):
    entries = tracked_entries(root)
    failures = 0
    with owned_git_batch(root, scope) as batch:
        for path, oid in entries:
            classification = "tracked_run_artifact" if EVIDENCE.search(path) else (
                "browser_profile_artifact" if PROFILE.search(path) else None)
            if classification:
                diagnostic(path, 0, classification, scope)
                failures += 1
            else:
                data = index_bytes(batch, path, oid) if batch else checkout_bytes(root, path)
                for line, classification in content_findings(data, path):
                    diagnostic(path, line, classification, scope)
                    failures += 1
                    if failures >= MAX_DIAGNOSTICS:
                        raise ScanError("diagnostic_limit_exceeded")
            if failures >= MAX_DIAGNOSTICS:
                raise ScanError("diagnostic_limit_exceeded")
    print(json.dumps({"scope": scope, "tracked_files": len(entries), "findings": failures,
                      "status": "fail" if failures else "pass"}))
    return int(bool(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--scope", choices=("checkout", "index"), default="checkout",
                        help="tracked checkout bytes (default) or exact staged blobs")
    args = parser.parse_args()
    try:
        return scan(git_root(), args.scope)
    except ScanError as error:
        diagnostic(error.path, 0, error.classification, args.scope)
    except OSError:
        diagnostic(".", 0, "scan_io_failed", args.scope)
    return 1


if __name__ == "__main__":
    sys.exit(main())
