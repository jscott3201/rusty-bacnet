"""Offline installed-extension reports, with independent OpenSSL/hashlib inputs."""
import errno
import hashlib
import importlib.util
import ipaddress
import io
import math
import os
import pickle
import ssl
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from rusty_bacnet import (
    ScPemInspection, ScInspectedCertificate, ScInspectionFinding,
    ScExtensionInspection, ScKeyFilePermissions, inspect_sc_pem, inspect_sc_pem_files,
)


def assert_unix_second_sample(test, before, observed, after, resolution):
    # CPython 3.12 on Windows can read a coarser clock than Rust. Account
    # for that tick and float rounding, then intersect with the sampled
    # integer second's [observed, observed + 1) interval.
    tolerance = resolution + 2 * max(
        math.ulp(before), math.ulp(observed), math.ulp(after))
    test.assertLess(before - tolerance, observed + 1)
    test.assertLessEqual(observed, after + tolerance)


class ScPemInspectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-offline-pem-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)

        def run(*args):
            return subprocess.run(["openssl", *args], cwd=cls.root, check=True,
                                  capture_output=True, timeout=15).stdout
        cls.openssl = staticmethod(run)
        for name in ("identity", "other"):
            run("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-days", "25", "-subj", f"/CN={name}",
                "-keyout", f"{name}.key", "-out", f"{name}.pem",
                "-addext", "keyUsage=critical,digitalSignature",
                "-addext", "extendedKeyUsage=serverAuth,clientAuth",
                "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1")
        cls.identity = (cls.root / "identity.pem").read_bytes()
        cls.key = (cls.root / "identity.key").read_bytes()
        cls.trust = (cls.root / "other.pem").read_bytes()
        cls.der = ssl.PEM_cert_to_DER_cert(cls.identity.decode("ascii"))
        dates = run("x509", "-in", "identity.pem", "-noout", "-dates").decode().splitlines()
        cls.before, cls.after = (int(ssl.cert_time_to_seconds(line.split("=", 1)[1])) for line in dates)

    def report(self, **kwargs):
        return inspect_sc_pem(self.identity, private_key_pem=self.key,
                              inspected_at_unix=self.before, **kwargs)

    def test_bytes_and_file_reports_match_independent_metadata(self):
        report = self.report(peer_trust_pem=self.trust)
        disk = inspect_sc_pem_files(str(self.root / "identity.pem"),
                                   peer_trust_path=self.root / "other.pem",
                                   private_key_path=self.root / "identity.key",
                                   inspected_at_unix=self.before)
        self.assertIsInstance(report, ScPemInspection)
        self.assertEqual(report.inspected_at_unix, self.before)
        self.assertTrue(report.peer_trust_supplied)
        self.assertEqual(report.key_consistency, "match")
        self.assertIsNone(report.key_file_permissions)
        self.assertEqual([c.sha256 for c in report.certificates], [c.sha256 for c in disk.certificates])
        self.assertEqual([(c.role, c.index) for c in report.certificates], [("identity", 0), ("peer_trust", 0)])
        cert = report.certificates[0]
        self.assertIsInstance(cert, ScInspectedCertificate)
        self.assertEqual(cert.sha256, hashlib.sha256(self.der).digest())
        self.assertEqual(cert.metadata.subject, "CN=identity")
        self.assertEqual(cert.metadata.issuer, "CN=identity")
        self.assertEqual((cert.metadata.not_before_unix, cert.metadata.not_after_unix), (self.before, self.after))
        self.assertEqual(cert.metadata.public_key_algorithm_oid, "1.2.840.10045.2.1")
        self.assertEqual(cert.metadata.signature_algorithm_oid, "1.2.840.10045.4.3.2")
        self.assertEqual((cert.validity, cert.expiry), ("within_interval", "warning"))
        self.assertIsInstance(cert.san, ScExtensionInspection)
        self.assertTrue(cert.san.assessed and cert.san.present)
        self.assertEqual(cert.san.values, ("DNS:localhost", "IP:127.0.0.1"))
        self.assertEqual(cert.key_usage.values, ("digital_signature",))
        self.assertEqual(cert.extended_key_usage.values, ("server_auth", "client_auth"))
        self.assertEqual(cert.findings, ())

    def test_subject_alt_names_keep_the_certificate_text(self):
        # #2022: exact text, no case folding, wildcards as written; reprs
        # name the kind only.
        self.openssl(
            "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
            "-nodes", "-days", "25", "-subj", "/CN=sans",
            "-keyout", "sans.key", "-out", "sans.pem",
            "-addext", "subjectAltName=DNS:*.Example.COM,URI:urn:site:ahu-1,"
                       "email:ops@example.com,IP:10.0.0.7,IP:2001:db8::1,DNS:localhost")
        report = inspect_sc_pem((self.root / "sans.pem").read_bytes(),
                                private_key_pem=(self.root / "sans.key").read_bytes(),
                                inspected_at_unix=self.before)
        names = report.certificates[0].metadata.subject_alt_names
        self.assertIsInstance(names, tuple)
        self.assertEqual([(name.kind, name.value) for name in names], [
            ("dns", "*.Example.COM"), ("uri", "urn:site:ahu-1"), ("email", "ops@example.com"),
            ("ip", ipaddress.IPv4Address("10.0.0.7")), ("ip", ipaddress.IPv6Address("2001:db8::1")),
            ("dns", "localhost"),
        ])
        self.assertEqual(repr(names[0]), "ScSubjectAltName(kind='dns')")
        for name in names:
            self.assertNotIn(str(name.value), repr(name))
            with self.assertRaises(AttributeError):
                name.value = "changed"
            with self.assertRaises(TypeError):
                pickle.dumps(name)
        identity = self.report().certificates[0].metadata.subject_alt_names
        self.assertEqual([(name.kind, name.value) for name in identity],
                         [("dns", "localhost"), ("ip", ipaddress.IPv4Address("127.0.0.1"))])

    def test_nested_values_are_frozen_independent_and_reprs_omit_credentials(self):
        source, key = bytearray(self.identity), bytearray(self.key)
        report = inspect_sc_pem(source, private_key_pem=key, inspected_at_unix=self.before)
        source[:] = b"changed"
        key[:] = b"changed"
        cert = report.certificates[0]
        for obj, attribute, value in [(report, "key_consistency", "mismatch"),
                                     (cert, "sha256", b"changed"), (cert.san, "present", False),
                                     (cert.metadata, "subject", "changed")]:
            with self.assertRaises(AttributeError):
                setattr(obj, attribute, value)
            self.assertNotIn("CN=", repr(obj))
            self.assertNotIn(self.key.decode(), repr(obj))
            with self.assertRaises(TypeError):
                pickle.dumps(obj)
        self.assertIsInstance(report.certificates, tuple)
        self.assertIsInstance(cert.findings, tuple)
        self.assertIsInstance(cert.san.values, tuple)
        self.assertEqual(cert.sha256, hashlib.sha256(self.der).digest())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "identity.pem"
            path.write_bytes(self.identity)
            retained = inspect_sc_pem_files(path, inspected_at_unix=self.before)
            path.unlink()
        self.assertEqual(retained.certificates[0].metadata.subject, "CN=identity")

    def test_bytearray_subclass_keeps_native_contents_and_bypasses_buffer_override(self):
        for advertised in (self.trust, bytearray(self.trust), None):
            with self.subTest(advertised_type=type(advertised).__name__):
                calls = []

                class Redirected(bytearray):
                    def __buffer__(self, flags):
                        calls.append(flags)
                        if advertised is None:
                            raise RuntimeError("untrusted buffer hook")
                        return memoryview(advertised)

                value = Redirected(self.identity)
                try:
                    report = inspect_sc_pem(value, inspected_at_unix=self.before)
                    self.assertEqual(report.certificates[0].sha256, hashlib.sha256(self.der).digest())
                    self.assertEqual(calls, [])
                finally:
                    value.clear()

    def test_bytearray_subclass_cannot_hide_oversized_native_contents(self):
        advertised = self.identity
        calls = []

        class Redirected(bytearray):
            def __buffer__(self, flags):
                calls.append(flags)
                return memoryview(advertised)

        value = Redirected(b" " * (1024 * 1024 + 1))
        try:
            with self.assertRaisesRegex(ValueError, "input_too_large"):
                inspect_sc_pem(value)
            self.assertEqual(calls, [])
        finally:
            value.clear()

    def test_bytearray_release_callback_cannot_change_the_owned_snapshot(self):
        calls = []
        unraisable = []

        class Releases(bytearray):
            def __buffer__(self, flags):
                raise AssertionError("acquisition override must not run")

            def __release_buffer__(self, view):
                calls.append("released")
                self[0] = ord("!")
                raise RuntimeError("release callback fixture")

        value = Releases(self.identity)
        original_hook = sys.unraisablehook
        sys.unraisablehook = unraisable.append
        try:
            report = inspect_sc_pem(value, inspected_at_unix=self.before)
        finally:
            sys.unraisablehook = original_hook
            value.clear()
        self.assertEqual(report.certificates[0].sha256, hashlib.sha256(self.der).digest())
        if sys.version_info >= (3, 12):
            self.assertEqual(calls, ["released"])
            self.assertEqual(len(unraisable), 1)
            self.assertIs(unraisable[0].exc_type, RuntimeError)
        else:
            self.assertEqual(calls, [])
            self.assertEqual(unraisable, [])

    def test_time_types_extremes_thresholds_and_clock_sample(self):
        for now, validity in [(self.before-1, "not_yet_valid"), (self.before, "within_interval"),
                              (self.after, "within_interval"), (self.after+1, "expired"),
                              (-(2**63), "not_yet_valid"), (2**63-1, "expired")]:
            cert = inspect_sc_pem(self.identity, inspected_at_unix=now).certificates[0]
            self.assertEqual(cert.validity, validity)
            if validity != "within_interval":
                self.assertEqual(cert.expiry, "not_assessed")
        for remaining, expiry in [(7*86400, "alarm"), (7*86400+1, "warning")]:
            cert = inspect_sc_pem(self.identity, inspected_at_unix=self.after-remaining).certificates[0]
            self.assertEqual(cert.expiry, expiry)
        for now in (True, False, 1.5, "123"):
            with self.assertRaises(TypeError):
                inspect_sc_pem(self.identity, inspected_at_unix=now)
        for now in (-(2**63)-1, 2**63):
            with self.assertRaises(OverflowError):
                inspect_sc_pem(self.identity, inspected_at_unix=now)
        start = time.time()
        report = inspect_sc_pem(self.identity)
        end = time.time()
        self.assertIsInstance(report.inspected_at_unix, int)
        assert_unix_second_sample(self, start, report.inspected_at_unix, end,
                                  time.get_clock_info("time").resolution)

    def test_default_clock_assertion_handles_coarse_second_boundaries(self):
        tick = 0.015625
        assert_unix_second_sample(self, 1000.99, 1001, 1000.99, tick)
        assert_unix_second_sample(self, 1001.001, 1000, 1001.001, tick)
        assert_unix_second_sample(self, 1000.75, 1000, 1000.8, tick)
        for before, observed, after in [(1000.5, 1001, 1000.5),
                                        (1001.5, 1000, 1001.5)]:
            with self.assertRaises(AssertionError):
                assert_unix_second_sample(self, before, observed, after, tick)

    def test_missing_malformed_oversized_inputs_and_safe_errors(self):
        for kwargs in ({"peer_trust_pem": b""}, {"private_key_pem": b""}):
            with self.assertRaises(ValueError):
                inspect_sc_pem(self.identity, **kwargs)
        for value in ("looks like PEM", [1, 2], memoryview(self.identity)):
            with self.assertRaises(TypeError):
                inspect_sc_pem(value)
        with self.assertRaisesRegex(ValueError, "input_too_large"):
            inspect_sc_pem(b" " * (1024 * 1024 + 1))
        # Reject the combined size before parsing, and release every export
        # so callers can immediately resize their inputs on error.
        mutable = bytearray(b" " * (1024 * 1024))
        trust = bytearray(b"x")
        with self.assertRaisesRegex(ValueError, "input_too_large"):
            inspect_sc_pem(mutable, peer_trust_pem=trust)
        mutable.clear()
        trust.clear()
        too_large = bytearray(b" " * (1024 * 1024 + 1))
        with self.assertRaisesRegex(ValueError, "input_too_large"):
            inspect_sc_pem(too_large)
        too_large.clear()
        for sentinel in (b"-----BEGIN SENTINEL-SECRET invalid\n", b"-----BEGIN SENTINEL-SECRET-----\n"):
            with self.assertRaises(ValueError) as caught:
                inspect_sc_pem(self.identity, private_key_pem=sentinel)
            for text in (str(caught.exception), repr(caught.exception)):
                self.assertNotIn("SENTINEL", text)
                self.assertNotIn("83, 69, 78, 84, 73, 78, 69, 76", text)
                self.assertLess(len(text), 200)
        invalid = b"-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n"
        cert = inspect_sc_pem(invalid).certificates[0]
        self.assertIsNone(cert.metadata)
        self.assertFalse(cert.san.assessed)
        finding = cert.findings[0]
        self.assertIsInstance(finding, ScInspectionFinding)
        self.assertEqual((finding.area, finding.kind), ("certificate", "malformed"))
        with self.assertRaises(AttributeError):
            finding.kind = "changed"

    def test_key_outcomes_first_key_and_encoding_contracts(self):
        self.assertEqual(inspect_sc_pem(self.identity).key_consistency, "not_assessed")
        other = (self.root / "other.key").read_bytes()
        self.assertEqual(inspect_sc_pem(self.identity, private_key_pem=other).key_consistency, "mismatch")
        unusable = b"-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----\n"
        self.assertEqual(inspect_sc_pem(self.identity, private_key_pem=unusable).key_consistency, "unusable")
        tail = self.key + other + b"-----BEGIN SECRET invalid\n"
        self.assertEqual(inspect_sc_pem(self.identity, private_key_pem=tail).key_consistency, "match")
        sec1 = self.openssl("ec", "-in", "identity.key", "-outform", "PEM")
        self.assertIn(b"EC PRIVATE KEY", sec1)
        self.assertEqual(inspect_sc_pem(self.identity, private_key_pem=sec1).key_consistency, "match")
        encrypted = self.openssl("pkcs8", "-topk8", "-in", "identity.key", "-passout", "pass:fixture-only")
        with self.assertRaises(ValueError):
            inspect_sc_pem(self.identity, private_key_pem=encrypted)

    def test_rsa_pkcs1_key_matches_its_identity(self):
        self.openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                     "-subj", "/CN=rsa fixture", "-keyout", "rsa.key", "-out", "rsa.pem")
        # `-traditional` requests the supported PKCS#1 representation explicitly.
        key = self.openssl("rsa", "-in", "rsa.key", "-traditional", "-outform", "PEM")
        self.assertIn(b"RSA PRIVATE KEY", key)
        self.assertEqual(inspect_sc_pem((self.root / "rsa.pem").read_bytes(),
                                       private_key_pem=key).key_consistency, "match")

    def test_paths_errno_and_opened_file_mode_observations(self):
        with self.assertRaises(OSError) as caught:
            inspect_sc_pem_files(self.root / "missing.pem")
        self.assertEqual(caught.exception.errno, errno.ENOENT)
        self.assertIsInstance(caught.exception, FileNotFoundError)
        self.assertTrue(caught.exception.strerror)
        for path in (bytes(self.root), 123):
            with self.assertRaises(TypeError):
                inspect_sc_pem_files(path)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "key.pem"
            path.write_bytes(self.key)
            for mode in (0o600, 0o640, 0o604, 0o644):
                path.chmod(mode)
                report = inspect_sc_pem_files(self.root / "identity.pem", private_key_path=path)
                observation = report.key_file_permissions
                if os.name == "posix":
                    self.assertIsInstance(observation, ScKeyFilePermissions)
                    self.assertEqual(observation.group_readable, bool(mode & 0o040))
                    self.assertEqual(observation.world_readable, bool(mode & 0o004))
                    with self.assertRaises(AttributeError):
                        observation.group_readable = False
                else:
                    self.assertIsNone(observation)

    def test_offline_example_uses_only_supplied_files(self):
        example = Path(__file__).resolve().parents[3] / "examples/python/sc_inspect_pem.py"
        spec = importlib.util.spec_from_file_location("sc_inspect_example", example)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with redirect_stdout(io.StringIO()) as out:
            report = module.inspect(self.root / "identity.pem", peer_trust=self.root / "other.pem",
                                    key=self.root / "identity.key", inspected_at_unix=self.before)
        self.assertEqual(report.key_consistency, "match")
        self.assertIn(hashlib.sha256(self.der).hexdigest(), out.getvalue())
        self.assertNotIn(self.key.decode(), out.getvalue())


if __name__ == "__main__":
    unittest.main()
