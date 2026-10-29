#!/usr/bin/env python3
"""Run a separate fixed matrix using source-bound captures and one immutable image."""
import datetime
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

from capture import load_inputs


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def command_for(root, inputs, variant, out, name, n, runtime):
    return [
        'docker', 'run', '--rm', '--name', 'rb-batch10-' + name,
        '-v', variant['root'] + ':/wt:ro', '-v', str(root / 'harness') + ':/h:ro',
        '-v', variant['volume'] + ':/target:ro', '-v', str(out) + ':/out',
        inputs['image_id'], 'sh', '-c',
        f'test "$(sha256sum /target/release/rp-harness | cut -d " " -f 1)" = '
        f'"{variant["linux_binary_sha256"]}" && exec valgrind --tool=callgrind '
        f'--callgrind-out-file=/out/{name}.cg /target/release/rp-harness {n} {runtime} rp',
    ]


def main(root):
    inputs = load_inputs(root)
    out = root / 'outputs/callgrind-final'
    out.mkdir(exist_ok=False)
    manifest = {'order': list('ABBA') * 3, 'runs': [], 'inputs': inputs}

    def save():
        (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    def verify():
        for relative, expected in inputs['common'].items():
            if sha(root / relative) != expected:
                raise ValueError(f'common input changed: {relative}')
        for variant in inputs['variants'].values():
            for relative, expected in variant['source_hashes'].items():
                if sha(Path(variant['root']) / relative) != expected:
                    raise ValueError(f'source changed: {relative}')

    try:
        verify()
        selected = inputs['image_id']
        observed = subprocess.check_output(
            ['docker', 'image', 'inspect', selected, '--format', '{{.Id}}'], text=True).strip()
        if observed != selected:
            raise ValueError('frozen image identity is unavailable or mismatched')
        for runtime in ['ct', 'mt']:
            counts = dict(A=0, B=0)
            for position, label in enumerate(manifest['order'], 1):
                counts[label] += 1
                variant = inputs['variants'][label]
                for n in [1000, 10000]:
                    name = f'{runtime}-{position:02}-{label}{counts[label]}-{n}'
                    command = command_for(root, inputs, variant, out, name, n, runtime)
                    record = {'name': name, 'runtime': runtime, 'position': position,
                              'label': label, 'ordinal': counts[label], 'n': n, 'command': command,
                              'started': datetime.datetime.now(datetime.timezone.utc).isoformat()}
                    manifest['runs'].append(record)
                    save()
                    print('START', name, flush=True)
                    with (out / (name + '.out')).open('w') as stdout, (out / (name + '.err')).open('w') as stderr:
                        result = subprocess.run(command, stdout=stdout, stderr=stderr)
                    record.update(exit_code=result.returncode,
                                  ended=datetime.datetime.now(datetime.timezone.utc).isoformat())
                    save()
                    if result.returncode:
                        raise RuntimeError(f'{name} failed, no retry')
                    cg = (out / (name + '.cg')).read_text()
                    err = (out / (name + '.err')).read_text()
                    record['instructions'] = int(re.search(r'^summary: (\d+)', cg, re.M)[1])
                    counters = re.search(r'RESULT .* allocs=(\d+) bytes=(\d+) ', err)
                    record['allocs'], record['bytes'] = int(counters[1]), int(counters[2])
                    record['raw_hashes'] = {suffix: sha(out / (name + suffix)) for suffix in ['.cg', '.out', '.err']}
                    save()
                    print('END', name, record['instructions'], flush=True)
        verify()
        manifest['completed'] = True
        save()
    except BaseException as error:
        manifest['failure'] = repr(error)
        save()
        raise


if __name__ == '__main__':
    main(Path(sys.argv[1]).resolve())
