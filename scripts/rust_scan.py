"""How the repository's source checks read Rust: comments and literals
blanked with line numbers kept, and what only a test build compiles told
apart, for `check-core-touches-no-machine.py`.
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
