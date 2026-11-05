"""A key echo summary reports the clock offset its cross-check rests on.

Until 2026-10-10 `key-echo.mjs` took t0 on the driver's clock and t1 on the
page's, and the two differed by a constant that changed per run (+75, -21
and -22 ms in three runs), so a 5 ms comparison measured the offset. The
sample is now taken on the page's clock alone, and the driver's offset from
the page is kept per hop; this checks that `summarize.py echo` reports that
offset and the driver-clock samples per trial, and adds nothing for an echo
file that carries neither.
"""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SUMMARIZE = ROOT / 'scripts' / 'web-shell-measure' / 'summarize.py'


def run_summary(kind, docs, *args):
    with tempfile.TemporaryDirectory() as folder:
        paths = []
        for index, doc in enumerate(docs):
            path = Path(folder) / f'trial-{index}.json'
            path.write_text(json.dumps(doc))
            paths.append(str(path))
        result = subprocess.run(['python3', str(SUMMARIZE), kind, *args, *paths],
                                capture_output=True, text=True)
    return result.returncode, json.loads(result.stdout)


def record(samples):
    return {'samples': samples, 'expected_samples': len(samples), 'complete': True,
            'boundary': 'page_keydown_to_xterm_write'}


def summarize(doc):
    complete = {**record(doc['samples']), **doc}
    status, summary = run_summary('echo', [complete])
    if status != 0:
        raise AssertionError(summary)
    return summary['trials'][0]


class EchoSummaryClocks(unittest.TestCase):
    def test_a_key_echo_reports_its_offset_and_driver_clock_samples(self):
        hops = [{'clock_offset_ms': offset, 'clock_round_trip_ms': trip}
                for offset, trip in ((-21.5, 0.4), (-21.0, 0.9), (-22.0, 0.3), (-21.2, 0.5))]
        trial = summarize({'samples': [18.0, 19.0, 20.0, 21.0],
                           'node_t0_samples': [-2.0, -1.0, 0.0, 1.0], 'hops': hops})
        self.assertEqual((trial['p50_ms'], trial['p95_ms'], trial['negative']), (19.0, 21.0, 0))
        self.assertEqual(trial['clock_offset_ms'], {'p50': -21.5, 'min': -22.0, 'max': -21.0})
        self.assertEqual(trial['clock_round_trip_max_ms'], 0.9)
        self.assertEqual((trial['node_t0_p50_ms'], trial['node_t0_p95_ms']), (-1.0, 1.0))

    def test_an_echo_without_a_cross_check_reports_none(self):
        trial = summarize({'samples': [5.0, 6.0], 'hops': [{'sample': 0}, {'sample': 1}]})
        self.assertNotIn('clock_offset_ms', trial)
        self.assertNotIn('node_t0_p50_ms', trial)


class MeasurementValidity(unittest.TestCase):
    def test_impossible_samples_never_pass_or_contribute_percentiles(self):
        for samples in ([-5, -4, -3, -2, -1], [1, float('nan')],
                        [float('inf')], [True], ['1'], [None], []):
            with self.subTest(samples=samples):
                status, result = run_summary('echo', [record(samples)])
                self.assertNotEqual(status, 0)
                self.assertFalse(result['pass'])
                self.assertFalse(result['valid'])
                self.assertTrue(result['trials'][0]['errors'])
                self.assertIsNone(result['median_trial_p95_ms'])

    def test_unproven_clock_or_incomplete_trial_never_passes(self):
        cases = [dict(record([1, 2]), boundary='driver_cli_return_to_page_write'),
                 dict(record([1, 2]), expected_samples=3),
                 dict(record([1, 2]), complete=False),
                 dict(record([1, 2]), error='setup failed')]
        for field in ('boundary', 'expected_samples', 'complete', 'samples'):
            missing = record([1, 2])
            del missing[field]
            cases.append(missing)
        for doc in cases:
            with self.subTest(doc=doc):
                status, result = run_summary('echo', [doc])
                self.assertNotEqual(status, 0)
                self.assertFalse(result['valid'])
                self.assertFalse(result['pass'])

    def test_one_invalid_trial_cannot_be_hidden_by_good_trials(self):
        docs = [record([1, 2]), record([1, 2]), record([-1, 2])]
        status, result = run_summary('echo', docs)
        self.assertNotEqual(status, 0)
        self.assertFalse(result['pass'])
        self.assertIsNone(result['median_trial_p95_ms'])

    def test_valid_observations_have_no_implicit_historical_threshold(self):
        status, result = run_summary('echo', [record([1, 2, 3])])
        self.assertEqual(status, 0)
        self.assertTrue(result['valid'])
        self.assertEqual(result['median_trial_p95_ms'], 3)
        self.assertIsNone(result['threshold_ms'])
        self.assertIsNone(result['pass'])

    def test_an_explicit_threshold_judges_only_complete_valid_trials(self):
        for maximum, expected in ((10, True), (10.1, False)):
            with self.subTest(maximum=maximum):
                status, result = run_summary('echo', [record([1, 2, maximum])], '--max-p95-ms', '10')
                self.assertEqual(status, 0 if expected else 1)
                self.assertEqual(result['pass'], expected)
                self.assertTrue(result['valid'])

    def test_no_trials_is_an_invalid_measurement(self):
        status, result = run_summary('echo', [])
        self.assertNotEqual(status, 0)
        self.assertFalse(result['valid'])
        self.assertFalse(result['pass'])


if __name__ == '__main__':
    unittest.main()
