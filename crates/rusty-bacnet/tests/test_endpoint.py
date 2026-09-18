"""RB-19 Python endpoint ownership + migration (installed extension).

Covers, on supported interpreters via maturin develop:
- one B/IP endpoint receives + initiates through a single transport
  (Rust-side single-bind assertion via bind-conflict + bidirectional reads);
- I-Am/Device identity agreement (vendor + name + present-value);
- concurrent client/server use (asyncio.gather, poll pattern, no callbacks);
- cancellation (in-flight read cancel leaves endpoint usable) + idempotent close;
- live role handle after owner close fails closed via BacnetError;
- error propagation (protocol error attributes, unknown address timeout);
- constructor validation (ValueError/RuntimeError/OverflowError split);
- SC + MS/TP validation paths (no wire for MS/TP without hardware);
- BIPv6/Ethernet have no endpoint owner (explicit, never mapped).

Native-dependency/setup failures (missing openssl, serial hardware) are
reported separately from protocol failures via skips/explicit asserts.
"""

from __future__ import annotations

import asyncio
import socket
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any

from rusty_bacnet import (
    BacnetError,
    BacnetProtocolError,
    BipEndpoint,
    EndpointClient,
    EndpointServer,
    ScHub,
    MstpEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    ScEndpoint,
)


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def bip_kwargs(**overrides):
    args = {
        "device_instance": 1001,
        "vendor_id": 42,
        "interface": "127.0.0.1",
        "broadcast_address": "127.255.255.255",
    }
    args.update(overrides)
    return args


class EndpointSurfaceTests(unittest.TestCase):
    """No BIPv6/Ethernet endpoint scope; roles expose no lifecycle."""

    def test_no_bipv6_ethernet_endpoint_owner(self):
        import rusty_bacnet

        self.assertIsNone(getattr(rusty_bacnet, "Bipv6Endpoint", None))
        self.assertIsNone(getattr(rusty_bacnet, "EthernetEndpoint", None))
        self.assertIsNone(getattr(rusty_bacnet, "BacnetEndpoint", None))

    def test_roles_expose_no_lifecycle(self):
        for cls in (EndpointClient, EndpointServer):
            for forbidden in ("start", "stop", "close"):
                self.assertNotIn(forbidden, dir(cls), f"{cls.__name__}.{forbidden}")

    def test_no_python_callback_registry(self):
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            names = [
                name
                for name in dir(cls)
                if "callback" in name.lower() or name.startswith("on_")
            ]
            self.assertEqual(names, [], f"{cls.__name__} must not expose callbacks")


class BipConstructorTests(unittest.TestCase):
    """Builder-time validation only; no I/O here."""

    def test_queue_capacity_zero_rejected(self):
        with self.assertRaisesRegex(ValueError, "queue_capacity"):
            BipEndpoint(**bip_kwargs(port=free_port(), queue_capacity=0))

    def test_invalid_interface_rejected_before_bind(self):
        with self.assertRaises(ValueError):
            BipEndpoint(**bip_kwargs(port=free_port(), interface="not-an-ip"))

    def test_invalid_broadcast_rejected_before_bind(self):
        with self.assertRaises(ValueError):
            BipEndpoint(**bip_kwargs(port=free_port(), broadcast_address="bogus"))

    def test_invalid_instance_rejected(self):
        with self.assertRaises(ValueError):
            BipEndpoint(**bip_kwargs(port=free_port(), device_instance=4_194_304))

    def test_invalid_max_apdu_rejected(self):
        with self.assertRaisesRegex(ValueError, "max-APDU|APDU|apdu|invalid"):
            BipEndpoint(**bip_kwargs(port=free_port(), max_apdu=999))

    def test_bad_uuid_length_rejected(self):
        with self.assertRaisesRegex(ValueError, "16 bytes"):
            BipEndpoint(**bip_kwargs(port=free_port(), device_uuid=b"\x01\x02"))

    def test_negative_vendor_raises_overflow(self):
        with self.assertRaises(OverflowError):
            BipEndpoint(**bip_kwargs(port=free_port(), vendor_id=-1))

    def test_default_vendor_preserves_compat_through_single_identity(self):
        # Old BACnetServer hardcoded 555; the endpoint default preserves it
        # through the single DeviceIdentity (no second identity).
        endpoint = BipEndpoint(device_instance=1234, port=free_port())
        self.assertEqual(endpoint.vendor_id, 555)
        self.assertEqual(endpoint.device_instance, 1234)

    def test_pending_registrations_available_before_start(self):
        endpoint = BipEndpoint(**bip_kwargs(port=free_port()))
        pending = getattr(endpoint, "_pending_registration_count")
        self.assertEqual(pending(), 0)
        endpoint.add_analog_input(instance=1, name="Zone", present_value=1.0)
        endpoint.add_binary_value(instance=1, name="Override")
        self.assertEqual(pending(), 2)


