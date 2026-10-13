#!/usr/bin/env python3
"""Subprocess guard/command controls; these do not qualify Rust or real ELF links."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("check-msrv.sh")
# The no_std builds' run, after the baseline (#1613).
NO_STD_ALLOC = ["-p", "bacnet-types", "-p", "bacnet-encoding", "-p", "bacnet-services",
                "--no-default-features", "--features", "alloc"]
DRIVER = r'''
import json, os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
args = sys.argv[1:]
root = Path(os.environ["FIXTURE_ROOT"])
host = "aarch64-unknown-linux-gnu"
if name == "uname":
    print(os.environ.get("MOCK_OS", "Linux") if args == ["-s"] else "aarch64")
elif name == "rustup":
    if args == ["toolchain", "list"]:
        print("1.99.0-" + host if os.environ.get("MOCK_NO_TOOLCHAIN") else "1.93-" + host)
    else:
        assert args[:3] == ["which", "--toolchain", "1.93"], args
        print(root / "bin" / args[-1])
elif name == "rustc":
    print("release: " + os.environ.get("MOCK_RUST_VERSION", "1.93.0"))
    print("host: " + os.environ.get("MOCK_HOST", host))
elif name == "cargo":
    if args == ["--version"]:
        print("cargo " + os.environ.get("MOCK_CARGO_VERSION", "1.93.0") + " (fixture)")
    elif args[0] == "metadata":
        print(json.dumps({"packages": [
            {"name": "eligible", "publish": None},
            {"name": "newly-eligible", "publish": ["custom-registry"]},
            {"name": "private", "publish": []}]}))
    else:
        with (root / "commands.jsonl").open("a") as out:
            out.write(json.dumps({"args": args, "env": {key: os.environ.get(key) for key in [
                "RUSTC", "CARGO_BUILD_RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"]}}) + "\n")
        if os.environ.get("MOCK_FAIL_FEATURE") in args:
            sys.exit(5)
        if args[0] == "build":
            executable = Path(os.environ["CARGO_TARGET_DIR"]) / host / "debug" / "bacnet"
            executable.parent.mkdir(parents=True, exist_ok=True)
            executable.write_text("fixture, not an actual ELF\n")
            executable.chmod(0o755)
            artifact = {"reason": "compiler-artifact", "manifest_path": str(root / "crates/bacnet-cli/Cargo.toml"),
                "target": {"name": "bacnet", "kind": ["bin"]}, "executable": str(executable)}
            mode = os.environ.get("MOCK_ARTIFACT", "valid")
            if mode == "other-manifest": artifact["manifest_path"] = str(root / "other/Cargo.toml")
            if mode == "other-bin": artifact["target"]["name"] = "other"
            if mode != "absent": print(json.dumps(artifact))
            if mode == "duplicate": print(json.dumps(artifact))
            print(json.dumps({"reason": "build-finished", "success": mode != "failed"}))
            if mode == "bad-exit": sys.exit(5)
elif name == "pkg-config":
    if os.environ.get("MOCK_NO_PCAP"): sys.exit(1)
    if args == ["--modversion", "libpcap"]: print("1.10.fixture")
elif name == "file":
    print(os.environ.get("MOCK_FILE", "ELF 64-bit LSB pie executable, ARM aarch64, dynamically linked"))
elif name == "ldd":
    print(os.environ.get("MOCK_LDD", "libpcap.so.0.8 => /lib/libpcap.so.0.8 (0x1234)"))
    sys.exit(int(os.environ.get("MOCK_LDD_EXIT", "0")))
'''


class MsrvScriptTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="msrv-controls-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        script = self.root / "scripts/ci/check-msrv.sh"
        script.parent.mkdir(parents=True)
        shutil.copyfile(SCRIPT, script)
        self.script = script
        self.bin = self.root / "bin"
        self.bin.mkdir()
        # A closed PATH makes prerequisite absence tests portable across hosts.
        for name in ["dirname", "sort", "grep", "sed", "mktemp", "rm", "cat"]:
            (self.bin / name).symlink_to(shutil.which(name))
        (self.bin / "python3").symlink_to(sys.executable)
        for name in ["uname", "rustup", "rustc", "cargo", "pkg-config", "cc", "cmake", "perl", "file", "ldd"]:
            tool = self.bin / name
            tool.write_text(f"#!{sys.executable}\n" + DRIVER)
            tool.chmod(0o755)
        self.env = {"PATH": str(self.bin), "FIXTURE_ROOT": str(self.root),
                    "RUSTUP_TOOLCHAIN": "1.93", "CARGO_TARGET_DIR": str(self.root / "custom target")}

    def run_script(self, *args, **env):
        return subprocess.run(["/bin/bash", str(self.script), *args], env=self.env | env,
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    def commands(self):
        path = self.root / "commands.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def reject(self, expected, **env):
        result = self.run_script("--linux-native", **env)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(expected, result.stdout)
        self.assertEqual(self.commands(), [], "prerequisite failure must precede all builds")

    def test_default_retains_metadata_eligibility_and_feature_baseline(self):
        # No native-only prerequisite is allowed to leak into default mode.
        (self.bin / "rustup").unlink()
        (self.bin / "ldd").unlink()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stdout)
        commands = self.commands()
        self.assertEqual(len(commands), 2)
        self.assertEqual(commands[0]["args"], ["check", "--locked", "-p", "eligible", "-p", "newly-eligible",
            "--features", "bacnet-transport/sc-tls,bacnet-transport/ipv6,bacnet-client/sc-tls,bacnet-client/ipv6,bacnet-server/sc-tls,bacnet-cli/sc-tls,bacnet-cli/tui,bacnet-mstp-codec/bytes,bacnet-mstp-codec/heapless,bacnet-mstp-core/alloc,bacnet-bip-codec/bytes,bacnet-bip-codec/heapless,bacnet-router-core/bip,bacnet-router-core/mstp"])
        self.assertEqual(commands[1]["args"], ["check", "--locked", *NO_STD_ALLOC])

    def test_native_sequence_compiler_target_and_nondefault_artifact(self):
        result = self.run_script("--linux-native")
        self.assertEqual(result.returncode, 0, result.stdout)
        commands = self.commands()
        self.assertEqual(len(commands), 6)
        for command in commands:
            self.assertIn("--locked", command["args"])
            index = command["args"].index("--target")
            self.assertEqual(command["args"][index + 1], "aarch64-unknown-linux-gnu")
            for key in ["RUSTC", "CARGO_BUILD_RUSTC"]:
                self.assertEqual(command["env"][key], str(self.bin / "rustc"))
            for key in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"]:
                self.assertEqual(command["env"][key], "")
        self.assertEqual(commands[1]["args"], ["check", "--locked", *NO_STD_ALLOC, "--target", "aarch64-unknown-linux-gnu"])
        for command, feature in zip(commands[2:5], ["ethernet", "serial", "serial-gpio"]):
            self.assertEqual(command["args"][:6], ["check", "--locked", "-p", "bacnet-transport", "--features", feature])
        self.assertEqual(commands[5]["args"][:8], ["build", "--locked", "-p", "bacnet-cli", "--bin", "bacnet", "--features", "pcap"])
        self.assertIn("Cargo executable: " + self.env["CARGO_TARGET_DIR"], result.stdout)
        self.assertIn("OK: Linux native MSRV", result.stdout)

    def test_unsupported_host(self): self.reject("requires a native Linux", MOCK_OS="Darwin")
    def test_wrong_rust(self): self.reject("rustc must be stable 1.93", MOCK_RUST_VERSION="1.99.0")
    def test_wrong_cargo(self): self.reject("cargo must be stable 1.93", MOCK_CARGO_VERSION="1.99.0")
    def test_missing_toolchain(self): self.reject("is not installed", MOCK_NO_TOOLCHAIN="1")
    def test_musl_compiler(self): self.reject("unsupported or non-native", MOCK_HOST="aarch64-unknown-linux-musl")
    def test_cross_arch_compiler(self): self.reject("unsupported or non-native", MOCK_HOST="x86_64-unknown-linux-gnu")
    def test_cross_target(self): self.reject("CARGO_BUILD_TARGET conflicts", CARGO_BUILD_TARGET="wasm32-unknown-unknown")
    def test_conflicting_rustc(self): self.reject("conflicting RUSTC", RUSTC="/other/rustc")
    def test_conflicting_build_rustc(self): self.reject("conflicting CARGO_BUILD_RUSTC", CARGO_BUILD_RUSTC="/other/rustc")
    def test_wrapper(self):
        for key in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"]:
            with self.subTest(key=key):
                self.reject("compiler wrapper", **{key: "/wrapper"})
    def test_missing_pcap(self): self.reject("libpcap development package", MOCK_NO_PCAP="1")

    def test_missing_tool(self):
        (self.bin / "ldd").unlink()
        self.reject("missing prerequisite: ldd")

    def test_failed_feature_stops_remaining_builds(self):
        result = self.run_script("--linux-native", MOCK_FAIL_FEATURE="serial")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.commands()), 4)

    def test_failed_no_std_alloc_check_fails_both_modes(self):
        for args in [(), ("--linux-native",)]:
            with self.subTest(args=args):
                (self.root / "commands.jsonl").unlink(missing_ok=True)
                result = self.run_script(*args, MOCK_FAIL_FEATURE="alloc")
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertNotIn("OK: Linux native MSRV", result.stdout)
                self.assertEqual(len(self.commands()), 2)

    def test_artifact_requires_success_and_exact_unique_target(self):
        for mode in ["absent", "other-manifest", "other-bin", "duplicate", "failed", "bad-exit"]:
            with self.subTest(mode=mode):
                result = self.run_script("--linux-native", MOCK_ARTIFACT=mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertNotIn("OK: Linux native MSRV", result.stdout)

    def test_non_elf(self):
        result = self.run_script("--linux-native", MOCK_FILE="Mach-O 64-bit executable arm64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a native ELF", result.stdout)

    def test_successful_ldd_with_unresolved_or_absent_pcap_is_rejected(self):
        # Semantic output controls only, not an actual missing-library runtime.
        for output in ["libpcap.so.0.8 => not found", "libc.so.6 => /lib/libc.so.6 (0x1234)"]:
            with self.subTest(output=output):
                result = self.run_script("--linux-native", MOCK_LDD=output)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertNotIn("OK: Linux native MSRV", result.stdout)

    def test_ldd_failure_is_rejected(self):
        result = self.run_script("--linux-native", MOCK_LDD_EXIT="1")
        self.assertNotEqual(result.returncode, 0)

    def test_unknown_argument(self):
        result = self.run_script("--typo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("usage:", result.stdout)
        self.assertEqual(self.commands(), [])


if __name__ == "__main__":
    unittest.main()
