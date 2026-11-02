#!/usr/bin/env python3
"""Unit tests for mcpb.py and check_artifacts.py's bundle check (#2304), with
stand-in executables: python3 -m unittest discover -s scripts/release"""

import contextlib
import io
import json
import os
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
                         ["manifest.json", "server/bacnet", "LICENSE", "THIRD-PARTY-NOTICES"])
        self.assertEqual([(i.external_attr >> 16) & 0o777 for i in infos], [0o644, 0o755, 0o644, 0o644])
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