class MstpConstructorTests(unittest.TestCase):
    """MS/TP validation mirrors the current wrappers (no serial open here)."""

    def test_missing_serial_rejected(self):
        with self.assertRaises(TypeError):
            MstpEndpoint(device_instance=1)  # type: ignore[call-arg]

    def test_bad_baud_rejected(self):
        with self.assertRaisesRegex(ValueError, "mstp_baud"):
            MstpEndpoint(device_instance=1, serial_port="/tmp/nonexistent", mstp_baud=12345)

    def test_bad_mac_rejected(self):
        with self.assertRaisesRegex(ValueError, "mstp_mac"):
            MstpEndpoint(device_instance=1, serial_port="/tmp/nonexistent", mstp_mac=128)

    def test_mac_above_max_master_rejected(self):
        with self.assertRaisesRegex(ValueError, "mstp_mac"):
            MstpEndpoint(
                device_instance=1,
                serial_port="/tmp/nonexistent",
                mstp_mac=4,
                mstp_max_master=3,
            )

    def test_apdu_above_mstp_bound_rejected(self):
        with self.assertRaisesRegex(ValueError, "480"):
            MstpEndpoint(
                device_instance=1, serial_port="/tmp/nonexistent", max_apdu=1476
            )

    def test_queue_zero_rejected(self):
        with self.assertRaisesRegex(ValueError, "queue_capacity"):
            MstpEndpoint(
                device_instance=1, serial_port="/tmp/nonexistent", queue_capacity=0
            )


class ScConstructorTests(unittest.TestCase):
    """SC validation: single UUID source, hub parity for VMAC/heartbeat."""

    def sc_kwargs(self, **overrides):
        args = {
            "device_instance": 1001,
            "sc_hub": "wss://localhost:1",
            "sc_vmac": b"\x02\x00\x00\x00\x00\x01",
            "sc_ca_cert": "absent-ca.pem",
            "sc_client_cert": "absent-cert.pem",
            "sc_client_key": "absent-key.pem",
            "sc_device_uuid": bytes.fromhex("8e62ac46d7084226913776a32b619315"),
        }
        args.update(overrides)
        return args

    def test_missing_uuid_rejected(self):
        kwargs = self.sc_kwargs()
        del kwargs["sc_device_uuid"]
        with self.assertRaises(TypeError):
            ScEndpoint(**kwargs)  # type: ignore[call-arg]

    def test_zero_uuid_rejected(self):
        with self.assertRaisesRegex(ValueError, "all zero"):
            ScEndpoint(**self.sc_kwargs(sc_device_uuid=bytes(16)))

    def test_short_uuid_rejected(self):
        with self.assertRaisesRegex(ValueError, "16 bytes"):
            ScEndpoint(**self.sc_kwargs(sc_device_uuid=b"\x01\x02"))

    def test_vmac_length_is_runtime_error(self):
        with self.assertRaises(RuntimeError):
            ScEndpoint(**self.sc_kwargs(sc_vmac=b"\x01\x02"))

    def test_reserved_vmac_rejected(self):
        with self.assertRaises(ValueError):
            ScEndpoint(**self.sc_kwargs(sc_vmac=bytes(6)))

    def test_missing_credentials_rejected(self):
        with self.assertRaises(ValueError):
            ScEndpoint(**self.sc_kwargs(sc_ca_cert=""))

    def test_heartbeat_range_rejected(self):
        with self.assertRaises(ValueError):
            ScEndpoint(**self.sc_kwargs(sc_heartbeat_interval_ms=999))
        with self.assertRaises(ValueError):
            ScEndpoint(
                **self.sc_kwargs(
                    sc_heartbeat_interval_ms=30000, sc_heartbeat_timeout_ms=30000
                )
            )


class BipLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def test_pre_start_accessors_fail_and_close_idempotent(self):
        endpoint = BipEndpoint(**bip_kwargs(port=free_port()))
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await endpoint.status()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await endpoint.client()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await endpoint.server()
        # Close before start is a safe no-op.
        await asyncio.wait_for(endpoint.close(), 5)
        await asyncio.wait_for(endpoint.close(), 5)

    async def test_second_start_raises_without_rebinding(self):
        endpoint = BipEndpoint(**bip_kwargs(port=free_port()))
        await asyncio.wait_for(endpoint.start(), 10)
        try:
            with self.assertRaises(BacnetError):
                await asyncio.wait_for(endpoint.start(), 10)
        finally:
            await asyncio.wait_for(endpoint.close(), 5)

    async def test_add_after_start_rejected_and_pending_drained(self):
        endpoint = BipEndpoint(**bip_kwargs(port=free_port()))
        endpoint.add_analog_input(instance=1, name="Zone", present_value=1.0)
        pending = getattr(endpoint, "_pending_registration_count")
        self.assertEqual(pending(), 1)
        await asyncio.wait_for(endpoint.start(), 10)
        try:
            self.assertEqual(pending(), 0)
            with self.assertRaisesRegex(RuntimeError, "after start"):
                endpoint.add_analog_input(instance=2, name="Late")
        finally:
            await asyncio.wait_for(endpoint.close(), 5)

    async def test_context_manager_double_exit_safe(self):
        endpoint = BipEndpoint(**bip_kwargs(port=free_port()))
        async with endpoint:
            status = await endpoint.status()
            self.assertTrue(status["is_running"])
            await asyncio.wait_for(endpoint.close(), 5)
        await asyncio.wait_for(endpoint.close(), 5)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await endpoint.status()

    async def test_status_snapshot_keys_and_redaction(self):
        endpoint = BipEndpoint(
            **bip_kwargs(port=free_port(), device_instance=5555, vendor_id=99)
        )
        endpoint.add_analog_input(instance=1, name="Zone", present_value=3.0)
        await asyncio.wait_for(endpoint.start(), 10)
        try:
            status = await endpoint.status()
            self.assertEqual(
                set(status),
                {
                    "is_running",
                    "device_instance",
                    "vendor_id",
                    "max_apdu",
                    "transport",
                    "local_address",
                    "active_leases",
                    "ingress_policy",
                    "no_server_role",
                    "no_client_role",
                    "unclaimed_terminal",
                    "responder_declined",
                },
            )
            self.assertTrue(status["is_running"])
            self.assertEqual(status["device_instance"], 5555)
            self.assertEqual(status["vendor_id"], 99)
            self.assertEqual(status["transport"], "bip")
            self.assertIn("127.0.0.1", status["local_address"])
            blob = repr(status).lower()
            for token in ("key", "cert", "pem", "uuid", "begin"):
                self.assertNotIn(token, blob)
        finally:
            await asyncio.wait_for(endpoint.close(), 5)


