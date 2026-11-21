"""The screenshot gate refuses a silent omission and nothing else.

A screen change with no image and no declared reason fails; an image a
reviewer can open passes; a declared reason passes for the reviewer to judge;
an image only the author can open fails; a change that touches no screen, or
only tests, needs nothing.
"""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = importlib.util.spec_from_file_location('check_pr_screenshots', ROOT / 'scripts' / 'check-pr-screenshots.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)

SCREEN = ['web/src/TaskLens.tsx', 'web/src/taskList.ts']
BODY = '## Summary\n\nThe list groups 기다림 apart.\n\n## Review\n'


class AScreenChangeMustShowItself(unittest.TestCase):
    def test_no_image_and_no_reason_fails(self):
        ok, message = CHECK.verdict(BODY, SCREEN)
        self.assertFalse(ok)
        self.assertIn('web/src/TaskLens.tsx', message)
        self.assertIn('스크린샷 없음', message)

    def test_an_image_a_reviewer_can_open_passes(self):
        for line in [
            '![after](https://github.com/user-attachments/assets/3b296892-7f8f-4c99-a904-6d918fd468b8)',
            '<img alt="after" src="https://github.com/yansfil/pr-assets/blob/main/hide/x/abc1234/after.png?raw=true" width="480">',
            '![after](<https://github.com/user-attachments/assets/abc> "title")',
            '<img alt="Settings > Agents" src="https://github.com/user-attachments/assets/abc" width="480">',
        ]:
            with self.subTest(line=line):
                ok, _ = CHECK.verdict(BODY + '\n' + line + '\n', SCREEN)
                self.assertTrue(ok)

    def test_an_image_only_the_author_can_open_fails(self):
        for url in ['/Users/example/shot.png', 'file:///tmp/shot.png', 'agents/runs/x/after.png',
                    'https://raw.githubusercontent.com/yansfil/pr-assets/main/hide/after.png']:
            with self.subTest(url=url):
                ok, message = CHECK.verdict(BODY + f'\n![after]({url})\n', SCREEN)
                self.assertFalse(ok)
                self.assertIn('cannot open', message)

    def test_a_declared_reason_passes_for_the_reviewer(self):
        for line in ['스크린샷 없음: 한 단어만 바뀜, 표 참조', 'No screenshot: a refactor that draws the same pixels']:
            with self.subTest(line=line):
                ok, message = CHECK.verdict(BODY + '\n' + line + '\n', SCREEN)
                self.assertTrue(ok)
                self.assertIn('declared', message)

    def test_a_declaration_without_a_reason_is_an_omission(self):
        ok, _ = CHECK.verdict(BODY + '\n스크린샷 없음:\n', SCREEN)
        self.assertFalse(ok)

    def test_the_template_comments_count_for_nothing(self):
        body = BODY + '\n<!-- ![caption](https://example.invalid/x.png) 스크린샷 없음: 아직 -->\n'
        ok, _ = CHECK.verdict(body, SCREEN)
        self.assertFalse(ok)


class NoScreenNeedsNoScreenshot(unittest.TestCase):
    def test_a_change_outside_every_screen_passes(self):
        ok, _ = CHECK.verdict(BODY, ['hide-core/src/lib.rs', 'docs/UI_BEHAVIOR.md', 'scripts/ci-plan.py'])
        self.assertTrue(ok)

    def test_tests_and_snapshots_are_not_a_screen(self):
        ok, _ = CHECK.verdict(BODY, ['web/src/TaskLens.test.tsx', 'web/src/taskList.test.ts', 'web/src/x/__snapshots__/a.snap'])
        self.assertTrue(ok)

    def test_the_site_and_the_design_library_are_screens(self):
        for path in ['site/index.html', 'design/hide-ui.lib.pen', 'design/tokens.json']:
            with self.subTest(path=path):
                self.assertTrue(CHECK.is_screen(path))


if __name__ == '__main__':
    unittest.main()
