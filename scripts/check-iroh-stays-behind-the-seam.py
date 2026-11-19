#!/usr/bin/env python3
"""iroh and noq are linked by `hide-peer-iroh` and nothing else (PRD
device-connect-iroh D-63, D-64). Everything above sees `hide_peer`'s
`PeerTransport`, so the transport can be replaced, and its roughly 170
crates stay out of every build that does not open an endpoint.

This check reads the manifests and `Cargo.lock` and fails when:

- a member other than `hide-peer-iroh` names an iroh or noq crate;
- `hide-core`, `hide-node`, `hide-host`, `hide-holder` or `hide-agent-hooks`
  reaches `hide-peer-iroh` through any chain of members, in any dependency
  section;
- `workspace-hack` lists a crate only `hide-peer-iroh` reaches. The shared
  feature set does take what the transport turns on in crates the rest of
  the workspace already builds (docs/BUILD.md, "One feature set per
  dependency"); a crate nothing else reaches would become every member's
  build, and goes under `[final-excludes] third-party` in
  `.config/hakari.toml` instead;
- `hide-peer-iroh` asks for iroh or noq by anything but an exact version, or
  with default features.
"""
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent

SEAM = "hide-peer-iroh"
# The crates of the transport: iroh's own and its QUIC stack's.
TRANSPORT = re.compile(r"^(iroh|noq)(-|$)")
# Members whose builds must never contain the transport.
NEVER = ("hide-core", "hide-node", "hide-host", "hide-holder", "hide-agent-hooks")
# The transport crates the seam pins itself, each exactly.
PINNED = ("iroh", "noq")


def manifest(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


def dependency_tables(data):
    """Every dependency table of a manifest: the three sections, at the top
    and under each `[target.*]`."""
    scopes = [data] + list(data.get("target", {}).values())
    for scope in scopes:
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            yield scope.get(section, {})


def dependencies(data):
    """Each dependency as (the crate's name, how it is asked for)."""
    for table in dependency_tables(data):
        for name, spec in table.items():
            spec = spec if isinstance(spec, dict) else {"version": spec}
            yield spec.get("package", name), spec


def members(root):
    """Each member's name and manifest, from the workspace's own list."""
    found = {}
    for entry in manifest(root / "Cargo.toml")["workspace"]["members"]:
        data = manifest(root / entry / "Cargo.toml")
        found[data["package"]["name"]] = data
    return found


def reaches(crates, start, target, seen=None):
    """The chain of members from `start` to `target`, or None."""
    seen = seen if seen is not None else set()
    if start in seen or start not in crates:
        return None
    seen.add(start)
    for name, _ in dependencies(crates[start]):
        if name == target:
            return [start, target]
        chain = reaches(crates, name, target, seen)
        if chain:
            return [start] + chain
    return None


def only_the_seam_reaches(root, package):
    """The crates `package` depends on in `Cargo.lock` that no member but the
    seam reaches, as `name version`. A lock entry names a dependency by its
    name, and by its version too when the lock holds more than one."""
    packages = manifest(root / "Cargo.lock").get("package", [])
    by_name = {}
    for entry in packages:
        by_name.setdefault(entry["name"], []).append(entry)

    def resolve(dependency):
        name, _, rest = dependency.partition(" ")
        version = rest.split(" ")[0]
        return next(entry for entry in by_name[name] if not version or entry["version"] == version)

    def key(entry):
        return f"{entry['name']} {entry['version']}"

    # Neither the seam nor the shared feature crate, which every member
    # depends on, is a way a member reaches a crate.
    outside = (SEAM, package)
    reached = set()
    pending = [entry for entry in packages if "source" not in entry and entry["name"] not in outside]
    while pending:
        entry = pending.pop()
        if key(entry) in reached or entry["name"] in outside:
            continue
        reached.add(key(entry))
        pending.extend(resolve(dependency) for dependency in entry.get("dependencies", []))
    listed = by_name.get(package, [{}])[0].get("dependencies", [])
    return {key(resolve(dependency)) for dependency in listed} - reached


def findings(root):
    crates = members(root)
    found = []
    for name, data in sorted(crates.items()):
        if name in (SEAM, "workspace-hack"):
            continue
        for dependency, _ in dependencies(data):
            if TRANSPORT.match(dependency):
                found.append(
                    f"{name} depends on {dependency}: only {SEAM} links the transport; "
                    f"use hide_peer's PeerTransport"
                )
    for name in NEVER:
        chain = reaches(crates, name, SEAM)
        if chain:
            found.append(
                f"{name} reaches {SEAM} ({' -> '.join(chain)}): its build must not "
                f"contain the transport"
            )
    for dependency in sorted(only_the_seam_reaches(root, "workspace-hack")):
        found.append(
            f"workspace-hack lists {dependency}, which only {SEAM} reaches: every member "
            f"would build it; name it under `[final-excludes] third-party` in "
            f".config/hakari.toml and run `cargo hakari generate`"
        )
    if SEAM in crates:
        asked = dict(dependencies(crates[SEAM]))
        for dependency in PINNED:
            spec = asked.get(dependency)
            if spec is None:
                found.append(f"{SEAM} does not pin {dependency} itself")
                continue
            if not re.fullmatch(r"=\d+\.\d+\.\d+", str(spec.get("version", ""))):
                found.append(
                    f"{SEAM} asks for {dependency} {spec.get('version')!r}: pin it "
                    f"exactly (`=x.y.z`)"
                )
            if spec.get("default-features", True):
                found.append(f"{SEAM} takes {dependency} with its default features")
    else:
        found.append(f"the workspace has no {SEAM} member")
    return found


def main():
    found = findings(ROOT)
    for line in found:
        print(line, file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
