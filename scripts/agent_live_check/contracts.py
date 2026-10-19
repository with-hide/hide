"""Read current source declarations, rather than carrying a second bell list."""

import hashlib
import json
from pathlib import Path

from .protection import ProtectionError

SCENES = ("startup", "rest", "working", "shell_approval", "file_approval", "question",
          "plan_approval", "model_picker", "resume_picker", "mcp_approval")


def source_contract(checkout: Path) -> dict:
    source = checkout / "hide-agent-adapter/src/declarations.rs"
    rows = {}
    row = None
    # These are deliberately single-line declarations. A changed source form
    # requires updating this reader, never a default declaration of safety.
    for line in source.read_text().splitlines():
        if line == "    AgentAdapter {":
            if row is not None:
                raise ProtectionError("nested_adapter_declaration")
            row = {}
        elif row is not None and line.startswith('        id: '):
            row["id"] = json.loads(line.removeprefix("        id: ").removesuffix(","))
        elif row is not None and line.startswith("        bell: "):
            value = line.removeprefix("        bell: ").removesuffix(",")
            if value not in ("true", "false"):
                raise ProtectionError("unreadable_bell_declaration")
            row["bell"] = value == "true"
        elif line == "    }," and row is not None:
            if set(row) != {"id", "bell"} or row["id"] in rows:
                raise ProtectionError("incomplete_or_duplicate_adapter_declaration")
            rows[row["id"]] = row["bell"]
            row = None
    if not rows or row is not None:
        raise ProtectionError("adapter_declarations_unreadable")
    bell = json.loads(declaration(checkout / "herdr-core/src/delivery/bell.rs",
                                  "    const SAMPLE: &str = "))
    if not isinstance(bell, str) or not bell.startswith("🦉 ") or "\n" in bell:
        raise ProtectionError("invalid_bell_sample")
    quiet = declaration(checkout / "herdr-core/src/delivery/doorbell.rs", "const QUIET_MS: u64 = ")
    if not quiet.replace("_", "").isdigit():
        raise ProtectionError("doorbell_quiet_declaration_unreadable")
    return {"bell": bell, "quiet_seconds": int(quiet.replace("_", "")) / 1000, "targets": rows,
            "declaration_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}


def declaration(file: Path, prefix: str) -> str:
    """The value of the one single-line declaration in `file` that starts with `prefix`."""
    values = [line[len(prefix):-1] for line in file.read_text().splitlines()
              if line.startswith(prefix) and line.endswith(";")]
    if len(values) != 1:
        raise ProtectionError("declaration_unreadable:" + prefix.strip())
    return values[0]


def recipes(directory: Path, declared: dict) -> dict:
    result = {}
    for file in sorted(directory.glob("*.json")):
        value = json.loads(file.read_text())
        if (value["id"] in result or set(value["scenes"]) != set(SCENES)
                or not value["model"] or not value["argv"]):
            raise ProtectionError("incomplete_or_duplicate_scene_recipe")
        for name, scene in value["scenes"].items():
            expected = {"send", "arrived", "draft", "no_match", "unsafe"}
            if name == "question":
                expected.add("controls")
            if set(scene) != expected:
                raise ProtectionError("invalid_scene_recipe")
        shared = value.get("shared", {})
        if (not isinstance(shared, dict)
                or any(path not in value["known"] or format not in {"json", "toml"}
                       for path, format in shared.items())):
            raise ProtectionError("invalid_shared_configuration_declaration")
        preparation = value.get("startup_preparation")
        if preparation is not None and (
                not isinstance(preparation, dict) or set(preparation) != {"prompt", "default", "selected", "provenance"}
                or any(not isinstance(item, str) or not item for item in preparation.values())):
            raise ProtectionError("invalid_startup_preparation_declaration")
        result[value["id"]] = value
    if set(result) != set(declared):
        raise ProtectionError("recipe_adapter_coverage_mismatch")
    return result
