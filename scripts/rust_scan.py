"""How the repository's source checks read Rust: comments and literals
blanked with line numbers kept, what only a test build compiles told apart,
and the module each file of a crate's targets belongs to, for
`check-core-touches-no-machine.py`, `check-agent-folders.sh` and
`scripts/ci-repeat.py`.
"""
import pathlib
import re

_LITERALS = (
    r"b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^'\\\n])'"
    r"|(?<![A-Za-z0-9_])b?r(#*)\".*?\"\1"
    r"|b?\"(?:\\.|[^\"\\])*\""
)
_COMMENTS_AND_LITERALS = re.compile(r"/\*.*?\*/|//[^\n]*|" + _LITERALS, re.S)
_TEST_MODULE = re.compile(
    r"#\[cfg\((?:all\()?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
)
# What a module declaration looks like once comments and literals are blanked:
# the attributes written above it, the `mod name` and how it ends. A brace is
# its own token, so `mod x { ... }` can be told apart from `mod x;` and the
# enclosing items can be followed. The attributes are read with the
# declaration, because one written above it is the declaration's: a
# `#[cfg(test)]` there means only a test build compiles the module, and a
# `#[path]` there names its file.
_MODULE_TOKEN = re.compile(
    r"(?P<attrs>(?:#\[[^\]]*\]\s*)*)(?:\bpub(?:\([^)]*\))?\s+)?\bmod\s+(?P<name>[A-Za-z_]\w*)\s*(?P<end>[;{])"
    r"|(?P<brace>[{}])"
)
_PATH_ATTRIBUTE = re.compile(r'#\[path\s*=\s*"([^"]+)"\s*\]')
# A gate that turns a module on for a test build: `test`, `all(test, unix)`,
# `any(windows, test)`. `not(test)` is the one that turns it on elsewhere.
_TEST_GATE = re.compile(r"#\[cfg\((?!not\()(?P<body>[^]]*)\)\]")


def _blank(text: str) -> str:
    return re.sub(r"[^\n]", " ", text)


def strip(text: str, keep_literals: bool = False) -> str:
    """Blanks comments and, unless `keep_literals`, string and character
    literals, keeping line numbers: a brace in a string cannot end an item
    early and a path in a message is not code. A kept literal still hides
    the comment markers inside it."""

    def replace(match: re.Match) -> str:
        token = match.group(0)
        if keep_literals and not token.startswith(("//", "/*")):
            return token
        return _blank(token)

    return _COMMENTS_AND_LITERALS.sub(replace, text)


def strip_test_items(text: str) -> str:
    """Blanks every item behind `#[cfg(test)]`, keeping line numbers."""
    out = list(text)
    for match in re.finditer(r"#\[cfg\((all\()?test\b[^\]]*\]", text):
        start, position, depth, end = match.start(), match.end(), 0, None
        while position < len(text):
            char = text[position]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    end = position + 1
                    break
            elif char == ";" and depth == 0:
                end = position + 1
                break
            position += 1
        if end is None:
            continue
        for index in range(start, end):
            if out[index] != "\n":
                out[index] = " "
    return "".join(out)


def test_file(relative: str) -> bool:
    """A Rust file only tests use, by its name or a `tests` folder."""
    name = relative.rsplit("/", 1)[-1]
    return name == "tests.rs" or name.endswith("_tests.rs") or "/tests/" in f"/{relative}"


def test_modules(root: pathlib.Path, trees: list) -> set:
    """Module files (relative to `root`) a parent declares behind
    `#[cfg(test)]` (or a cfg that requires test), which only a test build
    compiles."""
    found = set()
    for parent in (path for tree in trees for path in tree.rglob("*.rs")):
        text = strip(parent.read_text())
        directory = parent.parent if parent.name in ("lib.rs", "mod.rs", "main.rs") else parent.with_suffix("")
        for match in _TEST_MODULE.finditer(text):
            name = match.group(1)
            for candidate in (directory / f"{name}.rs", directory / name / "mod.rs"):
                if candidate.exists():
                    found.add(candidate.relative_to(root).as_posix())
    return found


