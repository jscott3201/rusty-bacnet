#!/usr/bin/env python3
"""Unit tests for third_party_notices.py (no cargo): python3 -m unittest discover -s scripts/release"""

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import third_party_notices as tpn


class ParseTests(unittest.TestCase):
    def test_parse_tree(self):
        text = (
            "bacnet-cli v0.11.0 (/src/crates/bacnet-cli)\n"
            "tokio v1.53.1\n"
            "tokio v1.53.1 (*)\n"
            "bitflags v1.3.2\nbitflags v2.13.1\n\n"
        )
        self.assertEqual(tpn.parse_tree(text), {
            ("bacnet-cli", "0.11.0"), ("tokio", "1.53.1"), ("bitflags", "1.3.2"), ("bitflags", "2.13.1")})

    def test_normalize(self):
        self.assertEqual(tpn.normalize("\r\n\r\nMIT  \r\ntext\t\r\n\r\n"), "MIT\ntext\n")

    def test_linked_crates_cover_every_target_with_its_cli_features(self):
        calls = []

        def cargo(*args):
            calls.append(args)
            target = args[args.index("--target") + 1]
            crate = "pcap" if "sc-tls,pcap,serial" in args else "rusty" if "rusty-bacnet" in args else "cli"
            return f"{crate} v1.0.0\n{target.split('-')[0]}-only v1.0.0\nshared v2.0.0\n"

        with mock.patch.object(tpn, "cargo", cargo):
            used = tpn.linked_crates()
        self.assertEqual(len(calls), 2 * len(tpn.TARGETS))
        cli_features = {c[c.index("--target") + 1]: c[c.index("--features") + 1] for c in calls if "bacnet-cli" in c}
        self.assertEqual(cli_features, tpn.TARGETS)
        self.assertEqual(cli_features["x86_64-pc-windows-msvc"], "sc-tls")
        self.assertEqual(cli_features["aarch64-unknown-linux-gnu"], "sc-tls,pcap,serial")
        self.assertEqual(used[("pcap", "1.0.0")], {"CLI"})
        self.assertEqual(used[("shared", "2.0.0")], {"CLI", "Python"})
        self.assertIn(("aarch64-only", "1.0.0"), used)

    def test_two_sources_for_one_version_are_refused(self):
        packages = [{"name": "a", "version": "1.0.0"}, {"name": "a", "version": "1.0.0"}]
        with self.assertRaisesRegex(tpn.NoticesError, "two sources"):
            tpn.index_packages({"packages": packages})


