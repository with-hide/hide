#!/usr/bin/env python3
"""Hide AI writes what a person reads in the operator's language, which the
request carries and `hide_ai::AiRequest::system_prompt` names; no feature's
prompt names a language of its own (PRD ai-output-language D-01, B13).

This check fails when production string text in a Hide AI prompt source
holds Korean, Chinese or Japanese characters, or a fixed instruction about
the language to write in ("in Korean", "in the language of the title",
"in both Korean and English", "the card's own language").

The prompt sources are every production file that builds a
`hide_ai::AiRequest`, found by its construction, and the files listed in
`PROMPT_ONLY` whose prompts another file sends. `hide-ai` itself is the
boundary that owns the one language sentence and is not scanned. Test code
is exempt: test-only files and every item behind `#[cfg(test)]`.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Production trees whose crates may build a Hide AI request.
TREES = ["herdr-core/src", "hided/src", "hide-memory/src", "hide-kit/src", "hide-host/src"]

# Files whose prompts another file sends, with that file.
PROMPT_ONLY = {
    "herdr-core/src/runtime/memory_promotion.rs": "the first prompt of a Memory promotion's agent, whose reasons the operator reads",
}

# Text in a prompt source that is not a Hide AI prompt, by the number of
# findings it has and why, so a new one in the same file fails like one
# anywhere else.
NOT_PROMPTS = {}

CJK = re.compile(r"[\u1100-\u11ff\u3040-\u30ff\u3130-\u318f\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7a3]")
LANGUAGES = r"(?:English|Korean|Japanese|Chinese|Simplified Chinese)"
DIRECTIVES = [
    re.compile(rf"\b(?:in|into)\s+(?:both\s+)?{LANGUAGES}\b"),
    re.compile(r"\blanguage of (?:the|its|their|a|each)\b"),
    re.compile(r"\b(?:card's|source's|its|their|own) (?:own )?language\b"),
    re.compile(rf"\b{LANGUAGES}\s+and\s+{LANGUAGES}\b"),
]

LITERAL = re.compile(
    r"(?<![A-Za-z0-9_])b?r(#*)\"(.*?)\"\1"
    r"|b?\"((?:\\.|[^\"\\])*)\"",
    re.S,
)
COMMENT_OR_CHAR = re.compile(
    r"/\*.*?\*/"
    r"|//[^\n]*"
    r"|b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^'\\\n])'",
    re.S,
)
BUILDS_REQUEST = re.compile(r"\bhide_ai::AiRequest\s*\{|\bAiRequest\s*\{")
IMPORTS_REQUEST = re.compile(r"\buse\s+hide_ai::(?:\{[^}]*\bAiRequest\b[^}]*\}|AiRequest\b)|\bhide_ai::AiRequest\s*\{")


def blank(match: re.Match) -> str:
    return re.sub(r"[^\n]", " ", match.group(0))


def strip_comments(text: str) -> str:
    """Blanks comments and character literals, keeping string literals and
    line numbers. A comment opener inside a string is protected by blanking
    strings first and reading the comments from that copy."""
    masked = LITERAL.sub(blank, text)
    out = list(text)
    for match in COMMENT_OR_CHAR.finditer(masked):
        for index in range(match.start(), match.end()):
            if out[index] != "\n":
                out[index] = " "
    return "".join(out)


def test_ranges(text: str) -> list:
    """The spans of every item behind `#[cfg(test)]`, read with literals
    and comments blanked so a brace in a string cannot end one early."""
    code = COMMENT_OR_CHAR.sub(blank, LITERAL.sub(blank, text))
    ranges = []
    for match in re.finditer(r"#\[cfg\((all\()?test\b[^\]]*\]", code):
        position, depth = match.end(), 0
        while position < len(code):
            char = code[position]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    ranges.append((match.start(), position + 1))
                    break
            elif char == ";" and depth == 0:
                ranges.append((match.start(), position + 1))
                break
            position += 1
    return ranges


def test_only(relative: str) -> bool:
    name = relative.rsplit("/", 1)[-1]
    return name == "tests.rs" or name.endswith("_tests.rs") or "/tests/" in relative


def production(text: str) -> str:
    """The text with test items blanked."""
    out = list(text)
    for start, end in test_ranges(text):
        for index in range(start, end):
            if out[index] != "\n":
                out[index] = " "
    return "".join(out)


def builds_request(text: str) -> bool:
    code = production(COMMENT_OR_CHAR.sub(blank, LITERAL.sub(blank, text)))
    return bool(IMPORTS_REQUEST.search(code)) and bool(BUILDS_REQUEST.search(code))


def findings(path: pathlib.Path) -> list:
    relative = path.relative_to(ROOT).as_posix()
    text = production(strip_comments(path.read_text()))
    found = []
    for match in LITERAL.finditer(text):
        content = match.group(2) if match.group(2) is not None else match.group(3)
        line = text.count("\n", 0, match.start()) + 1
        if CJK.search(content):
            found.append(f"{relative}:{line}: Korean, Chinese or Japanese text in a prompt")
        for directive in DIRECTIVES:
            hit = directive.search(content)
            if hit:
                found.append(f"{relative}:{line}: names an output language: {hit.group(0)!r}")
    return found


def sources() -> list:
    found = set()
    for tree in TREES:
        for path in (ROOT / tree).rglob("*.rs"):
            relative = path.relative_to(ROOT).as_posix()
            if not test_only(relative) and builds_request(path.read_text()):
                found.add(relative)
    return sorted(found | set(PROMPT_ONLY))


def main() -> int:
    failures = []
    for listed, reason in sorted(PROMPT_ONLY.items()):
        if not (ROOT / listed).exists():
            failures.append(f"{listed}: listed as a prompt source ({reason}) but no longer exists; remove the entry")
    scanned = sources()
    if not scanned:
        failures.append("no Hide AI prompt source was found; the discovery pattern no longer matches")
    for listed, (_, reason) in sorted(NOT_PROMPTS.items()):
        if listed not in scanned:
            failures.append(f"{listed}: excused ({reason}) but is no prompt source; remove the entry")
    for relative in scanned:
        path = ROOT / relative
        if not path.exists():
            continue
        found = findings(path)
        if relative in NOT_PROMPTS:
            allowed, reason = NOT_PROMPTS[relative]
            if len(found) != allowed:
                failures.append(
                    f"{relative}: excused for {allowed} finding(s) ({reason}) but has {len(found)}; "
                    "take the language out of the new one, or change the count with its reason\n    "
                    + "\n    ".join(found)
                )
            continue
        failures.extend(found)
    if failures:
        print("Hide AI prompts must not name an output language; the request carries it:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"ok: {len(scanned)} Hide AI prompt sources name no output language")
    return 0


if __name__ == "__main__":
    sys.exit(main())
