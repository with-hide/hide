"""Deployment failure boundary: real staged bytes and symlinks, fake OS services.

The external systemd/Caddy/nftables boundary is the only double. No cloud or root writes.
"""
import datetime
import importlib.util
import io
import json
from pathlib import Path
import re
import sys
import tempfile
import tomllib
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    # Every file under deploy/relay is staged: leave no bytecode cache beside the meter.
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


DEPLOY = load("relay_deploy", "scripts/relay-deploy.py")
METER = load("iroh_relay_meter", "deploy/relay/iroh-relay-meter.py")
BUNDLE = ROOT / "deploy/relay"
GB = 10**9


def os_reply(*args):
    """A healthy VM: every unit loaded and inactive, both relays answering."""
    if args[:2] == ("systemctl", "show"):
        return b"loaded\n" if "--property=LoadState" in args else b"inactive\n"
    if args[0] == "curl":
        return b"200" if args[-1].startswith("https://device-relay.withhide.dev/") else b"ok\n\n200"
    return None


class RelayDeploy(unittest.TestCase):
    healthy = staticmethod(os_reply)

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
        (artifact / "iroh-relay").write_bytes(b"fixture device relay " + source.encode())
        checksum = DEPLOY.digest(artifact / "hide-relay")
        device = DEPLOY.digest(artifact / "iroh-relay")
        (artifact / "source.json").write_text(json.dumps({
            "source": source, "sha256": checksum, "repository": "with-hide/hide",
            "workflow": ".github/workflows/relay-artifact.yml", "target": "x86_64-unknown-linux-gnu",
            "iroh_relay": {"version": (BUNDLE / "iroh-relay.version").read_text().strip(), "sha256": device},
        }))
        values = dict(source=source, sha256=checksum, iroh_relay_sha256=device,
                      project="fixture-project", machine_id="a" * 32)
        output = self.base / ("stage-" + source)
        DEPLOY.stage(artifact, output, values)
        return output

    def installed(self):
        return self.root / "opt/hide-relay/current"

    def unloaded_systemd(self, reject_collector=False):
        # A newly created unit symlink is not loaded until daemon-reload.
        # Real Ubuntu systemctl refuses stop/disable/reset-failed for it.
        loaded = False

        def boundary(*args):
            nonlocal loaded
            if args[:2] == ("systemctl", "daemon-reload"):
                loaded = self.installed().exists()
            elif args[:2] == ("systemctl", "show") and "--property=LoadState" in args:
                return b"loaded\n" if loaded else b"not-found\n"
            elif args[:2] == ("systemctl", "reset-failed"):
                raise RuntimeError("inactive unit was released from memory")
            elif args[0] == "systemctl" and args[1] in ("stop", "disable", "reset-failed", "start"):
                if not loaded:
                    raise RuntimeError("unit not loaded (exit 5)")
            elif args[0] == "/opt/fluent-bit/bin/fluent-bit" and reject_collector:
                raise RuntimeError("collector configuration refused")
            return self.healthy(*args)

        return boundary

    def test_install_recovers_services_which_reached_the_start_limit(self):
        failed = set(DEPLOY.SERVICES)

        def boundary(*args):
            if args[:2] == ("systemctl", "show") and "--property=ActiveState" in args:
                return b"failed\n" if args[-1] in failed else b"inactive\n"
            if args[:2] == ("systemctl", "reset-failed"):
                failed.difference_update(args[2:])
            if args[:2] == ("systemctl", "start") and failed.intersection(args[2:]):
                raise RuntimeError("service start limit reached")
            return self.healthy(*args)

        DEPLOY.install(self.release(), self.root, boundary)
        self.assertTrue(self.installed().exists())
        self.assertFalse(failed)

    def test_first_install_loads_new_units_before_using_them(self):
        release = self.release()
        DEPLOY.install(release, self.root, self.unloaded_systemd())
        self.assertEqual((self.installed() / "hide-relay").read_bytes(), (release / "hide-relay").read_bytes())
        self.assertFalse((self.root / "opt/hide-relay/pending.json").exists())

    def test_first_install_configuration_failure_removes_active_release(self):
        with self.assertRaisesRegex(RuntimeError, "collector configuration refused.*previous state restored"):
            DEPLOY.install(self.release(), self.root, self.unloaded_systemd(reject_collector=True))
        self.assertFalse(self.installed().is_symlink())
        self.assertFalse((self.root / "opt/hide-relay/pending.json").exists())

    def test_first_install_interrupted_before_switch_recovers_without_loaded_units(self):
        base = self.root / "opt/hide-relay"
        base.mkdir(parents=True)
        pending = base / "pending.json"
        pending.write_text('{"previous": null}')
        DEPLOY.recover(self.root, self.unloaded_systemd())
        self.assertFalse(self.installed().is_symlink())
        self.assertFalse(pending.exists())

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
            return self.healthy(*args)
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
                      channel="projects/fixture-project/notificationChannels/123", currency="USD",
                      network="default", target_tag="hide-relay")
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


    def test_device_relay_runs_beside_the_phone_relay_and_its_limit_is_respected(self):
        DEPLOY.install(self.release(), self.root, self.healthy)
        units = self.root / "etc/systemd/system"
        for unit in DEPLOY.DEVICE_UNITS:
            self.assertEqual((units / unit).readlink(), Path("/opt/hide-relay/current") / unit)
        calls = []
        stopped = self.root / DEPLOY.METER_STATE / "stopped"
        stopped.parent.mkdir(parents=True)
        stopped.touch()
        def boundary(*args):
            calls.append(args)
            return self.healthy(*args)
        DEPLOY.install(self.release("2" * 40), self.root, boundary)
        self.assertIn(("systemctl", "start", "hide-iroh-relay-limits.service"), calls)
        self.assertIn(("systemctl", "start", "hide-iroh-relay-meter.service"), calls)
        enabled = {unit for args in calls if args[:2] == ("systemctl", "enable") for unit in args[2:]}
        # The meter, not boot, decides whether the relay itself runs.
        self.assertIn("hide-iroh-relay-meter.timer", enabled)
        self.assertNotIn("hide-iroh-relay.service", enabled)
        # A relay stopped by its monthly limit is not probed, so the release still installs.
        self.assertFalse(any("https://device-relay.withhide.dev/ping" in args for args in calls))
        self.assertEqual(self.installed().resolve().name, "2" * 40)

    def test_device_relay_probe_failure_rolls_back(self):
        DEPLOY.install(self.release(), self.root, self.healthy)
        def boundary(*args):
            if args[-1] == "https://device-relay.withhide.dev/ping" and self.installed().resolve().name == "2" * 40:
                return b"502"
            return self.healthy(*args)
        with self.assertRaisesRegex(RuntimeError, "device relay.*previous state restored"):
            DEPLOY.install(self.release("2" * 40), self.root, boundary)
        self.assertEqual(self.installed().resolve().name, "1" * 40)

    def test_update_from_a_release_without_the_device_relay_stops_only_known_units(self):
        DEPLOY.install(self.release(), self.root, self.healthy)
        known = set(DEPLOY.SERVICES) | {"hide-relay-retention.timer"}
        def older_systemd(*args):
            if args[:2] == ("systemctl", "daemon-reload"):
                known.update(DEPLOY.UNITS)
            if args[:2] == ("systemctl", "show") and "--property=LoadState" in args:
                return b"loaded\n" if args[-1] in known else b"not-found\n"
            if args[:2] == ("systemctl", "stop") and not known.issuperset(args[2:]):
                raise RuntimeError("Unit not loaded (exit 5)")
            return self.healthy(*args)
        DEPLOY.install(self.release("2" * 40), self.root, older_systemd)
        self.assertEqual(self.installed().resolve().name, "2" * 40)

    def production_release_from_before_the_device_relay(self, source="0" * 40):
        """What main's script staged and installed: no device files, no iroh-relay pin."""
        release = self.root / "opt/hide-relay/releases" / source
        release.mkdir(parents=True)
        device = {"iroh-relay.toml", "iroh-relay.nft", "iroh-relay.version", "iroh-relay-meter.py"}
        for template in BUNDLE.iterdir():
            if template.name not in device and not template.name.startswith("hide-iroh-relay"):
                (release / template.name).write_bytes(template.read_bytes())
        (release / "hide-relay").write_bytes(b"phone relay from main")
        checksum = DEPLOY.digest(release / "hide-relay")
        (release / "source.json").write_text(json.dumps({
            "source": source, "sha256": checksum, "repository": "with-hide/hide",
            "workflow": ".github/workflows/relay-artifact.yml", "target": "x86_64-unknown-linux-gnu"}))
        (release / "inputs.json").write_text(json.dumps({
            "source": source, "sha256": checksum, "project": "fixture-project", "machine_id": "a" * 32}))
        (release / "inventory.json").write_text(json.dumps({p.name: DEPLOY.digest(p) for p in release.iterdir()}))
        current = self.root / "opt/hide-relay/current"
        current.symlink_to("releases/" + source)
        units = self.root / "etc/systemd/system"
        units.mkdir(parents=True)
        for unit in DEPLOY.SERVICES + DEPLOY.RETENTION:
            (units / unit).symlink_to("/opt/hide-relay/current/" + unit)
        (self.root / "etc/systemd/journald@hide-relay.conf").symlink_to("/opt/hide-relay/current/journald.conf")

    def test_first_update_from_production_rolls_back_to_the_phone_relay_when_the_device_relay_fails(self):
        self.production_release_from_before_the_device_relay()
        calls = []
        def boundary(*args):
            calls.append(args)
            if args[-1] == "https://device-relay.withhide.dev/ping":
                return b"000"
            return self.healthy(*args)
        with self.assertRaisesRegex(RuntimeError, "device relay probe.*previous state restored$"):
            DEPLOY.install(self.release(), self.root, boundary)
        self.assertEqual(self.installed().resolve().name, "0" * 40)
        self.assertFalse((self.root / "opt/hide-relay/pending.json").exists())
        restored = calls[len(calls) - calls[::-1].index(("systemctl", "daemon-reload")):]
        self.assertIn(("systemctl", "start", *DEPLOY.SERVICES), restored)
        self.assertFalse([args for args in restored if args[1:2] == ("start",) and "iroh" in " ".join(args)])

    def test_recovery_keeps_the_phone_relay_when_only_the_device_relay_fails(self):
        DEPLOY.install(self.release(), self.root, self.healthy)
        stopped = []
        def boundary(*args):
            if args[0] == "curl" and self.installed().resolve().name == "2" * 40 and "relay.withhide.dev" in args[-1]:
                raise RuntimeError("candidate phone relay unhealthy")
            if args[-1] == "https://device-relay.withhide.dev/ping":
                return b"502"
            if args[:2] == ("systemctl", "stop"):
                stopped.append(args[2:])
            return self.healthy(*args)
        with self.assertRaisesRegex(RuntimeError, "previous state restored, but device relay stopped: HTTPS device"):
            DEPLOY.install(self.release("2" * 40), self.root, boundary)
        self.assertEqual(self.installed().resolve().name, "1" * 40)
        self.assertIn(DEPLOY.DEVICE_RELAY, stopped[-1])
        self.assertNotIn("hide-relay.service", stopped[-1])

    def test_device_relay_binary_identity_refused_before_output(self):
        artifact = self.base / ("artifact-" + "1" * 40)
        staged = self.release()
        values = json.loads((staged / "inputs.json").read_text())
        (artifact / "iroh-relay").write_bytes(b"another build")
        with self.assertRaisesRegex(ValueError, "provenance"):
            DEPLOY.stage(artifact, self.base / "refused", values)
        self.assertFalse((self.base / "refused").exists())

    def test_cloud_config_watches_the_device_relay(self):
        values = dict(project="fixture-project", project_number="123456789", instance_id="1234",
                      channel="projects/fixture-project/notificationChannels/123", currency="KRW",
                      network="default", target_tag="hide-relay")
        output = self.base / "cloud"
        DEPLOY.cloud_config(values, output)
        scope = (output / "relay-filter.txt").read_text().strip()
        for event in ("device_relay_budget_warning", "device_relay_stopped", "device_relay_meter_failed"):
            alert = json.loads((output / (event + ".json")).read_text())
            self.assertEqual(alert["conditions"][0]["conditionMatchedLog"]["filter"],
                             scope + f' AND jsonPayload.event="{event}"')
        firewall = json.loads((output / "device-relay-firewall.json").read_text())
        self.assertEqual(firewall["allowed"], [{"IPProtocol": "udp", "ports": ["7842"]}])
        self.assertEqual((firewall["network"], firewall["targetTags"]), ("global/networks/default", ["hide-relay"]))
        uptime = json.loads((output / "device-relay-uptime.json").read_text())
        self.assertEqual((uptime["monitoredResource"]["labels"]["host"], uptime["httpCheck"]["path"]),
                         ("device-relay.withhide.dev", "/ping"))


