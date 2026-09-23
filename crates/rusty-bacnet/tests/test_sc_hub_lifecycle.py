"""RB-14 Python hub lifecycle + static policy parity with Rust (RB-12/13).

Installed-native tests only: construction failure before bind, redacted
status, static allow/deny outcomes over real TLS, graceful/forceful close,
context-manager exit, repeated close, cancellation during async close, and
the vacuous no-callback registry (no Python callable can reach the native
registry lock).
"""
import asyncio
import inspect
import socket
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any

from rusty_bacnet import BacnetError, BACnetServer, ScHub

# Deterministic test provisioning only, not defaults for deployed devices.
HUB_UUID = bytes.fromhex("9a21f1641a15454d9ed7e3a2710d7001")
SERVER_UUID = bytes.fromhex("8e62ac46d7084226913776a32b619315")
HUB_VMAC = b"\x02\0\0\0\0\1"
SERVER_VMAC = b"\x00\x01\x02\x03\x04\x05"

STATUS_KEYS = {
    "listening",
    "max_clients",
    "max_handshakes",
    "client_count",
    "handshake_count",
    "admin_denied",
    "broadcast_sender_exhausted",
    "broadcast_global_exhausted",
}


def hub_kwargs(**overrides: Any) -> Any:
    args = {
        "listen": "127.0.0.1:0",
        "cert": "absent-cert.pem",
        "key": "absent-key.pem",
        "vmac": HUB_VMAC,
        "ca_cert": "absent-ca.pem",
        "device_uuid": HUB_UUID,
    }
    args.update(overrides)
    return args


class HubConstructorTests(unittest.TestCase):
    """Sync constructor validation: everything fails before bind, without I/O."""

    def test_probe_and_broadcast_configuration_validates_without_io(self):
        with socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            ScHub(**hub_kwargs(
                listen=f"127.0.0.1:{occupied.getsockname()[1]}",
                probe_scan_interval_ms=25, probe_idle_age_ms=75,
                probe_ack_age_ms=40, probe_send_budget_ms=30, unicast_send_budget_ms=25,
                broadcast_sender_burst=2, broadcast_sender_per_second=1,
                broadcast_global_burst=3, broadcast_global_per_second=1,
            ))

    def test_invalid_probe_unicast_and_rate_settings_fail_before_io(self):
        timing = ("probe_scan_interval_ms", "probe_idle_age_ms", "probe_ack_age_ms",
                  "probe_send_budget_ms", "unicast_send_budget_ms")
        rates = ("broadcast_sender_burst", "broadcast_sender_per_second",
                 "broadcast_global_burst", "broadcast_global_per_second")
        for field in timing + rates:
            for value, error in ((0, ValueError), (-1, OverflowError),
                                 (0.5, TypeError), (2**64, OverflowError)):
                with self.subTest(field=field, value=value):
                    with self.assertRaises(error):
                        ScHub(**hub_kwargs(**{field: value}))
            with self.subTest(field=field, excessive=True):
                maximum = (2**63 - 1) if field in timing else (2**64 - 1) // 1_000_000_000
                with self.assertRaises(ValueError):
                    ScHub(**hub_kwargs(**{field: maximum + 1}))

    def test_deny_uuid_replacement_is_validated_without_io(self):
        with socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            # A valid static policy is accepted before opening credential files
            # or binding; it adds no Python callback under the native lock.
            ScHub(**hub_kwargs(
                listen=f"127.0.0.1:{occupied.getsockname()[1]}",
                admission_policy="deny_uuid_replacement",
            ))

    def test_invalid_bounds_policy_timeouts_raise_before_bind(self):
        with socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            port = occupied.getsockname()[1]
            cases = [
                (dict(max_clients=0), ValueError, "nonzero"),
                (dict(max_handshakes=0), ValueError, "nonzero"),
                (dict(max_clients=2**64 - 1, max_handshakes=1), ValueError, "overflow"),
                (dict(admission_policy="deny_list"), ValueError, "allow_all.*deny_all"),
                (dict(admission_policy=""), ValueError, "allow_all.*deny_all"),
                (dict(graceful_disconnect_ack_ms=999), ValueError, "Disconnect-Ack"),
                (dict(graceful_ws_close_ms=999), ValueError, "WebSocket close"),
                (dict(graceful_overall_ms=2000), ValueError, "ack.*close"),
                (dict(graceful_overall_ms=301_000), ValueError, "overall"),
                (dict(handshake_tls_ms=0), ValueError, "TLS"),
                (dict(handshake_websocket_upgrade_ms=0), ValueError, "WebSocket upgrade"),
                (dict(handshake_connect_request_ms=1000), ValueError, "Connect-Request"),
                (dict(handshake_connect_request_ms=301_000), ValueError, "Connect-Request"),
            ]
            for kwargs, exc, message in cases:
                with self.subTest(kwargs=kwargs), self.assertRaisesRegex(exc, message):
                    ScHub(**hub_kwargs(listen=f"127.0.0.1:{port}", **kwargs))
            for kwargs in (dict(max_clients=-1), dict(max_handshakes=-1),
                           dict(graceful_overall_ms=-1)):
                with self.subTest(kwargs=kwargs), self.assertRaises(OverflowError):
                    ScHub(**hub_kwargs(listen=f"127.0.0.1:{port}", **kwargs))
            # No Python callable can become the registry-locked admin policy.
            with self.assertRaises(TypeError):
                ScHub(**hub_kwargs(admission_policy=lambda _: True))  # type: ignore[arg-type]
            with self.assertRaises(TypeError):
                ScHub(**hub_kwargs(admission_policy=None))  # type: ignore[arg-type]
            # A fully valid constructor still performs no I/O: occupied port
            # plus absent credential files must not raise.
            ScHub(**hub_kwargs(
                listen=f"127.0.0.1:{port}",
                admission_policy="deny_all",
                max_clients=1,
                max_handshakes=1,
                graceful_disconnect_ack_ms=1000,
                graceful_ws_close_ms=1000,
                graceful_overall_ms=2000,
                handshake_tls_ms=1000,
                handshake_websocket_upgrade_ms=1000,
                handshake_connect_request_ms=5000,
            ))
            # Positional layout is unchanged: still no sixth positional slot.
            with self.assertRaises(TypeError):
                ScHub("127.0.0.1:0", "a", "b", HUB_VMAC, "c", b"\1" * 16)  # type: ignore[call-arg]


