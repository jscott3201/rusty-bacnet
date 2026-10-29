#!/usr/bin/env python3
"""Challenge semantic evidence guards with isolated, rehashed corrupt data copies."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def reseal(directory, relative):
    path = directory / 'sha256.json'
    manifest = json.loads(path.read_text())
    manifest[relative] = hashlib.sha256((directory / relative).read_bytes()).hexdigest()
    path.write_text(json.dumps(manifest, indent=2) + '\n')


def main():
    source = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parent
    subprocess.run([sys.executable, str(source / 'reconstruct.py'), str(source)], check=True,
                   stdout=subprocess.DEVNULL)
    results = []
    for case, diagnostic in [('metric', 'Callgrind metric differs'),
                             ('source_identity', 'wrong source identity'),
                             ('missing_sample', 'missing Criterion sample')]:
        with tempfile.TemporaryDirectory(prefix='1978-integrity-') as temporary:
            directory = Path(temporary) / 'evidence'
            shutil.copytree(source, directory)
            if case == 'metric':
                path = directory / 'runs.json'
                data = json.loads(path.read_text())
                data['callgrind'][0]['instructions'] += 1
                path.write_text(json.dumps(data, indent=2) + '\n')
                reseal(directory, 'runs.json')
            elif case == 'source_identity':
                path = directory / 'inputs.json'
                data = json.loads(path.read_text())
                data['variants']['A']['commit'] = '0' * 40
                path.write_text(json.dumps(data, indent=2) + '\n')
                reseal(directory, 'inputs.json')
            else:
                path = directory / 'criterion.jsonl'
                data = [json.loads(line) for line in path.read_text().splitlines()]
                sample = json.loads(data[0]['sample'])
                sample['times'].pop()
                sample['iters'].pop()
                data[0]['sample'] = json.dumps(sample)
                data[0]['sample_sha256'] = hashlib.sha256(data[0]['sample'].encode()).hexdigest()
                path.write_text(''.join(json.dumps(row) + '\n' for row in data))
                reseal(directory, 'criterion.jsonl')
            observed = subprocess.run([sys.executable, str(directory / 'reconstruct.py'), str(directory)],
                                      capture_output=True, text=True)
            if observed.returncode == 0 or diagnostic not in observed.stderr:
                raise RuntimeError(f'guard did not reject {case}: {observed.stderr}')
            results.append({'case': case, 'outer_hashes_resealed': True, 'rejected': True,
                            'exit_code': observed.returncode,
                            'diagnostic': observed.stderr.splitlines()[-1]})
    subprocess.run([sys.executable, str(source / 'reconstruct.py'), str(source)], check=True,
                   stdout=subprocess.DEVNULL)
    print(json.dumps({'untampered_before_and_after': 'passed', 'cases': results}, indent=2))


if __name__ == '__main__':
    main()