class BipFunctionalTests(unittest.IsolatedAsyncioTestCase):
    """One endpoint receives + initiates; single-bind assertion via conflict."""

    async def test_single_transport_receives_and_initiates(self):
        port_a, port_b = free_port(), free_port()
        self.assertNotEqual(port_a, port_b)
        first = BipEndpoint(
            **bip_kwargs(port=port_a, device_instance=1001, device_name="Endpoint A")
        )
        first.add_analog_input(instance=1, name="A-Temp", units=62, present_value=21.5)
        second = BipEndpoint(
            **bip_kwargs(port=port_b, device_instance=1002, device_name="Endpoint B")
        )
        second.add_analog_input(instance=1, name="B-Temp", units=62, present_value=99.0)
        async with first:
            async with second:
                addr_a = await first.local_address()
                addr_b = await second.local_address()
                self.assertIn(str(port_a), addr_a)
                self.assertIn(str(port_b), addr_b)
                first_client = await first.client()
                second_client = await second.client()
                oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
                a_reads_b, b_reads_a = await asyncio.gather(
                    asyncio.wait_for(
                        first_client.read_property(
                            f"127.0.0.1:{port_b}",
                            oid,
                            PropertyIdentifier.PRESENT_VALUE,
                        ),
                        10,
                    ),
                    asyncio.wait_for(
                        second_client.read_property(
                            f"127.0.0.1:{port_a}",
                            oid,
                            PropertyIdentifier.PRESENT_VALUE,
                        ),
                        10,
                    ),
                )
                # Each single socket handled one direction out + the other in.
                self.assertEqual(a_reads_b.value, 99.0)
                self.assertEqual(b_reads_a.value, 21.5)

    async def test_single_bind_conflict_proves_one_socket(self):
        port = free_port()
        first = BipEndpoint(**bip_kwargs(port=port, device_instance=2001))
        await asyncio.wait_for(first.start(), 10)
        try:
            second = BipEndpoint(**bip_kwargs(port=port, device_instance=2002))
            # Rust-side single-bind assertion: the first endpoint holds the
            # one UDP bind, so a second bind on the same port must fail
            # (BacnetError via the transport mapping, never silent sharing).
            with self.assertRaises(BacnetError):
                await asyncio.wait_for(second.start(), 10)
            await asyncio.wait_for(second.close(), 5)
        finally:
            await asyncio.wait_for(first.close(), 5)

    async def test_identity_agreement_iam_device(self):
        port_a, port_b = free_port(), free_port()
        first = BipEndpoint(
            **bip_kwargs(port=port_a, device_instance=3001, vendor_id=77)
        )
        first.add_analog_input(instance=1, name="A", present_value=1.0)
        second = BipEndpoint(
            **bip_kwargs(
                port=port_b,
                device_instance=3002,
                device_name="Identity B",
                vendor_id=78,
            )
        )
        second.add_analog_input(instance=7, name="B-Point", units=62, present_value=42.0)
        async with first:
            async with second:
                client = await first.client()
                device_b = ObjectIdentifier(ObjectType.DEVICE, 3002)
                name = await asyncio.wait_for(
                    client.read_property(
                        f"127.0.0.1:{port_b}", device_b, PropertyIdentifier.OBJECT_NAME
                    ),
                    10,
                )
                vendor = await asyncio.wait_for(
                    client.read_property(
                        f"127.0.0.1:{port_b}",
                        device_b,
                        PropertyIdentifier.VENDOR_IDENTIFIER,
                    ),
                    10,
                )
                self.assertEqual(name.value, "Identity B")
                self.assertEqual(vendor.value, 78)
                point = ObjectIdentifier(ObjectType.ANALOG_INPUT, 7)
                present = await asyncio.wait_for(
                    client.read_property(
                        f"127.0.0.1:{port_b}",
                        point,
                        PropertyIdentifier.PRESENT_VALUE,
                    ),
                    10,
                )
                self.assertEqual(present.value, 42.0)
                # I-Am broadcast path is consistent with the identity (no send error).
                await asyncio.wait_for(second.broadcast_i_am(), 10)

    async def test_concurrent_client_server_use(self):
        port_a, port_b = free_port(), free_port()
        first = BipEndpoint(**bip_kwargs(port=port_a, device_instance=4001))
        first.add_analog_input(instance=1, name="A", present_value=5.0)
        second = BipEndpoint(**bip_kwargs(port=port_b, device_instance=4002))
        second.add_analog_input(instance=1, name="B", present_value=6.0)
        async with first:
            async with second:
                first_client = await first.client()
                second_client = await second.client()
                first_server = await first.server()
                oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

                async def read_a_to_b():
                    return await first_client.read_property(
                        f"127.0.0.1:{port_b}", oid, PropertyIdentifier.PRESENT_VALUE
                    )

                async def read_b_to_a():
                    return await second_client.read_property(
                        f"127.0.0.1:{port_a}", oid, PropertyIdentifier.PRESENT_VALUE
                    )

                async def poll_liveness():
                    for _ in range(10):
                        self.assertTrue(first_server.is_session_alive())
                        await asyncio.sleep(0.01)
                    return True

                a_val, b_val, alive = await asyncio.gather(
                    asyncio.wait_for(asyncio.ensure_future(read_a_to_b()), 10),
                    asyncio.wait_for(asyncio.ensure_future(read_b_to_a()), 10),
                    poll_liveness(),
                )
                self.assertEqual(a_val.value, 6.0)
                self.assertEqual(b_val.value, 5.0)
                self.assertTrue(alive)

    async def test_owner_close_with_live_handles_fails_closed(self):
        port_a, port_b = free_port(), free_port()
        first = BipEndpoint(**bip_kwargs(port=port_a, device_instance=5001))
        first.add_analog_input(instance=1, name="A", present_value=1.0)
        second = BipEndpoint(**bip_kwargs(port=port_b, device_instance=5002))
        second.add_analog_input(instance=1, name="B", present_value=2.0)
        await asyncio.wait_for(first.start(), 10)
        await asyncio.wait_for(second.start(), 10)
        try:
            live_client = await first.client()
            live_server = await first.server()
            self.assertTrue(live_server.is_session_alive())
            await asyncio.wait_for(first.close(), 5)
            self.assertFalse(live_server.is_session_alive())
            oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
            with self.assertRaises(BacnetError):
                await asyncio.wait_for(
                    live_client.read_property(
                        f"127.0.0.1:{port_b}", oid, PropertyIdentifier.PRESENT_VALUE
                    ),
                    10,
                )
            with self.assertRaises(BacnetError):
                live_server.suspend_next_reply()
            # Owner borrows released at termination.
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await first.client()
        finally:
            await asyncio.wait_for(first.close(), 5)
            await asyncio.wait_for(second.close(), 5)

    async def test_in_flight_cancellation_leaves_endpoint_usable(self):
        port_a, port_b = free_port(), free_port()
        first = BipEndpoint(
            **bip_kwargs(port=port_a, device_instance=6001, apdu_timeout_ms=5000)
        )
        first.add_analog_input(instance=1, name="A", present_value=1.0)
        second = BipEndpoint(**bip_kwargs(port=port_b, device_instance=6002))
        second.add_analog_input(instance=1, name="B", present_value=2.0)
        async with first:
            async with second:
                client = await first.client()
                oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)

                async def blackhole_read():
                    return await client.read_property(
                        "127.0.0.1:47999", oid, PropertyIdentifier.PRESENT_VALUE
                    )

                pending = asyncio.ensure_future(blackhole_read())
                await asyncio.sleep(0.2)
                pending.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await pending
                # Lease released via RAII; endpoint still usable.
                value = await asyncio.wait_for(
                    client.read_property(
                        f"127.0.0.1:{port_b}", oid, PropertyIdentifier.PRESENT_VALUE
                    ),
                    10,
                )
                self.assertEqual(value.value, 2.0)

    async def test_error_propagation_protocol_attributes(self):
        port_a, port_b = free_port(), free_port()
        first = BipEndpoint(**bip_kwargs(port=port_a, device_instance=7001))
        first.add_analog_input(instance=1, name="A", present_value=1.0)
        second = BipEndpoint(**bip_kwargs(port=port_b, device_instance=7002))
        second.add_analog_input(instance=1, name="B", present_value=2.0)
        async with first:
            async with second:
                client = await first.client()
                missing = ObjectIdentifier(ObjectType.ANALOG_INPUT, 999)
                with self.assertRaises(BacnetProtocolError) as ctx:
                    await asyncio.wait_for(
                        client.read_property(
                            f"127.0.0.1:{port_b}",
                            missing,
                            PropertyIdentifier.PRESENT_VALUE,
                        ),
                        10,
                    )
                # Structured attributes survive the boundary (no string parsing).
                self.assertIsInstance(ctx.exception.error_class, int)
                self.assertIsInstance(ctx.exception.error_code, int)


