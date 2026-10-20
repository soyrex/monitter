#!/usr/bin/env python3
"""Run predeclared disconnected paired workloads; never installs an application."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def compiler_processes():
    result = subprocess.run(['/bin/ps', '-axo', 'pid=,stat=,comm='],
                            capture_output=True, text=True, check=True)
    names = {'cargo', 'rustc', 'clang', 'clang++', 'cc', 'ld', 'ninja', 'make', 'swift-frontend'}
    found = []
    for line in result.stdout.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) == 3 and not parts[1].startswith('Z') and Path(parts[2]).name in names:
            found.append({'pid': int(parts[0]), 'name': Path(parts[2]).name})
    return found


def diagnostic_exclusions(output, scenario):
    """Use the selector's exact diagnostic rules before scheduling another run."""
    spec = importlib.util.spec_from_file_location('gpui_fast_selector', Path(__file__).with_name('gpui-fast-select.py'))
    selector = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(selector)
    try:
        metrics = json.loads((Path(output) / 'metrics.json').read_text())
        if not isinstance(metrics, dict):
            return ['metrics.json is not an object']
        return selector.inspect_window_conditions(metrics, scenario)
    except (OSError, ValueError) as error:
        return [f'cannot read complete metrics diagnostic: {error}']


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--reference', type=Path, required=True)
    p.add_argument('--treatment', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--repetitions', type=int, default=3)
    p.add_argument('--first-repetition', type=int, default=1)
    p.add_argument('--scenarios', nargs='+', choices=('streaming-body', 'pipeline-updates', 'idle'),
                   default=('streaming-body', 'pipeline-updates', 'idle'))
    a = p.parse_args()
    if any(os.environ.get(name) == '0' for name in ('GPUI_VIEW_RETENTION', 'GPUI_SCROLL_LAYERS')):
        p.error('comparison requires default Fast retention and scroll layers')
    if a.repetitions < 1 or a.first_repetition < 1:
        p.error('positive repetition counts and indices are required; analysis enforces total coverage')
    root = a.output.resolve()
    for side in ('reference', 'treatment'):
        if not (root / side / 'machine-conditions.json').is_file():
            p.error(f'{side}/machine-conditions.json is required')
    schedule = []
    for repetition in range(a.first_repetition, a.first_repetition + a.repetitions):
        for scenario in a.scenarios:
            for side in (('reference', 'treatment') if repetition % 2 else ('treatment', 'reference')):
                output = root / side / f'{scenario}-{repetition:02d}'
                if output.exists():
                    p.error(f'refusing to overwrite {output}')
                schedule.append({'side': side, 'scenario': scenario, 'repetition': repetition,
                                 'binary': str(getattr(a, side).resolve()), 'output': str(output)})
    plan = root / 'benchmark-schedule.json'
    if plan.exists():
        p.error('schedule already exists; use a fresh output root')
    plan.write_text(json.dumps({'schemaVersion': 1, 'schedule': schedule}, indent=2) + '\n')
    runner = Path(__file__).with_name('gpui-fast-session.py')
    receipts = []
    for run in schedule:
        active = compiler_processes()
        if active:
            raise SystemExit(f'compiler activity before launch; no fixture started: {active}')
        print(json.dumps({'starting': run}), flush=True)
        result = subprocess.Popen([sys.executable, str(runner), run['binary'], '--output', run['output'],
                                 '--scenario', run['scenario'], '--rows', '10000', '--motion-mode', 'off',
                                 '--warmup', '15', '--seconds', '60'])
        observations = []
        while result.poll() is None:
            observations.append({'unixMs': int(time.time() * 1000), 'compilers': compiler_processes()})
            time.sleep(1)
        guard = {'schemaVersion': 1, 'observations': observations,
                 'compilerOverlap': any(x['compilers'] for x in observations),
                 'scope': 'One-second compiler process sampling; does not prove absence of other host load or input.'}
        Path(run['output'], 'environment-guard.json').write_text(json.dumps(guard, indent=2) + '\n')
        diagnostic_reasons = diagnostic_exclusions(run['output'], run['scenario'])
        receipts.append({**run, 'exitCode': result.returncode, 'compilerOverlap': guard['compilerOverlap'],
                         'windowConditionsEligible': not diagnostic_reasons,
                         'windowConditionsExclusionReasons': diagnostic_reasons,
                         'finishedAtUnixMs': int(time.time() * 1000)})
        (root / 'benchmark-receipts.json').write_text(json.dumps(receipts, indent=2) + '\n')
        if result.returncode:
            raise SystemExit(result.returncode)
        if guard['compilerOverlap']:
            raise SystemExit('compiler overlap detected: preserve this run as excluded and stop scheduling')
        if diagnostic_reasons:
            raise SystemExit('window/workload diagnostic rejected: preserve this run and stop scheduling: '
                             + '; '.join(diagnostic_reasons))
        time.sleep(5)


if __name__ == '__main__':
    main()
