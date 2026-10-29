#!/usr/bin/env python3
"""Freeze one qualified historical benchmark binary and its source hashes for run_matrix.py."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('variant', choices=list('PQR'))
    parser.add_argument('root', type=Path)
    parser.add_argument('cargo_json', type=Path)
    parser.add_argument('output', type=Path, help='shared new directory for P/Q/R captures')
    args = parser.parse_args()
    root = args.root.resolve()
    subprocess.run(['python3', str(HERE / 'prepare.py'), args.variant, str(root), '--verify'], check=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    inputs_path = output / 'inputs.json'
    inputs = json.loads(inputs_path.read_text()) if inputs_path.exists() else {'variants': {}}
    binary = output / (args.variant + '-encoding')
    if args.variant in inputs['variants'] or binary.exists():
        raise RuntimeError('capture already exists; select a fresh output directory')
    artifacts = []
    for line in args.cargo_json.read_text().splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get('reason') == 'compiler-artifact' and record.get('target', {}).get('name') == 'encoding' and record.get('executable'):
            artifacts.append(record)
    if len(artifacts) != 1:
        raise RuntimeError('expected exactly one encoding executable in Cargo JSON')
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=root, text=True).splitlines()
    hashes = {name: sha(root / name) for name in sorted(set(names)) if (root / name).is_file()}
    with binary.open('xb') as target, Path(artifacts[0]['executable']).open('rb') as source:
        shutil.copyfileobj(source, target)
    binary.chmod(0o555)
    inputs['variants'][args.variant] = {
        'root': str(root), 'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
        'source_hashes': hashes, 'binary': {'path': str(binary), 'sha256': sha(binary)},
        'cargo_artifact': artifacts[0], 'cargo_json_sha256': sha(args.cargo_json)}
    temporary = output / 'inputs.tmp'
    with temporary.open('x') as target:
        target.write(json.dumps(inputs, indent=2) + '\n')
    temporary.replace(inputs_path)
    print('Captured', args.variant, sha(binary))


if __name__ == '__main__':
    main()
