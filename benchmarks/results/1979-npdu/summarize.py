#!/usr/bin/env python3
"""Verify the distributed NPDU observations and recompute all invocation summaries."""
import csv
import hashlib
import io
import json
import math
from pathlib import Path
import statistics

HERE = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    provenance = json.loads((HERE / 'provenance.json').read_text())
    for name, expected in provenance['distributed_sha256'].items():
        require(hashlib.sha256((HERE / name).read_bytes()).hexdigest() == expected, 'hash mismatch: ' + name)
    manifest = json.loads((HERE / 'runs.json').read_text())
    hashes = json.loads((HERE / 'raw-sha256.json').read_text())
    require(manifest.get('completed') and not manifest.get('failure'), 'incomplete matrix')
    require(manifest['order'] == list('PQRRQP') * 3 and len(manifest['runs']) == 18, 'wrong run order')
    records = [json.loads(line) for line in (HERE / 'criterion.jsonl').read_text().splitlines()]
    require(len(records) == 54, 'wrong observation count')
    rows = []
    counts = dict.fromkeys('PQR', 0)
    for position, run in enumerate(manifest['runs'], 1):
        label = manifest['order'][position - 1]
        counts[label] += 1
        require((run['position'], run['label'], run['ordinal']) == (position, label, counts[label]), 'run identity')
        require(run['exit_code'] == 0 and run['criterion_copied'], 'failed run')
        for workload in manifest['workloads']:
            record = records[len(rows)]
            require((record['position'], record['variant'], record['ordinal'], record['workload']) ==
                    (position, label, counts[label], workload), 'observation identity')
            parsed = {}
            for kind in ['estimates', 'sample']:
                raw = record[kind + '_raw'].encode()
                relative = f"criterion/{run['name']}/{workload}/{kind}.json"
                require(hashlib.sha256(raw).hexdigest() == hashes[relative], 'raw hash mismatch: ' + relative)
                parsed[kind] = json.loads(raw)
            sample = parsed['sample']
            require(len(sample['iters']) == len(sample['times']) == 100, 'sample count')
            require(all(math.isfinite(x) and x > 0 for x in sample['iters'] + sample['times']), 'invalid sample')
            e = parsed['estimates']; median = e['median']
            rows.append({'position': position, 'variant': label, 'ordinal': counts[label], 'workload': workload,
                         'median_ns': median['point_estimate'],
                         'median_ci_low_ns': median['confidence_interval']['lower_bound'],
                         'median_ci_high_ns': median['confidence_interval']['upper_bound'],
                         'mean_ns': e['mean']['point_estimate'],
                         'slope_ns': e['slope']['point_estimate'] if e.get('slope') else ''})
    csv_text = io.StringIO(newline='')
    writer = csv.DictWriter(csv_text, fieldnames=list(rows[0]), lineterminator='\n')
    writer.writeheader(); writer.writerows(rows)
    require(csv_text.getvalue().encode() == (HERE / 'encoding.csv').read_bytes(), 'derived CSV mismatch')
    summary = {}
    for workload in manifest['workloads']:
        data = {label: [r['median_ns'] for r in rows if r['workload'] == workload and r['variant'] == label] for label in 'PQR'}
        require(all(len(v) == 6 for v in data.values()), 'repeat count')
        stats = {label: {'median_ns': statistics.median(v), 'min_ns': min(v), 'max_ns': max(v)} for label, v in data.items()}
        ratios = {}
        for numerator, denominator in [('Q', 'P'), ('R', 'Q'), ('R', 'P')]:
            values = [a / b for a, b in zip(data[numerator], data[denominator])]
            ratios[numerator + '/' + denominator] = {'median': statistics.median(values), 'min': min(values), 'max': max(values), 'values': values}
        summary[workload] = {'invocation_medians': data, 'summary': stats, 'paired_ratios': ratios}
    require(summary == json.loads((HERE / 'summary.json').read_text()), 'summary mismatch')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
