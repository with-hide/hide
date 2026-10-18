"""The keyless lane (`--fake-model`): each real CLI, with Hide's real kit,
pointed at the journey's fake model (fake_model.py) instead of a provider
(PRD agent-journey-check D-46, as the lead's 2026-10-18 decision changes it).

This table is the lane's only per-agent CLI knowledge: the environment and
the files under the run's daemon home that send the CLI's model requests to
`url`, in either mode. CI and guarded local fake runs use that private HOME;
real local logins keep the operator's HOME. No login is supplied to a fake agent.
"""

import json
from pathlib import Path

from .fake_private import configure_update

from agent_live_check.protection import private_directory, write_private

from . import ci, opencode_service
from .fake_model import MODEL

# Not a secret: the fake model takes any key, and a CLI needs one to start.
KEY = "journey-fake-key-0000000000000000"
PROVIDER = "journey"

# What a fake model's answer cannot show, said in its report.
LIMITS = [
    "The model is the journey's fake (scripts/agent_journey/fake_model.py): it answers each turn with the token "
    "the turn carries and never calls a tool, so it proves the CLI, Hide's kit and Herdr's integration, never a "
    "provider, its login or its quota.",
    "Usage is the fake's estimate in the provider's shape; a CLI's own accounting of a real answer is not seen.",
]

# Update policies come from the Rust adapters. Telemetry stays off, and
# requests to the fake skip any proxy the runner has.
QUIET = {"NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
         "PI_TELEMETRY": "0",
         "GROK_TELEMETRY_ENABLED": "0",
         "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"}


def claude_config(root):
    """Claude's global config: onboarding done, the fake key accepted, and the
    measured repository trusted; the never-trusted repository is left out."""
    return {"hasCompletedOnboarding": True,
            "customApiKeyResponses": {"approved": [KEY[-20:]], "rejected": []},
            "projects": {ci.claude_project(root): {"hasTrustDialogAccepted": True}}}


def claude(home, url, root, local):
    """Claude Code takes the base URL and key from its environment. Locally its
    configuration folder is the daemon home's `.claude`, whose `.config.json`
    is the global config Claude reads there and the file Hide records its
    folder trust in (hide-agent-hooks runtime.rs, `trust_path`); in CI mode
    it is `~/.claude.json` of the disposable HOME, as a new machine's."""
    config = home / ".claude/.config.json" if local else home / ".claude.json"
    private_directory(config.parent)
    write_private(config, json.dumps(claude_config(root)).encode())
    env = {"ANTHROPIC_BASE_URL": url, "ANTHROPIC_API_KEY": KEY}
    if local:
        env["CLAUDE_CONFIG_DIR"] = str(home / ".claude")
    return env


def codex(home, url, root, local):
    """Codex 0.163 reads no base URL from its environment: a provider of its
    own in CODEX_HOME's config.toml, whose key it reads from `env_key`."""
    del root, local
    config = home / ".codex/config.toml"
    private_directory(config.parent)
    write_private(config, (f'model = "{MODEL}"\nmodel_provider = "{PROVIDER}"\n'
                           f'[model_providers.{PROVIDER}]\nname = "{PROVIDER}"\nbase_url = "{url}/v1"\n'
                           f'wire_api = "responses"\nenv_key = "JOURNEY_FAKE_KEY"\n').encode())
    return {"JOURNEY_FAKE_KEY": KEY}


def grok(home, url, root, local):
    """Grok 1.0.52's custom model in GROK_HOME's config.toml
    (~/.grok/docs/user-guide/11-custom-models.md), its default."""
    del root, local
    config = home / ".grok/config.toml"
    private_directory(config.parent)
    write_private(config, (f'[models]\ndefault = "{PROVIDER}"\n[model.{PROVIDER}]\nmodel = "{MODEL}"\n'
                           f'base_url = "{url}/v1"\nenv_key = "JOURNEY_FAKE_KEY"\napi_backend = "chat_completions"\n'
                           f'name = "Journey"\ncontext_window = 128000\n').encode())
    return {"JOURNEY_FAKE_KEY": KEY}


