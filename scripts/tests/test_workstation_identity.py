"""Exercise the public privacy boundary in real temporary Git repositories."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


CHECK = Path(__file__).resolve().parents[1] / "check-no-workstation-identity.py"


def home(platform, account):
    roots = {"macos": "/Users/", "linux": "/home/", "windows": "C:\\Users\\"}
    return roots[platform] + account


class WorkstationIdentityTests(unittest.TestCase):
    def setUp(self):
        runs = CHECK.parent.parent / "agents" / "runs" / "privacy-tests"
        runs.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=runs)
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "--quiet")

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.root), *args], check=True,
                              capture_output=True)

    def tracked(self, path, contents):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(contents.encode() if isinstance(contents, str) else contents)
        self.git("add", "-f", "--", path)

    def check(self, scope="checkout", checker=CHECK):
        return subprocess.run([sys.executable, str(checker), "--scope", scope],
                              cwd=self.root, capture_output=True, text=True, timeout=15)

    def checker_with_limit(self, name, value):
        """Exercise declared caps with small real fixtures, not huge allocations."""
        runs = CHECK.parent.parent / "agents" / "runs" / "privacy-tests"
        runs.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(dir=runs)
        self.addCleanup(temporary.cleanup)
        checker = Path(temporary.name) / "checker.py"
        lines = CHECK.read_text().splitlines(keepends=True)
        matches = [i for i, line in enumerate(lines) if line.startswith(name + " = ")]
        self.assertEqual(len(matches), 1)
        lines[matches[0]] = name + " = " + str(value) + "\n"
        checker.write_text("".join(lines))
        return checker

    def classes(self, result):
        return [json.loads(line)["class"] for line in result.stderr.splitlines()]

    def test_contact_address_is_rejected_without_disclosing_it(self):
        address = "private-fixture-person" + "@" + "mail.vendor.com"
        self.tracked("contact.txt", "contact: " + address)
        for scope in ("checkout", "index"):
            result = self.check(scope)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(self.classes(result), ["contact_email"])
            self.assertNotIn(address, result.stdout + result.stderr)

    def test_only_the_published_contact_address_is_allowed(self):
        published = "hello" + "@" + "withhide.dev"
        self.tracked("site.html", '<a href="mailto:' + published + '">' + published + "</a>")
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope).returncode, 0)
        # The domain is not allowed, nor is a lookalike of the published address.
        for other in ("someone" + "@" + "withhide.dev", "xhello" + "@" + "withhide.dev",
                      "hello" + "@" + "mail.withhide.dev", "hello" + "@" + "withhide.dev.example.org"):
            self.tracked("site.html", "contact: " + other)
            result = self.check()
            self.assertEqual(result.returncode, 1, other)
            self.assertEqual(self.classes(result), ["contact_email"])
            self.assertNotIn(other, result.stdout + result.stderr)

    def test_email_in_a_url_path_or_before_colon_is_still_private(self):
        address = "private-fixture-person" + "@" + "mail.vendor.com"
        for value in ("https://host.vendor.com/contact/" + address, address + ": contact"):
            self.tracked("contact.txt", value)
            self.assertEqual(self.classes(self.check()), ["contact_email"])

    def test_only_code_owned_systemd_namespace_identifiers_are_allowed(self):
        self.tracked("service.conf", "After=systemd-journald" + "@" + "hide-relay.service\n"
                     "/etc/systemd/journald" + "@" + "hide-relay.conf\n")
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope).returncode, 0)
        for value in ("person" + "@" + "hide-relay.service",
                      "systemd-journald" + "@" + "other.service",
                      "systemd-journald" + "@" + "hide-relay.service.example.org",
                      "journald" + "@" + "other.conf"):
            self.tracked("service.conf", value)
            self.assertEqual(self.classes(self.check()), ["contact_email"])

    def test_only_a_registry_deprecation_notice_in_the_lockfile_may_name_its_author(self):
        author = "package-author" + "@" + "mail.vendor.com"
        notice = "  glob@7.2.3:\n    deprecated: Old versions are unsupported, contact " + author + "\n"
        self.tracked("pnpm-lock.yaml", notice)
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope).returncode, 0)
        # Anywhere else in the lockfile, or the same line in another file, is still private.
        for path, contents in (("pnpm-lock.yaml", notice + "  author: " + author + "\n"),
                               ("web/pnpm-lock.yaml", notice), ("notes.yaml", notice)):
            self.git("rm", "-q", "--cached", "-r", "--ignore-unmatch", ".")
            self.tracked(path, contents)
            result = self.check()
            self.assertEqual(self.classes(result), ["contact_email"], path)
            self.assertNotIn(author, result.stdout + result.stderr)
            (self.root / path).unlink()

    def test_contact_address_in_a_filename_is_redacted(self):
        address = "private-fixture-person" + "@" + "mail.vendor.com"
        self.tracked(address + ".txt", address)
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(address, result.stdout + result.stderr)
        self.assertIn("[redacted-email]", result.stderr)

    def test_reserved_addresses_and_remote_authorities_are_not_contacts(self):
        values = ["a@example.com", "a@example.net", "a@example.org", "a@fixture.invalid",
                  "a@fixture.test", "a@fixture.example", "https://user@host.vendor.com/a",
                  "git@host.vendor.com:repo.git", "package@1.2.3", "org/repo@v2.1.0"]
        self.tracked("remotes.txt", "\n".join(values))
        self.assertEqual(self.check().returncode, 0)

    def test_utf16_bom_home_path_is_rejected_in_both_scopes(self):
        for encoding in ("utf-16-le", "utf-16-be"):
            bom = b"\xff\xfe" if encoding.endswith("le") else b"\xfe\xff"
            self.tracked("windows.txt", bom + home("windows", "private-fixture-person").encode(encoding))
            for scope in ("checkout", "index"):
                result = self.check(scope)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(self.classes(result), ["windows_home"])
                self.assertNotIn("private-fixture-person", result.stderr)

    def test_explicit_neutral_accounts_pass_both_scopes(self):
        accounts = ("example", "alice", "al", "me", "remote", "u")
        self.tracked("fixture.txt", "\n".join(home(p, a) + "/project"
                                              for p in ("macos", "linux", "windows")
                                              for a in accounts))
        for scope in ("checkout", "index"):
            with self.subTest(scope=scope):
                result = self.check(scope)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout), {
                    "scope": scope, "tracked_files": 1, "findings": 0, "status": "pass"})

    def test_mixed_neutral_and_private_same_line(self):
        account = "private-fixture-person"
        for platform, classification in (("macos", "macos_home"), ("linux", "linux_home"),
                                         ("windows", "windows_home")):
            with self.subTest(platform=platform):
                # The separator proves the neutral account's component ends.
                self.tracked("fixture.txt", home(platform, "example") + "/project "
                             + home(platform, account))
                result = self.check("index")
                self.assertEqual(result.returncode, 1)
                self.assertNotIn(account, result.stdout + result.stderr)
                self.assertEqual(json.loads(result.stderr), {
                    "path": "fixture.txt", "line": 1, "class": classification, "scope": "index"})

    def test_neutral_prefix_is_not_an_allow(self):
        self.tracked("fixture.txt", home("macos", "example-private"))
        self.assertEqual(self.classes(self.check()), ["macos_home"])

    def test_neutral_prefix_with_whitespace_or_punctuation_fails_both_scopes(self):
        for platform, classification in (("macos", "macos_home"), ("linux", "linux_home"),
                                         ("windows", "windows_home")):
            for suffix in (" private", ",private", ";private", ":private", "(private)",
                           "\tprivate"):
                for quote in ("", '"', "'", "`"):
                    with self.subTest(platform=platform, suffix=suffix, quote=quote):
                        account = "example" + suffix
                        value = quote + home(platform, account) + "/project" + quote
                        self.tracked("fixture.txt", value)
                        for scope in ("checkout", "index"):
                            result = self.check(scope)
                            self.assertEqual(result.returncode, 1)
                            self.assertEqual(self.classes(result), [classification])
                            self.assertNotIn(account, result.stdout + result.stderr)

    def test_terminal_neutral_components_need_unambiguous_quotes(self):
        for platform, classification in (("macos", "macos_home"), ("linux", "linux_home"),
                                         ("windows", "windows_home")):
            for prefix, suffix, expected in (("", "", 0), ('"', '"', 0),
                                             ("'", "'", 0), ("`", "`", 0),
                                             ('"', "", 1), ('"', "'", 1),
                                             ('"', '"private/project', 1),
                                             ('"', '\\"', 1)):
                with self.subTest(platform=platform, prefix=prefix, suffix=suffix):
                    self.tracked("fixture.txt", prefix + home(platform, "example") + suffix)
                    for scope in ("checkout", "index"):
                        result = self.check(scope)
                        self.assertEqual(result.returncode, expected)
                        self.assertEqual(self.classes(result), [classification] if expected else [])

    def test_dynamic_accounts_need_a_complete_component(self):
        for platform, classification in (("macos", "macos_home"), ("linux", "linux_home"),
                                         ("windows", "windows_home")):
            for account, expected in (("$USER", 0), ("${provider}", 0), (".${provider}", 0),
                                      ("$USER private", 1), ("${provider},private", 1),
                                      (".${provider};private", 1), ("$USER-private", 1)):
                with self.subTest(platform=platform, account=account):
                    self.tracked("fixture.txt", '"' + home(platform, account) + '/project"')
                    for scope in ("checkout", "index"):
                        result = self.check(scope)
                        self.assertEqual(result.returncode, expected)
                        self.assertEqual(self.classes(result), [classification] if expected else [])

    def test_audited_nested_fixture_quotes_keep_exact_components(self):
        values = ["// returns `" + home("linux", "remote") + "`.",
                  'path: "' + home("macos", "example") + '/project".to_string(),',
                  'r#"' + json.dumps({"command": "cd '" + home("macos", "example")
                                     + "/project'; pwd"}) + '"#,']
        for depth in (1, 3):
            quote = "\\" * depth + '"'
            values.append('"{' + quote + 'cwd' + quote + ':' + quote
                          + home("macos", "example") + quote + '}"')
        self.tracked("fixture.txt", "\n".join(values))
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope).returncode, 0)
        # The same encodings must reject suffixes inside the paired delimiters.
        self.tracked("fixture.txt", "\n".join(value.replace("example", "example private")
                                              .replace("remote", "remote,private")
                                              for value in values))
        for scope in ("checkout", "index"):
            result = self.check(scope)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(len(self.classes(result)), len(values))

    def test_file_url_home_is_checked(self):
        self.tracked("fixture.txt", "file://" + home("macos", "private-fixture-person"))
        self.assertEqual(self.classes(self.check()), ["macos_home"])

    def test_index_and_checkout_disagreement_in_both_directions(self):
        private = home("macos", "private-fixture-person")
        neutral = home("macos", "example")
        self.tracked("fixture.txt", private)
        (self.root / "fixture.txt").write_text(neutral)
        self.assertEqual(self.check().returncode, 0)
        self.assertEqual(self.check("index").returncode, 1)
        self.git("add", "--", "fixture.txt")
        (self.root / "fixture.txt").write_text(private)
        self.assertEqual(self.check().returncode, 1)
        self.assertEqual(self.check("index").returncode, 0)

    def test_untracked_files_are_outside_the_claim(self):
        self.tracked("tracked.txt", "neutral\n")
        (self.root / "untracked.txt").write_text(home("macos", "private-fixture-person"))
        result = self.check()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["tracked_files"], 1)

    def test_binary_and_attributes_cannot_hide_a_literal(self):
        self.tracked(".gitattributes", "fixture.dat -diff\n")
        self.tracked("fixture.dat", b"\x00" + home("macos", "private-fixture-person").encode())
        self.assertEqual(self.classes(self.check("index")), ["macos_home"])

    def test_named_workstation_is_generic_and_value_free(self):
        suffix = "MacBook-Pro"
        private = "private-fixture-" + suffix
        self.tracked("fixture.txt", private)
        result = self.check()
        self.assertEqual(self.classes(result), ["named_workstation"])
        self.assertNotIn(private, result.stderr + result.stdout)
        (self.root / "fixture.txt").write_text("example-" + suffix)
        self.assertEqual(self.check().returncode, 0)

    def test_workstation_dns_name_is_also_checked(self):
        private = "private-fixture-" + "mbp.tailnet.ts.net"
        self.tracked("fixture.txt", "https://" + private)
        result = self.check("index")
        self.assertEqual(self.classes(result), ["named_workstation"])
        self.assertNotIn(private, result.stderr + result.stdout)

    def test_force_added_evidence_and_profiles_fail_by_path(self):
        paths = {
            "docs/verification/run.txt": "tracked_run_artifact",
            "docs/screenshots/run.png": "tracked_run_artifact",
            "spikes/example/evidence/run.bin": "tracked_run_artifact",
            "profile/Cookies": "browser_profile_artifact",
            "profile/History": "browser_profile_artifact",
            "profile/Login Data": "browser_profile_artifact",
            "profile/Web Data": "browser_profile_artifact",
            "profile/Local State": "browser_profile_artifact",
            "profile/Cookies-journal": "browser_profile_artifact",
            "profile/Preferences": "browser_profile_artifact",
            "profile/metrics.pma": "browser_profile_artifact",
            "browser-profile/arbitrary.bin": "browser_profile_artifact",
            "profile/Session_1234": "browser_profile_artifact",
        }
        self.tracked(".gitignore", "docs/\nspikes/\nprofile/\nbrowser-profile/\n")
        for path in paths:
            self.tracked(path, "private-content-marker")
        for scope in ("checkout", "index"):
            result = self.check(scope)
            self.assertEqual(result.returncode, 1)
            self.assertNotIn("private-content-marker", result.stdout + result.stderr)
            rows = [json.loads(line) for line in result.stderr.splitlines()]
            self.assertEqual({row["path"]: row["class"] for row in rows}, paths)
            self.assertTrue(all(row["line"] == 0 and row["scope"] == scope for row in rows))

    def test_similarly_named_source_is_not_a_profile(self):
        self.tracked("session_search.rs", "// ordinary source\n")
        self.assertEqual(self.check().returncode, 0)

    def test_sqlite_profile_sidecar_families_fail_both_scopes(self):
        paths = ["Default/Network/" + database + "-" + sidecar
                 for database in ("Cookies", "History", "Login Data", "Web Data")
                 for sidecar in ("journal", "wal", "shm")]
        for path in paths:
            self.tracked(path, "private-content-marker")
        for scope in ("checkout", "index"):
            result = self.check(scope)
            self.assertEqual(result.returncode, 1)
            self.assertNotIn("private-content-marker", result.stdout + result.stderr)
            rows = [json.loads(line) for line in result.stderr.splitlines()]
            self.assertEqual({row["path"] for row in rows}, set(paths))
            self.assertTrue(all(row["class"] == "browser_profile_artifact" for row in rows))

    def test_sqlite_sidecar_source_suffixes_are_not_profiles(self):
        for path in ("Default/Network/Cookies-wal.rs", "Default/History-shm.md",
                     "Default/Login Data-wal.txt", "Default/Web Data-shm.test.ts",
                     "Default/Other-wal", "Default/History-walker"):
            self.tracked(path, "ordinary source\n")
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope).returncode, 0)

    def test_symlink_target_is_scanned_without_following_it(self):
        target = self.root / "link"
        target.symlink_to(home("macos", "private-fixture-person") + "/missing")
        self.git("add", "--", "link")
        for scope in ("checkout", "index"):
            self.assertEqual(self.classes(self.check(scope)), ["macos_home"])

    def test_missing_checkout_file_fails_instead_of_silent_skip(self):
        self.tracked("fixture.txt", "neutral\n")
        (self.root / "fixture.txt").unlink()
        self.assertEqual(self.classes(self.check()), ["checkout_read_failed"])
        self.assertEqual(self.check("index").returncode, 0)

    def test_dynamic_account_cannot_hide_literal_suffix(self):
        self.tracked("fixture.txt", home("linux", "${provider}") + "/session")
        self.assertEqual(self.check().returncode, 0)
        (self.root / "fixture.txt").write_text(home("linux", ".${provider}") + "/session")
        self.assertEqual(self.check().returncode, 0)
        (self.root / "fixture.txt").write_text(home("linux", "$USER-private"))
        self.assertEqual(self.classes(self.check()), ["linux_home"])

    def test_empty_repository_fails_explicitly(self):
        result = self.check()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.classes(result), ["no_tracked_files"])

    def test_manifest_entry_cap_boundary_fails_both_scopes(self):
        checker = self.checker_with_limit("MAX_TRACKED_FILES", 2)
        self.tracked("one.txt", "neutral")
        self.tracked("two.txt", "neutral")
        for scope in ("checkout", "index"):
            self.assertEqual(self.check(scope, checker).returncode, 0)
        self.tracked("three.txt", "neutral")
        for scope in ("checkout", "index"):
            result = self.check(scope, checker)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(self.classes(result), ["tracked_file_limit_exceeded"])
            self.assertFalse(result.stdout)

    def test_manifest_byte_cap_boundary_fails_both_scopes(self):
        self.tracked("fixture.txt", "neutral")
        size = len(self.git("ls-files", "--stage", "-z").stdout)
        for cap, expected in ((size, 0), (size - 1, 1)):
            checker = self.checker_with_limit("MAX_MANIFEST_BYTES", cap)
            for scope in ("checkout", "index"):
                result = self.check(scope, checker)
                self.assertEqual(result.returncode, expected)
                self.assertEqual(self.classes(result),
                                 ["git_manifest_byte_limit_exceeded"] if expected else [])
                if expected:
                    self.assertFalse(result.stdout)

    def test_git_read_deadline_reports_a_structured_failure(self):
        self.tracked("fixture.txt", "neutral")
        checker = self.checker_with_limit("GIT_READ_SECONDS", 0)
        for scope in ("checkout", "index"):
            result = self.check(scope, checker)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(self.classes(result), ["git_read_timeout"])
            self.assertFalse(result.stdout)

    def test_blob_limit_fails_with_value_free_diagnostic(self):
        self.tracked("fixture.txt", "neutral\n")
        checker = self.checker_with_limit("MAX_BYTES", 1024)
        with (self.root / "fixture.txt").open("r+b") as target:
            target.truncate(1024 + 1)
        result = self.check("checkout", checker)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.classes(result), ["tracked_blob_too_large"])
        self.assertEqual(self.check("index", checker).returncode, 0)

    def test_only_the_named_large_source_reads_past_the_blob_limit(self):
        self.tracked("design/hide-screens.pen", "neutral\n")
        self.tracked("other.pen", "neutral\n")
        checker = self.checker_with_limit("MAX_BYTES", 1024)
        for name in ("design/hide-screens.pen", "other.pen"):
            with (self.root / name).open("r+b") as target:
                target.truncate(1024 + 1)
            self.git("add", "-f", "--", name)
        for scope in ("checkout", "index"):
            result = self.check(scope, checker)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(self.classes(result), ["tracked_blob_too_large"])
            self.assertNotIn("design/hide-screens.pen", result.stderr)
            self.assertIn("other.pen", result.stderr)

    def test_diagnostic_limit_is_reported_instead_of_truncated_success(self):
        self.tracked("fixture.txt", (home("macos", "private-fixture-person") + "\n") * 201)
        result = self.check("index")
        self.assertEqual(result.returncode, 1)
        rows = [json.loads(line) for line in result.stderr.splitlines()]
        self.assertEqual(len(rows), 201)
        self.assertEqual(rows[-1]["class"], "diagnostic_limit_exceeded")
        self.assertNotIn("private-fixture-person", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
