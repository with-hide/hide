"""The #343 release contract rejects incomplete, mixed or published packages."""
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts/check-release-assets.mjs"


class ReleaseAssetsTest(unittest.TestCase):
    def setUp(self):
        runs = ROOT / "agents/runs/release-assets"
        runs.mkdir(parents=True, exist_ok=True)
        self.scratch = tempfile.TemporaryDirectory(dir=runs)
        self.addCleanup(self.scratch.cleanup)
        self.write_packages("v1.2.3")

    def write_packages(self, tag):
        """D-35's set for one tag: each installer with its checksum, the three
        update files naming each system's update files by sha512 and size, and
        the two blockmaps. The bytes are fixtures; nothing is installed."""
        self.directory = Path(self.scratch.name) / f"packages-{tag}"
        self.directory.mkdir()
        base = f"hide-{tag}"
        self.names = [
            f"{base}-macos-arm64.dmg",
            f"{base}-macos-arm64.zip",
            f"{base}-windows-x64-setup.exe",
            f"{base}-linux-x64.deb",
            f"{base}-linux-x64.rpm",
        ]
        for name in self.names:
            data = f"fixture for {name}".encode()
            (self.directory / name).write_bytes(data)
            (self.directory / f"{name}.sha256").write_text(f"{hashlib.sha256(data).hexdigest()}  {name}\n")
        for name in [f"{base}-macos-arm64.zip.blockmap", f"{base}-windows-x64-setup.exe.blockmap"]:
            (self.directory / name).write_bytes(b"blockmap fixture")
        updates = {
            "latest-mac.yml": [f"{base}-macos-arm64.zip", f"{base}-macos-arm64.dmg"],
            "latest.yml": [f"{base}-windows-x64-setup.exe"],
            "latest-linux.yml": [f"{base}-linux-x64.deb", f"{base}-linux-x64.rpm"],
        }
        platforms = {}
        for platform in ["linux-x86_64", "linux-aarch64"]:
            name = f"hide-device-{platform}-{tag[1:]}.tar.gz"
            data = f"device archive fixture {platform}".encode()
            (self.directory / name).write_bytes(data)
            platforms[platform] = {"asset": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                                   "files": {name: {"size": 64, "sha256": "a" * 64} for name in ["hided", "hide", "hide-agent-hooks", "herdr"]}}
        (self.directory / "device-bundles.json").write_text(json.dumps({
            "format": 1, "version": tag[1:], "commit": "1" * 40, "order": "42", "platforms": platforms}))
        for info, files in updates.items():
            self.write_update_info(info, tag[1:], files)

    def write_update_info(self, info, version, files):
        """An update file in the shape electron-builder writes."""
        lines = [f"version: {version}", "files:"]
        digests = []
        for name in files:
            data = (self.directory / name).read_bytes()
            digest = base64.b64encode(hashlib.sha512(data).digest()).decode()
            digests.append(digest)
            lines += [f"  - url: {name}", f"    sha512: {digest}", f"    size: {len(data)}"]
        lines += [f"path: {files[0]}", f"sha512: {digests[0]}", "releaseDate: '2026-10-16T00:00:00.000Z'"]
        (self.directory / info).write_text("\n".join(lines) + "\n")

    def check(self, tag="v1.2.3", release=None, pages=None):
        args = ["node", str(CHECK), tag, str(self.directory)]
        if release is not None or pages is not None:
            record = Path(self.scratch.name) / "release.json"
            record.write_text(json.dumps([[release]] if release is not None else pages))
            args.append(str(record))
        return subprocess.run(args, capture_output=True, text=True, timeout=10)

    def test_complete_same_version_packages_pass(self):
        self.assertEqual(self.check().returncode, 0)

    def test_installers_do_not_require_the_device_files_added_after_packaging(self):
        # The Mac installer builder runs before package.mjs copies device kits.
        result = subprocess.run([
            "node", "--input-type=module", "-e",
            f"import {{ systemAssetNames, expectedAssetNames }} from {json.dumps(CHECK.as_uri())};"
            "console.log(JSON.stringify({installers: systemAssetNames('1.2.3', 'macos'),"
            "release: expectedAssetNames('v1.2.3')}));",
        ], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        inventory = json.loads(result.stdout)
        self.assertEqual(inventory["installers"], sorted([
            "hide-v1.2.3-macos-arm64.dmg", "hide-v1.2.3-macos-arm64.dmg.sha256",
            "hide-v1.2.3-macos-arm64.zip", "hide-v1.2.3-macos-arm64.zip.sha256",
            "hide-v1.2.3-macos-arm64.zip.blockmap", "latest-mac.yml",
        ]))
        self.assertEqual(inventory["release"], sorted(file.name for file in self.directory.iterdir()))

    def test_device_bundle_manifest_and_archive_must_agree(self):
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("18 files", result.stdout)
        bundle = self.directory / "hide-device-linux-aarch64-1.2.3.tar.gz"
        original = bundle.read_bytes()
        bundle.write_bytes(original + b"changed")
        self.assertNotEqual(self.check().returncode, 0)
        bundle.write_bytes(original)
        manifest_file = self.directory / "device-bundles.json"
        manifest = json.loads(manifest_file.read_text())
        for change in [lambda m: m.update(version="1.2.2"),
                       lambda m: m["platforms"].pop("linux-x86_64"),
                       lambda m: m["platforms"]["linux-aarch64"]["files"].pop("herdr"),
                       lambda m: m["platforms"]["linux-aarch64"].update(sha256="0" * 64)]:
            value = json.loads(json.dumps(manifest))
            change(value)
            manifest_file.write_text(json.dumps(value))
            self.assertNotEqual(self.check().returncode, 0)
        manifest_file.write_text(json.dumps(manifest))
        bundle.unlink()
        self.assertNotEqual(self.check().returncode, 0)

    def test_checksums_cannot_hide_a_missing_target(self):
        name = self.names[2]
        other = name.replace("windows", "other")
        (self.directory / name).rename(self.directory / other)
        checksum = self.directory / f"{name}.sha256"
        content = checksum.read_text().replace(name, other)
        checksum.unlink()
        (self.directory / f"{other}.sha256").write_text(content)
        self.assertNotEqual(self.check().returncode, 0)

    def test_mixed_version_and_extra_assets_fail(self):
        (self.directory / "hide-v1.2.2-macos-arm64.zip").write_bytes(b"old")
        self.assertNotEqual(self.check().returncode, 0)

    def test_retired_archives_are_not_released(self):
        for name in ["hide-v1.2.3-windows-x64.zip", "hide-v1.2.3-linux-x64.tar.gz"]:
            with self.subTest(name=name):
                (self.directory / name).write_bytes(b"old archive")
                self.assertNotEqual(self.check().returncode, 0)
                (self.directory / name).unlink()

    def test_update_file_must_match_the_files_beside_it(self):
        self.assertEqual(self.check().returncode, 0)
        setup = self.directory / self.names[2]
        setup.write_bytes(b"another installer")
        (self.directory / f"{self.names[2]}.sha256").write_text(f"{hashlib.sha256(b'another installer').hexdigest()}  {self.names[2]}\n")
        result = self.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("latest.yml", result.stderr)

    def test_update_file_must_list_every_update_and_its_version(self):
        self.write_update_info("latest-linux.yml", "1.2.3", [self.names[3]])
        result = self.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not list", result.stderr)
        self.write_update_info("latest-linux.yml", "1.2.2", [self.names[3], self.names[4]])
        self.assertNotEqual(self.check().returncode, 0)
        self.write_update_info("latest-linux.yml", "1.2.3", [self.names[3], self.names[4]])
        self.assertEqual(self.check().returncode, 0)
        self.write_update_info("latest-mac.yml", "1.2.3", [self.names[1], self.names[2]])
        self.assertNotEqual(self.check().returncode, 0)

    def test_corruption_and_wrong_checksum_filename_fail(self):
        name = self.names[0]
        original = (self.directory / name).read_bytes()
        (self.directory / name).write_bytes(b"changed")
        self.assertNotEqual(self.check().returncode, 0)
        (self.directory / name).write_bytes(original)
        checksum = self.directory / f"{name}.sha256"
        checksum.write_text(checksum.read_text().replace(name, self.names[1]))
        self.assertNotEqual(self.check().returncode, 0)

    def test_symlinks_and_empty_packages_fail(self):
        name = self.names[0]
        archive = self.directory / name
        archive.unlink()
        archive.symlink_to(self.directory / self.names[1])
        self.assertNotEqual(self.check().returncode, 0)
        archive.unlink()
        archive.write_bytes(b"")
        self.assertNotEqual(self.check().returncode, 0)

    def test_only_stable_and_rc_tags_are_released(self):
        for tag in ["v1.2.3-beta.1", "v1.2.3+build", "v01.2.3", "1.2.3", "v1.2.3-rc", "v1.2.3-rc.0",
                    "v1.2.3-rc.01", "v1.2.3-RC.1", "v1.2.3-rc.1.2", "v1.2.3-rc.1+build"]:
            with self.subTest(tag=tag):
                self.assertNotEqual(self.check(tag).returncode, 0)

    def test_rc_packages_pass_and_only_a_prerelease_draft_resumes(self):
        self.write_packages("v0.4.0-rc.1")
        self.assertEqual(self.check("v0.4.0-rc.1").returncode, 0)
        self.assertNotEqual(self.check("v0.4.0").returncode, 0)
        draft = {"tag_name": "v0.4.0-rc.1", "draft": True, "prerelease": True,
                 "assets": [{"name": "hide-v0.4.0-rc.1-macos-arm64.dmg"}]}
        self.assertEqual(self.check("v0.4.0-rc.1", release=draft).returncode, 0)
        draft["prerelease"] = False
        self.assertNotEqual(self.check("v0.4.0-rc.1", release=draft).returncode, 0)

    def test_stable_tag_never_resumes_a_prerelease_draft(self):
        draft = {"tag_name": "v1.2.3", "draft": True, "prerelease": True, "assets": []}
        self.assertNotEqual(self.check(release=draft).returncode, 0)

    def test_partial_draft_retry_passes_and_public_release_fails(self):
        draft = {"tag_name": "v1.2.3", "draft": True, "prerelease": False,
                 "assets": [{"name": self.names[0]}]}
        self.assertEqual(self.check(release=draft).returncode, 0)
        draft["draft"] = False
        self.assertNotEqual(self.check(release=draft).returncode, 0)

    def test_mixed_or_duplicate_existing_draft_assets_fail(self):
        draft = {"tag_name": "v1.2.3", "draft": True, "prerelease": False,
                 "assets": [{"name": "hide-v1.2.2-windows-x64-setup.exe"}]}
        self.assertNotEqual(self.check(release=draft).returncode, 0)
        draft["assets"] = [{"name": self.names[0]}, {"name": self.names[0]}]
        self.assertNotEqual(self.check(release=draft).returncode, 0)

    def test_late_page_draft_and_ambiguous_tags_cannot_bypass_inventory(self):
        old = {"tag_name": "v0.0.1", "draft": False, "prerelease": False, "assets": []}
        stale = {"tag_name": "v1.2.3", "draft": True, "prerelease": False,
                 "assets": [{"name": "hide-v1.2.2-windows-x64-setup.exe"}]}
        self.assertNotEqual(self.check(pages=[[old], [old], [stale]]).returncode, 0)
        good = {**stale, "assets": []}
        self.assertNotEqual(self.check(pages=[[good], [good]]).returncode, 0)
        self.assertEqual(self.check(pages=[[old], [good]]).returncode, 0)
        self.assertEqual(self.check(pages=[[]]).returncode, 0)

    def test_malformed_inventory_fails_instead_of_establishing_absence(self):
        for pages in [{"message": "API failure"}, [None], [[None]], [[{}]]]:
            with self.subTest(pages=pages):
                self.assertNotEqual(self.check(pages=pages).returncode, 0)


if __name__ == "__main__":
    unittest.main()