def _children_directory(file: pathlib.Path) -> pathlib.Path:
    """Where a module file's own children sit: beside `lib.rs`, `main.rs` and
    `mod.rs`, and below a file named after its module (`runtime.rs` keeps
    `runtime/`)."""
    return file.parent if file.name in ("lib.rs", "main.rs", "mod.rs") else file.with_suffix("")


def _declared_file(file: pathlib.Path, directory: pathlib.Path, name: str, path_attribute: str | None) -> pathlib.Path | None:
    """The file a module declaration reads.

    A `#[path]` is relative to the folder the declaring file sits in, so
    `#[path = "bootstrap.rs"]` in `ssh/host.rs` names `ssh/bootstrap.rs`.
    Without one, the name's own file and its `mod.rs` are tried in the folder
    the declaring module's children live in.
    """
    if path_attribute is not None:
        target = (file.parent / path_attribute)
        return target.resolve() if target.is_file() else None
    for candidate in (directory / f"{name}.rs", directory / name / "mod.rs"):
        if candidate.is_file():
            return candidate.resolve()
    return None


def module_tree(root: pathlib.Path, targets: list) -> dict:
    """Each source file's module path and what only a test build compiles in it.

    `targets` are a crate's build targets as `(name, path)` pairs
    (`("lib", src/lib.rs)`, `("bin", src/main.rs)`), whose modules are read the
    way rustc reads them: a declaration in one file reaches the file its name
    or `#[path]` names, and a `mod name { ... }` block holds what it declares.
    The answer, keyed by the resolved path, is `{"target": "lib", "module":
    "runtime::tests", "test_only": True, "inline_tests": ["clear_tests"]}`: the
    target the file is built into, the module every test name of that file
    begins with, whether only a test build compiles the file, and the
    test-gated module blocks it declares.

    `scripts/ci-repeat.py` selects a changed test by these names, so a file
    no target declares reaches neither key and the caller falls back to the
    file's own path.
    """
    found = {}
    for name, target in targets:
        _read_modules(pathlib.Path(target).resolve(), name, "", False, found, set())
    return found


def _read_modules(file: pathlib.Path, target: str, module: str, test_only: bool, found: dict, seen: set) -> None:
    if file in seen:
        return
    seen.add(file)
    entry = found.setdefault(file, {"target": target, "module": module, "test_only": test_only, "inline_tests": []})
    raw = file.read_text(encoding="utf-8", errors="replace")
    # `strip` keeps every offset, so a span found in the stripped text reads
    # the same span of the raw text: the attributes are read there, because a
    # `#[path = "x.rs"]` value is a string literal `strip` blanks.
    text = strip(raw)
    # Each frame is one open item: the module it declares and the folder its
    # children read from. The file's own frame is the first.
    frames = [{"module": module, "directory": _children_directory(file), "test": test_only}]
    for match in _MODULE_TOKEN.finditer(text):
        if match.group("brace") == "{":
            frames.append(dict(frames[-1]))
            continue
        if match.group("brace") == "}":
            if len(frames) > 1:
                frames.pop()
            continue
        attributes = raw[match.start("attrs"):match.end("attrs")]
        name = match.group("name")
        enclosing = frames[-1]
        child = f"{enclosing['module']}::{name}" if enclosing["module"] else name
        gated = enclosing["test"] or _test_gate(attributes)
        path_attribute = _PATH_ATTRIBUTE.search(attributes)
        path_attribute = path_attribute.group(1) if path_attribute else None
        if match.group("end") == "{":
            frames.append({"module": child, "directory": enclosing["directory"] / name, "test": gated})
            if gated:
                entry["inline_tests"].append(child)
        else:
            declared = _declared_file(file, enclosing["directory"], name, path_attribute)
            if declared is not None:
                _read_modules(declared, target, child, gated, found, seen)


def _test_gate(attributes: str) -> bool:
    """True when a `#[cfg]` among a declaration's attributes turns it on only
    for a test build: `test`, `all(test, unix)` or `any(windows, test)`."""
    return any(re.search(r"(?<![A-Za-z_])test(?![A-Za-z_])", match.group("body"))
               for match in _TEST_GATE.finditer(attributes))
