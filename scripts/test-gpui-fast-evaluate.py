#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('evaluate',Path(__file__).with_name('gpui-fast-evaluate.py'))
e=importlib.util.module_from_spec(spec)
spec.loader.exec_module(e)


class EvaluationTests(unittest.TestCase):
    def fixture(self):
        pairs=[]
        refs=[]
        treatments=[]
        for rep in (2,7,11):
            pair=dict(scenario='idle',repetition=rep,eligible=True,
                      referencePath=f'/synthetic/reference/idle-{rep}',treatmentPath=f'/synthetic/treatment/idle-{rep}')
            pairs.append(pair)
            refs.append(dict(path=pair['referencePath'],values={'drawMs.p50':100}))
            treatments.append(dict(path=pair['treatmentPath'],values={'drawMs.p50':80}))
        comparison=dict(groups=[dict(settings=dict(scenario='idle'),referenceRuns=refs[::-1],treatmentRuns=treatments)])
        manifest=dict(status='ready_for_descriptive_comparison',selectedPairs={'idle':pairs})
        return comparison,manifest

    def test_noncontiguous_exact_pairing(self):
        c,m=self.fixture()
        metric=e.evaluate(c,m)['groups'][0]['metrics']['drawMs.p50']
        self.assertEqual(list(metric['pairsByRepetition']),['2','7','11'])
        self.assertTrue(metric['noiseGatePassed'])
        self.assertTrue(metric['consistentPairDirection'])
        self.assertFalse(metric['rangesOverlap'])
        self.assertEqual(metric['pairsByRepetition']['7']['deltaTreatmentMinusReference'],-20)

    def test_missing_values_do_not_pass(self):
        c,m=self.fixture()
        c['groups'][0]['treatmentRuns'][0]['values']['drawMs.p50']=None
        metrics=e.evaluate(c,m)['groups'][0]['metrics']
        self.assertFalse(metrics['drawMs.p50']['complete'])
        self.assertFalse(metrics['drawMs.p50']['noiseGatePassed'])
        self.assertIsNone(metrics['drawMs.p50']['pairsByRepetition']['2']['deltaTreatmentMinusReference'])
        self.assertFalse(metrics['drawMs.p99']['available'])

    def test_omitted_or_unknown_run_refused(self):
        for change in ('missing','unknown'):
            c,m=self.fixture()
            if change=='missing':c['groups'][0]['referenceRuns'].pop()
            else:c['groups'][0]['referenceRuns'][0]['path']='/unselected'
            with self.assertRaises(ValueError):e.evaluate(c,m)

    def test_strict_gates_and_zero_baseline(self):
        pairs=[(r,dict(values={'m':100}),dict(values={'m':105})) for r in (2,7,11)]
        self.assertFalse(e.evaluate_metric(pairs,'m')['noiseGatePassed'])
        pairs=[(r,dict(values={'m':0}),dict(values={'m':10})) for r in (2,7,11)]
        metric=e.evaluate_metric(pairs,'m')
        self.assertIsNone(metric['deltaPercent'])
        self.assertFalse(metric['noiseGatePassed'])


if __name__=='__main__':unittest.main()
