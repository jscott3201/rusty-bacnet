#!/usr/bin/env python3
"""Real capture/parser/file logic; all Git/compiler/Docker processes are mocked.

These are deterministic provenance controls, not a Cargo/Docker build or timing
qualification. Temporary executable bytes are inert fixtures and never run.
"""
import argparse
import contextlib
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import capture
import callgrind_matrix

IMAGE = 'sha256:' + '1' * 64
REBOUND = 'sha256:' + '2' * 64
PACKAGE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


class ExternalCommands:
    """Model only the external process boundary, refusing unexpected commands."""
    def __init__(self, roots, commits):
        self.roots, self.commits = roots, commits
        self.commands, self.volumes, self.owners = [], set(), {}
        self.tag_id = IMAGE
        self.wrong_native_root = False
        self.wrong_linux_root = False
        self.stale_metadata = False
        self.fail_native = False
        self.fail_linux = False
        self.drift = None
        self.missing_success = False
        self.lose_volume = False

    def run(self, command, *, cwd=None, env=None, capture_output=False):
        assert capture_output
        self.commands.append(command)
        code, output = 0, ''
        cwd = Path(cwd) if cwd else None
        if command == ['git', 'rev-parse', 'HEAD']:
            output = self.commits[cwd]
        elif command == ['git', 'status', '--porcelain']:
            output = ''
        elif command == ['rustc', '+1.99.0', '-Vv']:
            output = 'rustc 1.99.0 (fixture compiler)\n'
        elif command == ['cargo', '+1.99.0', '-V']:
            output = 'cargo 1.99.0 (fixture cargo)\n'
        elif command[:3] == ['docker', 'image', 'inspect']:
            output = self.tag_id if command[3] == 'fixture:tag' else command[3]
        elif command[:3] == ['docker', 'volume', 'ls']:
            output = '\n'.join(sorted(self.volumes))
        elif command[:3] == ['docker', 'volume', 'create']:
            self.volumes.add(command[-1])
            self.owners[command[-1]] = command[4].split('=', 1)[1]
            output = command[-1]
        elif command[:3] == ['docker', 'volume', 'inspect']:
            output = 'another-owner' if self.lose_volume else self.owners[command[3]]
        elif command[:3] == ['cargo', '+1.99.0', 'bench']:
            assert command == ['cargo', '+1.99.0', 'bench', '--locked', '-p',
                               'bacnet-benchmarks', '--bench', 'bip_latency',
                               '--no-run', '--message-format=json']
            assert env['CARGO_INCREMENTAL'] == '0'
            target = Path(env['CARGO_TARGET_DIR'])
            assert target.is_dir() and not list(target.iterdir())
            binary = target / 'release/deps/bip_latency-fixture'
            binary.parent.mkdir(parents=True)
            # Independently built A and B may legitimately have identical bytes.
            binary.write_bytes(b'independent equal executable fixture')
            source = self.roots['A'] if self.wrong_native_root else cwd
            output = self.metadata('bip_latency', source / 'benchmarks/Cargo.toml',
                                   source / 'benchmarks/benches/bip_latency.rs', binary)
            if self.drift == 'native':
                (cwd / 'product.rs').write_text('drift during native build\n')
            if self.fail_native:
                code = 101
        elif command[:3] == ['docker', 'run', '--rm'] and 'sha256sum' in command:
            output = digest(b'independent equal Linux fixture') + '  /target/release/rp-harness\n'
        elif command[:3] == ['docker', 'run', '--rm'] and 'build' in command:
            assert command[-5:] == ['cargo', 'build', '--release', '--locked', '--message-format=json']
            source_mount = next(value for value in command if value.endswith(':/wt:ro'))
            assert Path(source_mount[:-7]) in self.roots.values()
            assert 'CARGO_INCREMENTAL=0' in command and 'CARGO_TARGET_DIR=/target' in command
            base = Path('/wrong') if self.wrong_linux_root else Path('/h')
            output = self.metadata('rp-harness', base / 'Cargo.toml', base / 'src/main.rs',
                                   Path('/target/release/rp-harness'))
            if self.drift == 'linux':
                (Path(source_mount[:-7]) / 'product.rs').write_text('drift during Linux build\n')
            if self.fail_linux:
                code = 101
        elif command[:3] == ['docker', 'run', '--rm']:
            assert command[-3:] == ['sh', '-c', 'rustc -Vv && cargo -V && valgrind --version']
            output = 'rustc 1.99.0 (fixture compiler)\ncargo 1.99.0 (fixture cargo)\nvalgrind-3.22.0\n'
        else:
            raise AssertionError(f'unexpected external command: {command}')
        return subprocess.CompletedProcess(command, code, output.encode(),
                                           b'fixture build failure\n' if code else b'')

    def metadata(self, name, manifest, source, binary):
        rows = [dict(reason='compiler-artifact', manifest_path=str(manifest),
                     package_id='path+file://' + str(manifest.parent) + '#fixture@0.0.0',
                     target=dict(name=name, src_path=str(source)), executable=str(binary),
                     fresh=self.stale_metadata),
                dict(reason='build-finished', success=not self.missing_success)]
        return ''.join(json.dumps(row) + '\n' for row in rows)

    def build_commands(self):
        return [c for c in self.commands if 'bench' in c or 'build' in c]


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='1978-capture-test-')
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.evidence = self.directory / 'evidence'
        self.evidence.mkdir()
        self.output = self.directory / 'new-experiment'
        self.roots = {label: self.directory / ('source-' + label) for label in ['A', 'B']}
        variants = {}
        for label, root in self.roots.items():
            root.mkdir()
            for name in ['product.rs', 'benchmarks/Cargo.toml', 'benchmarks/benches/bip_latency.rs']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(label + ' source for ' + name + '\n')
            variants[label] = dict(commit=label.lower() * 40,
                                   source_hashes={str(p.relative_to(root)): capture.sha(p)
                                                  for p in root.rglob('*') if p.is_file()})
        common = {}
        for name in ['harness/Cargo.toml', 'harness/Cargo.lock', 'harness/src/main.rs', 'protocol.txt']:
            path = self.evidence / 'common' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('frozen common fixture ' + name + '\n')
            common[name] = capture.sha(path)
        for name in capture.SCRIPTS:
            shutil.copy2(PACKAGE / name, self.evidence / name)
        capture.write_json(self.evidence / 'inputs.json', {'variants': variants, 'common': common})
        self.external = ExternalCommands(self.roots, {root: variants[label]['commit']
                                                     for label, root in self.roots.items()})
        for active in [patch.object(capture, 'HERE', self.evidence),
                       patch.object(capture.platform, 'platform', return_value='fixture-platform'),
                       patch.object(capture.subprocess, 'run', side_effect=self.external.run)]:
            active.start()
            self.addCleanup(active.stop)

    def args(self, label):
        return argparse.Namespace(label=label, root=self.roots[label],
                                  native_target=self.directory / ('target-' + label),
                                  linux_volume='fixture-volume-' + label.lower(),
                                  output=self.output, image='fixture:tag')

    def run_capture(self, label):
        return capture.capture(self.args(label))

    def assert_unpublished(self, label):
        path = self.output / 'inputs.json'
        if path.exists():
            self.assertNotIn(label, json.loads(path.read_text())['variants'])
        self.assertFalse((self.output / 'capture.lock').exists())

    def test_valid_A_B_independent_equal_binaries_and_receipts(self):
        a, b = self.run_capture('A'), self.run_capture('B')
        self.assertEqual(a['native_binary_sha256'], b['native_binary_sha256'])
        self.assertNotEqual(a['native_target'], b['native_target'])
        inputs = capture.load_inputs(self.output)
        self.assertEqual(inputs['image_id'], IMAGE)
        self.assertEqual(len(self.external.build_commands()), 4)
        for label in ['A', 'B']:
            record = inputs['variants'][label]
            receipt_path = self.output / record['build_receipt']
            receipt = json.loads(receipt_path.read_text())
            self.assertTrue(receipt['completed'])
            self.assertEqual(receipt['source_manifest_before'], receipt['source_manifest_after_linux'])
            self.assertEqual(receipt['commands_sha256'], capture.sha(receipt_path.parent / 'commands.json'))
            for command in json.loads((receipt_path.parent / 'commands.json').read_text()):
                self.assertEqual(command['exit_code'], 0)
                self.assertEqual(command['stdout_sha256'], capture.sha(receipt_path.parent / command['stdout']))
        self.assertTrue(all(IMAGE in c and 'fixture:tag' not in c for c in self.external.commands
                            if c[:3] == ['docker', 'run', '--rm']))

    def test_wrong_A_root_successful_metadata_refused_for_B(self):
        self.external.wrong_native_root = True
        with self.assertRaisesRegex(ValueError, 'different source root'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertEqual(len(self.external.build_commands()), 1)
        self.assertTrue((self.output / 'preparations/B/failure.json').exists())

    def test_stale_metadata_from_same_root_refused(self):
        self.external.stale_metadata = True
        with self.assertRaisesRegex(ValueError, 'stale executable'):
            self.run_capture('B')
        self.assert_unpublished('B')

    def test_existing_native_target_refused_before_build(self):
        self.args('B').native_target.mkdir()
        with self.assertRaises(FileExistsError):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertFalse(self.external.build_commands())

    def test_existing_linux_volume_refused_before_build(self):
        self.external.volumes.add(self.args('B').linux_volume)
        with self.assertRaisesRegex(ValueError, 'volume already exists'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertFalse(self.external.build_commands())

    def test_cross_label_native_target_refused_even_if_removed(self):
        self.run_capture('A')
        args = self.args('B')
        args.native_target = self.args('A').native_target
        shutil.rmtree(args.native_target)
        with self.assertRaisesRegex(ValueError, 'reused across capture attempts'):
            capture.capture(args)
        self.assert_unpublished('B')
        self.assertEqual(len(self.external.build_commands()), 2)

    def test_cross_label_linux_volume_refused_even_if_removed(self):
        self.run_capture('A')
        args = self.args('B')
        args.linux_volume = self.args('A').linux_volume
        self.external.volumes.remove(args.linux_volume)
        with self.assertRaisesRegex(ValueError, 'reused across capture attempts'):
            capture.capture(args)
        self.assert_unpublished('B')

    def test_volume_create_race_refused(self):
        self.external.lose_volume = True
        with self.assertRaisesRegex(ValueError, 'volume reservation lost'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertFalse(self.external.build_commands())

    def test_source_drift_during_native_build_refused(self):
        self.external.drift = 'native'
        with self.assertRaisesRegex(ValueError, 'source drift'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertEqual(len(self.external.build_commands()), 1)

    def test_source_drift_during_linux_build_refused(self):
        self.external.drift = 'linux'
        with self.assertRaisesRegex(ValueError, 'source drift'):
            self.run_capture('B')
        self.assert_unpublished('B')

    def test_failed_native_build_never_hashes_or_runs_fallback(self):
        self.external.fail_native = True
        with self.assertRaisesRegex(RuntimeError, 'command failed'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertFalse(any('sha256sum' in c for c in self.external.commands))
        directory = self.output / 'preparations/B'
        commands = json.loads((directory / 'commands.json').read_text())
        self.assertEqual(commands[-1]['exit_code'], 101)
        self.assertEqual((directory / commands[-1]['stderr']).read_text(), 'fixture build failure\n')
        self.assertTrue((directory / 'failure.json').exists())

    def test_failed_linux_build_leaves_valid_A_and_no_B(self):
        self.run_capture('A')
        previous = (self.output / 'inputs.json').read_bytes()
        self.external.fail_linux = True
        with self.assertRaisesRegex(RuntimeError, 'command failed'):
            self.run_capture('B')
        self.assertEqual((self.output / 'inputs.json').read_bytes(), previous)
        self.assert_unpublished('B')

    def test_zero_exit_without_success_marker_refused(self):
        self.external.missing_success = True
        with self.assertRaisesRegex(ValueError, 'successful completed build'):
            self.run_capture('B')
        self.assert_unpublished('B')

    def test_wrong_linux_artifact_root_refused(self):
        self.external.wrong_linux_root = True
        with self.assertRaisesRegex(ValueError, 'different source root'):
            self.run_capture('B')
        self.assert_unpublished('B')

    def test_wrong_revision_refused_before_build(self):
        self.external.commits[self.roots['B']] = '0' * 40
        with self.assertRaisesRegex(ValueError, 'checkout must be at'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertFalse(self.external.build_commands())

    def test_same_label_and_failed_attempt_cannot_be_reused(self):
        self.run_capture('A')
        previous = (self.output / 'inputs.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'label already captured'):
            self.run_capture('A')
        self.assertEqual((self.output / 'inputs.json').read_bytes(), previous)
        self.external.fail_native = True
        with self.assertRaises(RuntimeError):
            self.run_capture('B')
        with self.assertRaisesRegex(ValueError, 'reused across capture attempts'):
            self.run_capture('B')
        self.assert_unpublished('B')

    def test_tag_rebind_between_captures_executes_frozen_ID(self):
        self.run_capture('A')
        self.external.tag_id = REBOUND
        self.run_capture('B')
        inputs = capture.load_inputs(self.output)
        self.assertEqual(inputs['image_id'], IMAGE)
        runs = [c for c in self.external.commands if c[:3] == ['docker', 'run', '--rm']]
        self.assertTrue(all(IMAGE in c and REBOUND not in c and 'fixture:tag' not in c for c in runs))
        selectors = [c[3] for c in self.external.commands if c[:3] == ['docker', 'image', 'inspect']]
        self.assertEqual(selectors.count('fixture:tag'), 1)

    def test_matrix_after_tag_rebind_uses_frozen_ID(self):
        self.run_capture('A')
        self.run_capture('B')
        self.external.tag_id = REBOUND
        executed = []
        def stop_before_timing(command, **kwargs):
            executed.append(command)
            raise RuntimeError('fixture stops before timing')
        def inspect(command, **kwargs):
            self.assertEqual(command[3], IMAGE)
            return IMAGE + '\n'
        with patch.object(callgrind_matrix.subprocess, 'check_output', side_effect=inspect), \
             patch.object(callgrind_matrix.subprocess, 'run', side_effect=stop_before_timing), \
             contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(RuntimeError, 'fixture stops before timing'):
                callgrind_matrix.main(self.output)
        self.assertEqual(len(executed), 1)
        self.assertIn(IMAGE, executed[0])
        self.assertNotIn('fixture:tag', executed[0])
        self.assertNotIn(REBOUND, executed[0])

    def test_matrix_rejects_unbound_old_capture(self):
        self.output.mkdir()
        capture.write_json(self.output / 'inputs.json', {'variants': {'A': {}, 'B': {}}})
        with self.assertRaisesRegex(ValueError, 'fresh owned builds'):
            capture.load_inputs(self.output)
        self.assertFalse(self.external.commands)

    def test_tools_changed_between_captures_refused(self):
        self.run_capture('A')
        previous = (self.output / 'inputs.json').read_bytes()
        with (self.evidence / 'capture.py').open('a') as script:
            script.write('\n# changed capture implementation\n')
        with self.assertRaisesRegex(ValueError, 'scripts changed between variant builds'):
            self.run_capture('B')
        self.assert_unpublished('B')
        self.assertEqual((self.output / 'inputs.json').read_bytes(), previous)

    def test_matrix_rejects_changed_receipt(self):
        a = self.run_capture('A')
        self.run_capture('B')
        with (self.output / a['build_receipt']).open('a') as receipt:
            receipt.write(' ')
        with self.assertRaisesRegex(ValueError, 'receipt changed'):
            capture.load_inputs(self.output)


if __name__ == '__main__':
    unittest.main(verbosity=2)