class DeviceRelayBundle(unittest.TestCase):
    """The pieces that must agree: Caddy, the relay, its limits and its meter."""

    def setUp(self):
        self.config = tomllib.loads((BUNDLE / "iroh-relay.toml").read_text())
        self.unit = (BUNDLE / "hide-iroh-relay.service").read_text()
        self.limits = (BUNDLE / "iroh-relay.nft").read_text()

    def credential(self, name):
        return Path(re.search(rf"^LoadCredential={name}:(\S+)$", self.unit, re.M)[1])

    def test_caddy_terminates_tls_and_reencrypts_to_the_relay_on_its_own_host_name(self):
        caddy = (BUNDLE / "Caddyfile").read_text()
        site = re.search(r"^device-relay\.withhide\.dev \{\n(.*?)^\}", caddy, re.M | re.S)[1]
        https = self.config["tls"]["https_bind_addr"]
        self.assertTrue(https.startswith("127.0.0.1:"))
        self.assertIn(f"reverse_proxy https://{https} {{", site)
        self.assertIn("tls_server_name device-relay.withhide.dev", site)
        self.assertNotIn("dangerous_http_only", (BUNDLE / "iroh-relay.toml").read_text())
        # The phone relay's site still forwards only its own paths to its own process.
        phone = re.search(r"^relay\.withhide\.dev \{\n(.*?)^\}", caddy, re.M | re.S)[1]
        self.assertNotIn("3443", phone)

    def test_relay_answers_address_discovery_itself_with_caddys_certificate(self):
        self.assertTrue(self.config["enable_quic_addr_discovery"])
        self.assertEqual(self.config["tls"]["quic_bind_addr"], "0.0.0.0:7842")
        self.assertEqual(self.config["tls"]["cert_mode"], "Manual")
        loaded = Path("/run/credentials/hide-iroh-relay.service")
        self.assertEqual(Path(self.config["tls"]["manual_cert_path"]), loaded / "tls.crt")
        self.assertEqual(Path(self.config["tls"]["manual_key_path"]), loaded / "tls.key")
        self.assertEqual(self.credential("tls.crt"), METER.CERTIFICATE)
        self.assertEqual(self.credential("tls.key"), METER.CERTIFICATE.with_suffix(".key"))
        # Caddy's storage path for the host name under the pinned ACME issuer.
        self.assertEqual(METER.CERTIFICATE.parts[-3:-1],
                         ("acme-v02.api.letsencrypt.org-directory", "device-relay.withhide.dev"))

    def test_per_connection_caps_and_process_bounds(self):
        self.assertEqual(self.config["limits"]["client"]["rx"]["bytes_per_second"], 2 * 1024 * 1024)
        port = self.config["tls"]["https_bind_addr"].rsplit(":", 1)[1]
        self.assertIn(f'iif "lo" tcp dport {port} ct state new limit rate over 10/second burst 30 packets reject',
                      self.limits)
        device = int(re.search(rf'iif "lo" tcp dport {port} ct state new ct count over (\d+) reject', self.limits)[1])
        # Every device connection holds two descriptors in the Caddy the phone relay shares.
        phone = int(re.search(r"MAX_CONNECTIONS: usize = (\d+);", (ROOT / "hide-relay/src/limits.rs").read_text())[1])
        proxy = int(re.search(r"(?m)^LimitNOFILE=(\d+)$", (BUNDLE / "hide-relay-proxy.service").read_text())[1])
        self.assertLessEqual(2 * device + 2 * phone + 1024, proxy)
        self.assertIn(f'oif "lo" tcp sport {port} counter name "sent"', self.limits)
        self.assertIn('udp sport 7842 counter name "sent"', self.limits)
        self.assertRegex(self.unit, r"(?m)^MemoryMax=\d+M$")
        self.assertIn("Requires=hide-iroh-relay-limits.service", self.unit)
        self.assertNotIn("[Install]", self.unit)
        # Nothing the relay prints, which can carry endpoint ids, is kept.
        self.assertIn("StandardOutput=null", self.unit)
        self.assertIn("StandardError=null", self.unit)

    def test_no_cap_binds_the_phone_relays_connections_on_the_shared_port(self):
        # The phone relay limits each address by rate, not by open connections.
        self.assertNotRegex(self.limits, r"dport 443\b")

    def test_the_meters_state_folder_is_named_the_same_everywhere(self):
        folder = Path("/") / DEPLOY.METER_STATE
        self.assertEqual(METER.STATE, folder)
        self.assertIn(f"\n\t\troot {folder}\n", (BUNDLE / "Caddyfile").read_text())
        self.assertIn(f"\nStateDirectory={folder.name}\n", (BUNDLE / "hide-iroh-relay-meter.service").read_text())

    def test_only_categorical_alerts_leave_the_machine(self):
        collector = (BUNDLE / "fluent-bit.conf").read_text()
        allowed = set(re.search(r"Regex event \^\((.*)\)\$", collector)[1].split("|"))
        self.assertEqual(allowed, {"cap_exceeded", "server_failed", "collector_canary", "device_relay_budget_warning",
                                   "device_relay_stopped", "device_relay_meter_failed"})
        self.assertIn("Systemd_Filter _SYSTEMD_UNIT=hide-iroh-relay-meter.service", collector)

    def test_one_iroh_relay_version_for_the_server_and_any_client_in_the_workspace(self):
        pinned = (BUNDLE / "iroh-relay.version").read_text().strip()
        self.assertRegex(pinned, r"^\d+\.\d+\.\d+$")
        locked = re.findall(r'name = "iroh-relay"\nversion = "([^"]+)"', (ROOT / "Cargo.lock").read_text())
        self.assertLessEqual(set(locked), {pinned}, "bump deploy/relay/iroh-relay.version with the workspace's iroh")


