#!/usr/bin/env python3
"""website_samples.py (#2648): python3 -m unittest discover -s scripts/release"""

import io
import json
import re
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

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
VERSIONS = {"bacnet-server": "0.16.0", "bacnet-types": "0.16.0", "bacnet-time": "0.13.2"}
PINNED = MANIFEST.replace(f"{{ {DEV}, ", '{ version = "=0.16.0", ').replace(f"{{ {DEV} }}", '"=0.16.0"')


def ok(command, **_):
    return subprocess.CompletedProcess(command, 0)


def examples(root, *texts, python=()):
    """A samples tree with one Rust sample per manifest text and the named
    Python programs, and its registry."""
    registry = {}
    for index, text in enumerate(texts):
        sample = root / f"rust/sample-{index}"
        (sample / "src").mkdir(parents=True)
        (sample / "Cargo.toml").write_text(text, encoding="utf-8")
        (sample / "src/main.rs").write_text("fn main() {}\n", encoding="utf-8")
        (sample / "build.rs").write_text("fn main() {}\n", encoding="utf-8")
        registry[f"sample-{index}"] = {"path": f"rust/sample-{index}", "language": "rust"}
    for name in python:
        (root / "python").mkdir(parents=True, exist_ok=True)
        (root / "python" / name).write_text("print('ok')\n", encoding="utf-8")
        registry[name] = {"path": f"python/{name}", "language": "python"}
    return registry


class RewriteTests(unittest.TestCase):
    def test_pin_and_follow_dev_rewrite_only_the_bacnet_crates(self):
        self.assertEqual(website_samples.rewrite(MANIFEST, VERSIONS), (PINNED, None))
        self.assertEqual(website_samples.rewrite(PINNED, None), (MANIFEST, {"bacnet-server": "0.16.0", "bacnet-types": "0.16.0"}))
        self.assertIn('bacnet-server = { version = "=0.16.0", features = ["sc-tls"] }\n', PINNED)
        self.assertIn('bacnet-types = "=0.16.0"\n', PINNED)
        self.assertIn('tokio = { version = "1", features = ["macros"] }\n', PINNED)

    def test_each_crate_takes_its_own_version(self):
        text = MANIFEST.replace("bacnet-types = ", "bacnet-time = ")
        pinned = website_samples.rewrite(text, VERSIONS)[0]
        self.assertIn('bacnet-time = "=0.13.2"\n', pinned)
        self.assertIn('bacnet-server = { version = "=0.16.0"', pinned)
        self.assertEqual(website_samples.form(pinned), {"bacnet-server": "0.16.0", "bacnet-time": "0.13.2"})
        with self.assertRaisesRegex(SampleError, "bacnet-client is not a published workspace crate"):
            website_samples.rewrite(MANIFEST.replace("bacnet-types = ", "bacnet-client = "), VERSIONS)

    def test_a_pinned_manifest_moves_to_the_next_release(self):
        nxt = {crate: "0.17.0" for crate in VERSIONS}
        self.assertEqual(website_samples.rewrite(PINNED, nxt)[0], website_samples.rewrite(MANIFEST, nxt)[0])

    def test_the_committed_manifests_round_trip(self):
        for name, sample in website_samples.load_registry().items():
            if sample["language"] != "rust":
                continue
            text = (website_samples.EXAMPLES / sample["path"] / "Cargo.toml").read_text(encoding="utf-8")
            state = website_samples.form(text)
            pins = state or {crate: "9.9.9" for crate in re.findall(r"^(bacnet-[a-z0-9-]+) =", text, re.M)}
            with self.subTest(sample=name):
                self.assertEqual(website_samples.rewrite(text, state)[0], text)
                other = website_samples.rewrite(text, None if state else pins)[0]
                self.assertNotEqual(other, text)
                self.assertEqual(website_samples.rewrite(other, state)[0], text)

    def test_anything_else_fails(self):
        for text, error in [
            (MANIFEST.replace(f"{{ {DEV} }}", '"=0.16.0"'), "pins some BACnet crates and takes others from dev"),
            (MANIFEST.replace(f"{{ {DEV} }}", '"0.16"'), "neither"),
            (MANIFEST.replace(f"{{ {DEV} }}", '"=0.16"'), "neither"),
            (MANIFEST.replace('branch = "dev" }', 'branch = "main" }'), "neither"),
            (MANIFEST.replace(REPOSITORY, "https://example.invalid/fork"), "neither"),
            (MANIFEST.replace(f"{{ {DEV} }}", f'{{ {DEV}, path = "../local" }}'), "neither"),
            (MANIFEST.replace('features = ["sc-tls"] }', 'features = ["sc-tls"], rev = "0123456" }'), "neither"),
            (PINNED.replace('"=0.16.0"\n', '{ version = "=0.16.0", path = "../local" }\n'), "neither"),
            (MANIFEST + '\n[patch.crates-io]\nbacnet-types = { path = "../local" }\n', r"in \[patch.crates-io\]"),
            # Ways of naming a crate that the line pattern doesn't see.
            (MANIFEST + "bacnet-client.workspace = true\n", r"bacnet-client in \[dependencies\]"),
            (MANIFEST + '\n[dependencies.bacnet-client]\nversion = "=0.16.0"\n', r"bacnet-client in \[dependencies\]"),
            (MANIFEST + '\n[target.\'cfg(unix)\'.dependencies]\nbacnet-client = "=0.16.0"\n', r"bacnet-client is in \[target"),
            (MANIFEST + '\n[dev-dependencies]\n"bacnet-client" = "=0.16.0"\n', r"bacnet-client in \[dev-dependencies\]"),
            (MANIFEST + "[broken\n", "not TOML"),
            (re.sub(r"^bacnet-.*\n", "", MANIFEST, flags=re.M), "names no BACnet crate"),
        ]:
            with self.subTest(text=text), self.assertRaisesRegex(SampleError, error):
                website_samples.rewrite(text, VERSIONS)

    def test_the_repository_is_the_sites(self):
        site = (ROOT / "website/src/lib/site.mjs").read_text(encoding="utf-8")
        self.assertIn(f"export const repository = '{REPOSITORY}';", site)


