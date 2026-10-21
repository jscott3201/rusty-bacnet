#!/usr/bin/env python3
"""release_version.py (#1768): python3 -m unittest discover -s scripts/release"""

import io
import json
import re
import sys
import tomllib
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import release_version  # noqa: E402
from release_version import INDEPENDENT, VersionError  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def package(name, version, publish=None):
    return {"name": name, "version": version, "publish": publish}


def metadata(*packages):
    return {"packages": list(packages)}


class ReleaseVersionTests(unittest.TestCase):
    def test_an_independent_crate_may_carry_another_version(self):
        found = release_version.release_version(metadata(
            package("bacnet-types", "0.14.0"),
            package("bacnet-time", "0.13.1"),
            package("bacnet-client", "0.14.0"),
        ))
        self.assertEqual(found, "0.14.0")

    def test_it_may_also_carry_the_release_version(self):
        found = release_version.release_version(metadata(
            package("bacnet-types", "0.14.0"), package("bacnet-time", "0.14.0")))
        self.assertEqual(found, "0.14.0")

    def test_any_other_crate_must_carry_the_shared_version(self):
        with self.assertRaisesRegex(VersionError, r"disagree: 0\.13\.1: bacnet-client; 0\.14\.0: bacnet-types"):
            release_version.release_version(metadata(
                package("bacnet-types", "0.14.0"),
                package("bacnet-time", "0.13.0"),
                package("bacnet-client", "0.13.1"),
            ))

    def test_crates_that_dont_publish_are_left_out(self):
        found = release_version.release_version(metadata(
            package("bacnet-types", "0.14.0"),
            package("bacnet-time", "0.13.0"),
            package("bacnet-benchmarks", "0.9.0", publish=[]),
        ))
        self.assertEqual(found, "0.14.0")

    def test_an_independent_crate_must_be_a_publishable_member(self):
        with self.assertRaisesRegex(VersionError, "bacnet-time"):
            release_version.release_version(metadata(package("bacnet-types", "0.14.0")))
        with self.assertRaisesRegex(VersionError, "bacnet-time"):
            release_version.release_version(metadata(
                package("bacnet-types", "0.14.0"), package("bacnet-time", "0.13.0", publish=[])))

    def test_no_crates_besides_the_independent_ones_is_an_error(self):
        with self.assertRaises(VersionError):
            release_version.release_version(metadata(package("bacnet-time", "0.13.0")))
        with self.assertRaises(VersionError):
            release_version.release_version(metadata())

    def test_main_prints_the_version_or_fails_with_an_annotation(self):
        good = json.dumps(metadata(package("bacnet-types", "0.14.0"), package("bacnet-time", "0.13.0")))
        out = io.StringIO()
        with mock.patch.object(sys, "stdin", io.StringIO(good)), redirect_stdout(out):
            self.assertEqual(release_version.main(), 0)
        self.assertEqual(out.getvalue(), "0.14.0\n")
        bad = json.dumps(metadata(package("bacnet-types", "0.14.0"), package("bacnet-client", "0.13.0"),
                                  package("bacnet-time", "0.13.0")))
        err = io.StringIO()
        with mock.patch.object(sys, "stdin", io.StringIO(bad)), redirect_stderr(err):
            self.assertEqual(release_version.main(), 1)
        self.assertIn("::error::publishable crates disagree", err.getvalue())


class ManifestTests(unittest.TestCase):
    """INDEPENDENT names exactly the publishable members whose manifests set
    their own version instead of inheriting the workspace's."""

    def test_independent_matches_the_manifests(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        own = set()
        for member in root["workspace"]["members"]:
            manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text(encoding="utf-8"))
            crate = manifest["package"]
            if crate.get("publish", True) is False or crate.get("publish") == []:
                continue
            if crate.get("version") != {"workspace": True}:
                own.add(crate["name"])
        self.assertEqual(own, set(INDEPENDENT))

    def test_the_workspace_dependency_pins_each_ones_own_version(self):
        root = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        for name in INDEPENDENT:
            with self.subTest(crate=name):
                entry = root["workspace"]["dependencies"][name]
                manifest = tomllib.loads((ROOT / entry["path"] / "Cargo.toml").read_text(encoding="utf-8"))
                self.assertEqual(entry["version"], manifest["package"]["version"])

    def test_release_yml_reads_the_version_from_this_script(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertRegex(workflow, re.compile(
            r"version=\$\(cargo metadata --no-deps --format-version 1 --locked \| python3 scripts/release/release_version\.py\)"))


if __name__ == "__main__":
    unittest.main()