class MstpStartupTests(unittest.IsolatedAsyncioTestCase):
    async def test_nonexistent_serial_fails_as_setup_not_protocol(self):
        endpoint = MstpEndpoint(
            device_instance=8001,
            serial_port="/tmp/rusty-bacnet-nonexistent-serial",
            mstp_mac=3,
        )
        # Native-dependency/setup failure: RuntimeError from serial open,
        # distinct from BacnetError protocol failures. Pending preserved.
        with self.assertRaises(RuntimeError):
            await asyncio.wait_for(endpoint.start(), 10)
        await asyncio.wait_for(endpoint.close(), 5)


HUB_UUID = bytes.fromhex("9a21f1641a15454d9ed7e3a2710d7001")
SC_A_UUID = bytes.fromhex("8e62ac46d7084226913776a32b619315")
SC_B_UUID = bytes.fromhex("7c31d9e2b4f84a2d9c1e5f6a7b8c9d0e")
HUB_VMAC = b"\x02\x00\x00\x00\x00\x01"
SC_A_VMAC = b"\x02\x00\x00\x00\x00\x0a"
SC_B_VMAC = b"\x02\x00\x00\x00\x00\x0b"


def vmac_hex(vmac: bytes) -> str:
    return ":".join(f"{b:02x}" for b in vmac)


