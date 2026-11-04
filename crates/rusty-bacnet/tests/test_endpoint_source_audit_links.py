"""The source Audit Reporter keywords on ScEndpoint and MstpEndpoint (#2357):
each binding address and address recipient in its link's format, refused at
construction by that link's rules, and an SC endpoint's records arriving at an
SC server's Audit Log. Routed bindings, "network:0xMAC" on every endpoint
(#2371), are checked on the wire in Rust (source_routed_tests.rs). MS/TP records are checked on the wire in Rust
(bacnet-endpoint's source_link_tests.rs): an MS/TP endpoint needs a serial
port here."""
import asyncio
import subprocess
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import (AuditOperation, BACnetClient, BACnetServer, BacnetError, BipEndpoint,
                          MstpEndpoint, ObjectIdentifier, ObjectType, PropertyIdentifier,
                          ReinitializedState, ScEndpoint, ScHub)

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
                {"audit_device_bindings": [(LOGGER, "7"), (LOGGER, "8")]},
                {"source_audit_reporter": None, "audit_device_bindings": [(LOGGER, "7")]},
            ]
            for overrides in refused:
                with self.subTest(make=make.__name__, overrides=overrides):
                    with self.assertRaises(ValueError):
                        make(**overrides)

    def test_each_link_parses_only_its_own_exact_format(self):
        cases = {
            sc_endpoint: ["::02000000000b::", "0:2:0:0:0:0:0:0:0:0:0:b", "02-00-00-00-00-0b",
                          "020:00:00:00:00:b", "02:00:00:00:00:0b ", " 02000000000b",
                          "02:00:00:00:00:0b:", "02000000000", "0x02000000000b"],
            mstp_endpoint: ["+7", "-0", " 7", "7 ", "", "7.0", "0x7", "\u0663"],
        }
        for make, refused in cases.items():
            for address in refused:
                with self.subTest(make=make.__name__, address=address):
                    with self.assertRaisesRegex(ValueError, r"audit_device_bindings\[0\] address"):
                        make(audit_device_bindings=[(LOGGER, address)])

    def test_an_address_recipient_follows_the_links_rule(self):
        def address(mac, network=0):
            return {"kind": "address", "network_number": network, "mac_address": mac}
        accepted = {
            sc_endpoint: [address(b"\x02\x00\x00\x00\x00\x0b"),
                          address(b"\x02\x00\x00\x00\x00\x0b", network=5)],
            mstp_endpoint: [address(b"\x07"), address(b"\x00"), address(b"\x07", network=5)],
        }
        for make, recipients in accepted.items():
            for recipient in recipients:
                with self.subTest(make=make.__name__, recipient=recipient):
                    owner = make(audit_recipient=recipient)
                    owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                             auditable_operations=RESET)
                    self.assertEqual(owner._pending_registration_count(), 1)
        refused = {
            # Broadcast, all-zero, own, of another length, empty, global.
            sc_endpoint: [address(b"\xff" * 6), address(b"\x00" * 6), address(SC_VMAC),
                          address(b"\x07"), address(b""),
                          address(b"\x02\x00\x00\x00\x00\x0b", network=65535)],
            mstp_endpoint: [address(b"\xff"), address(b"\x03"), address(SC_VMAC), address(b""),
                            address(b"\x07", network=65535)],
        }
        for make, recipients in refused.items():
            for recipient in recipients:
                with self.subTest(make=make.__name__, recipient=recipient):
                    with self.assertRaisesRegex(ValueError, "audit_recipient"):
                        make(audit_recipient=recipient)

    def test_add_audit_reporter_needs_its_level_and_operations(self):
        for owner in (sc_endpoint(), mstp_endpoint()):
            with self.subTest(owner=type(owner).__name__):
                with self.assertRaises(TypeError):
                    owner.add_audit_reporter(1, "Source")
                with self.assertRaises(ValueError):
                    owner.add_audit_reporter(1, "Source", audit_level="sometimes",
                                             auditable_operations=RESET)
                self.assertEqual(owner._pending_registration_count(), 0)


