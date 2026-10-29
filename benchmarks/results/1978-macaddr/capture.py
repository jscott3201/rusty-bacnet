#!/usr/bin/env python3
"""Own fresh source-bound A/B builds for a separate, opt-in measurement matrix."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import uuid

HERE = Path(__file__).resolve().parent
FORMAT = 'source-owned-build-v1'
SCRIPTS = ['capture.py', 'callgrind_matrix.py', 'native_matrix.py', 'analyze.py']


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def image_identity(value):
    if not isinstance(value, str) or not re.fullmatch(r'sha256:[0-9a-f]{64}', value):
        raise ValueError('expected a complete immutable Docker image ID')
    return value


class Commands:
    """Retain every attempted external command, including preparation failures."""
    def __init__(self, directory):
        self.directory = directory
        self.records = []

    def run(self, command, *, cwd=None, env=None):
        stem = f'{len(self.records):02}'
        record = {'command': command, 'cwd': str(cwd) if cwd else None,
                  'environment_overrides': env or {}, 'stdout': stem + '.stdout',
                  'stderr': stem + '.stderr'}
        self.records.append(record)
        write_json(self.directory / 'commands.json', self.records)
        actual_env = os.environ.copy()
        actual_env.update(env or {})
        try:
            result = subprocess.run(command, cwd=cwd, env=actual_env, capture_output=True)
        except BaseException as error:
            record['preparation_error'] = repr(error)
            write_json(self.directory / 'commands.json', self.records)
            raise
        (self.directory / record['stdout']).write_bytes(result.stdout)
        (self.directory / record['stderr']).write_bytes(result.stderr)
        record.update(exit_code=result.returncode,
                      stdout_sha256=hashlib.sha256(result.stdout).hexdigest(),
                      stderr_sha256=hashlib.sha256(result.stderr).hexdigest())
        write_json(self.directory / 'commands.json', self.records)
        if result.returncode:
            raise RuntimeError(f'command failed ({result.returncode}): {command!r}')
        return result.stdout.decode()


def verify_tree(label, root, expected, commands=None):
    def git(*args):
        command = ['git', *args]
        if commands:
            return commands.run(command, cwd=root).strip()
        return subprocess.check_output(command, cwd=root, text=True).strip()
    if git('rev-parse', 'HEAD') != expected['commit']:
        raise ValueError(f'{label}: checkout must be at {expected["commit"]}')
    if git('status', '--porcelain'):
        raise ValueError(f'{label}: source checkout is not clean')
    actual = {relative: sha(root / relative) for relative in expected['source_hashes']}
    if actual != expected['source_hashes']:
        raise ValueError(f'{label}: source drift')
    return hashlib.sha256(json.dumps(actual, sort_keys=True).encode()).hexdigest()


def common_inputs(output, original):
    for relative, expected in original['common'].items():
        source, destination = HERE / 'common' / relative, output / relative
        if sha(source) != expected:
            raise ValueError(f'distributed common input changed: {relative}')
        if not destination.exists():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        if sha(destination) != expected:
            raise ValueError(f'existing common input differs: {relative}')


def artifact(stdout, *, name, manifest, source, target):
    rows = [json.loads(line) for line in stdout.splitlines() if line.startswith('{')]
    finished = [row for row in rows if row.get('reason') == 'build-finished']
    if len(finished) != 1 or finished[0].get('success') is not True:
        raise ValueError('Cargo JSON lacks one successful completed build')
    matches = [row for row in rows if row.get('reason') == 'compiler-artifact'
               and row.get('target', {}).get('name') == name and row.get('executable')]
    if len(matches) != 1:
        raise ValueError(f'expected one {name} executable')
    row = matches[0]
    if row.get('manifest_path') != str(manifest) or row['target'].get('src_path') != str(source):
        raise ValueError('Cargo artifact belongs to a different source root')
    binary = Path(row['executable'])
    if not binary.is_absolute() or not binary.is_relative_to(target):
        raise ValueError('executable is outside the fresh target')
    # A fresh target cannot legitimately reuse a fresh=true top-level executable.
    if row.get('fresh') is not False:
        raise ValueError('Cargo reused a stale executable in a supposedly fresh target')
    return binary, row


def capture(args):
    original = json.loads((HERE / 'inputs.json').read_text())
    root, target, output = args.root.resolve(), args.native_target.resolve(), args.output.resolve()
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]+', args.linux_volume):
        raise ValueError('use a named private Docker volume, not a bind path')
    if target.is_relative_to(root) or root.is_relative_to(target):
        raise ValueError('native target must be separate from source checkout')
    if output.is_relative_to(target) or target == output:
        raise ValueError('native target cannot contain the capture output')
    output.mkdir(parents=True, exist_ok=True)
    lock = output / 'capture.lock'
    # Serialize publication and resource admission across labels. Never remove a
    # lock another process owns; an interrupted preparation needs a fresh output.
    with lock.open('x'):
        pass
    try:
        return capture_locked(args, original, root, target, output)
    finally:
        lock.unlink()


def capture_locked(args, original, root, target, output):
    inputs_path = output / 'inputs.json'
    inputs = json.loads(inputs_path.read_text()) if inputs_path.exists() else None
    if inputs and (inputs.get('capture_format') != FORMAT or args.label in inputs['variants']):
        raise ValueError('label already captured or output uses an incompatible capture format')
    if inputs and any(sha(HERE / name) != inputs['measurement_scripts'][name] for name in SCRIPTS):
        raise ValueError('reproduction scripts changed between variant builds')
    preparations = output / 'preparations'
    preparations.mkdir(exist_ok=True)
    for reservation in preparations.glob('*/reservation.json'):
        previous = json.loads(reservation.read_text())
        if str(target) == previous['native_target'] or args.linux_volume == previous['volume']:
            raise ValueError('native target or Linux volume reused across capture attempts')
    directory = preparations / args.label
    directory.mkdir(exist_ok=False)
    reservation = {'label': args.label, 'root': str(root), 'native_target': str(target),
                   'volume': args.linux_volume, 'owner': uuid.uuid4().hex}
    write_json(directory / 'reservation.json', reservation)
    commands = Commands(directory)
    try:
        before = verify_tree(args.label, root, original['variants'][args.label], commands)
        common_inputs(output, original)
        # mkdir is the admission operation, not a check followed by reuse.
        target.parent.mkdir(parents=True, exist_ok=True)
        target.mkdir(exist_ok=False)
        volumes = commands.run(['docker', 'volume', 'ls', '--format', '{{.Name}}']).splitlines()
        if args.linux_volume in volumes:
            raise ValueError('Linux volume already exists; use a fresh private volume')
        if inputs:
            if args.image != inputs['image']:
                raise ValueError('A and B must select the same image label')
            selected = image_identity(inputs['image_id'])
        else:
            selected = image_identity(commands.run(
                ['docker', 'image', 'inspect', args.image, '--format', '{{.Id}}']).strip())
        # All Docker executions use the selected ID. A later tag rebind is irrelevant.
        if commands.run(['docker', 'image', 'inspect', selected, '--format', '{{.Id}}']).strip() != selected:
            raise ValueError('selected immutable image is unavailable or mismatched')
        native = {'platform': platform.platform(),
                  'rustc': commands.run(['rustc', '+1.99.0', '-Vv'], cwd=root),
                  'cargo': commands.run(['cargo', '+1.99.0', '-V'], cwd=root),
                  'workers': 2, 'CARGO_INCREMENTAL': '0'}
        linux = commands.run(['docker', 'run', '--rm', selected, 'sh', '-c',
                              'rustc -Vv && cargo -V && valgrind --version'])
        if not native['rustc'].startswith('rustc 1.99.0 ') or not native['cargo'].startswith('cargo 1.99.0 ') or not linux.startswith('rustc 1.99.0 ') or 'cargo 1.99.0 ' not in linux or 'valgrind-3.22.0' not in linux.splitlines():
            raise ValueError('wrong compiler, Cargo or Callgrind version')
        if inputs and (native != inputs['native_environment'] or linux != inputs['linux_environment']):
            raise ValueError('A/B toolchain or platform observations differ')
        commands.run(['docker', 'volume', 'create', '--label',
                      'codex.macaddr.capture=' + reservation['owner'], args.linux_volume])
        owner = commands.run(['docker', 'volume', 'inspect', args.linux_volume, '--format',
                              '{{index .Labels "codex.macaddr.capture"}}']).strip()
        if owner != reservation['owner']:
            raise ValueError('Linux volume reservation lost; refusing another build artifact')
        native_stdout = commands.run(
            ['cargo', '+1.99.0', 'bench', '--locked', '-p', 'bacnet-benchmarks',
             '--bench', 'bip_latency', '--no-run', '--message-format=json'], cwd=root,
            env={'CARGO_INCREMENTAL': '0', 'CARGO_TARGET_DIR': str(target)})
        binary, native_artifact = artifact(
            native_stdout, name='bip_latency', manifest=root / 'benchmarks/Cargo.toml',
            source=root / 'benchmarks/benches/bip_latency.rs', target=target)
        if binary.resolve() != binary or not binary.is_file():
            raise ValueError('native executable is missing or redirected by a symlink')
        native_hash = sha(binary)
        after_native = verify_tree(args.label, root, original['variants'][args.label], commands)
        common_inputs(output, original)
        linux_stdout = commands.run(
            ['docker', 'run', '--rm', '-e', 'CARGO_INCREMENTAL=0', '-e', 'CARGO_TARGET_DIR=/target',
             '-v', str(root) + ':/wt:ro', '-v', str(output / 'harness') + ':/h:ro',
             '-v', args.linux_volume + ':/target', '-w', '/h', selected,
             'cargo', 'build', '--release', '--locked', '--message-format=json'])
        linux_binary, linux_artifact = artifact(
            linux_stdout, name='rp-harness', manifest=Path('/h/Cargo.toml'),
            source=Path('/h/src/main.rs'), target=Path('/target'))
        if linux_binary != Path('/target/release/rp-harness'):
            raise ValueError('unexpected Linux harness executable')
        linux_hash = commands.run(
            ['docker', 'run', '--rm', '-v', args.linux_volume + ':/target:ro', selected,
             'sha256sum', str(linux_binary)]).split()[0]
        if not re.fullmatch(r'[0-9a-f]{64}', linux_hash):
            raise ValueError('invalid Linux executable hash')
        after_linux = verify_tree(args.label, root, original['variants'][args.label], commands)
        common_inputs(output, original)
        if before != after_native or before != after_linux or sha(binary) != native_hash:
            raise ValueError('source or native executable changed during capture')
        receipt = {**reservation, 'source_manifest_before': before,
                   'source_manifest_after_native': after_native, 'source_manifest_after_linux': after_linux,
                   'image_id': selected, 'native_artifact': native_artifact, 'linux_artifact': linux_artifact,
                   'native_binary_sha256': native_hash, 'linux_binary_sha256': linux_hash,
                   'commands_sha256': sha(directory / 'commands.json'), 'completed': True}
        receipt_path = directory / 'receipt.json'
        write_json(receipt_path, receipt)
        if inputs is None:
            inputs = {'capture_format': FORMAT, 'variants': {}, 'common': original['common'],
                      'image': args.image, 'image_id': selected, 'native_environment': native,
                      'linux_environment': linux,
                      'reproduction_of_original_inputs_sha256': sha(HERE / 'inputs.json')}
        record = dict(original['variants'][args.label])
        record.update(root=str(root), native_binary=str(binary), native_binary_sha256=native_hash,
                      native_target=str(target), volume=args.linux_volume, linux_binary_sha256=linux_hash,
                      build_receipt=str(receipt_path.relative_to(output)), build_receipt_sha256=sha(receipt_path))
        inputs['variants'][args.label] = record
        inputs['measurement_scripts'] = {name: sha(HERE / name) for name in SCRIPTS}
        (output / 'outputs').mkdir(exist_ok=True)
        temporary = output / 'inputs.pending.json'
        write_json(temporary, inputs)
        temporary.replace(inputs_path)
        return record
    except BaseException as error:
        write_json(directory / 'failure.json', {'error': repr(error), 'variant_published': False})
        raise


def load_inputs(root):
    """Refuse old unbound captures before a fresh matrix executes anything."""
    inputs = json.loads((root / 'inputs.json').read_text())
    if inputs.get('capture_format') != FORMAT or set(inputs['variants']) != {'A', 'B'}:
        raise ValueError('matrix requires both variants from fresh owned builds')
    selected = image_identity(inputs['image_id'])
    if len({v['native_target'] for v in inputs['variants'].values()}) != 2 or len({v['volume'] for v in inputs['variants'].values()}) != 2:
        raise ValueError('matrix targets or volumes are shared')
    for name in SCRIPTS:
        if sha(HERE / name) != inputs['measurement_scripts'][name]:
            raise ValueError('reproduction script changed after capture')
    for label, variant in inputs['variants'].items():
        path = root / variant['build_receipt']
        if sha(path) != variant['build_receipt_sha256']:
            raise ValueError('build receipt changed after capture')
        receipt = json.loads(path.read_text())
        if receipt.get('completed') is not True or receipt['label'] != label or receipt['image_id'] != selected:
            raise ValueError('build receipt identity differs from capture')
        for key in ['root', 'native_target', 'volume', 'native_binary_sha256', 'linux_binary_sha256']:
            if receipt[key] != variant[key]:
                raise ValueError('build receipt artifact differs from capture')
    return inputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('label', choices=['A', 'B'])
    parser.add_argument('root', type=Path)
    parser.add_argument('native_target', type=Path)
    parser.add_argument('linux_volume')
    parser.add_argument('output', type=Path)
    parser.add_argument('--image', required=True, help='local image selector; resolved once to an immutable ID')
    args = parser.parse_args()
    record = capture(args)
    print('Captured', args.label, record['native_binary_sha256'], record['linux_binary_sha256'])


if __name__ == '__main__':
    main()
