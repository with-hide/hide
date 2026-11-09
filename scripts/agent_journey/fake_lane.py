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


def claude_config(root, key=KEY):
    """Claude's global config: onboarding done, the fake key accepted, and the
    measured repository trusted; the never-trusted repository is left out."""
    return {"hasCompletedOnboarding": True,
            "customApiKeyResponses": {"approved": [key[-20:]], "rejected": []},
            "projects": {ci.claude_project(root): {"hasTrustDialogAccepted": True}}}


def claude(home, url, root, local, key=KEY):
    """Claude Code takes the base URL and key from its environment. Locally its
    configuration folder is the daemon home's `.claude`, whose `.config.json`
    is the global config Claude reads there and the file Hide records its
    folder trust in (hide-agent-hooks runtime.rs, `trust_path`); in CI mode
    it is `~/.claude.json` of the disposable HOME, as a new machine's."""
    if local:
        claude_files(home / ".claude", root, key)
    else:
        private_directory(home)
        write_private(home / ".claude.json", json.dumps(claude_config(root, key)).encode())
    env = {"ANTHROPIC_BASE_URL": url, "ANTHROPIC_API_KEY": key}
    if local:
        env["CLAUDE_CONFIG_DIR"] = str(home / ".claude")
    return env


def claude_files(folder, root, key=KEY):
    """Claude's global config in the account `folder` (CLAUDE_CONFIG_DIR)."""
    private_directory(folder)
    write_private(folder / ".config.json", json.dumps(claude_config(root, key)).encode())


def codex(home, url, root, local, key=KEY):
    """Codex 0.163 reads no base URL from its environment: a provider of its
    own in CODEX_HOME's config.toml, whose key it reads from `env_key`."""
    del root, local
    codex_files(home / ".codex", url)
    return {"JOURNEY_FAKE_KEY": key}


def codex_files(folder, url):
    """The fake provider in the account `folder` (CODEX_HOME)."""
    private_directory(folder)
    write_private(folder / "config.toml", (f'model = "{MODEL}"\nmodel_provider = "{PROVIDER}"\n'
                           f'[model_providers.{PROVIDER}]\nname = "{PROVIDER}"\nbase_url = "{url}/v1"\n'
                           f'wire_api = "responses"\nenv_key = "JOURNEY_FAKE_KEY"\n').encode())


def grok(home, url, root, local, key=KEY):
    """Grok 1.0.52's custom model in GROK_HOME's config.toml
    (~/.grok/docs/user-guide/11-custom-models.md), its default."""
    del root, local
    grok_files(home / ".grok", url)
    return {"JOURNEY_FAKE_KEY": key}


def grok_files(folder, url):
    """The fake model in the account `folder` (GROK_HOME)."""
    private_directory(folder)
    write_private(folder / "config.toml", (f'[models]\ndefault = "{PROVIDER}"\n[model.{PROVIDER}]\nmodel = "{MODEL}"\n'
                           f'base_url = "{url}/v1"\nenv_key = "JOURNEY_FAKE_KEY"\napi_backend = "chat_completions"\n'
                           f'name = "Journey"\ncontext_window = 128000\n').encode())


def pi(home, url, root, local, key=KEY):
    """Pi 1.0.4's custom provider (`models.json`) and default model
    (`settings.json`) in its agent folder."""
    del root, local
    pi_files(home / ".pi/agent", url)
    return {"JOURNEY_FAKE_KEY": key}


def pi_files(folder, url):
    """The fake provider and default model in the account `folder`
    (PI_CODING_AGENT_DIR)."""
    private_directory(folder)
    write_private(folder / "models.json", json.dumps({"providers": {PROVIDER: {
        "baseUrl": f"{url}/v1", "api": "openai-completions", "apiKey": "${JOURNEY_FAKE_KEY}",
        "models": [{"id": MODEL, "name": "Journey"}]}}}).encode())
    write_private(folder / "settings.json", json.dumps({"defaultProvider": PROVIDER, "defaultModel": MODEL}).encode())


def omp(home, url, root, local, key=KEY):
    """omp 18.7.0's custom provider (`models.yml`) and default model role
    (`config.yml`) in its agent folder; its key is read from the variable
    `apiKey` names."""
    del root, local
    omp_files(home / ".omp/agent", url)
    return {"JOURNEY_FAKE_KEY": key}


