"""Deployment failure boundary: real staged bytes and symlinks, fake OS services.

The external systemd/Caddy boundary is the only double. No cloud or root writes.
"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("relay_deploy", ROOT / "scripts/relay-deploy.py")
DEPLOY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DEPLOY)


class RelayDeploy(unittest.TestCase):
    @staticmethod
    def healthy(*args):
        return b"ok\n\n200" if args[0] == "curl" else None

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "vm"
        (self.root / "etc").mkdir(parents=True)
        (self.root / "etc/machine-id").write_text("a" * 32 + "\n")

    def release(self, source="1" * 40):
        artifact = self.base / ("artifact-" + source)
        artifact.mkdir()
        (artifact / "hide-relay").write_bytes(b"fixture binary " + source.encode())
        checksum = DEPLOY.digest(artifact / "hide-relay")
        (artifact / "source.json").write_text(json.dumps({
            "source": source, "sha256": checksum, "repository": "with-hide/hide",
            "workflow": ".github/workflows/relay-artifact.yml", "target": "x86_64-unknown-linux-gnu",
        }))
        values = dict(source=source, sha256=checksum, project="fixture-project", machine_id="a" * 32)
        output = self.base / ("stage-" + source)
        DEPLOY.stage(artifact, output, values)
        return output

    def installed(self):
        return self.root / "opt/hide-relay/current"

    def test_success_retry_and_failed_update_restore_binary_and_configuration(self):
        old = self.release()
        DEPLOY.install(old, self.root, self.healthy)
        DEPLOY.install(old, self.root, self.healthy)
        candidate = self.release("2" * 40)
        def os_boundary(*args):
            if args[0] == "curl" and self.installed().resolve().name == "2" * 40:
                raise RuntimeError("candidate health failed")
            return self.healthy(*args)
        with self.assertRaisesRegex(RuntimeError, "previous state restored"):
            DEPLOY.install(candidate, self.root, os_boundary)
        self.assertEqual((self.installed() / "hide-relay").read_bytes(), (old / "hide-relay").read_bytes())
        self.assertEqual((self.installed() / "fluent-bit.conf").read_bytes(), (old / "fluent-bit.conf").read_bytes())
        self.assertFalse((self.root / "opt/hide-relay/pending.json").exists())

    def test_failed_recovery_retains_intent_and_explicit_recovery_finishes_it(self):
        old = self.release()
        DEPLOY.install(old, self.root, self.healthy)
        candidate = self.release("2" * 40)
        def unavailable(*args):
            if args[0] == "curl":
                raise RuntimeError("health unavailable")
        with self.assertRaisesRegex(RuntimeError, "recovery incomplete"):
            DEPLOY.install(candidate, self.root, unavailable)
        with self.assertRaisesRegex(ValueError, "interrupted install"):
            DEPLOY.install(candidate, self.root, self.healthy)
        DEPLOY.recover(self.root, self.healthy)
        self.assertEqual(self.installed().resolve().name, "1" * 40)

    def test_interrupted_pending_write_leaves_running_release_retryable(self):
        old = self.release()
        DEPLOY.install(old, self.root, self.healthy)
        candidate = self.release("2" * 40)
        called = []
        def interrupted(record, stream):
            stream.write('{"previous":')
            stream.flush()
            raise KeyboardInterrupt("simulated process interruption")
        with patch.object(DEPLOY.json, "dump", interrupted):
            with self.assertRaises(KeyboardInterrupt):
                DEPLOY.install(candidate, self.root, lambda *args: called.append(args))
        self.assertFalse((self.root / "opt/hide-relay/pending.json").exists())
        self.assertEqual(self.installed().resolve().name, "1" * 40)
        self.assertFalse(any(args[:2] == ("systemctl", "stop") for args in called))
        DEPLOY.install(candidate, self.root, self.healthy)
        self.assertEqual(self.installed().resolve().name, "2" * 40)

    def test_https_redirect_or_wrong_body_rolls_back(self):
        old = self.release()
        DEPLOY.install(old, self.root, self.healthy)
        candidate = self.release("2" * 40)
        for response in (b"\n308", b"wrong\n200"):
            def boundary(*args):
                if args[-1] == "https://relay.withhide.dev/health" and self.installed().resolve().name == "2" * 40:
                    return response
                return self.healthy(*args)
            with self.assertRaisesRegex(RuntimeError, "previous state restored"):
                DEPLOY.install(candidate, self.root, boundary)
            self.assertEqual(self.installed().resolve().name, "1" * 40)

    def test_interruption_after_intent_publication_is_recoverable(self):
        old = self.release()
        DEPLOY.install(old, self.root, self.healthy)
        candidate = self.release("2" * 40)
        def interrupt(*args):
            if args[:2] == ("systemctl", "stop"):
                raise KeyboardInterrupt("simulated process interruption")
        with self.assertRaises(KeyboardInterrupt):
            DEPLOY.install(candidate, self.root, interrupt)
        pending = self.root / "opt/hide-relay/pending.json"
        self.assertEqual(json.loads(pending.read_text()), {"previous": "releases/" + "1" * 40})
        DEPLOY.recover(self.root, self.healthy)
        self.assertFalse(pending.exists())
        self.assertEqual(self.installed().resolve().name, "1" * 40)

    def test_first_install_failure_leaves_no_active_release(self):
        def unavailable(*args):
            if args[0] == "curl":
                raise RuntimeError("health unavailable")
        with self.assertRaisesRegex(RuntimeError, "previous state restored"):
            DEPLOY.install(self.release(), self.root, unavailable)
        self.assertFalse(self.installed().is_symlink())

    def test_bad_bytes_and_wrong_machine_do_not_stop_service(self):
        staged = self.release()
        called = []
        (staged / "hide-relay").write_bytes(b"corrupted")
        with self.assertRaisesRegex(ValueError, "changed"):
            DEPLOY.install(staged, self.root, lambda *args: called.append(args))
        self.assertEqual(called, [])
        staged = self.release("2" * 40)
        (self.root / "etc/machine-id").write_text("b" * 32)
        with self.assertRaisesRegex(ValueError, "machine_id"):
            DEPLOY.install(staged, self.root, lambda *args: called.append(args))
        self.assertEqual(called, [])

    def test_unmanaged_unit_is_preserved(self):
        unit = self.root / "etc/systemd/system/hide-relay.service"
        unit.parent.mkdir(parents=True)
        unit.write_text("another owner's service")
        with self.assertRaisesRegex(ValueError, "unmanaged"):
            DEPLOY.install(self.release(), self.root, lambda *args: self.fail("external mutation"))
        self.assertEqual(unit.read_text(), "another owner's service")

    def test_artifact_identity_refused_before_output(self):
        staged = self.release()
        values = json.loads((staged / "inputs.json").read_text())
        values["source"] = "2" * 40
        output = self.base / "refused"
        with self.assertRaisesRegex(ValueError, "provenance"):
            DEPLOY.stage(self.base / ("artifact-" + "1" * 40), output, values)
        self.assertFalse(output.exists())

    def test_cloud_scope_and_currency_refusal(self):
        values = dict(project="fixture-project", project_number="123456789", instance_id="1234",
                      channel="projects/fixture-project/notificationChannels/123", currency="USD")
        output = self.base / "cloud"
        with self.assertRaisesRegex(ValueError, "currency"):
            DEPLOY.cloud_config(values, output)
        self.assertFalse(output.exists())
        values["currency"] = "KRW"
        DEPLOY.cloud_config(values, output)
        budget = json.loads((output / "budget.json").read_text())
        self.assertEqual(budget["amount"]["specifiedAmount"], {"currencyCode": "KRW", "units": "100000"})
        self.assertEqual(budget["budgetFilter"]["projects"], ["projects/123456789"])
        # Billing Budgets v1 accepts notificationsRule, not allUpdatesRule.
        self.assertEqual(set(budget), {"displayName", "budgetFilter", "amount", "thresholdRules", "notificationsRule"})
        self.assertEqual(budget["notificationsRule"], {
            "monitoringNotificationChannels": [values["channel"]], "disableDefaultIamRecipients": False,
        })
        scope = (output / "relay-filter.txt").read_text().strip()
        self.assertIn('resource.labels.instance_id="1234"', scope)
        self.assertIn('log_id("hide-relay")', scope)
        alert = json.loads((output / "cap_exceeded.json").read_text())
        self.assertEqual(alert["conditions"][0]["conditionMatchedLog"]["filter"],
                         scope + ' AND jsonPayload.event="cap_exceeded"')

    def test_example_has_exact_input_contract(self):
        example = json.loads((ROOT / "deploy/relay-inputs.example.json").read_text())
        self.assertEqual(set(example), set(DEPLOY.INPUTS) | set(DEPLOY.CLOUD_INPUTS))


if __name__ == "__main__":
    unittest.main()
