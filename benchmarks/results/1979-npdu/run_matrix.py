#!/usr/bin/env python3
"""Run the fixed P/Q/R NPDU attribution matrix from frozen binaries. No builds/retries."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inputs', type=Path)
    parser.add_argument('output', type=Path, help='must not already exist')
    args = parser.parse_args()
    inputs = json.loads(args.inputs.read_text())
    args.output.mkdir(parents=True, exist_ok=False)
    shutil.copy2(args.inputs, args.output / 'inputs.json')
    order = list('PQRRQP') * 3
    workloads = ['decode_npdu_100b_payload',
                 'npdu_diagnostic_borrowed_decode_100b_payload',
                 'npdu_diagnostic_shared_bytes_clone_drop_102b']
    manifest = {'started': now(), 'inputs_sha256': sha(args.inputs),
                'runner_sha256': sha(__file__), 'order': order,
                'workloads': workloads, 'runs': []}

    def save():
        (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    def verify():
        for variant in inputs['variants'].values():
            root = Path(variant['root'])
            for relative, expected in variant['source_hashes'].items():
                if sha(root / relative) != expected:
                    raise RuntimeError(f'source changed: {root / relative}')
            if sha(variant['binary']['path']) != variant['binary']['sha256']:
                raise RuntimeError(f'binary changed: {variant["binary"]["path"]}')

    try:
        verify()
        counts = {label: 0 for label in 'PQR'}
        for position, label in enumerate(order, 1):
            variant = inputs['variants'][label]
            root = Path(variant['root'])
            if shutil.disk_usage(root).free < 50 * 1024**3:
                raise RuntimeError('less than 50 GiB free')
            binary = variant['binary']
            if sha(binary['path']) != binary['sha256']:
                raise RuntimeError(f'binary changed before run {position}')
            counts[label] += 1
            name = f'{position:02}-{label}-{counts[label]:02}'
            baseline = '1979-npdu-' + name
            for workload in workloads:
                if (root / 'target/criterion' / workload / baseline).exists():
                    raise RuntimeError(f'baseline already exists: {baseline}')
            env = os.environ.copy()
            env['CARGO_TARGET_DIR'] = str(root / 'target')
            command = [binary['path'], '--bench', '^(' + '|'.join(workloads) + ')$',
                       '--sample-size', '100', '--warm-up-time', '3',
                       '--measurement-time', '5', '--noplot', '--save-baseline', baseline]
            record = {'position': position, 'label': label, 'ordinal': counts[label],
                      'name': name, 'started': now(), 'cwd': str(root), 'command': command,
                      'CARGO_TARGET_DIR': env['CARGO_TARGET_DIR'], 'binary_sha256': sha(binary['path']),
                      'load_before': os.getloadavg(), 'free_bytes_before': shutil.disk_usage(root).free,
                      'power_before': subprocess.check_output(['pmset', '-g', 'batt'], text=True).strip()}
            manifest['runs'].append(record)
            save()
            print('START', name, flush=True)
            with (args.output / (name + '.out')).open('w') as stdout, (args.output / (name + '.err')).open('w') as stderr:
                result = subprocess.run(command, cwd=root, env=env, stdout=stdout, stderr=stderr)
            record.update(ended=now(), exit_code=result.returncode, load_after=os.getloadavg())
            for suffix in ['out', 'err']:
                record[suffix + '_sha256'] = sha(args.output / (name + '.' + suffix))
            save()
            if result.returncode:
                raise RuntimeError(f'{name} failed; matrix stopped, no retry')
            for workload in workloads:
                source = root / 'target/criterion' / workload / baseline
                destination = args.output / 'criterion' / name / workload
                shutil.copytree(source, destination)
                for required in ['estimates.json', 'sample.json']:
                    if not (destination / required).is_file():
                        raise RuntimeError(f'missing {required} for {name} {workload}')
                sample = json.loads((destination / 'sample.json').read_text())
                if len(sample['iters']) != 100 or len(sample['times']) != 100:
                    raise RuntimeError(f'wrong sample count for {name} {workload}')
                if any(value <= 0 for value in sample['iters'] + sample['times']):
                    raise RuntimeError(f'invalid sample for {name} {workload}')
            record['criterion_copied'] = True
            save()
            print('END', name, flush=True)
        verify()
        manifest['completed'] = now()
    except BaseException as error:
        manifest['failure'] = repr(error)
        manifest['stopped'] = now()
        save()
        raise
    save()


if __name__ == '__main__':
    main()