class VersionTests(unittest.TestCase):
    FILES = {
        "Cargo.toml": '[workspace]\nmembers = ["crates/bacnet-types", "crates/bacnet-time", "crates/bacnet-sim", "benchmarks"]\n'
                      '[workspace.package]\nversion = "0.16.0"\n',
        "crates/bacnet-types/Cargo.toml": '[package]\nname = "bacnet-types"\nversion.workspace = true\n',
        "crates/bacnet-time/Cargo.toml": '[package]\nname = "bacnet-time"\nversion = "0.13.2"\n',
        "crates/bacnet-sim/Cargo.toml": '[package]\nname = "bacnet-sim"\nversion.workspace = true\npublish = false\n',
        "benchmarks/Cargo.toml": '[package]\nname = "benchmarks"\nversion.workspace = true\n',
    }

    def test_each_published_crate_has_its_own_version(self):
        versions = website_samples.crate_versions("0.16.0", read=self.FILES.__getitem__)
        self.assertEqual(versions, {"bacnet-types": "0.16.0", "bacnet-time": "0.13.2"})

    def test_the_workspace_must_be_at_that_release(self):
        with self.assertRaisesRegex(SampleError, "the workspace is at 0.16.0"):
            website_samples.crate_versions("0.17.0", read=self.FILES.__getitem__)

    def test_the_tag_gives_the_versions_when_it_exists(self):
        if website_samples.git("rev-parse", "--verify", "--quiet", "v0.15.0^{commit}").returncode:
            self.skipTest("no v0.15.0 tag in this clone")
        versions = website_samples.crate_versions("0.15.0")
        self.assertEqual(versions["bacnet-types"], "0.15.0")
        self.assertEqual(versions["bacnet-time"], "0.13.1")


