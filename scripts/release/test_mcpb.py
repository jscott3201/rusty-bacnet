#!/usr/bin/env python3
"""Unit tests for mcpb.py and check_artifacts.py's bundle check (#2304), with
stand-in executables: python3 -m unittest discover -s scripts/release"""

import contextlib
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
import zipfile
from pathlib import Path
from unittest import mock

import check_artifacts
import mcpb

# A stand-in for `bacnet mcp serve --stdio --profile NAME`: it refuses a
# missing or unauthorized profile as the CLI does, then answers initialize
# and tools/list with the manifest's tools. MODE makes it misbehave.
FAKE = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, sys
    MODE = "{mode}"
    name = sys.argv[sys.argv.index("--profile") + 1]
    path = os.path.join(os.environ["BACNET_CONFIG_DIR"], "profiles", name + ".toml")
    if MODE != "ignores-consent":
        if not os.path.exists(path):
            sys.exit(f"Error: no profile named {{name}}; run `bacnet mcp authorize --profile {{name}}`")
        if "authorized_transport" not in open(path).read():
            sys.exit(f"Error: not authorized; run `bacnet mcp authorize --profile {{name}}`")
    here = os.path.dirname(os.path.abspath(__file__))
    tools = json.load(open(os.path.join(here, "..", "manifest.json")))["tools"]
    notes = {{"readOnlyHint": True, "destructiveHint": False}}
    listed = [dict(t, annotations=notes) for t in tools]
    if MODE == "writes":
        listed.append({{"name": "bacnet_write", "annotations": {{"readOnlyHint": False}}}})
    if MODE == "unannotated":
        listed[0].pop("annotations")
    if MODE == "destructive":
        listed[0]["annotations"] = {{"readOnlyHint": True, "destructiveHint": True}}
    for line in sys.stdin:
        message = json.loads(line)
        if "id" not in message:
            continue
        if MODE == "chatty":
            print("hello", flush=True)
        if message["method"] == "initialize":
            result = {{"serverInfo": {{"name": "fake", "version": "0", "title": "fake (read-only)"}}}}
        else:
            result = {{"tools": listed}}
        print(json.dumps({{"jsonrpc": "2.0", "id": message["id"], "result": result}}), flush=True)