def bip_endpoint(**overrides):
    return BipEndpoint(2357, interface="127.0.0.1", port=0,
                       broadcast_address="127.255.255.255", **source(**overrides))


class RoutedBindingTests(unittest.IsolatedAsyncioTestCase):
    """A Device behind a router, written "network:0xMAC" as the CLI writes a
    routed target, binds on every link (#2371)."""

    MAKERS = (bip_endpoint, sc_endpoint, mstp_endpoint)

    def test_every_link_takes_a_routed_station(self):
        # The MAC is in the remote network's format: any length to 18 octets.
        for make in self.MAKERS:
            for address in ("2:0x05", "65534:0X0A000001BAC0", "1:0x" + "ab" * 18):
                with self.subTest(make=make.__name__, address=address):
                    owner = make(audit_device_bindings=[(LOGGER, address)])
                    owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                             auditable_operations=RESET)
                    self.assertEqual(owner._pending_registration_count(), 1)

    def test_a_routed_binding_names_one_station_on_another_network(self):
        # Network 0 is an address's name for this network and 65535 every
        # network; an empty MAC is the network's broadcast.
        refused = ("0:0x05", "65535:0x05", "65536:0x05", "2:0x", "2:0x5", "2:0xzz",
                   "2:0x" + "ab" * 19, "+2:0x05", " 2:0x05", "2:0x05 ", "-2:0x05",
                   "2:05")
        for make in self.MAKERS:
            for address in refused:
                with self.subTest(make=make.__name__, address=address):
                    with self.assertRaisesRegex(ValueError, r"audit_device_bindings\[0\]"):
                        make(audit_device_bindings=[(LOGGER, address)])
            with self.subTest(make=make.__name__, repeated=True):
                with self.assertRaisesRegex(ValueError, "repeats a Device"):
                    make(audit_device_bindings=[(LOGGER, "2:0x05"), (LOGGER, "3:0x05")])

    async def test_a_routed_binding_resolves_in_a_running_session(self):
        """The Reporter of a B/IP endpoint bound to a Device behind a router
        reads NO_FAULT_DETECTED: its recipient has a route."""
        owner = bip_endpoint(audit_device_bindings=[(LOGGER, "5:0x7f000001bac0")])
        owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                 auditable_operations=RESET)
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await client.__aenter__()
        try:
            async with owner:
                reliability = await client.read_property(
                    await owner.local_address(), REPORTER, PropertyIdentifier.RELIABILITY)
                self.assertEqual(reliability.value, 0)  # NO_FAULT_DETECTED
        finally:
            await client.stop()


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
        for name in ("hub", "node", "sink", "client"):
            run("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-subj", f"/CN={name}", "-keyout", f"{name}.key",
                "-out", f"{name}.csr")
            run("x509", "-req", "-in", f"{name}.csr", "-CA", "site.pem", "-CAkey",
                "site.key", "-CAcreateserial", "-days", "1", "-extfile", "leaf.cnf",
                "-extensions", "leaf", "-out", f"{name}.pem")

    def path(self, name):
        return str(self.root / name)

    async def stop(self, owner):
        await asyncio.wait_for(owner.stop(), 5)

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


    async def test_an_sc_endpoints_records_reach_an_sc_audit_log(self):
        """A peer's reset and a local reset, sent to an SC server's Audit
        Log, once to a bound Device recipient and once to an address one."""
        sink_device = ObjectIdentifier(O.DEVICE, 2358)
        sink_vmac = b"\x02\x00\x00\x00\x00\x0c"
        hub = ScHub(listen="127.0.0.1:0", cert=self.path("hub.pem"), key=self.path("hub.key"),
                    vmac=HUB_VMAC, ca_cert=self.path("site.pem"), device_uuid=HUB_UUID)
        await asyncio.wait_for(hub.start(), 10)
        self.addAsyncCleanup(self.stop, hub)
        url = await hub.url()
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        sink = BACnetServer(2358, transport="sc", sc_hub=url, sc_vmac=sink_vmac,
                            sc_device_uuid=bytes.fromhex("5c1d2e3f40514263748596a7b8c9dae0"),
                            sc_ca_cert=self.path("site.pem"),
                            sc_client_cert=self.path("sink.pem"),
                            sc_client_key=self.path("sink.key"))
        sink.add_audit_log(1, "sink", str(Path(directory.name) / "audit"), buffer_size=32)
        sink.configure_audit_notification_sink(1, policy="allow_all")
        await asyncio.wait_for(sink.start(), 10)
        self.addAsyncCleanup(self.stop, sink)
        client = BACnetClient(transport="sc", sc_hub=url, sc_vmac=b"\x02\x00\x00\x00\x00\x0d",
                              sc_device_uuid=bytes.fromhex("6d2e3f40516273849506a7b8c9daebf1"),
                              sc_ca_cert=self.path("site.pem"),
                              sc_client_cert=self.path("client.pem"),
                              sc_client_key=self.path("client.key"), apdu_timeout_ms=2000)
        await asyncio.wait_for(client.__aenter__(), 10)
        self.addAsyncCleanup(self.stop, client)

        async def records():
            query = {"audit_log": ObjectIdentifier(O.AUDIT_LOG, 1),
                     "query_parameters": {
                         "kind": "by_target", "successful_actions_only": 0,
                         "target_device_identifier": ObjectIdentifier(O.DEVICE, 2357)},
                     "requested_count": 32}
            result = await client.audit_log_query_typed("02:00:00:00:00:0c", query)
            return [entry["record"]["datum"]["audit_notification"]
                    for entry in result["records"]]

        cases = (
            ("device", {"kind": "device", "object_identifier": sink_device},
             [(sink_device, "02:00:00:00:00:0c")], b"\x02\x00\x00\x00\x00\x0e"),
            ("address", {"kind": "address", "network_number": 0, "mac_address": sink_vmac},
             None, b"\x02\x00\x00\x00\x00\x0f"),
        )
        this_device = {"kind": "device", "object_identifier": ObjectIdentifier(O.DEVICE, 2357)}
        for kind, recipient, bindings, vmac in cases:
            with self.subTest(recipient=kind):
                before = len(await records())
                owner = ScEndpoint(2357, url, vmac, self.path("site.pem"),
                                   self.path("node.pem"), self.path("node.key"),
                                   sc_device_uuid=bytes([vmac[-1]]) + SC_UUID[1:],
                                   **source(audit_recipient=recipient,
                                            audit_device_bindings=bindings))
                owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                         auditable_operations=RESET)
                await asyncio.wait_for(owner.start(), 15)
                try:
                    address = await owner.local_address()
                    await client.reinitialize_device(address, ReinitializedState.WARMSTART)
                    self.assertIsNone(await owner.report_device_reset_local())
                    async with asyncio.timeout(10):
                        while len(found := await records()) < before + 2:
                            await asyncio.sleep(0.05)
                finally:
                    await asyncio.wait_for(owner.close(), 5)
                # The log lists the newest first: the local reset, then the
                # peer's, which names the peer and its invoke ID.
                local, peer = found[0], found[1]
                for record in (local, peer):
                    self.assertEqual(record["operation"], AuditOperation.DEVICE_RESET)
                    self.assertEqual(record["target_device"], this_device)
                self.assertEqual(local["source_device"], this_device)
                self.assertIsNone(local["invoke_id"])
                self.assertEqual(peer["source_device"],
                                 {"kind": "address", "network_number": 0,
                                  "mac_address": b"\x02\x00\x00\x00\x00\x0d"})
                self.assertIsNotNone(peer["invoke_id"])


if __name__ == "__main__":
    unittest.main()
