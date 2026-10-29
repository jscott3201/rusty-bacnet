#!/usr/bin/env python3
"""Apply frozen historical experiment overlays to clean scratch checkouts only."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


def run(root, *args):
    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('variant', choices=list('PQR'))
    parser.add_argument('root', type=Path)
    parser.add_argument('--verify', action='store_true', help='verify an already prepared checkout without edits')
    args = parser.parse_args()
    expected = json.loads((HERE / 'overlays/expected.json').read_text())[args.variant]
    root = args.root.resolve()
    if run(root, 'rev-parse', 'HEAD') != expected['revision']:
        raise RuntimeError('wrong historical revision; use an isolated scratch checkout')
    if not args.verify:
        if run(root, 'status', '--porcelain'):
            raise RuntimeError('scratch checkout must be clean')
        patch = HERE / 'overlays' / (args.variant + '.patch')
        subprocess.run(['git', 'apply', '--check', str(patch)], cwd=root, check=True)
        subprocess.run(['git', 'apply', str(patch)], cwd=root, check=True)
        for relative in expected['extra_files']:
            destination = root / relative
            destination.parent.mkdir(exist_ok=True, parents=True)
            with destination.open('xb') as target:
                target.write((HERE / 'overlays' / args.variant / relative).read_bytes())
    for relative, digest in expected['files'].items():
        if hashlib.sha256((root / relative).read_bytes()).hexdigest() != digest:
            raise RuntimeError('overlay hash mismatch: ' + relative)
    print(args.variant, 'overlay verified', expected['revision'])


if __name__ == '__main__':
    main()