class Systemd:
    """The relay unit as systemd would hold it, plus the counters nftables keeps."""

    def __init__(self, folder):
        self.folder = folder
        self.active = b"inactive"
        self.table = {"table": 7, "sent": 0, "connections": 0}
        self.actions = []
        self.certificate = folder / "caddy.crt"
        self.certificate.write_text("first")

    def __call__(self, *args):
        if args[:2] == ("/usr/sbin/nft", "-j"):
            if args[2:4] == ("list", "tables"):
                tables = [{"table": {"family": "inet", "name": "hide_iroh_relay", "handle": self.table["table"]}}
                          ] if self.table else []
                return json.dumps({"nftables": [{"metainfo": {}}] + tables}).encode()
            assert args[2:5] == ("list", "counter", "inet") and args[5] == "hide_iroh_relay", args
            values = {"connections": {"packets": self.table["connections"], "bytes": 60},
                      "sent": {"packets": 3, "bytes": self.table["sent"]}}[args[6]]
            return json.dumps({"nftables": [{"metainfo": {}}, {"counter": {"name": args[6], **values}}]}).encode()
        if args[:2] == ("systemctl", "show"):
            return self.active + b"\n"
        self.actions.append(args[1])
        if args[1] in ("start", "restart"):
            self.active = b"active"
        elif args[1] == "stop":
            self.active = b"inactive"
        return b""