def pi(home, url, root, local):
    """Pi 1.0.4's custom provider (`models.json`) and default model
    (`settings.json`) in its agent folder."""
    del root, local
    folder = home / ".pi/agent"
    private_directory(folder)
    write_private(folder / "models.json", json.dumps({"providers": {PROVIDER: {
        "baseUrl": f"{url}/v1", "api": "openai-completions", "apiKey": "${JOURNEY_FAKE_KEY}",
        "models": [{"id": MODEL, "name": "Journey"}]}}}).encode())
    write_private(folder / "settings.json", json.dumps({"defaultProvider": PROVIDER, "defaultModel": MODEL}).encode())
    return {"JOURNEY_FAKE_KEY": KEY}


def pi_trust(home, roots):
    """The trusted-start fixture; never include the untrusted-start sibling.

    Pi 1.0.4 dist/core/trust-manager.js reads canonical folder -> boolean.
    """
    write_private(home / ".pi/agent/trust.json",
                  json.dumps({str(path.resolve()): True for path in roots}).encode())


def omp(home, url, root, local):
    """omp 18.7.0's custom provider (`models.yml`) and default model role
    (`config.yml`) in its agent folder; its key is read from the variable
    `apiKey` names."""
    del root, local
    folder = home / ".omp/agent"
    private_directory(folder)
    write_private(folder / "models.yml", (f"providers:\n  {PROVIDER}:\n    baseUrl: {url}/v1\n"
                                          f"    apiKey: JOURNEY_FAKE_KEY\n    api: openai-completions\n"
                                          f"    models:\n      - id: {MODEL}\n        name: Journey\n").encode())
    write_private(folder / "config.yml", (ci.OMP_SET_UP + f"modelRoles:\n  default: {PROVIDER}/{MODEL}\n").encode())
    return {"JOURNEY_FAKE_KEY": KEY}


def agy(home, url, root, local):
    """agy 1.4.0 signs in with GEMINI_API_KEY once its settings name the
    Gemini provider (ci.py); its base URL variable is the Gemini SDK's."""
    del root, local
    ci.agy_set_up(home)
    return {"GEMINI_API_KEY": KEY, "GOOGLE_GEMINI_BASE_URL": url}


def opencode(home, url, root, local):
    """OpenCode 2.0.29's custom provider and default model in its global
    `opencode.json` (opencode.ai/v2/docs/providers), its key read from the
    variable `env` names, and no update check."""
    del root, local
    folder = home / opencode_service.CONFIG
    private_directory(folder)
    write_private(folder / "opencode.json", json.dumps({
        "model": f"{PROVIDER}/{MODEL}",
        "providers": {PROVIDER: {"name": "Journey", "env": ["JOURNEY_FAKE_KEY"],
                                 "package": "@opencode/ai/providers/openai-compatible",
                                 "settings": {"baseURL": f"{url}/v1"}, "models": {MODEL: {"name": "Journey"}}}}}).encode())
    return {"JOURNEY_FAKE_KEY": KEY}


PROVIDERS = {"claude": claude, "codex": codex, "grok": grok, "pi": pi, "omp": omp, "agy": agy, "opencode": opencode}


def configure(kind, home, url, root, local):
    """Writes the files that point `kind` at the fake model at `url` and
    returns the environment it runs with."""
    env = {**QUIET, **PROVIDERS[kind](home, url, root, local)}
    rows = json.loads((Path(__file__).resolve().parents[2] / "contracts/agent-journey.json").read_text())
    declaration = next(row.get("fake_home", {}) for row in rows if row["herdr_kind"] == kind)
    if declaration.get("auto_update"):
        env.update(configure_update(declaration, home))
    return env


def requests(log):
    """The model requests the fake answered, from its log: what a provider
    would have been asked, a models listing or a token count left out."""
    if not log.is_file():
        return 0
    return sum(1 for line in log.read_text().splitlines() if json.loads(line).get("generation"))
