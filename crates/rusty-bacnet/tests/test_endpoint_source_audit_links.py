"""The source Audit Reporter keywords on ScEndpoint and MstpEndpoint (#2357):
each binding address in its link's format, refused at construction by that
link's rules, and the keywords reach the SC session at start. The records
themselves are checked on the wire in Rust (bacnet-endpoint's
source_link_tests.rs): an MS/TP endpoint needs a serial port here."""
import asyncio
import subprocess
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (BacnetError, MstpEndpoint, ObjectIdentifier, ObjectType, ScEndpoint,
                          ScHub)

O = ObjectType
REPORTER = ObjectIdentifier(O.AUDIT_REPORTER, 1)
LOGGER = ObjectIdentifier(O.DEVICE, 999)
RESET = 1 << 8
SC_UUID = bytes.fromhex("8e62ac46d7084226913776a32b619315")
HUB_UUID = bytes.fromhex("9a21f1641a15454d9ed7e3a2710d7001")
SC_VMAC = b"\x02\x00\x00\x00\x00\x0a"
HUB_VMAC = b"\x02\x00\x00\x00\x00\x01"


def source(**overrides):
    keywords = {
        "on_reinitialize": lambda request: None,
        "device_write_policy": lambda context: True,
        "source_audit_reporter": REPORTER,
        "audit_recipient": {"kind": "device", "object_identifier": LOGGER},
    }
    keywords.update(overrides)
    return keywords


def sc_endpoint(hub="wss://localhost:1", ca="absent-ca.pem", cert="absent.pem",
                key="absent.key", **overrides):
    return ScEndpoint(2357, hub, SC_VMAC, ca, cert, key, sc_device_uuid=SC_UUID,
                      **source(**overrides))


def mstp_endpoint(**overrides):
    return MstpEndpoint(2357, "/tmp/nonexistent", mstp_mac=3, **source(**overrides))


class LinkBindingTests(unittest.TestCase):
    def test_each_link_takes_its_own_address_format(self):
        for make, good in ((sc_endpoint, ["02:00:00:00:00:0b", "02000000000b"]),
                           (mstp_endpoint, ["7", "0", "254"])):
            for address in good:
                with self.subTest(make=make.__name__, address=address):
                    owner = make(audit_device_bindings=[(LOGGER, address)])
                    owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                             auditable_operations=RESET)
                    self.assertEqual(owner._pending_registration_count(), 1)

    def test_each_link_refuses_what_cant_be_a_unicast_recipient_there(self):
        device = ObjectIdentifier(O.DEVICE, 7)
        cases = {
            sc_endpoint: ["ff:ff:ff:ff:ff:ff", "00:00:00:00:00:00", "02:00:00:00:00:0a",
                          "02:00:00:00:0b", "127.0.0.1:47808", "7", "zz:00:00:00:00:0b"],
            mstp_endpoint: ["255", "3", "256", "-1", "02:00:00:00:00:0b",
                            "127.0.0.1:47808", "seven"],
        }
        for make, refused in cases.items():
            for address in refused:
                with self.subTest(make=make.__name__, address=address):
                    with self.assertRaisesRegex(ValueError, r"audit_device_bindings\[0\]"):
                        make(audit_device_bindings=[(device, address)])

    def test_the_other_keywords_are_refused_as_on_bip(self):
        for make in (sc_endpoint, mstp_endpoint):
            refused = [
                {"source_audit_reporter": ObjectIdentifier(O.ANALOG_VALUE, 1)},
                {"device_write_policy": None},
                {"audit_recipient": None},
                # An address recipient needs B/IP.
                {"audit_recipient": {"kind": "address", "network_number": 0,
                                     "mac_address": b"\x07"}},
                {"audit_device_bindings": [(LOGGER, "7"), (LOGGER, "8")]},
                {"source_audit_reporter": None, "audit_device_bindings": [(LOGGER, "7")]},
            ]
            for overrides in refused:
                with self.subTest(make=make.__name__, overrides=overrides):
                    with self.assertRaises(ValueError):
                        make(**overrides)

    def test_add_audit_reporter_needs_its_level_and_operations(self):
        for owner in (sc_endpoint(), mstp_endpoint()):
            with self.subTest(owner=type(owner).__name__):
                with self.assertRaises(TypeError):
                    owner.add_audit_reporter(1, "Source")
                with self.assertRaises(ValueError):
                    owner.add_audit_reporter(1, "Source", audit_level="sometimes",
                                             auditable_operations=RESET)
                self.assertEqual(owner._pending_registration_count(), 0)


class ScSessionTests(unittest.IsolatedAsyncioTestCase):
    """The keywords reach a running SC session through a hub."""

    @classmethod
    def setUpClass(cls):
        try:
            subprocess.run(["openssl", "version"], check=True, capture_output=True)
        except Exception as exc:
            raise unittest.SkipTest(f"openssl unavailable: {exc}")
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-source-links-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)

        def run(*args):
            subprocess.run(["openssl", *args], cwd=cls.root, check=True,
                           capture_output=True, timeout=15)

        run("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
            "-nodes", "-days", "2", "-subj", "/CN=site", "-keyout", "site.key",
            "-out", "site.pem", "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        (cls.root / "leaf.cnf").write_text(
            "[leaf]\nbasicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature\n"
            "extendedKeyUsage=serverAuth,clientAuth\n"
            "subjectAltName=DNS:localhost,IP:127.0.0.1\n")
        for name in ("hub", "node"):
            run("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-subj", f"/CN={name}", "-keyout", f"{name}.key",
                "-out", f"{name}.csr")
            run("x509", "-req", "-in", f"{name}.csr", "-CA", "site.pem", "-CAkey",
                "site.key", "-CAcreateserial", "-days", "1", "-extfile", "leaf.cnf",
                "-extensions", "leaf", "-out", f"{name}.pem")

    def path(self, name):
        return str(self.root / name)

    async def test_an_sc_session_selects_the_reporter_at_start(self):
        hub = ScHub(listen="127.0.0.1:0", cert=self.path("hub.pem"), key=self.path("hub.key"),
                    vmac=HUB_VMAC, ca_cert=self.path("site.pem"), device_uuid=HUB_UUID)
        await asyncio.wait_for(hub.start(), 10)
        try:
            url = await hub.url()

            def owner():
                return sc_endpoint(url, self.path("site.pem"), self.path("node.pem"),
                                   self.path("node.key"),
                                   audit_device_bindings=[(LOGGER, "02:00:00:00:00:0b")])

            # The session checks that the Reporter was added.
            missing = owner()
            with self.assertRaisesRegex(BacnetError, "absent from the database"):
                await asyncio.wait_for(missing.start(), 15)
            await missing.close()
            # Added, the session starts with it, and a local reset is reported.
            added = owner()
            added.add_audit_reporter(1, "Source", audit_level="audit_all",
                                     auditable_operations=RESET)
            await asyncio.wait_for(added.start(), 15)
            try:
                self.assertIsNone(await added.report_device_reset_local())
            finally:
                await asyncio.wait_for(added.close(), 5)
        finally:
            await asyncio.wait_for(hub.stop(), 5)


if __name__ == "__main__":
    unittest.main()
