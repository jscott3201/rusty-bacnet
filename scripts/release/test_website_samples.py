#!/usr/bin/env python3
"""website_samples.py (#2648): python3 -m unittest discover -s scripts/release"""

import io
import re
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import website_samples  # noqa: E402
from website_samples import DEV, REPOSITORY, SampleError  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

MANIFEST = f"""[package]
name = "sample"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
bacnet-server = {{ {DEV}, features = ["sc-tls"] }}
bacnet-types = {{ {DEV} }}
tokio = {{ version = "1", features = ["macros"] }}
"""
PINNED = MANIFEST.replace(f"{{ {DEV}, ", '{ version = "=0.16.0", ').replace(f"{{ {DEV} }}", '"=0.16.0"')


def samples(root, *texts):
    """A samples directory with one sample per manifest text."""
    for index, text in enumerate(texts):
        sample = root / f"sample-{index}"
        (sample / "src").mkdir(parents=True)
        (sample / "Cargo.toml").write_text(text, encoding="utf-8")
        (sample / "src/main.rs").write_text("fn main() {}\n", encoding="utf-8")
    return root


class RewriteTests(unittest.TestCase):
    def test_pin_and_follow_dev_rewrite_only_the_bacnet_crates(self):
        self.assertEqual(website_samples.rewrite(MANIFEST, "0.16.0"), (PINNED, None))
        self.assertEqual(website_samples.rewrite(PINNED, None), (MANIFEST, "0.16.0"))
        self.assertIn('bacnet-server = { version = "=0.16.0", features = ["sc-tls"] }\n', PINNED)
        self.assertIn('bacnet-types = "=0.16.0"\n', PINNED)
        self.assertIn('tokio = { version = "1", features = ["macros"] }\n', PINNED)

    def test_a_pinned_manifest_moves_to_the_next_release(self):
        self.assertEqual(website_samples.rewrite(PINNED, "0.17.0")[0],
                         website_samples.rewrite(MANIFEST, "0.17.0")[0])

    def test_form_names_the_pinned_release_or_none_for_dev(self):
        self.assertIsNone(website_samples.form(MANIFEST))
        self.assertEqual(website_samples.form(PINNED), "0.16.0")

    def test_the_committed_manifests_round_trip(self):
        for path in website_samples.manifests():
            text = path.read_text(encoding="utf-8")
            state = website_samples.form(text)
            with self.subTest(path=path.parent.name):
                self.assertEqual(website_samples.rewrite(text, state)[0], text)
                other = website_samples.rewrite(text, "9.9.9" if state is None else None)[0]
                self.assertNotEqual(other, text)
                self.assertEqual(website_samples.rewrite(other, state)[0], text)

    def test_anything_else_fails(self):
        for text, error in [
            (MANIFEST.replace(f"{{ {DEV} }}", '"=0.16.0"'), "mixes 0.16.0 and dev"),
            (MANIFEST.replace(f"{{ {DEV} }}", '"0.16"'), "neither"),
            (MANIFEST.replace(f"{{ {DEV} }}", '"=0.16"'), "neither"),
            (MANIFEST.replace('branch = "dev" }', 'branch = "main" }'), "neither"),
            (MANIFEST.replace(REPOSITORY, "https://example.invalid/fork"), "neither"),
            (MANIFEST.replace(f"{{ {DEV} }}", f'{{ {DEV}, path = "../local" }}'), "neither"),
            (MANIFEST.replace('features = ["sc-tls"] }', 'features = ["sc-tls"], rev = "0123456" }'), "neither"),
            (PINNED.replace('"=0.16.0"\n', '{ version = "=0.16.0", path = "../local" }\n'), "neither"),
            (MANIFEST + '\n[patch.crates-io]\nbacnet-types = { path = "../local" }\n', r"in \[patch.crates-io\]"),
            (re.sub(r"^bacnet-.*\n", "", MANIFEST, flags=re.M), "names no BACnet crate"),
        ]:
            with self.subTest(text=text), self.assertRaisesRegex(SampleError, error):
                website_samples.rewrite(text, "0.16.0")

    def test_the_repository_is_the_sites(self):
        site = (ROOT / "website/src/lib/site.mjs").read_text(encoding="utf-8")
        self.assertIn(f"export const repository = '{REPOSITORY}';", site)


class SetSourceTests(unittest.TestCase):
    def test_nothing_is_written_unless_every_manifest_parses(self):
        with tempfile.TemporaryDirectory() as temp:
            root = samples(Path(temp), MANIFEST, MANIFEST.replace(f"{{ {DEV} }}", '"0.16"'))
            with self.assertRaisesRegex(SampleError, "sample-1/Cargo.toml: bacnet-types"):
                website_samples.set_source("0.16.0", root)
            self.assertEqual((root / "sample-0/Cargo.toml").read_text(encoding="utf-8"), MANIFEST)

    def test_every_manifest_is_rewritten(self):
        with tempfile.TemporaryDirectory() as temp:
            root = samples(Path(temp), MANIFEST, PINNED)
            website_samples.set_source("0.16.0", root)
            for path in website_samples.manifests(root):
                self.assertEqual(path.read_text(encoding="utf-8"), PINNED)


class BuildTests(unittest.TestCase):
    def test_every_sample_is_built_and_every_failure_listed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = samples(Path(temp) / "samples", PINNED, PINNED, PINNED)
            calls = []

            def run(command, cwd, env):
                calls.append((command, Path(cwd), env["CARGO_TARGET_DIR"]))
                failed = command[1] == "build" and Path(cwd).name in ("sample-0", "sample-2")
                return subprocess.CompletedProcess(command, 101 if failed else 0)

            with redirect_stdout(io.StringIO()):
                failed = website_samples.build(Path(temp) / "work", root, run=run)
            self.assertEqual(failed, ["sample-0", "sample-2"])
            builds = [call for call in calls if call[0][1] == "build"]
            self.assertEqual([cwd.name for _, cwd, _ in builds], ["sample-0", "sample-1", "sample-2"])
            for command, cwd, target in builds:
                # A copy outside the samples, with its manifest, built in a target of the run's own.
                self.assertEqual(command, ["cargo", "build", "--manifest-path", str(cwd / "Cargo.toml")])
                self.assertEqual(Path(target), cwd.parent / "target")
                self.assertTrue(cwd.is_relative_to((Path(temp) / "work").resolve()))
            # The copies and the target are removed afterwards.
            self.assertEqual(list((Path(temp) / "work").iterdir()), [])

    def test_it_refuses_to_build_inside_the_repository(self):
        with self.assertRaisesRegex(SampleError, "inside the repository"):
            website_samples.build(ROOT / "target", run=None)


class MainTests(unittest.TestCase):
    def test_pin_takes_a_release(self):
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            self.assertEqual(website_samples.main(["--pin", "0.16"]), 1)
        self.assertIn("--pin takes a release", stderr.getvalue())

    def test_pin_and_follow_dev_exclude_each_other(self):
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            website_samples.main(["--pin", "0.16.0", "--follow-dev"])


if __name__ == "__main__":
    unittest.main()
