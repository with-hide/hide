"""The Hide AI prompt check must find a named language in production prompts only."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

REQUEST = (
    'use hide_ai::{{AiRequest, RequestId}};\n'
    'fn ask() -> AiRequest {{ AiRequest {{ system: "{system}".into(), input: String::new() }} }}\n'
)


def load_check(root):
    spec = importlib.util.spec_from_file_location(
        'check_ai_prompt_language', ROOT / 'scripts/check-ai-prompt-language.py'
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.ROOT = root
    module.PROMPT_ONLY = {}
    module.NOT_PROMPTS = {}
    return module


class AiPromptLanguage(unittest.TestCase):
    def tree(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for tree in load_check(root).TREES:
            (root / tree).mkdir(parents=True, exist_ok=True)
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root, load_check(root)

    def test_korean_in_a_prompt_is_found(self):
        _, check = self.tree({'herdr-core/src/label.rs': REQUEST.format(system='goal은 한국어 명사구입니다.')})
        self.assertEqual(check.main(), 1)

    def test_a_fixed_output_language_is_found(self):
        for system in (
            'Summarize it in the language of the title.',
            'Write the card in Korean.',
            'Aliases in both Korean and English.',
            "Keep the card's own language.",
        ):
            root, check = self.tree({'hide-memory/src/engine.rs': REQUEST.format(system=system)})
            found = check.findings(root / 'hide-memory/src/engine.rs')
            self.assertTrue(found, system)

    def test_a_raw_string_prompt_is_read(self):
        text = 'use hide_ai::AiRequest;\nconst P: &str = r#"Answer in Japanese."#;\nfn a() -> AiRequest { AiRequest { system: P.into() } }\n'
        root, check = self.tree({'hide-memory/src/verify.rs': text})
        self.assertEqual(len(check.findings(root / 'hide-memory/src/verify.rs')), 1)

    def test_tests_comments_and_files_without_a_request_are_not_prompts(self):
        clean = REQUEST.format(system='Name the goal.') + (
            '// 한국어 주석은 프롬프트가 아니다\n'
            '#[cfg(test)]\nmod tests { const FIXTURE: &str = "파서 버그 고쳐줘 in Korean"; }\n'
        )
        root, check = self.tree({
            'herdr-core/src/label.rs': clean,
            'herdr-core/src/other.rs': 'fn word() -> &\'static str { "요청" }\n',
        })
        self.assertEqual(check.sources(), ['herdr-core/src/label.rs'])
        self.assertEqual(check.main(), 0)

    def test_a_listed_prompt_source_that_is_gone_fails(self):
        root, check = self.tree({'herdr-core/src/label.rs': REQUEST.format(system='Name the goal.')})
        check.PROMPT_ONLY = {'hide-kit/src/missing_prompt.rs': 'gone'}
        self.assertEqual(check.main(), 1)

    def test_an_excused_file_fails_on_a_new_finding(self):
        text = REQUEST.format(system='Name the goal.') + 'fn tail() -> &\'static str { "읽으세요" }\n'
        root, check = self.tree({'herdr-core/src/fixture.rs': text})
        check.NOT_PROMPTS = {'herdr-core/src/fixture.rs': (1, 'a worker prompt')}
        self.assertEqual(check.main(), 0)
        (root / 'herdr-core/src/fixture.rs').write_text(text + 'fn more() -> &\'static str { "in Korean" }\n')
        self.assertEqual(check.main(), 1)

    def test_the_repository_prompts_pass(self):
        spec = importlib.util.spec_from_file_location(
            'check_ai_prompt_language_repo', ROOT / 'scripts/check-ai-prompt-language.py'
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertIn('herdr-core/src/labels/context_label.rs', module.sources())
        self.assertEqual(module.main(), 0)


if __name__ == '__main__':
    unittest.main()