def omp_files(folder, url):
    """The fake provider and default model role in omp's agent `folder`."""
    private_directory(folder)
    write_private(folder / "models.yml", (f"providers:\n  {PROVIDER}:\n    baseUrl: {url}/v1\n"
                                          f"    apiKey: JOURNEY_FAKE_KEY\n    api: openai-completions\n"
                                          f"    models:\n      - id: {MODEL}\n        name: Journey\n").encode())
    write_private(folder / "config.yml", (ci.OMP_SET_UP + f"modelRoles:\n  default: {PROVIDER}/{MODEL}\n").encode())


def agy(home, url, root, local, key=KEY):
    """agy 1.4.0 signs in with GEMINI_API_KEY once its settings name the
    Gemini provider (ci.py); its base URL variable is the Gemini SDK's."""
    del root, local
    ci.agy_set_up(home)
    return {"GEMINI_API_KEY": key, "GOOGLE_GEMINI_BASE_URL": url}


def opencode(home, url, root, local, key=KEY):
    """OpenCode 2.0.29's custom provider and default model in its global
    `opencode.json` (opencode.ai/v2/docs/providers), its key read from the
    variable `env` names, and no update check. Every other provider is
    denied (`provider.use` policies, what its `enabled_providers` key reads
    as): OpenCode submits a start prompt before it has read this file, and a
    start that lost that race took its own free hosted model (macOS, run
    41415065150, 2026-11-02); Hide types OpenCode's brief since, but a first
    prompt still rides the start, and a run's text must never reach a hosted
    model."""
    del root, local
    opencode_files(home / opencode_service.CONFIG, url)
    return {"JOURNEY_FAKE_KEY": key}


def opencode_files(folder, url):
    """The fake provider in OpenCode's configuration `folder`."""
    private_directory(folder)
    write_private(folder / "opencode.json", json.dumps({
        "model": f"{PROVIDER}/{MODEL}",
        "policies": [{"action": "provider.use", "resource": "*", "effect": "deny"},
                     {"action": "provider.use", "resource": PROVIDER, "effect": "allow"}],
        "providers": {PROVIDER: {"name": "Journey", "env": ["JOURNEY_FAKE_KEY"],
                                 "package": "@opencode/ai/providers/openai-compatible",
                                 "settings": {"baseURL": f"{url}/v1"}, "models": {MODEL: {"name": "Journey"}}}}}).encode())


# The account a profile of each agent runs in for the `profile` cell, under
# the run's HOME: a folder its account variable names, or omp's profile name
# with the agent folder omp keeps for it (hide-agent-adapter/agents/*.toml,
# `[account]`), and the files that point the agent at the fake model there.
PROFILE_NAME = "journey"
PROFILE_ACCOUNTS = {
    "claude": (".journey-profile/claude", lambda folder, url, root, key: claude_files(folder, root, key)),
    "codex": (".journey-profile/codex", lambda folder, url, root, key: codex_files(folder, url)),
    "grok": (".journey-profile/grok", lambda folder, url, root, key: grok_files(folder, url)),
    "pi": (".journey-profile/pi", lambda folder, url, root, key: pi_files(folder, url)),
}


def profile_account(kind, home, url, root, key=KEY):
    """Writes the files that point `kind` at the fake model at `url` in the
    account its journey profile runs in, and answers that account: the
    profile file's `account`, the reader's spelling of it, and the folder its
    sessions lie under."""
    if kind == "opencode":
        # Its XDG folders under the account; its background server takes a
        # port of its own there (opencode_service), which the run ends as it
        # ends the default account's.
        folder = home / ".journey-profile/opencode"
        config = folder / "config/opencode"
        opencode_files(config, url)
        opencode_service.configure(config)
        return {"account": {"folder": str(folder)}, "reader": "folder:" + str(folder), "folder": str(folder)}
    if kind == "omp":
        folder = home / ".omp/profiles" / PROFILE_NAME / "agent"
        omp_files(folder, url)
        return {"account": {"name": PROFILE_NAME}, "reader": "name:" + PROFILE_NAME, "folder": str(folder)}
    relative, write = PROFILE_ACCOUNTS[kind]
    folder = home / relative
    write(folder, url, root, key)
    return {"account": {"folder": str(folder)}, "reader": "folder:" + str(folder), "folder": str(folder)}


PROVIDERS = {"claude": claude, "codex": codex, "grok": grok, "pi": pi, "omp": omp, "agy": agy, "opencode": opencode}


def configure(kind, home, url, root, local, key=KEY):
    """Writes the files that point `kind` at the fake model at `url` and
    returns the environment it runs with."""
    env = {**QUIET, **PROVIDERS[kind](home, url, root, local, key)}
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