class LicenseFilesTests(unittest.TestCase):
    def package(self, root, name="demo", **extra):
        return {"name": name, "version": "1.0.0", "manifest_path": str(root / "Cargo.toml"), **extra}

    def test_root_licence_files_and_declared_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ("LICENSE-MIT", "LICENSE-APACHE", "COPYING", "NOTICE.txt", "README.md", "license.rs"):
                (root / name).write_text(f"{name}\n")
            (root / "legal").mkdir()
            (root / "legal" / "TERMS").write_text("terms\n")
            files = tpn.license_files(self.package(root, license_file="legal/TERMS"))
        self.assertEqual([f for f, _ in files], ["COPYING", "LICENSE-APACHE", "LICENSE-MIT", "NOTICE.txt", "legal/TERMS"])

    JITTER_H = (
        "/*\n * Non-physical true random number generator based on timing jitter.\n *\n"
        " * Copyright Stephan Mueller <smueller@chronox.de>, 2014 - 2025\n *\n"
        " * Redistribution and use in source and binary forms, with or without\n"
        " * modification, are permitted.\n */\n\n#ifndef _JITTERENTROPY_H\n"
    )

    def aws_lc(self, root):
        for path, text in (("aws-lc/LICENSE", "OpenSSL and SSLeay\n"),
                           ("aws-lc/third_party/fiat/LICENSE", "The MIT License (MIT)\n"),
                           ("aws-lc/third_party/jitterentropy/jitterentropy-library/jitterentropy.h",
                            self.JITTER_H)):
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text)

    def test_aws_lc_bundled_licences(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.aws_lc(root)
            files = tpn.license_files(self.package(root, name="aws-lc-sys"))
        self.assertEqual([f.split(":")[0] for f, _ in files], [
            "aws-lc/LICENSE", "aws-lc/third_party/fiat/LICENSE",
            "aws-lc/third_party/jitterentropy/jitterentropy-library/jitterentropy.h"])
        self.assertIn("BSD-3-Clause", files[2][0])
        self.assertEqual(files[2][1], (
            "Non-physical true random number generator based on timing jitter.\n\n"
            "Copyright Stephan Mueller <smueller@chronox.de>, 2014 - 2025\n\n"
            "Redistribution and use in source and binary forms, with or without\n"
            "modification, are permitted.\n"))

    def test_extra_licences_must_exist(self):
        for path in ("aws-lc/LICENSE", "aws-lc/third_party/fiat/LICENSE",
                     "aws-lc/third_party/jitterentropy/jitterentropy-library/jitterentropy.h"):
            with self.subTest(path=path), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.aws_lc(root)
                (root / path).unlink()
                with self.assertRaisesRegex(tpn.NoticesError, f"no longer has {path}"):
                    tpn.license_files(self.package(root, name="aws-lc-sys"))

    def test_vendored_licence_for_its_pinned_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            files = tpn.license_files(self.package(root, name="tui-tree-widget", version="0.24.1"))
            self.assertEqual(len(files), 1)
            self.assertTrue(files[0][0].startswith("tui-tree-widget-0.24.1-LICENSE: "))
            self.assertIn("Copyright (c) EdJoPaTo", files[0][1])
            with self.assertRaisesRegex(tpn.NoticesError, "vendored licence is for 0.24.1"):
                tpn.license_files(self.package(root, name="tui-tree-widget", version="0.25.0"))

    def test_licence_comment_must_still_be_a_licence(self):
        with tempfile.TemporaryDirectory() as tmp:
            header = Path(tmp, "x.h")
            header.write_text("/* Just a description. */\n")
            with self.assertRaisesRegex(tpn.NoticesError, "no longer its licence"):
                tpn.licence_comment(header)
            header.write_text("#include <x.h>\n/* Copyright. Redistribution and use */\n")
            with self.assertRaisesRegex(tpn.NoticesError, "doesn't start with a comment"):
                tpn.licence_comment(header)


class LicenceRuleTests(unittest.TestCase):
    def test_needs_notice(self):
        for expression in ("MIT", "MIT OR Apache-2.0", "MIT/Apache-2.0", "Apache-2.0 WITH LLVM-exception",
                           "(MIT OR Apache-2.0) AND Unicode-3.0", "MPL-2.0", "BSD-3-Clause", "ISC",
                           "Unlicense OR MIT", "LicenseRef-unknown", None, ""):
            with self.subTest(expression=expression):
                self.assertTrue(tpn.needs_notice(expression))
        for expression in ("Unlicense", "CC0-1.0 OR MIT-0", "0BSD", "Unlicense/CC0-1.0", "BSL-1.0"):
            with self.subTest(expression=expression):
                self.assertFalse(tpn.needs_notice(expression))

    def crate(self, name, lic, files):
        return (name, "1.0.0", lic, f"https://crates.io/crates/{name}/1.0.0", {"CLI"}, files)

    def test_crate_that_needs_a_notice_must_ship_a_licence_file(self):
        crates = [self.crate("a", "MIT", []), self.crate("b", "MIT", [("LICENSE", "MIT\n")]),
                  self.crate("c", "CC0-1.0", []), self.crate("d", None, [])]
        errors = tpn.check_license_files(crates)
        self.assertEqual(len(errors), 2)
        self.assertIn("a 1.0.0 (MIT) ships no licence file", errors[0])
        self.assertIn("d 1.0.0 (no licence declared)", errors[1])
        with mock.patch.dict(tpn.ALLOW_NO_LICENSE_FILE, {"a": "why", "d": "why"}):
            self.assertEqual(tpn.check_license_files(crates), [])

    def test_the_allow_list_starts_empty(self):
        self.assertEqual(tpn.ALLOW_NO_LICENSE_FILE, {})

    def test_source_url(self):
        crates_io = {"name": "serialport", "version": "4.9.0", "repository": "https://example.invalid/sp",
                     "source": "registry+https://github.com/rust-lang/crates.io-index"}
        self.assertEqual(tpn.source_url(crates_io), "https://crates.io/crates/serialport/4.9.0")
        self.assertEqual(tpn.source_url({**crates_io, "source": "sparse+https://index.crates.io/"}),
                         "https://crates.io/crates/serialport/4.9.0")
        git = {**crates_io, "source": "git+https://example.invalid/sp?rev=1#abc"}
        self.assertEqual(tpn.source_url(git), "https://example.invalid/sp")
        with self.assertRaisesRegex(tpn.NoticesError, "declares no repository"):
            tpn.source_url({**git, "repository": None})


class RenderTests(unittest.TestCase):
    def test_render_groups_identical_texts_and_lists_gaps(self):
        crates = [
            ("b", "2.0.0", "MIT", "https://crates.io/crates/b/2.0.0", {"CLI"}, [("LICENSE", "MIT text\n")]),
            ("a", "1.0.0", "MIT OR Apache-2.0", "https://crates.io/crates/a/1.0.0", {"CLI", "Python"},
             [("LICENSE-APACHE", "Apache text\n"), ("LICENSE-MIT", "MIT text\n")]),
            ("c", "0.1.0", "Zlib", "https://example.invalid/c", {"Python"}, []),
        ]
        with mock.patch.dict(tpn.ALLOW_NO_LICENSE_FILE, {"c": "allowed for the test"}):
            text = tpn.render("1.2.3", "Our MIT\n", crates, ("1.10.7", "libpcap licence\n"))
        self.assertIn("Rusty BACnet 1.2.3 release binaries", text)
        self.assertIn("a 1.0.0: MIT OR Apache-2.0 [CLI Python]\n  source: https://crates.io/crates/a/1.0.0\n", text)
        self.assertIn("libpcap 1.10.7: BSD-3-Clause [CLI]\n"
                      "  source: https://www.tcpdump.org/release/libpcap-1.10.7.tar.xz\n", text)
        self.assertIn("libpcap licence", text)
        self.assertEqual(text.count("MIT text"), 1)
        self.assertIn("  a 1.0.0 (LICENSE-MIT)\n  b 2.0.0 (LICENSE)\n", text)
        self.assertIn("c 0.1.0: Zlib [Python]\n  source: https://example.invalid/c\n", text)
        self.assertIn("c 0.1.0: Zlib; allowed for the test", text)
        boost = ("d", "5.4.1", "BSL-1.0", "https://crates.io/crates/d/5.4.1", {"CLI"}, [])
        self.assertIn("d 5.4.1: BSL-1.0; its licence doesn't ask for the notice in a binary",
                      tpn.render("1.2.3", "Our MIT\n", [boost], None))
        self.assertLess(text.index("Apache text"), text.index("MIT text"))
        with mock.patch.dict(tpn.ALLOW_NO_LICENSE_FILE, {"c": "allowed for the test"}):
            again = tpn.render("1.2.3", "Our MIT\n", list(reversed(crates)), ("1.10.7", "libpcap licence\n"))
        self.assertEqual(text, again)


if __name__ == "__main__":
    unittest.main()