class DeviceRelayMeter(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.state = Path(temp.name)
        self.os = Systemd(self.state)
        self.now = datetime.datetime(2026, 11, 19, 10, 0, tzinfo=datetime.timezone.utc)
        self.printed = []

    def tick(self, sent=0, connections=0, minutes=1, action="tick"):
        self.now += datetime.timedelta(minutes=minutes)
        if self.os.table is not None:
            self.os.table["sent"] += sent
            self.os.table["connections"] += connections
        output = io.StringIO()
        with redirect_stdout(output):
            getattr(METER, action)(self.state, self.now, self.os, self.os.certificate)
        lines = [json.loads(line) for line in output.getvalue().splitlines()]
        self.printed += lines
        return [line["event"] for line in lines]

    def month(self):
        return json.loads((self.state / "state.json").read_text())["sent"]

    def test_warns_once_at_80_percent_and_stops_at_300_gb_until_the_next_month(self):
        self.assertEqual(self.tick(), [])
        self.assertEqual(self.os.actions, ["start"])
        self.assertEqual(self.tick(sent=240 * GB - 1), [])
        self.assertEqual(self.tick(sent=1), ["device_relay_budget_warning"])
        self.assertEqual(self.tick(sent=10 * GB), [])
        self.assertEqual(self.tick(sent=50 * GB), ["device_relay_stopped"])
        self.assertEqual(self.os.active, b"inactive")
        self.assertTrue((self.state / "stopped").exists(), "Caddy's marker for the 503 answer")
        # Stopped stays stopped, even across a VM restart that reloads the table.
        self.os.table = {"table": 8, "sent": 0, "connections": 0}
        self.assertEqual(self.tick(minutes=60), ["device_relay_hour"])
        self.assertEqual(self.os.actions, ["start", "stop"])
        self.now = datetime.datetime(2026, 11, 30, 23, 58, tzinfo=datetime.timezone.utc)
        self.tick()
        self.assertEqual(self.tick(minutes=2), ["device_relay_hour"])
        self.assertEqual(self.month(), 0)
        self.assertEqual(self.os.active, b"active")
        self.assertFalse((self.state / "stopped").exists())
        self.assertEqual(self.tick(sent=240 * GB), ["device_relay_budget_warning"])

    def test_counts_across_a_reloaded_table_and_reports_each_hour_without_identifiers(self):
        self.tick(sent=5, connections=2)
        self.tick(sent=7, connections=1)
        self.os.table = {"table": 9, "sent": 0, "connections": 0}
        self.tick(sent=11, connections=4)
        self.os.table = None
        self.tick()
        self.assertEqual(self.month(), 23)
        self.assertEqual(self.tick(minutes=60), ["device_relay_hour"])
        self.assertEqual(self.printed[-1], {"event": "device_relay_hour", "hour": "2026-11-19T10",
                                            "connections": 7, "sent_bytes": 23})
        for line in self.printed:
            self.assertLessEqual(set(line), {"event", "hour", "connections", "sent_bytes"})

    def test_operator_resumes_a_stopped_relay_for_the_rest_of_the_month(self):
        with self.assertRaisesRegex(ValueError, "not stopped"):
            self.tick(action="resume")
        self.tick(sent=300 * GB)
        self.assertEqual(self.os.active, b"inactive")
        self.assertEqual(self.tick(action="resume"), ["device_relay_resumed"])
        self.assertEqual(self.os.active, b"active")
        self.assertEqual(self.tick(sent=100 * GB), [])
        self.assertEqual(self.os.active, b"active")
        self.assertFalse((self.state / "stopped").exists())

    def test_relay_waits_for_caddys_certificate_and_restarts_when_it_is_renewed(self):
        self.os.certificate.unlink()
        self.tick()
        self.assertEqual(self.os.actions, [])
        self.os.certificate.write_text("first")
        for _ in range(5):
            self.tick()
        # A running relay with the certificate it started with is left alone.
        self.assertEqual(self.os.actions, ["start"])
        self.os.certificate.write_text("renewed")
        self.tick()
        self.tick()
        self.assertEqual(self.os.actions, ["start", "restart"])


if __name__ == "__main__":
    unittest.main()
