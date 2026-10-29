#!/usr/bin/env python3
"""Verify distributed observations and reconstruct the fixed decision without timing."""
import argparse
import gzip
import hashlib
import itertools
import json
import math
from pathlib import Path
import re
import statistics

A = '2d82e9a949bb57702f4672705a46b19345bb3d59'
B = 'b1f6845eca586bc03b98c4445e82b54fd49fdc98'
WORKLOADS = ['bip_read_property_latency', 'bip_write_property_latency',
             'bip_rpm_10_objects_latency']
ORDER = list('ABBA') * 3


def digest(data):
    return hashlib.sha256(data).hexdigest()


def describe(values):
    return {'values': values, 'median': statistics.median(values),
            'range': [min(values), max(values)]}


def percentile(values, q):
    ordered = sorted(values)
    position = (len(ordered) - 1) * q
    low = int(position)
    high = min(low + 1, len(ordered) - 1)
    return ordered[low] + (ordered[high] - ordered[low]) * (position - low)


def reconstruct(root):
    manifest = json.loads((root / 'sha256.json').read_text())
    for name, expected in manifest.items():
        if digest((root / name).read_bytes()) != expected:
            raise ValueError(f'distributed hash mismatch: {name}')
    provenance = json.loads((root / 'provenance.json').read_text())
    patch = gzip.decompress((root / 'candidate.patch.gz').read_bytes())
    if digest(patch) != provenance['candidate_patch_sha256']:
        raise ValueError('decompressed candidate patch differs from original')
    inputs = json.loads((root / 'inputs.json').read_text())
    for label, expected in [('A', A), ('B', B)]:
        if inputs['variants'][label]['commit'] != expected:
            raise ValueError(f'wrong source identity: {label}')
    for relative, expected in inputs['common'].items():
        if digest((root / 'common' / relative).read_bytes()) != expected:
            raise ValueError(f'common input drift: {relative}')
    runs = json.loads((root / 'runs.json').read_text())
    cg = runs['callgrind']
    native = runs['native']
    if len(cg) != 48 or len(native) != 12:
        raise ValueError('incomplete matrix')
    for mode in ['ct', 'mt']:
        rows = [r for r in cg if r['runtime'] == mode]
        if [(r['label'], r['n']) for r in rows] != [(v, n) for v in ORDER for n in [1000, 10000]]:
            raise ValueError('wrong Callgrind order or sizes')
        counts = dict(A=0, B=0)
        for offset in range(0, len(rows), 2):
            counts[rows[offset]['label']] += 1
            for row in rows[offset:offset + 2]:
                if row['ordinal'] != counts[row['label']]:
                    raise ValueError('wrong Callgrind pairing')
    if [r['label'] for r in native] != ORDER:
        raise ValueError('wrong native order')
    for row in cg + native:
        if row['exit_code'] != 0:
            raise ValueError('unsuccessful invocation')
        for stream in ['stdout', 'stderr']:
            if digest(row[stream].encode()) != row[stream + '_sha256']:
                raise ValueError('original log drift')
    for row in cg:
        observed = re.search(r'Collected\s*:\s*(\d+)', row['stderr'])
        counters = re.search(r'RESULT n=(\d+) mt=(\w+) wp=false workers=(\d+) allocs=(\d+) bytes=(\d+) ', row['stderr'])
        if not observed or int(observed[1]) != row['instructions']:
            raise ValueError('Callgrind metric differs from original stderr')
        if not counters or (int(counters[1]), counters[2], int(counters[3]), int(counters[4]), int(counters[5])) != (row['n'], str(row['runtime'] == 'mt').lower(), 2 if row['runtime'] == 'mt' else 1, row['allocs'], row['bytes']):
            raise ValueError('allocation or runtime identity differs from original stderr')
    observations = [json.loads(line) for line in (root / 'criterion.jsonl').read_text().splitlines()]
    expected_keys = {(r['name'], w) for r in native for w in WORKLOADS}
    if len(observations) != 36 or {(r['run'], r['workload']) for r in observations} != expected_keys:
        raise ValueError('wrong Criterion observation identities')
    values = {w: {'A': [], 'B': []} for w in WORKLOADS}
    counts = dict(A=0, B=0)
    for run in native:
        counts[run['label']] += 1
        if run['ordinal'] != counts[run['label']] or run['TOKIO_WORKER_THREADS'] != 2:
            raise ValueError('wrong native pairing or worker count')
        for workload in WORKLOADS:
            row = next(o for o in observations if o['run'] == run['name'] and o['workload'] == workload)
            for name in ['estimates', 'sample']:
                if digest(row[name].encode()) != row[name + '_sha256']:
                    raise ValueError('original Criterion bytes changed')
            sample = json.loads(row['sample'])
            if len(sample['iters']) != 100 or len(sample['times']) != 100:
                raise ValueError('missing Criterion sample')
            if not all(math.isfinite(v) and v > 0 for v in sample['iters'] + sample['times']):
                raise ValueError('invalid Criterion sample')
            estimate = json.loads(row['estimates'])['median']['point_estimate']
            if not math.isfinite(estimate) or estimate <= 0:
                raise ValueError('invalid Criterion median')
            raw_median = statistics.median([t / i for t, i in zip(sample['times'], sample['iters'])])
            if not math.isclose(raw_median, estimate, rel_tol=1e-12, abs_tol=1e-9):
                raise ValueError('Criterion median differs from raw samples')
            values[workload][run['label']].append(estimate)
    result = {}
    for mode in ['ct', 'mt']:
        summary, slopes, rates = {}, {}, {}
        for label in ['A', 'B']:
            def by(i, n):
                return next(r for r in cg if r['runtime'] == mode and r['label'] == label and r['ordinal'] == i and r['n'] == n)
            slopes[label] = [(by(i, 10000)['instructions'] - by(i, 1000)['instructions']) / 9000 for i in range(1, 7)]
            rates[label] = {name: [by(i, 10000)[name] / 10000 for i in range(1, 7)] for name in ['allocs', 'bytes']}
            summary[label] = {'instructions_per_request': describe(slopes[label]),
                              **{name + '_per_request': describe(v) for name, v in rates[label].items()},
                              'allocation_slopes': {name: describe([(by(i, 10000)[name] - by(i, 1000)[name]) / 9000 for i in range(1, 7)]) for name in ['allocs', 'bytes']}}
        gains = [a - b for a, b in zip(slopes['A'], slopes['B'])]
        drift = max(slopes['A']) - min(slopes['A'])
        gain = statistics.median(slopes['A']) - statistics.median(slopes['B'])
        summary.update(paired_instruction_savings=describe(gains), median_instruction_savings=gain,
                       repeated_A_drift=drift, all_six_B_lower=all(v > 0 for v in gains),
                       gain_exceeds_A_drift=gain > drift,
                       allocations_nonincreasing=all(statistics.median(rates['B'][n]) <= statistics.median(rates['A'][n]) for n in ['allocs', 'bytes']))
        summary['accepted'] = summary['all_six_B_lower'] and summary['gain_exceeds_A_drift'] and summary['allocations_nonincreasing']
        result[mode] = summary
    result['native'] = {}
    for workload, v in values.items():
        bootstrap = []
        for indices in itertools.product(range(6), repeat=6):
            a = statistics.median([v['A'][i] for i in indices])
            b = statistics.median([v['B'][i] for i in indices])
            bootstrap.append(b / a - 1)
        ci = [percentile(bootstrap, .025), percentile(bootstrap, .975)]
        result['native'][workload] = {'unit': 'ns', 'A': describe(v['A']), 'B': describe(v['B']),
            'paired_difference': describe([b - a for a, b in zip(v['A'], v['B'])]),
            'paired_slowdown_fraction': describe([b / a - 1 for a, b in zip(v['A'], v['B'])]),
            'aggregate_slowdown_fraction': statistics.median(v['B']) / statistics.median(v['A']) - 1,
            'paired_bootstrap95_slowdown_fraction': ci,
            'observed_median_nonincreasing': statistics.median(v['B']) <= statistics.median(v['A']),
            'upper_bound_at_most_2percent': ci[1] <= .02,
            'accepted': statistics.median(v['B']) <= statistics.median(v['A']) and ci[1] <= .02}
    result['accepted'] = result['ct']['accepted'] and result['mt']['accepted'] and all(v['accepted'] for v in result['native'].values())
    expected = json.loads((root / 'summary.json').read_text())
    if result != expected:
        raise ValueError('reconstructed decision differs from frozen analysis')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    result = reconstruct(args.directory)
    print('Verified 48 Callgrind runs, 12 native invocations, 36 estimates and 3600 samples.')
    print('Overall accepted:', result['accepted'])


if __name__ == '__main__':
    main()