''')


class Files:
    """A temporary directory with a license, notices and a stand-in binary."""

    def __init__(self, test, binary=b"\\x7fELF stand-in"):
        tmp = tempfile.TemporaryDirectory()
        test.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)
        self.license = self.dir / "LICENSE"
        self.license.write_text("MIT License\n")
        self.notices = self.dir / "THIRD-PARTY-NOTICES"
        self.notices.write_text("notices\n")
        self.binary = self.dir / "bacnet-macos-arm64"
        self.binary.write_bytes(binary)

    def pack(self, platform="macos-arm64", version="1.2.0", binary=None):
        return mcpb.pack(binary or self.binary, platform, version, self.license, self.notices, self.dir / "out")


def rewrite(path, change):
    """Rewrite a bundle's entries through change(list of (ZipInfo, bytes))."""
    with zipfile.ZipFile(path) as bundle:
        entries = [(info, bundle.read(info)) for info in bundle.infolist()]
    entries = change(entries)
    with zipfile.ZipFile(path, "w") as bundle:
        for info, data in entries:
            bundle.writestr(info, data)


def signature_block(blob=b"\x30\x82 stand-in PKCS#7"):
    """What `mcpb sign` appends: header, little-endian length, signature, footer."""
    return b"MCPB_SIG_V1" + len(blob).to_bytes(4, "little") + blob + b"MCPB_SIG_END"


def declare(unsigned, comment_length):
    """`unsigned` with its end-of-central-directory record's comment length
    set, as a signer with mcpb#278's fix (modelcontextprotocol/mcpb#204)
    raises it to count its block."""
    eocd = unsigned.rfind(b"PK\x05\x06")
    return unsigned[:eocd + 20] + comment_length.to_bytes(2, "little") + unsigned[eocd + 22:]


def edit_manifest(path, edit):
    def change(entries):
        out = []
        for info, data in entries:
            if info.filename == "manifest.json":
                manifest = json.loads(data)
                edit(manifest)
                data = json.dumps(manifest).encode()
            out.append((info, data))
        return out
    rewrite(path, change)


class ManifestTests(unittest.TestCase):
    def test_every_platform_conforms(self):
        for platform, (system, exe) in mcpb.PLATFORMS.items():
            with self.subTest(platform=platform):
                manifest = mcpb.manifest_for("1.2.0", platform)
                self.assertEqual(mcpb.schema_errors(manifest), [])
                self.assertEqual(manifest["manifest_version"], "0.3")
                self.assertEqual(manifest["version"], "1.2.0")
                self.assertEqual(manifest["server"]["entry_point"], "server/bacnet" + exe)
                self.assertEqual(manifest["server"]["mcp_config"]["command"], "${__dirname}/server/bacnet" + exe)
                self.assertEqual(manifest["server"]["mcp_config"]["args"],
                                 ["mcp", "serve", "--stdio", "--profile", "${user_config.profile}"])
                self.assertEqual(manifest["compatibility"], {"platforms": [system]})
                self.assertTrue(manifest["user_config"]["profile"]["required"])

    def test_template_lists_only_read_tools(self):
        names = [t["name"] for t in mcpb.manifest_for("1.2.0", "linux-x86_64")["tools"]]
        self.assertEqual(len(names), 11)
        self.assertFalse(set(names) & mcpb.WRITE_TOOLS)

    def test_schema_refuses_what_0_3_refuses(self):
        good = mcpb.manifest_for("1.2.0", "macos-arm64")
        cases = {
            "lacks author": lambda m: m.pop("author"),
            "unknown field extra": lambda m: m.update(extra=1),
            "expected a boolean": lambda m: m.update(tools_generated="yes"),
            "lacks title": lambda m: m["user_config"]["profile"].pop("title"),
            "is not one of": lambda m: m["compatibility"].update(platforms=["macos"]),
            "unknown field nick": lambda m: m["author"].update(nick="x"),
            "expected a number": lambda m: m["user_config"]["profile"].update(min=True),
            "must be '0.3'": lambda m: m.update(manifest_version="0.4"),
            "is not a URI": lambda m: m.update(homepage="not a uri"),
            "lacks command": lambda m: m["server"]["mcp_config"].pop("command"),
            "matches none": lambda m: m["user_config"]["profile"].update(default={"a": 1}),
        }
        for want, edit in cases.items():
            with self.subTest(want=want):
                manifest = json.loads(json.dumps(good))
                edit(manifest)
                errors = mcpb.schema_errors(manifest)
                self.assertTrue(any(want in e for e in errors), errors)

    def test_bundle_names(self):
        self.assertEqual(mcpb.bundle_name("bacnet-windows-amd64.exe"), "bacnet-windows-amd64.mcpb")
        self.assertEqual(mcpb.bundle_name("bacnet-linux-arm64"), "bacnet-linux-arm64.mcpb")


class PackAndCheckTests(unittest.TestCase):
    def setUp(self):
        self.files = Files(self)

    def check(self, path, platform="macos-arm64", version="1.2.0"):
        return mcpb.check_bundle(path, platform, version, binary=self.files.binary, notices=self.files.notices)

    def test_pack_is_reproducible_and_checks(self):
        with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1793577600"}):
            first = self.files.pack().read_bytes()
            second = self.files.pack()
        self.assertEqual(first, second.read_bytes())
        self.assertEqual(second.name, "bacnet-macos-arm64.mcpb")
        with zipfile.ZipFile(second) as bundle:
            infos = bundle.infolist()
        self.assertEqual([i.filename for i in infos],
                         ["manifest.json", "server/bacnet", "icon.png", "LICENSE", "THIRD-PARTY-NOTICES"])
        self.assertEqual([(i.external_attr >> 16) & 0o777 for i in infos], [0o644, 0o755, 0o644, 0o644, 0o644])
        self.assertEqual(infos[0].date_time, (2026, 11, 2, 0, 0, 0))
        self.assertEqual(self.check(second), [])

    def test_windows_bundle(self):
        exe = self.files.dir / "bacnet-windows-amd64.exe"
        exe.write_bytes(self.files.binary.read_bytes())
        path = self.files.pack("windows-x86_64", binary=exe)
        self.assertEqual(path.name, "bacnet-windows-amd64.mcpb")
        self.assertEqual(mcpb.check_bundle(path, "windows-x86_64", "1.2.0", binary=exe), [])
        self.assertTrue(any("holds" in e for e in mcpb.check_bundle(path, "macos-arm64", "1.2.0")))

    def test_pack_refuses_a_bad_version_or_platform(self):
        with self.assertRaises(ValueError):
            self.files.pack(version="1.2")
        with self.assertRaises(ValueError):
            self.files.pack(platform="freebsd-x86_64")

    def test_check_catches_each_fault(self):
        other = self.files.dir / "other"
        other.write_bytes(b"another build")

        def not_executable(entries):
            for info, _ in entries:
                if info.filename == "server/bacnet":
                    info.external_attr = 0o644 << 16
            return entries

        cases = {
            "differs from bacnet-macos-arm64": lambda p: rewrite(p, lambda e: [
                (i, b"tampered" if i.filename == "server/bacnet" else d) for i, d in e]),
            "THIRD-PARTY-NOTICES differs": lambda p: rewrite(p, lambda e: [
                (i, b"old" if i.filename == "THIRD-PARTY-NOTICES" else d) for i, d in e]),
            "not stored executable": lambda p: rewrite(p, not_executable),
            "holds": lambda p: rewrite(p, lambda e: e + [(zipfile.ZipInfo("../escape"), b"x")]),
            "version 9.9.9": lambda p: edit_manifest(p, lambda m: m.update(version="9.9.9")),
            "compatibility.platforms": lambda p: edit_manifest(p, lambda m: m.update(
                compatibility={"platforms": ["linux"]})),
            "no host substitutes": lambda p: edit_manifest(p, lambda m: m["server"]["mcp_config"]["args"].append(
                "${user_config.network}")),
            "user_config.profile must be required": lambda p: edit_manifest(
                p, lambda m: m["user_config"]["profile"].update(required=False)),
            "no write tool": lambda p: edit_manifest(p, lambda m: m["tools"].append({"name": "bacnet_write"})),
            "must start mcp serve --stdio": lambda p: edit_manifest(
                p, lambda m: m["server"]["mcp_config"].update(args=["tui"])),
            "unknown field": lambda p: edit_manifest(p, lambda m: m.update(signed=True)),
        }
        for want, fault in cases.items():
            with self.subTest(want=want):
                path = self.files.pack()
                fault(path)
                errors = self.check(path)
                self.assertTrue(any(want in e for e in errors), errors)
        path = self.files.pack()
        path.write_bytes(b"not a zip")
        self.assertTrue(any("not a zip" in e for e in self.check(path)))

    def test_the_icon_is_the_512_png(self):
        path = self.files.pack()
        with zipfile.ZipFile(path) as bundle:
            icon = bundle.read("icon.png")
            manifest = json.loads(bundle.read("manifest.json"))
        self.assertEqual(icon, mcpb.ICON_FILE.read_bytes())
        self.assertEqual(manifest["icon"], "icon.png")
        self.assertEqual(mcpb.png_errors(icon), [])
        self.assertEqual(self.check(path), [])

    def test_check_refuses_a_wrong_or_missing_icon(self):
        good = mcpb.ICON_FILE.read_bytes()
        # The same PNG declaring another size in its IHDR.
        small = good[:16] + (256).to_bytes(4, "big") * 2 + good[24:]
        cases = {
            "is 256x256, not 512x512": lambda e: [(i, small if i.filename == "icon.png" else d) for i, d in e],
            "icon.png is not a PNG": lambda e: [(i, b"GIF89a" if i.filename == "icon.png" else d) for i, d in e],
            "holds": lambda e: [(i, d) for i, d in e if i.filename != "icon.png"],
        }
        for want, change in cases.items():
            with self.subTest(want=want):
                path = self.files.pack()
                rewrite(path, change)
                errors = self.check(path)
                self.assertTrue(any(want in e for e in errors), errors)
        path = self.files.pack()
        edit_manifest(path, lambda m: m.pop("icon"))
        self.assertTrue(any("icon must be 'icon.png'" in e for e in self.check(path)))

    def test_a_signed_bundle_checks_without_its_block(self):
        for comment in (b"", b"release 1.2.0"):  # a real ZipFile.comment too
            with self.subTest(comment=comment):
                path = self.files.pack()
                if comment:
                    with zipfile.ZipFile(path, "a") as bundle:
                        bundle.comment = comment
                unsigned = path.read_bytes()
                self.assertEqual(unsigned[-len(comment) - 2:], len(comment).to_bytes(2, "little") + comment)
                digest = hashlib.sha256(unsigned).hexdigest()
                self.assertEqual(mcpb.signature(path), ("unsigned", digest))
                block = signature_block()
                for layout, signed in (("2.1.2", unsigned + block),
                                       ("#204", declare(unsigned, len(comment) + len(block)) + block)):
                    with self.subTest(layout=layout):
                        path.write_bytes(signed)
                        self.assertEqual(mcpb.signature(path), ("signed", digest))
                        self.assertEqual(self.check(path), [])
                        # Only the #204 layout declares the block, as a strict reader needs.
                        self.assertEqual(mcpb.trailer_errors(signed) == [], layout == "#204")

    def test_a_raised_comment_length_is_the_only_change_forgiven(self):
        unsigned = self.files.pack().read_bytes()
        block = signature_block()
        signed = declare(unsigned, len(block))
        eocd = unsigned.rfind(b"PK\x05\x06")
        self.assertLess(len(unsigned), 16384)  # small enough to change every byte
        # Every byte of the bundle under the block, changed in turn, shows in
        # the content, except the comment length, which is put back and which
        # the trailer check then refuses.
        forgiven = []
        for at in range(len(signed)):
            changed = bytearray(signed)
            changed[at] ^= 0x01
            content, state = mcpb.split_signature(bytes(changed) + block)
            self.assertEqual(state, "signed")
            if content == unsigned:
                forgiven.append(at)
                self.assertTrue(mcpb.trailer_errors(bytes(changed) + block))
        self.assertEqual(forgiven, [eocd + 20, eocd + 21])

    def test_a_strict_reader_refuses_an_undeclared_trailing_block(self):
        unsigned = self.files.pack().read_bytes()
        self.assertEqual(mcpb.trailer_errors(unsigned), [])
        block = signature_block(b"stand-in")
        # As mcpb sign 2.1.2 appends it (mcpb#278): the record still declares no comment.
        errors = mcpb.trailer_errors(unsigned + block)
        self.assertTrue(any(f"has {len(block)} bytes after" in e and "declares 0" in e for e in errors), errors)
        # Declared in the comment-length field, a strict reader takes it.
        self.assertEqual(mcpb.trailer_errors(declare(unsigned, len(block)) + block), [])
        self.assertTrue(mcpb.trailer_errors(b"not a zip"))
        # The last record counts: an earlier record magic (say, inside an
        # entry) changes nothing.
        earlier = b"PK\x05\x06" + bytes(40)
        self.assertEqual(mcpb.trailer_errors(earlier + unsigned), [])
        self.assertTrue(mcpb.trailer_errors(earlier + unsigned + block))
        signed = earlier + declare(unsigned, len(block)) + block
        self.assertEqual(mcpb.trailer_errors(signed), [])
        self.assertEqual(mcpb.split_signature(signed), (earlier + unsigned, "signed"))
        # A record cut short is no record, and isn't rewritten.
        for cut in (1, 5, 21):
            with self.subTest(cut=cut):
                self.assertEqual(mcpb.trailer_errors(unsigned[:-cut]), ["has no end-of-central-directory record"])
                self.assertEqual(mcpb._undeclare(unsigned[:-cut]), unsigned[:-cut])
        path = self.files.dir / "trailer.mcpb"
        path.write_bytes(unsigned + block)
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(mcpb.main(["trailer", str(path)]), 1)
            path.write_bytes(unsigned)
            self.assertEqual(mcpb.main(["trailer", str(path)]), 0)

    def test_a_malformed_signature_block_is_reported(self):
        unsigned = self.files.pack().read_bytes()
        path = self.files.dir / "out" / "bacnet-macos-arm64.mcpb"
        blob = b"stand-in"
        for tail in (b"MCPB_SIG_END",  # a footer with no header
                     b"MCPB_SIG_V1" + (len(blob) + 1).to_bytes(4, "little") + blob + b"MCPB_SIG_END",
                     b"MCPB_SIG_V1\x01MCPB_SIG_END"):  # no room for the length
            with self.subTest(tail=tail):
                path.write_bytes(unsigned + tail)
                self.assertEqual(mcpb.signature(path)[0], "malformed")
                self.assertTrue(any("malformed signature block" in e for e in self.check(path)))
        # Markers inside the bundle, not at its end, are no signature.
        path.write_bytes(unsigned + b"MCPB_SIG_V1 trailing")
        self.assertEqual(mcpb.signature(path)[0], "unsigned")

    def test_extract_refuses_a_path_out_of_the_bundle(self):
        path = self.files.pack()
        rewrite(path, lambda e: e + [(zipfile.ZipInfo("../escape"), b"x")])
        with self.assertRaises(ValueError):
            mcpb.extract(path, self.files.dir / "x")

    def test_command_fills_the_profile(self):
        manifest = mcpb.manifest_for("1.2.0", "linux-x86_64")
        argv = mcpb.command(manifest, "/opt/b", "plant-a")
        self.assertEqual(argv, ["/opt/b/server/bacnet", "mcp", "serve", "--stdio", "--profile", "plant-a"])
        manifest["server"]["mcp_config"]["args"].append("${user_config.other}")
        with self.assertRaises(ValueError):
            mcpb.command(manifest, "/opt/b", "plant-a")


class AdoptTests(unittest.TestCase):
    """mcpb.py adopt: what the GitHub draft takes from the signing job."""

    def setUp(self):
        self.files = Files(self)
        self.release = self.files.dir / "release"
        self.bundles = self.files.dir / "bundles"
        self.release.mkdir()
        self.bundles.mkdir()
        for platform in ("macos-arm64", "linux-x86_64"):
            path = self.files.pack(platform=platform)
            (self.release / path.name).write_bytes(path.read_bytes())
            (self.bundles / path.name).write_bytes(path.read_bytes())
        (self.release / "rusty_bacnet-1.2.0-cp312-cp312-win_amd64.whl").write_bytes(b"wheel")
        self.before = {p.name: p.read_bytes() for p in self.release.iterdir()}
        self.mac = "bacnet-macos-arm64.mcpb"

    def adopt(self):
        return mcpb.adopt(self.release, self.bundles)

    def unchanged(self):
        self.assertEqual({p.name: p.read_bytes() for p in self.release.iterdir()}, self.before)

    def test_unsigned_bundles_change_nothing(self):
        self.assertEqual(self.adopt(), ([], []))
        self.unchanged()

    def test_a_correctly_signed_bundle_is_taken(self):
        block = signature_block()
        signed = declare(self.before[self.mac], len(block)) + block
        (self.bundles / self.mac).write_bytes(signed)
        self.assertEqual(self.adopt(), ([], [self.mac]))
        self.assertEqual((self.release / self.mac).read_bytes(), signed)
        self.before[self.mac] = signed
        self.unchanged()  # nothing else changed

    def test_each_fault_changes_nothing(self):
        block = signature_block()
        original = self.before[self.mac]
        faults = {
            "2.1.2 layout": (lambda: (self.bundles / self.mac).write_bytes(original + block), "strict zip reader"),
            "changed": (lambda: (self.bundles / self.mac).write_bytes(original[:-1] + b"\x01"),
                        "neither the release's bundle nor that bundle signed"),
            "changed, then signed": (lambda: (self.bundles / self.mac).write_bytes(
                b"Q" + declare(original, len(block))[1:] + block), "neither"),
            "malformed block": (lambda: (self.bundles / self.mac).write_bytes(original + b"MCPB_SIG_END"), "neither"),
            "added bundle": (lambda: (self.bundles / "bacnet-extra.mcpb").write_bytes(original),
                             "not one of the release's bundles"),
            "missing bundle": (lambda: (self.bundles / self.mac).unlink(), "is missing"),
            "a wheel": (lambda: (self.bundles / "rusty_bacnet-1.2.0-cp312-cp312-win_amd64.whl").write_bytes(b"other"),
                        "only MCP bundles"),
            "a directory": (lambda: (self.bundles / "x.mcpb").mkdir(), "only MCP bundles"),
            "a link": (lambda: (self.bundles / self.mac).unlink() or (self.bundles / self.mac).symlink_to(
                self.release / self.mac), "only MCP bundles"),
        }
        for label, (fault, want) in faults.items():
            with self.subTest(fault=label):
                for folder in (self.release, self.bundles):
                    shutil.rmtree(folder)
                    folder.mkdir()
                for name, data in self.before.items():
                    (self.release / name).write_bytes(data)
                    if name.endswith(".mcpb"):
                        (self.bundles / name).write_bytes(data)
                fault()
                errors, signed = self.adopt()
                self.assertTrue(any(want in e for e in errors), errors)
                self.assertEqual(signed, [])
                self.unchanged()

    def test_a_signed_original_or_no_bundles_fails(self):
        signed = self.before[self.mac] + signature_block()
        (self.release / self.mac).write_bytes(signed)
        (self.bundles / self.mac).write_bytes(signed + b"x")
        self.assertTrue(any("not unsigned" in e for e in mcpb.adopt_errors(self.release, self.bundles)))
        self.assertTrue(mcpb.adopt_errors(self.release, self.files.dir / "absent"))
        empty = self.files.dir / "empty"
        empty.mkdir()
        self.assertTrue(any("holds no MCP bundle" in e for e in mcpb.adopt_errors(empty, self.bundles)))

    def test_main(self):
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(mcpb.main(["adopt", str(self.release), str(self.bundles)]), 0)
            self.assertIn("OK: 0 signed", out.getvalue())
            (self.bundles / "extra.txt").write_text("x")
            self.assertEqual(mcpb.main(["adopt", str(self.release), str(self.bundles)]), 1)


class CheckArtifactsBundleTests(unittest.TestCase):
    def test_bundles_against_their_cli(self):
        files = Files(self)
        path = files.pack(binary=files.binary)
        directory = path.parent
        (directory / files.binary.name).write_bytes(files.binary.read_bytes())
        clis = {"bacnet-macos-arm64": "macosx_11_0_arm64"}
        spec = ["bacnet-macos-arm64.mcpb=bacnet-macos-arm64"]
        self.assertEqual(check_artifacts.check_bundles(directory, spec, clis, "1.2.0", files.notices), [])
        errors = check_artifacts.check_bundles(directory, spec, clis, "1.3.0")
        self.assertTrue(any("the release 1.3.0" in e for e in errors), errors)
        (directory / "bacnet-extra.mcpb").write_bytes(b"")
        errors = check_artifacts.check_bundles(directory, spec + ["gone.mcpb=bacnet-macos-arm64"], clis, "1.2.0")
        self.assertIn("unexpected MCP bundle bacnet-extra.mcpb", errors)
        self.assertIn("missing MCP bundle gone.mcpb", errors)
        errors = check_artifacts.check_bundles(directory, ["x.mcpb=bacnet-nope"], clis, "1.2.0")
        self.assertTrue(any("not one of the --cli binaries" in e for e in errors), errors)

    def test_bundles_are_compared_with_the_cli_and_notices(self):
        files = Files(self)
        directory = files.pack().parent
        cli = directory / files.binary.name
        clis = {cli.name: "macosx_11_0_arm64"}
        spec = [f"bacnet-macos-arm64.mcpb={cli.name}"]
        cli.write_bytes(b"another build of the CLI")
        errors = check_artifacts.check_bundles(directory, spec, clis, "1.2.0", files.notices)
        self.assertIn("bacnet-macos-arm64.mcpb: server/bacnet differs from bacnet-macos-arm64", errors)
        cli.write_bytes(files.binary.read_bytes())
        other = files.dir / "OTHER-NOTICES"
        other.write_text("other notices\n")
        errors = check_artifacts.check_bundles(directory, spec, clis, "1.2.0", other)
        self.assertTrue(any("THIRD-PARTY-NOTICES differs" in e for e in errors), errors)
        cli.unlink()
        errors = check_artifacts.check_bundles(directory, spec, clis, "1.2.0", files.notices)
        self.assertTrue(any("is missing, so it can't be compared" in e for e in errors), errors)
        self.assertTrue(any("no " in e for e in mcpb.check_bundle(
            directory / "bacnet-macos-arm64.mcpb", "macos-arm64", "1.2.0", binary=cli, notices=other.with_name("x"))))

    def test_wheel_smoke_loads_check_artifacts_alone(self):
        # The wheel smoke runs under -I and loads check_artifacts.py by path,
        # with this directory off sys.path: it must import nothing local.
        script = Path(__file__).with_name("wheel_smoke.py")
        code = ("import importlib.util, sys; spec = importlib.util.spec_from_file_location('w', sys.argv[1]); "
                "module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module); "
                "print(module.pep440('1.2.0-rc.1'))")
        with tempfile.TemporaryDirectory() as tmp:
            done = subprocess.run([sys.executable, "-I", "-c", code, str(script)], cwd=tmp,
                                  capture_output=True, text=True, check=False)
        self.assertEqual((done.returncode, done.stdout.strip()), (0, "1.2.0rc1"), done.stderr)

    def test_release_platform(self):
        cases = {"linux_x86_64": "linux-x86_64", "linux_aarch64": "linux-aarch64",
                 "macosx_10_12_x86_64": "macos-x86_64", "macosx_11_0_arm64": "macos-arm64",
                 "win_amd64": "windows-x86_64"}
        for tag, platform in cases.items():
            with self.subTest(tag=tag):
                self.assertEqual(check_artifacts.release_platform(tag), platform)
                self.assertIn(platform, mcpb.PLATFORMS)


@unittest.skipIf(os.name == "nt", "the stand-in server is a script with a #! line")
class SmokeTests(unittest.TestCase):
    PLATFORM = {"darwin": "macos-arm64", "linux": "linux-x86_64"}.get(sys.platform)

    def bundle(self, mode):
        files = Files(self, FAKE.format(mode=mode).encode())
        return files.pack(platform=self.PLATFORM)

    def smoke(self, mode, timeout=5.0):
        with contextlib.redirect_stdout(io.StringIO()) as shown:
            mcpb.smoke(self.bundle(mode), timeout=timeout)
        return shown.getvalue()

    def test_a_read_only_server_passes(self):
        shown = self.smoke("good")
        self.assertIn("profile unauthorized: refused at start", shown)
        self.assertIn("profile missing: refused at start", shown)
        self.assertIn("11 read-only tools", shown)

    def test_each_fault_fails(self):
        cases = {"writes": "tools/list gave", "unannotated": "not annotated read-only",
                 "destructive": "not annotated read-only",
                 "chatty": "not JSON-RPC", "ignores-consent": "kept running"}
        for mode, want in cases.items():
            with self.subTest(mode=mode):
                with self.assertRaises(mcpb.SmokeError) as caught, contextlib.redirect_stdout(io.StringIO()):
                    self.smoke(mode, timeout=2.0)
                self.assertIn(want, str(caught.exception))

    def test_another_platforms_bundle_fails(self):
        files = Files(self, FAKE.format(mode="good").encode())
        path = files.pack(platform="windows-x86_64")
        with self.assertRaises(mcpb.SmokeError) as caught:
            mcpb.smoke(path, timeout=2.0)
        self.assertIn("is for ['win32']", str(caught.exception))

    def test_the_unauthorized_profile_drops_only_the_consent(self):
        text = mcpb.SMOKE_PROFILE.read_text(encoding="utf-8")
        self.assertIn("authorized_transport", text)
        bare = mcpb.unauthorized(text)
        self.assertNotIn("authorized", bare.replace("# ", "").split("version = 1")[1])
        self.assertIn("[mcp]\nnetworks = [0]\n", bare)


if __name__ == "__main__":
    unittest.main()