class StateTests(unittest.TestCase):
    def test_nothing_is_written_unless_every_manifest_rewrites(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            registry = examples(root, MANIFEST, MANIFEST.replace(f"{{ {DEV} }}", '"0.16"'))
            with self.assertRaisesRegex(SampleError, "sample-1/Cargo.toml: bacnet-types"):
                website_samples.set_state(list(registry), "0.16.0", registry, root, root / "samples.json", VERSIONS)
            self.assertEqual((root / "rust/sample-0/Cargo.toml").read_text(encoding="utf-8"), MANIFEST)

    def test_named_samples_move_and_python_samples_follow_in_the_registry(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            registry = examples(root, MANIFEST, MANIFEST, python=["a.py"])
            path = root / "samples.json"
            website_samples.set_state(["sample-1", "a.py"], None, registry, root, path)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["a.py"]["follows"], "dev")
            self.assertEqual(path.read_text(encoding="utf-8"), json.dumps(registry, indent=2) + "\n")
            website_samples.set_state(["sample-1", "a.py"], "0.16.0", registry, root, path, VERSIONS)
            self.assertEqual((root / "rust/sample-0/Cargo.toml").read_text(encoding="utf-8"), MANIFEST)
            self.assertEqual((root / "rust/sample-1/Cargo.toml").read_text(encoding="utf-8"), PINNED)
            self.assertNotIn("follows", json.loads(path.read_text(encoding="utf-8"))["a.py"])

    def test_moving_a_sample_back_and_forth_leaves_the_registry_as_it_was(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "samples.json"
            original = website_samples.REGISTRY.read_text(encoding="utf-8")
            path.write_text(original, encoding="utf-8")
            python = [name for name, sample in json.loads(original).items() if sample["language"] == "python"]
            for version in ("0.15.0", None):
                registry = website_samples.load_registry(path)
                website_samples.set_state(python, version, registry, Path(temp), path, {}, untagged=False)
            registry = website_samples.load_registry(path)
            on_dev = [name for name, sample in json.loads(original).items() if sample.get("follows") == "dev"]
            website_samples.set_state([n for n in python if n not in on_dev], "0.15.0", registry, Path(temp), path, {}, untagged=False)
            self.assertEqual(path.read_text(encoding="utf-8"), original)

    def test_the_release_pr_drops_tag_page_and_since(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            registry = examples(root, MANIFEST, python=["a.py", "b.py"])
            registry["sample-0"]["tagPage"] = "start/x.md"
            registry["a.py"].update(follows="dev", since="0.16.0")
            registry["b.py"]["tagPage"] = "start/b.md"
            path = root / "samples.json"
            # A pin to a tagged release keeps them; the untagged one, the release PR, ships each as a file.
            website_samples.set_state(["sample-0", "b.py"], "0.15.0", registry, root, path, VERSIONS, untagged=False)
            self.assertEqual(registry["b.py"]["tagPage"], "start/b.md")
            website_samples.set_state(list(registry), "0.16.0", registry, root, path, VERSIONS, untagged=True)
            saved = json.loads(path.read_text(encoding="utf-8"))
            for name in registry:
                self.assertEqual(set(saved[name]), {"path", "language"}, name)

    def test_an_unknown_sample_is_refused(self):
        with self.assertRaisesRegex(SampleError, "no sample named nope"):
            website_samples.selected({"a": {}}, ["nope"])


class BuildTests(unittest.TestCase):
    def test_every_sample_is_built_from_a_whole_copy_and_every_failure_listed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "examples"
            registry = examples(root, PINNED, PINNED, PINNED)
            calls = []

            def run(command, cwd, env):
                calls.append((command, Path(cwd), env["CARGO_TARGET_DIR"]))
                if command[1] == "build":
                    self.assertTrue((Path(cwd) / "build.rs").is_file(), "every file of the sample is copied")
                failed = command[1] == "build" and Path(cwd).name in ("sample-0", "sample-2")
                return subprocess.CompletedProcess(command, 101 if failed else 0)

            with redirect_stdout(io.StringIO()):
                failed = website_samples.build(list(registry), registry, Path(temp) / "work", root, run=run)
            self.assertEqual(failed, ["sample-0", "sample-2"])
            self.assertEqual(calls[0][0], ["cargo", "--version"])
            builds = [call for call in calls if call[0][1] == "build"]
            self.assertEqual([cwd.name for _, cwd, _ in builds], ["sample-0", "sample-1", "sample-2"])
            for command, cwd, target in builds:
                self.assertEqual(command, ["cargo", "build", "--manifest-path", str(cwd / "Cargo.toml")])
                self.assertEqual(Path(target), cwd.parent / "target")
                self.assertTrue(cwd.is_relative_to((Path(temp) / "work").resolve()))
            # The copies and the target are removed afterwards.
            self.assertEqual(list((Path(temp) / "work").iterdir()), [])

    def test_a_copy_error_is_one_failure_and_the_rest_still_build(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "examples"
            registry = examples(root, PINNED, PINNED)
            registry["sample-0"]["path"] = "rust/missing"
            with redirect_stdout(io.StringIO()):
                failed = website_samples.build(list(registry), registry, Path(temp) / "work", root, run=ok)
            self.assertEqual(failed, ["sample-0"])

    def test_cargo_must_answer(self):
        with tempfile.TemporaryDirectory() as temp:
            for run in (lambda command, **_: subprocess.CompletedProcess(command, 1),
                        mock.Mock(side_effect=FileNotFoundError("cargo"))):
                with self.subTest(run=run), self.assertRaisesRegex(SampleError, "cargo"):
                    website_samples.build([], {}, temp, run=run)

    def test_it_refuses_to_build_inside_the_repository(self):
        with self.assertRaisesRegex(SampleError, "inside the repository"):
            website_samples.build([], {}, ROOT / "target", run=None)


class PythonTests(unittest.TestCase):
    def test_every_python_sample_has_a_way_to_run(self):
        registry = website_samples.load_registry()
        python = sorted(name for name, sample in registry.items() if sample["language"] == "python")
        self.assertEqual(sorted(website_samples.PYTHON_RUNS), python)

    def test_the_release_wheel_is_installed_and_each_sample_run(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "examples"
            registry = examples(root, python=["bip_router.py", "first_read.py", "server_device.py"])
            calls = []

            def run(command, cwd, **kwargs):
                calls.append((command, kwargs))
                failed = command[-1].endswith("first_read.py") or command[-1] == "test_website_first_read.py"
                return subprocess.CompletedProcess(command, 1 if failed else 0)

            with redirect_stdout(io.StringIO()):
                failed = website_samples.build_python(list(registry), registry, "0.15.0", Path(temp) / "work", root, python="py", run=run)
            self.assertEqual(failed, ["first_read.py"])
            venv = calls[0][0][-1]
            python = str(website_samples.venv_python(Path(venv)))
            self.assertEqual(calls[0][0], ["py", "-m", "venv", venv])
            self.assertEqual(calls[1][0], [python, "-m", "pip", "install", "--only-binary=:all:", "rusty-bacnet==0.15.0"])
            scripts = {Path(command[-1]).name: (command, kwargs) for command, kwargs in calls[2:]}
            self.assertEqual(scripts["bip_router.py"][0][:2], [python, "-I"])
            self.assertEqual(scripts["test_website_first_read.py"][0][1:5], ["-I", "-m", "unittest", "discover"])
            for command, kwargs in scripts.values():
                # stdin at end of input stops server_device.py; every run is bounded.
                self.assertEqual(kwargs["stdin"], subprocess.DEVNULL)
                self.assertEqual(kwargs["timeout"], website_samples.PYTHON_TIMEOUT)
                self.assertEqual(kwargs["env"]["BACNET_LAB_EXPECTED_VERSION"], "0.15.0")
            self.assertEqual(list((Path(temp) / "work").iterdir()), [])

    def test_a_failed_install_fails_the_run(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "examples"
            registry = examples(root, python=["bip_router.py"])

            def run(command, cwd, **_):
                return subprocess.CompletedProcess(command, 1 if "install" in command else 0)

            failed = website_samples.build_python(list(registry), registry, "0.15.0", Path(temp) / "work", root, python="py", run=run)
            self.assertEqual(failed, ["rusty-bacnet==0.15.0 (-m pip install)"])

    def test_a_hung_sample_is_a_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "examples"
            registry = examples(root, python=["bip_router.py"])

            def run(command, cwd, **kwargs):
                if "timeout" in kwargs:
                    raise subprocess.TimeoutExpired(command, kwargs["timeout"])
                return subprocess.CompletedProcess(command, 0)

            with redirect_stdout(io.StringIO()):
                failed = website_samples.build_python(list(registry), registry, "0.15.0", Path(temp) / "work", root, python="py", run=run)
            self.assertEqual(failed, ["bip_router.py"])


class MainTests(unittest.TestCase):
    def run_main(self, argv, **patches):
        out, err = io.StringIO(), io.StringIO()
        # set_state is never reached by these argv; patching it keeps main off the files.
        with mock.patch.multiple(website_samples, set_state=mock.DEFAULT, **patches), redirect_stdout(out), redirect_stderr(err):
            code = website_samples.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_a_release_is_x_y_z(self):
        for argv in (["--pin", "0.16"], ["--build-python", "latest"]):
            with self.subTest(argv=argv):
                code, _, err = self.run_main(argv)
                self.assertEqual(code, 1)
                self.assertIn("a release is x.y.z", err)

    def test_pin_and_follow_dev_exclude_each_other(self):
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            website_samples.main(["--pin", "0.16.0", "--follow-dev"])

    def test_the_exit_code_says_whether_every_sample_passed(self):
        code, out, _ = self.run_main(["--build"], build=mock.Mock(return_value=[]))
        self.assertEqual((code, out.splitlines()[-1]), (0, "Every sample passed."))
        code, _, err = self.run_main(["--build", "--build-python", "0.15.0"],
                                     build=mock.Mock(return_value=["bip-router"]),
                                     build_python=mock.Mock(return_value=["bip_router.py"]))
        self.assertEqual(code, 1)
        self.assertIn("samples that failed: bip-router, bip_router.py", err)
        code, _, err = self.run_main(["nope"])
        self.assertEqual(code, 1)
        self.assertIn("no sample named nope", err)


if __name__ == "__main__":
    unittest.main()
