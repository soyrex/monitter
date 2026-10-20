#!/usr/bin/env python3
"""Evaluate descriptive noise gates and exact selected-pair differences."""
import argparse
import json
import math
from pathlib import Path
import statistics

METRICS = ('drawMs.p50', 'drawMs.p95', 'drawMs.p99',
           'osSampledProcessCpuSeconds', 'osSampledProcessCpuSecondsPerWallSecond', 'osSamplePeakPhysicalFootprintBytes',
           'osSamplePackageIdleWakeupDelta', 'osSampleInterruptWakeupDelta', 'chatChromeRenders')


def numeric(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def canonical(path):
    path = Path(path)
    source = path / 'source-run'
    return str((source if source.is_symlink() else path).resolve())


def summary(values):
    median = statistics.median(values)
    return dict(n=len(values), median=median, min=min(values), max=max(values),
                mad=statistics.median(abs(value-median) for value in values))


def evaluate_metric(pairs, name):
    differences = []
    for repetition, ref, treatment in pairs:
        left, right = ref['values'].get(name), treatment['values'].get(name)
        available = numeric(left) and numeric(right)
        differences.append(dict(repetition=repetition, available=available,
                                reference=left if numeric(left) else None,
                                treatment=right if numeric(right) else None,
                                deltaTreatmentMinusReference=right-left if available else None,
                                deltaPercent=(right-left)/abs(left)*100 if available and left else None))
    valid = [item for item in differences if item['available']]
    result = dict(available=bool(valid), complete=len(valid)==len(pairs),
                  pairsByRepetition={str(item['repetition']): item for item in differences},
                  status='unavailable', noiseGatePassed=False)
    if not valid:
        return result
    left = summary([item['reference'] for item in valid])
    right = summary([item['treatment'] for item in valid])
    delta = right['median']-left['median']
    percent = delta/abs(left['median'])*100 if left['median'] else None
    exceeds_percent = abs(percent)>5 if percent is not None else False
    exceeds_mad = abs(delta)>max(left['mad'], right['mad'])
    direction = 'decrease' if delta<0 else 'increase' if delta>0 else 'unchanged'
    consistent = all(item['deltaTreatmentMinusReference']<0 for item in valid) if delta<0 else (
        all(item['deltaTreatmentMinusReference']>0 for item in valid) if delta>0 else
        all(item['deltaTreatmentMinusReference']==0 for item in valid))
    gate = result['complete'] and len(valid)>=3 and exceeds_percent and exceeds_mad
    result.update(reference=left, treatment=right, deltaTreatmentMinusReference=delta,
                  deltaPercent=percent, direction=direction, consistentPairDirection=consistent,
                  rangesOverlap=max(left['min'],right['min'])<=min(left['max'],right['max']),
                  exceedsFivePercent=exceeds_percent, exceedsLargerMad=exceeds_mad,
                  largerMad=max(left['mad'],right['mad']), noiseGatePassed=gate,
                  status=('incomplete_values' if not result['complete'] else
                          'descriptive_only_fewer_than_3_pairs' if len(valid)<3 else
                          'passes_operational_noise_gate' if gate else 'inconclusive_noise_gate'))
    return result


def evaluate(comparison, manifest):
    if manifest.get('status') != 'ready_for_descriptive_comparison':
        raise ValueError('selection manifest is not ready for descriptive comparison')
    selected = manifest.get('selectedPairs', {})
    expected = {}
    for scenario, pairs in selected.items():
        for pair in pairs:
            repetition = pair['repetition']
            if pair.get('scenario') != scenario or pair.get('eligible') is not True:
                raise ValueError('selected pair is ineligible or scenario mismatched')
            for side in ('reference', 'treatment'):
                path = canonical(pair[side+'Path'])
                if path in expected:
                    raise ValueError('duplicate selected run path')
                expected[path] = (scenario, repetition, side)
    if not expected:
        raise ValueError('selection manifest contains no selected pairs')
    seen = set()
    groups = []
    for group in comparison['groups']:
        scenario = group['settings']['scenario']
        mapped = {}
        for side in ('reference','treatment'):
            for run in group[side+'Runs']:
                path = canonical(run['path'])
                identity = expected.get(path)
                if identity is None or identity[0] != scenario or identity[2] != side or path in seen:
                    raise ValueError('comparison contains unselected, mismatched, or duplicate run: '+path)
                seen.add(path)
                key = identity[1]
                mapped.setdefault(key,{})[side] = run
        if not mapped or any(set(sides) != {'reference','treatment'} for sides in mapped.values()):
            raise ValueError('comparison group contains incomplete selected pairs')
        pairs = [(rep, sides['reference'], sides['treatment']) for rep,sides in sorted(mapped.items())]
        groups.append(dict(settings=group['settings'], pairRepetitions=[p[0] for p in pairs],
                           metrics={name:evaluate_metric(pairs,name) for name in METRICS}))
    if seen != set(expected):
        raise ValueError('comparison omits selected runs')
    return dict(schemaVersion=1, claimLimit='Descriptive operational screening only; no statistical significance, causal, energy, or adoption claim.',
                gateRule='Complete values in at least 3 pairs; absolute median shift strictly exceeds 5 percent and the larger within-build MAD. Zero reference median cannot pass the percentage gate.',
                pairing='Exact selected source paths and repetition IDs; never array order.',
                caveats='Overlap and pair direction are reported separately from the median noise gate. Wakeup counters remain separate. Sampled footprint is not exact peak memory.', groups=groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('comparison',type=Path)
    parser.add_argument('manifest',type=Path)
    args=parser.parse_args()
    try:
        result=evaluate(json.loads(args.comparison.read_text()),json.loads(args.manifest.read_text()))
    except (OSError,ValueError,KeyError,TypeError) as error:
        parser.error(str(error))
    print(json.dumps(result,indent=2,sort_keys=True,allow_nan=False))


if __name__=='__main__':
    main()
