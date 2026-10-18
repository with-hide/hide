"""A login wall stops the run; a hint or a signed-in provider list does not."""

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.authentication import AuthenticationRequired, require_no_login

# Screens read from real runs of Pi 1.0.4 and omp 18.7.0 (2026-10-16).
PI_MODEL_PICKER = """\
  Only showing models from configured providers. Use /login to add providers.
  gpt-6-luna [openai]
  (1/479)
  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel
"""
OMP_PROVIDER_SETUP = """\
Pick a provider to sign in — you can connect more than one.
╭─ Select provider to login ─────────────────────────────╮
│ ❯ ChatGPT Plus/Pro (Codex Subscription) ● logged in (login) │
│   Anthropic (Claude Pro/Max)                                │
╰─────────────────────────────────────────────────────────────╯
"""
OMP_PROVIDER_SETUP_NOBODY = OMP_PROVIDER_SETUP.replace(" ● logged in (login)", "")


class RequireNoLogin(unittest.TestCase):
    def test_a_hint_naming_the_login_command_is_not_a_login_wall(self):
        require_no_login(PI_MODEL_PICKER)

    def test_a_provider_list_with_a_signed_in_account_is_not_a_login_wall(self):
        require_no_login(OMP_PROVIDER_SETUP)

    def test_a_provider_list_with_nobody_signed_in_is_a_login_wall(self):
        with self.assertRaises(AuthenticationRequired):
            require_no_login(OMP_PROVIDER_SETUP_NOBODY)

    def test_a_sign_in_prompt_is_a_login_wall(self):
        for screen in ("Please sign in to continue", "Not logged in. Run login.", "Authentication required"):
            with self.subTest(screen=screen), self.assertRaises(AuthenticationRequired):
                require_no_login(screen)

    def test_a_folder_named_login_in_a_prompt_is_not_a_login_wall(self):
        require_no_login("runner@host:~/work/hide/agents/runs/agent-journey-login-linux/probe/controller-journey$ ")
        require_no_login(r"PS D:\a\hide\agents\runs\agent-journey-login-windows\probe> ")

    def test_a_wall_that_names_a_login_page_still_stops(self):
        with self.assertRaises(AuthenticationRequired):
            require_no_login("Sign in at https://example.com/login to continue")

    def test_a_hint_beside_a_real_wall_still_stops(self):
        with self.assertRaises(AuthenticationRequired):
            require_no_login("Use /login to add providers.\nNot logged in")


if __name__ == "__main__":
    unittest.main()