class HubTlsFixture(unittest.IsolatedAsyncioTestCase):
    """Minimal site CA + hub/server certificates via OpenSSL CLI."""

    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-hub-lifecycle-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)

        def run(*args):
            subprocess.run(["openssl", *args], cwd=cls.root, check=True,
                           capture_output=True, timeout=10)

        run("req", "-x509", "-newkey", "ec", "-pkeyopt",
            "ec_paramgen_curve:prime256v1", "-nodes", "-days", "2",
            "-subj", "/CN=site", "-keyout", "site.key", "-out", "site.pem",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        (cls.root / "leaf.cnf").write_text(
            "[leaf]\nbasicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature\n"
            "extendedKeyUsage=serverAuth,clientAuth\n"
            "subjectAltName=DNS:localhost,IP:127.0.0.1\n")
        for name in ("hub", "server"):
            run("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-subj", f"/CN={name}", "-keyout", f"{name}.key",
                "-out", f"{name}.csr")
            run("x509", "-req", "-in", f"{name}.csr", "-CA", "site.pem",
                "-CAkey", "site.key", "-CAcreateserial", "-days", "1",
                "-extfile", "leaf.cnf", "-extensions", "leaf", "-out", f"{name}.pem")

    def path(self, name):
        return str(self.root / name)

    def make_hub(self, **overrides: Any) -> ScHub:
        return ScHub(listen=overrides.get("listen", "127.0.0.1:0"),
                     cert=overrides.get("cert", self.path("hub.pem")),
                     key=overrides.get("key", self.path("hub.key")),
                     vmac=overrides.get("vmac", HUB_VMAC),
                     ca_cert=overrides.get("ca_cert", self.path("site.pem")),
                     device_uuid=overrides.get("device_uuid", HUB_UUID),
                     max_clients=overrides.get("max_clients", 256),
                     max_handshakes=overrides.get("max_handshakes", 256),
                     admission_policy=overrides.get("admission_policy", "allow_all"),
                     graceful_disconnect_ack_ms=overrides.get(
                         "graceful_disconnect_ack_ms", 5000),
                     graceful_ws_close_ms=overrides.get("graceful_ws_close_ms", 5000),
                     graceful_overall_ms=overrides.get("graceful_overall_ms", 15000),
                     handshake_tls_ms=overrides.get("handshake_tls_ms", 10000),
                     handshake_websocket_upgrade_ms=overrides.get(
                         "handshake_websocket_upgrade_ms", 10000),
                     handshake_connect_request_ms=overrides.get(
                         "handshake_connect_request_ms", 10000))

    def make_server(self, url):
        return BACnetServer(
            device_instance=1000, device_name="Lifecycle Device",
            transport="sc", sc_hub=url, sc_vmac=SERVER_VMAC,
            sc_device_uuid=SERVER_UUID, sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"),
        )


class HubLifecycleTests(HubTlsFixture):
    async def test_pre_start_state_and_idempotent_stop(self):
        hub = self.make_hub()
        self.assertIsNone(await hub.address())
        self.assertIsNone(await hub.url())
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.status()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.shutdown_gracefully()
        await asyncio.wait_for(hub.stop(), 5)

    async def test_status_snapshot_is_redacted_counts_and_labels(self):
        hub = self.make_hub(max_clients=7, max_handshakes=9)
        await asyncio.wait_for(hub.start(), 10)
        try:
            status = await hub.status()
            self.assertEqual(set(status), STATUS_KEYS)
            self.assertIs(status["listening"], True)
            self.assertEqual(status["max_clients"], 7)
            self.assertEqual(status["max_handshakes"], 9)
            for key in STATUS_KEYS - {"listening"}:
                self.assertIsInstance(status[key], int)
            self.assertEqual(status["client_count"], 0)
            self.assertEqual(status["handshake_count"], 0)
            self.assertEqual(status["admin_denied"], 0)
            # Redacted by construction: no key/cert/VMAC/UUID material.
            blob = repr(status).lower()
            for token in ("key", "cert", "pem", "vmac", "uuid", "BEGIN"):
                self.assertNotIn(token, blob)
        finally:
            await asyncio.wait_for(hub.stop(), 5)

    async def test_static_allow_and_deny_policy_outcomes(self):
        for policy, admitted in (("allow_all", True), ("deny_all", False)):
            with self.subTest(policy=policy):
                hub = self.make_hub(admission_policy=policy)
                await asyncio.wait_for(hub.start(), 10)
                try:
                    server = self.make_server(await hub.url())
                    if admitted:
                        await asyncio.wait_for(server.start(), 15)
                    else:
                        with self.assertRaises(BacnetError):
                            await asyncio.wait_for(server.start(), 15)
                    status = await hub.status()
                    self.assertEqual(status["client_count"], 1 if admitted else 0)
                    self.assertEqual(status["admin_denied"], 0 if admitted else 1)
                    if admitted:
                        await asyncio.wait_for(server.stop(), 5)
                finally:
                    await asyncio.wait_for(hub.stop(), 5)

    async def test_graceful_no_peers_then_use_after_close(self):
        hub = self.make_hub()
        await asyncio.wait_for(hub.start(), 10)
        self.assertEqual(await asyncio.wait_for(hub.shutdown_gracefully(), 20), "graceful")
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.status()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.shutdown_gracefully()
        # stop() stays a safe no-op after a graceful run.
        await asyncio.wait_for(hub.stop(), 5)

    async def test_repeated_forceful_close_is_idempotent(self):
        hub = self.make_hub()
        await asyncio.wait_for(hub.start(), 10)
        await asyncio.wait_for(hub.stop(), 5)
        await asyncio.wait_for(hub.stop(), 5)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.status()

    async def test_context_manager_exit_and_double_exit(self):
        async with self.make_hub() as hub:
            self.assertIsNotNone(await hub.address())
            self.assertTrue((await hub.status())["listening"])
            await asyncio.wait_for(hub.stop(), 5)
        # Explicit stop inside the body plus exit: still a safe no-op.
        await asyncio.wait_for(hub.stop(), 5)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await hub.status()

    async def test_cancellation_during_async_close(self):
        hub = self.make_hub()
        await asyncio.wait_for(hub.start(), 10)
        graceful = asyncio.ensure_future(hub.shutdown_gracefully())
        await asyncio.sleep(0)
        graceful.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await graceful
        # Shutdown ownership stays native: a later forceful close is safe.
        await asyncio.wait_for(hub.stop(), 5)

        hub = self.make_hub()
        await asyncio.wait_for(hub.start(), 10)
        stopping = asyncio.ensure_future(hub.stop())
        await asyncio.sleep(0)
        stopping.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await stopping
        await asyncio.wait_for(hub.stop(), 5)

    async def test_no_callback_registry_after_teardown(self):
        # Vacuous by design: no Python callback can be installed, so shutdown
        # races with active callbacks reduce to close-idempotence (proven
        # above). Assert the registry surface does not exist.
        hub = self.make_hub()
        names = [name for name in dir(hub)
                 if "callback" in name.lower() or name.startswith("on_")]
        self.assertEqual(names, [])
        params = inspect.signature(ScHub).parameters
        self.assertEqual(params["admission_policy"].default, "allow_all")
        await asyncio.wait_for(hub.start(), 10)
        await asyncio.wait_for(hub.stop(), 5)
        self.assertEqual(
            [name for name in dir(hub)
             if "callback" in name.lower() or name.startswith("on_")], [])