class ScEndpointHubTests(unittest.IsolatedAsyncioTestCase):
    """One SC hub connection per endpoint handles both roles (real TLS hub)."""

    @classmethod
    def setUpClass(cls):
        try:
            subprocess.run(["openssl", "version"], check=True, capture_output=True)
        except Exception as exc:
            raise unittest.SkipTest(f"openssl unavailable: {exc}")
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-endpoint-sc-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)

        def run(*args: str):
            subprocess.run(
                ["openssl", *args],
                cwd=cls.root,
                check=True,
                capture_output=True,
                timeout=15,
            )

        run(
            "req", "-x509", "-newkey", "ec", "-pkeyopt",
            "ec_paramgen_curve:prime256v1", "-nodes", "-days", "2",
            "-subj", "/CN=site", "-keyout", "site.key", "-out", "site.pem",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign",
        )
        (cls.root / "leaf.cnf").write_text(
            "[leaf]\nbasicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature\n"
            "extendedKeyUsage=serverAuth,clientAuth\n"
            "subjectAltName=DNS:localhost,IP:127.0.0.1\n"
        )
        for name in ("hub", "a", "b"):
            run(
                "req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-subj", f"/CN={name}", "-keyout", f"{name}.key",
                "-out", f"{name}.csr",
            )
            run(
                "x509", "-req", "-in", f"{name}.csr", "-CA", "site.pem",
                "-CAkey", "site.key", "-CAcreateserial", "-days", "1",
                "-extfile", "leaf.cnf", "-extensions", "leaf", "-out", f"{name}.pem",
            )

    def path(self, name: str) -> str:
        return str(self.root / name)

    def make_endpoint(self, vmac: bytes, uuid: bytes, instance: int, **overrides: Any):
        args = {
            "device_instance": instance,
            "sc_hub": self.hub_url,
            "sc_vmac": vmac,
            "sc_ca_cert": self.path("site.pem"),
            "sc_client_cert": self.path(overrides.pop("cert", "a.pem")),
            "sc_client_key": self.path(overrides.pop("key", "a.key")),
            "sc_device_uuid": uuid,
        }
        args.update(overrides)
        return ScEndpoint(**args)

    async def test_sc_endpoints_exchange_both_directions(self):
        hub = ScHub(
            listen="127.0.0.1:0",
            cert=self.path("hub.pem"),
            key=self.path("hub.key"),
            vmac=HUB_VMAC,
            ca_cert=self.path("site.pem"),
            device_uuid=HUB_UUID,
        )
        await asyncio.wait_for(hub.start(), 10)
        try:
            self.hub_url = await hub.url()
            first = self.make_endpoint(
                SC_A_VMAC, SC_A_UUID, 9001, cert="a.pem", key="a.key"
            )
            first.add_analog_input(instance=1, name="SC-A", present_value=11.0)
            second = self.make_endpoint(
                SC_B_VMAC, SC_B_UUID, 9002, cert="b.pem", key="b.key"
            )
            second.add_analog_input(instance=1, name="SC-B", present_value=22.0)
            await asyncio.wait_for(first.start(), 15)
            await asyncio.wait_for(second.start(), 15)
            try:
                first_client = await first.client()
                second_client = await second.client()
                oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
                a_reads_b, b_reads_a = await asyncio.gather(
                    asyncio.wait_for(
                        first_client.read_property(
                            vmac_hex(SC_B_VMAC), oid, PropertyIdentifier.PRESENT_VALUE
                        ),
                        15,
                    ),
                    asyncio.wait_for(
                        second_client.read_property(
                            vmac_hex(SC_A_VMAC), oid, PropertyIdentifier.PRESENT_VALUE
                        ),
                        15,
                    ),
                )
                self.assertEqual(a_reads_b.value, 22.0)
                self.assertEqual(b_reads_a.value, 11.0)
                # Single-UUID identity: DEVICE_UUID readback equals the one
                # durable UUID (no split transport/identity values).
                device_b = ObjectIdentifier(ObjectType.DEVICE, 9002)
                uuid_value = await asyncio.wait_for(
                    first_client.read_property(
                        vmac_hex(SC_B_VMAC),
                        device_b,
                        PropertyIdentifier.DEVICE_UUID,
                    ),
                    15,
                )
                self.assertEqual(uuid_value.tag, "octet_string")
                self.assertEqual(bytes(uuid_value.value), SC_B_UUID)
            finally:
                await asyncio.wait_for(first.close(), 5)
                await asyncio.wait_for(second.close(), 5)
        finally:
            await asyncio.wait_for(hub.stop(), 5)


if __name__ == "__main__":
    unittest.main()
