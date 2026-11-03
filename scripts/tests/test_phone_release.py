"""B25/B29: main CI uploads retained phone bundles before any release artifacts.

GitHub interprets the dependency graph; test that contract here and the actual
filesystem/HTTP boundary in Node. No test contacts or deploys to the host.
"""
from pathlib import Path
import os
import hashlib
import http.server
import json
import shlex
import threading
from urllib.parse import urlsplit
import re
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]


class PhoneRelease(unittest.TestCase):
    def test_retention_production_bytes_and_refusals(self):
        result = subprocess.run(["node", "--test", "scripts/tests/phone-release.test.mjs"],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_failed_upload_cannot_produce_a_release_candidate(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        jobs = workflow["jobs"]
        phone = jobs["phone"]
        self.assertEqual(phone["needs"], "validate")
        self.assertEqual(phone["environment"], "phone-production")
        self.assertIn("github.ref == 'refs/heads/main'", phone["if"])
        self.assertIn("github.repository == 'with-hide/hide'", phone["if"])
        self.assertEqual(phone["concurrency"], {"group": "phone-production", "cancel-in-progress": False})
        self.assertLessEqual(phone["timeout-minutes"], 30)
        for name in ["packages", "macos-binaries", "receipt"]:
            self.assertIn("phone", jobs[name]["needs"])
            self.assertNotIn("if", jobs[name])  # GitHub's default success dependency
        for job in [phone, jobs["packages"], jobs["macos-binaries"], jobs["receipt"]]:
            self.assertNotIn("continue-on-error", job)
            for step in job.get("steps", []):
                self.assertNotIn("continue-on-error", step)
                self.assertNotIn("if", step)
        commands = [step["run"] for step in phone["steps"] if "run" in step]
        self.assertEqual(len(commands), 5)
        self.assertIn("phone-release.mjs check", commands[0])
        self.assertIn("verify-web.sh web build:phone", commands[1])
        self.assertIn("phone-release.mjs prepare", commands[2])
        self.assertIn("deploy --prebuilt --prod", commands[3])
        self.assertIn("phone-release.mjs confirm", commands[4])
        # No other workflow may deploy this host or receive its credential.
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            if path.name != "release-candidate.yml":
                self.assertNotIn("PHONE_VERCEL_TOKEN", path.read_text())

    def test_named_secrets_and_example_are_one_contract(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        phone = workflow["jobs"]["phone"]
        keys = {line.split("=", 1)[0] for line in (ROOT / "scripts/phone-release.env.example").read_text().splitlines()
                if line and not line.startswith("#")}
        self.assertFalse(keys.intersection(workflow.get("env", {})))
        self.assertFalse(keys.intersection(phone.get("env", {})))
        for step in phone["steps"]:
            command = step.get("run", "")
            expected = (keys if "phone-release.mjs check" in command or "phone-release.mjs prepare" in command
                        else {"PHONE_VERCEL_TOKEN"} if "deploy --prebuilt --prod" in command else set())
            env = {key: value for key, value in step.get("env", {}).items() if key in keys}
            self.assertEqual(set(env), expected, step.get("name", step.get("uses")))
            for key in expected:
                self.assertEqual(env[key], "${{ secrets." + key + " }}")
        result = subprocess.run(["node", "--input-type=module", "-e",
                                 "import {SECRETS} from './scripts/phone-release.mjs'; console.log(Object.keys(SECRETS).sort().join('\\n'))"],
                                cwd=ROOT, capture_output=True, text=True, timeout=10, check=True)
        self.assertEqual(set(result.stdout.splitlines()), keys)

    def test_phone_job_actions_are_immutable_with_version_notes(self):
        text = (ROOT / ".github/workflows/release-candidate.yml").read_text()
        phone = yaml.safe_load(text)["jobs"]["phone"]
        versions = {"actions/checkout": "v7", "actions/setup-node": "v4", "pnpm/action-setup": "v6"}
        actions = [step["uses"] for step in phone["steps"] if "uses" in step]
        self.assertEqual({action.split("@")[0] for action in actions}, set(versions))
        for action in actions:
            name, revision = action.split("@")
            self.assertRegex(revision, r"^[a-f0-9]{40}$")
            self.assertRegex(text, rf"uses: {re.escape(action)} # {versions[name]}(?:\s|$)")

    def test_upload_command_propagates_the_hosts_failure(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        upload = next(step["run"] for step in workflow["jobs"]["phone"]["steps"]
                      if step.get("name") == "Upload only the CI-built static site")
        runs = ROOT / "agents/runs/phone-release-tests"
        runs.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=runs) as directory:
            # Substitute only the external upload executable. Run the actual
            # workflow command and require its failure status to reach GitHub.
            fake = Path(directory) / "npx"
            fake.write_text("#!/bin/sh\nexit 23\n")
            fake.chmod(0o755)
            result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", upload], cwd=ROOT,
                                    env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"],
                                         "PHONE_VERCEL_TOKEN": "fixture-token"},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 23, result.stdout + result.stderr)


@unittest.skipUnless(os.environ.get("PHONE_RELEASE_VERCEL_CLI") and
                     os.environ.get("PHONE_RELEASE_OLD_VERCEL_CLI"),
                     "install both npm CLIs and set PHONE_RELEASE_{OLD_,}VERCEL_CLI; see docs/BUILD.md")
class PhoneReleaseCli(unittest.TestCase):
    """Real npm entrypoints; only Vercel HTTP is replaced, never CLI internals."""

    def test_project_token_deploys_prebuilt_and_refusals_stop(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        command = next(step["run"] for step in workflow["jobs"]["phone"]["steps"]
                       if step.get("name") == "Upload only the CI-built static site")
        argv = shlex.split(command)
        self.assertEqual(argv[:2], ["npx", "--yes"])
        current = Path(os.environ["PHONE_RELEASE_VERCEL_CLI"]).resolve()
        old = Path(os.environ["PHONE_RELEASE_OLD_VERCEL_CLI"]).resolve()
        for cli, version in [(current, argv[2].removeprefix("vercel@")), (old, "50.1.3")]:
            self.assertEqual(json.loads((cli.parent.parent / "package.json").read_text())["version"], version)
        runs = ROOT / "agents/runs/mission-27-phone-deploy"
        runs.mkdir(parents=True, exist_ok=True)
        for cli, failure in [(old, "old owner lookup"), (current, None),
                             (current, "project missing"), (current, "upload denied"),
                             (current, "other team error")]:
            with self.subTest(version=json.loads((cli.parent.parent / "package.json").read_text())["version"], failure=failure), tempfile.TemporaryDirectory(dir=runs) as tmp:
                root = Path(tmp)
                state = {"requests": [], "deployments": [], "uploads": {}, "unexpected": []}

                class Api(http.server.BaseHTTPRequestHandler):
                    def log_message(self, *_args):
                        pass

                    def reply(self, status, data):
                        self.send_response(status)
                        self.send_header("Content-Type", "application/json")
                        self.end_headers()
                        self.wfile.write(json.dumps(data).encode())

                    def do_GET(self):
                        path = urlsplit(self.path).path
                        state["requests"].append(path)
                        if path == "/teams/team_fixture":
                            code = "forbidden" if failure == "other team error" else "team_unauthorized"
                            self.reply(403, {"error": {"code": code, "message": "No team access"}})
                        elif path == "/v9/projects/prj_fixture":
                            if failure == "project missing" and state["prepared"]:
                                self.reply(404, {"error": {"code": "not_found", "message": "No project access"}})
                            else:
                                self.reply(200, {"id": "prj_fixture", "name": "phone-fixture",
                                                 "accountId": "team_fixture", "framework": None})
                        elif path == "/v1/teams":
                            self.reply(403, {"error": {"code": "team_unauthorized", "message": "No team access"}})
                        elif path == "/v2/user":
                            self.reply(403, {"error": {"code": "forbidden", "message": "No user access"}})
                        else:
                            state["unexpected"].append(path)
                            self.reply(400, {"error": {"code": "fixture_unexpected", "message": "Unexpected path"}})

                    def do_POST(self):
                        path = urlsplit(self.path).path
                        size = int(self.headers.get("Content-Length", "0"))
                        if size > 1024 * 1024:
                            self.reply(413, {"error": {"code": "too_large"}})
                            return
                        data = self.rfile.read(size)
                        if path == "/v13/deployments":
                            body = json.loads(data)
                            state["deployments"].append(body)
                            if failure == "upload denied":
                                self.reply(403, {"error": {"code": "forbidden", "message": "Upload denied"}})
                                return
                            missing = [item["sha"] for item in body["files"]
                                       if item["sha"] not in state["uploads"]]
                            if missing:
                                self.reply(400, {"error": {"code": "missing_files", "missing": missing}})
                            else:
                                self.reply(200, {"id": "dpl_fixture", "url": "fixture.vercel.app",
                                                 "readyState": "READY", "status": "READY", "target": "production",
                                                 "aliasAssigned": True, "alias": [], "inspectorUrl": "https://fixture.invalid"})
                        elif path == "/v2/files":
                            state["uploads"][hashlib.sha1(data).hexdigest()] = data
                            self.reply(200, {})
                        else:
                            state["unexpected"].append(path)
                            self.reply(400, {"error": {"code": "fixture_unexpected", "message": "Unexpected path"}})

                server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Api)
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    env = {"PATH": os.environ["PATH"], "HOME": str(root / "home"),
                           "XDG_CONFIG_HOME": str(root / "home"), "CI": "1",
                           "VERCEL_TELEMETRY_DISABLED": "1", "NO_UPDATE_NOTIFIER": "1",
                           "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
                    for key in ("SystemRoot", "COMSPEC", "PATHEXT"):
                        if key in os.environ:
                            env[key] = os.environ[key]
                    for key in ("USERPROFILE", "APPDATA", "LOCALAPPDATA"):
                        env[key] = str(root / "home")
                    # A private repository keeps the CLI away from workstation git metadata.
                    subprocess.run(["git", "init", "-q", "--initial-branch=main", str(root)],
                                   env=env, check=True, capture_output=True, timeout=10)
                    api = f"http://127.0.0.1:{server.server_port}"
                    state["prepared"] = False
                    stage = root / "stage"
                    prepare = r"""
                      import { preparePhone } from './scripts/phone-release.mjs';
                      import { mkdir, writeFile } from 'node:fs/promises';
                      import { join } from 'node:path';
                      const [root, api] = process.argv.slice(1);
                      const dist = join(root, 'dist');
                      await mkdir(join(dist, 'p/2/assets'), { recursive: true });
                      const page = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self' wss://relay.withhide.dev">`;
                      for (const path of ['index.html', 'p/2/index.html']) await writeFile(join(dist, path), page);
                      await writeFile(join(dist, 'protocols.json'), '[2]');
                      await writeFile(join(dist, 'p/2/assets/fixture.js'), 'prebuilt bytes');
                      await preparePhone({ source: 'a'.repeat(40), dist, stage: join(root, 'stage'),
                        secrets: { PHONE_VERCEL_TOKEN: 'fixtureToken', PHONE_VERCEL_PROJECT_ID: 'prj_fixture', PHONE_VERCEL_ORG_ID: 'team_fixture' },
                        fetcher: (url, options) => fetch(api + new URL(url).pathname, options) });
                    """
                    subprocess.run(["node", "--input-type=module", "-e", prepare, str(root), api],
                                   cwd=ROOT, env=env, check=True, capture_output=True, timeout=15)
                    state["prepared"] = True
                    args = argv[3:]
                    args[args.index("--cwd") + 1] = str(stage)
                    args[args.index("--token") + 1] = "fixtureToken"
                    result = subprocess.run(["node", str(cli), *args, "--api", api,
                                             "--global-config", str(root / "config")],
                                            cwd=root, env=env, capture_output=True, text=True, timeout=30)
                    output = result.stdout + result.stderr
                    self.assertFalse(state["unexpected"], state["unexpected"])
                    self.assertIn("/teams/team_fixture", state["requests"])
                    self.assertIn("/v9/projects/prj_fixture", state["requests"])
                    if failure:
                        self.assertNotEqual(result.returncode, 0, output)
                        self.assertFalse(state["uploads"])
                        if failure == "upload denied":
                            self.assertTrue(state["deployments"], output)
                        else:
                            self.assertFalse(state["deployments"], output)
                        if failure == "old owner lookup":
                            self.assertIn("Could not retrieve Project Settings", output)
                    else:
                        self.assertEqual(result.returncode, 0, output)
                        self.assertIn("https://fixture.vercel.app", result.stdout)
                        self.assertTrue(state["uploads"], output)
                        expected = {path.relative_to(stage).as_posix(): path.read_bytes()
                                    for path in (stage / ".vercel/output").rglob("*") if path.is_file()}
                        sent = state["deployments"][-1]
                        self.assertEqual(sent["target"], "production")
                        self.assertEqual(sent["project"], "prj_fixture")
                        self.assertEqual({item["file"]: state["uploads"][item["sha"]]
                                          for item in sent["files"]}, expected)
                        self.assertNotIn(b"fixtureToken", b"".join(state["uploads"].values()))
                finally:
                    server.shutdown()
                    server.server_close()
                    thread.join(timeout=5)
                    self.assertFalse(thread.is_alive(), "fixture API did not stop")


if __name__ == "__main__":
    unittest.main()
