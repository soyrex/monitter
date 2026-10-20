#!/usr/bin/env python3
"""Focused selector regression checks using copied metadata, never app launches."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('selector', Path(__file__).with_name('gpui-fast-select.py'))
selector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selector)
ROOT = Path(__file__).resolve().parents[1] / 'artifacts/gpui-fast-poc/guarded-five-pair'


class SelectorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        source = ROOT / 'reference/idle-01'
        self.run = self.root / 'reference/idle-01'
        self.run.mkdir(parents=True)
        for name in selector.REQUIRED_FILES:
            (self.run / name).write_text((source / name).read_text())
        (self.run.parent / 'machine-conditions.json').write_text((source.parent / 'machine-conditions.json').read_text())
        self.candidate = selector.run_candidates([self.root])[0]
        measured = json.loads((self.run / 'metrics.json').read_text())['measuredSeconds']
        self.mutate('metrics.json', lambda d: d.update(windowConditions=dict(
            schemaVersion=1, durationSeconds=measured, activeSeconds=measured,
            visibleSeconds=measured, fullyActiveVisible=True, workloadUpdateCallbacks=0,
            transitions=[dict(elapsedSeconds=0, active=True, visible=True)])))

    def mutate(self, name, change):
        path = self.run / name
        data = json.loads(path.read_text())
        change(data)
        path.write_text(json.dumps(data))

    def inspect(self):
        return selector.inspect_run(self.candidate, 2.5)

    def test_metadata_with_complete_diagnostic_eligible(self):
        self.assertTrue(self.inspect()['eligible'])

    def test_rejections(self):
        changes = [
            ('collection-receipt.json', lambda d: d.pop('samplerExitCode')),
            ('metrics.json', lambda d: d.update(pid=123456)),
            ('metrics.json', lambda d: d.update(warmupSeconds=99)),
            ('metrics.json', lambda d: d.update(requestedSeconds=99)),
            ('metrics.json', lambda d: d.update(profilerEnabled=False)),
            ('metrics.json', lambda d: d['inputToPlatformSubmitMs'].update(count=1)),
            ('session.json', lambda d: d.update(sourceSha256=None)),
            ('session.json', lambda d: d.update(sha256='0'*64)),
            ('build.json', lambda d: d.update(lanAssets=False)),
            ('environment-guard.json', lambda d: d.update(compilerOverlap=True)),
        ]
        for name, change in changes:
            with self.subTest(name=name, change=change):
                original = (self.run / name).read_text()
                self.mutate(name, change)
                self.assertFalse(self.inspect()['eligible'])
                (self.run / name).write_text(original)

    def test_window_conditions_rejections(self):
        changes = [
            lambda d: d.pop('windowConditions'),
            lambda d: d['windowConditions'].update(schemaVersion=0),
            lambda d: d['windowConditions'].update(fullyActiveVisible=False),
            lambda d: d['windowConditions'].update(activeSeconds=0),
            lambda d: d['windowConditions'].update(visibleSeconds=0),
            lambda d: d['windowConditions'].update(durationSeconds=20),
            lambda d: d['windowConditions'].update(transitions=[]),
            lambda d: d['windowConditions']['transitions'].append(dict(elapsedSeconds=1, active=False, visible=True)),
            lambda d: d['windowConditions']['transitions'].append(dict(elapsedSeconds=1, active=True, visible=False)),
            lambda d: d['windowConditions']['transitions'].append(dict(elapsedSeconds=999, active=True, visible=True)),
            lambda d: d['drawMs'].update(count=0),
        ]
        for change in changes:
            original = (self.run / 'metrics.json').read_text()
            self.mutate('metrics.json', change)
            self.assertFalse(self.inspect()['eligible'])
            (self.run / 'metrics.json').write_text(original)

    def test_autonomous_requires_progress_without_invented_threshold(self):
        metrics = json.loads((self.run / 'metrics.json').read_text())
        for scenario in ('streaming-body', 'pipeline-updates'):
            self.assertIn('autonomous workload has no measured update callbacks', selector.inspect_window_conditions(metrics, scenario))
            metrics['windowConditions']['workloadUpdateCallbacks'] = 1
            self.assertEqual([], selector.inspect_window_conditions(metrics, scenario))
            metrics['windowConditions']['workloadUpdateCallbacks'] = 0

    def test_mixed_builds_refused(self):
        first = self.inspect()
        second = copy.deepcopy(first)
        second['sourceBuild']['desktopSha256'] = '0'*64
        original = selector.inspect_run
        selector.inspect_run = lambda candidate, gap: candidate
        try:
            with self.assertRaises(selector.SelectionError):
                selector.make_selection([first, second], 5)
        finally:
            selector.inspect_run = original

    def test_mixed_settings_refused(self):
        first = self.inspect()
        second = copy.deepcopy(first)
        second['settings']['motionMode'] = 'on'
        original = selector.inspect_run
        selector.inspect_run = lambda candidate, gap: candidate
        try:
            with self.assertRaises(selector.SelectionError):
                selector.make_selection([first, second], 5)
        finally:
            selector.inspect_run = original

    def test_feature_mismatch_pair_excluded(self):
        first = self.inspect()
        second = copy.deepcopy(first)
        second['side'] = 'treatment'
        second['sourceBuild']['lanAssets'] = False
        original = selector.inspect_run
        selector.inspect_run = lambda candidate, gap: candidate
        try:
            _, pairs, _, enough = selector.make_selection([first, second], 1)
            self.assertFalse(enough)
            self.assertIn('reference and treatment build feature lanAssets differs', pairs[0]['exclusionReasons'])
        finally:
            selector.inspect_run = original


if __name__ == '__main__':
    unittest.main()
