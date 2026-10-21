"""Installed-extension observations use independent OpenSSL/hashlib oracles."""
import asyncio
import hashlib
import math
import ssl
import subprocess
import time

from rusty_bacnet import (
    BACnetClient, ScCertificateMetadata, ScPeerCertificate, ScPeerInfo, ScReconnect,
)
from sc_client_fixture import ClientScFixture


class ScPeerInfoTests(ClientScFixture):
    async def test_exact_der_metadata_frozen_values_and_no_extra_connection(self):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        client = self.client(relay.url)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.sc_peer_info()
        await client.__aenter__()
        await relay.next_connection()
        before = time.time()
        peer = await client.sc_peer_info()
        after = time.time()
        self.assertIsInstance(peer, ScPeerInfo)
        observed = peer.observed_at_unix
        self.assertIsInstance(observed, float)
        self.assertTrue(math.isfinite(observed))
        # CPython 3.12 on Windows reads the coarse system clock, while Rust
        # reads its precise counterpart. Allow one reported clock tick plus
        # rounding of the independently converted floating-point timestamps.
        clock_tolerance = time.get_clock_info("time").resolution + 2 * max(
            math.ulp(before), math.ulp(observed), math.ulp(after))
        self.assertLessEqual(before - clock_tolerance, observed)
        self.assertLessEqual(observed, after + clock_tolerance)
        self.assertEqual(peer.server_name, "localhost")
        self.assertEqual(peer.peer_address, relay.url.replace("wss://localhost:", "127.0.0.1:"))
        self.assertEqual(peer.handshake_kind, "full")
        self.assertEqual(peer.protocol_version, "TLSv1_3")
        self.assertTrue(peer.cipher_suite.startswith("TLS13_"))
        self.assertIsInstance(peer.certificates, tuple)
        self.assertEqual(len(peer.certificates), 1)  # root omitted by the fixture
        cert = peer.certificates[0]
        self.assertIsInstance(cert, ScPeerCertificate)
        der = ssl.PEM_cert_to_DER_cert((self.root / "hub.pem").read_text())
        self.assertEqual(cert.der, der)
        self.assertEqual(peer.leaf_sha256, hashlib.sha256(der).digest())
        self.assertIsNone(cert.inspection_error)
        metadata = cert.metadata
        self.assertIsInstance(metadata, ScCertificateMetadata)
        self.assertEqual(metadata.subject, "CN=hub")
        self.assertEqual(metadata.issuer, "CN=site")
        dates = subprocess.run(
            ["openssl", "x509", "-in", self.path("hub.pem"), "-noout", "-dates"],
            check=True, capture_output=True, text=True, timeout=5).stdout.splitlines()
        self.assertEqual(metadata.not_before_unix, ssl.cert_time_to_seconds(dates[0].split("=", 1)[1]))
        self.assertEqual(metadata.not_after_unix, ssl.cert_time_to_seconds(dates[1].split("=", 1)[1]))
        self.assertEqual(metadata.public_key_algorithm_oid, "1.2.840.10045.2.1")
        self.assertEqual(metadata.signature_algorithm_oid, "1.2.840.10045.4.3.2")
        self.assertFalse(metadata.text_truncated)
        for value, attribute, replacement in [
            (peer, "connection_id", 1), (peer, "certificates", ()),
            (cert, "der", b"changed"), (metadata, "subject", "changed"),
        ]:
            with self.assertRaises(AttributeError):
                setattr(value, attribute, replacement)
        for value in [peer, cert, metadata]:
            self.assertNotIn("CN=", repr(value))
            self.assertNotIn(der.hex(), repr(value))
        for _ in range(10):
            again = await client.sc_peer_info()
            self.assertEqual(again.connection_id, peer.connection_id)
            self.assertEqual(again.certificates[0].der, der)
        self.assertEqual(len(relay.connections), 1)
        await asyncio.wait_for(client.stop(), 5)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.sc_peer_info()
        # Retained data must not keep the active Hub registration/socket alive.
        async def retired():
            while (await hub.status())["client_count"] != 1:
                await asyncio.sleep(0.01)
        await asyncio.wait_for(retired(), 3)
        self.assertEqual(peer.certificates[0].der, der)
        self.assertEqual(metadata.subject, "CN=hub")

    async def test_rotation_disconnect_and_retained_history(self):
        first_hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, first_hub)
        await first_hub.start()
        relay = await self.relay(await first_hub.url())
        client = self.client(relay.url, reconnect=ScReconnect(
            initial_delay_ms=80, max_delay_ms=80, max_retries=10))
        await client.__aenter__()
        await relay.next_connection()
        old = await client.sc_peer_info()
        relay.pause()
        await relay.disconnect()
        await relay.next_connection()  # fresh TCP, held before TLS/SC admission
        self.assertIsNone(await client.sc_peer_info())
        replacement = self.hub(cert=self.path("server.pem"), key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_hub, replacement)
        await replacement.start()
        relay.resume(await replacement.url())

        async def restored():
            while (peer := await client.sc_peer_info()) is None:
                await asyncio.sleep(0.01)
            return peer
        new = await asyncio.wait_for(restored(), 5)
        self.assertNotEqual(old.connection_id, new.connection_id)
        self.assertEqual(old.peer_address, new.peer_address)  # stable relay, rotated leaf
        self.assertNotEqual(old.leaf_sha256, new.leaf_sha256)
        der = ssl.PEM_cert_to_DER_cert((self.root / "server.pem").read_text())
        self.assertEqual(new.certificates[0].der, der)
        self.assertEqual(new.leaf_sha256, hashlib.sha256(der).digest())
        self.assertEqual(old.certificates[0].metadata.subject, "CN=hub")
        self.assertEqual(new.certificates[0].metadata.subject, "CN=server")
        await client.stop()

    async def test_cancelled_observation_and_concurrent_close_release_leases(self):
        hub, _ = await self.serving_hub()
        client = self.client(await hub.url())
        await client.__aenter__()
        retained = await client.sc_peer_info()
        observation = asyncio.ensure_future(client.sc_peer_info())
        observation.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await observation
        pending = [asyncio.ensure_future(client.sc_peer_info()) for _ in range(32)]
        close = asyncio.ensure_future(client.stop())
        results = await asyncio.wait_for(asyncio.gather(*pending, close, return_exceptions=True), 5)
        self.assertIsNone(results[-1])
        for value in results[:-1]:
            if isinstance(value, Exception):
                self.assertIsInstance(value, RuntimeError)
                self.assertTrue("closing" in str(value) or "not started" in str(value))
            else:
                self.assertEqual(value.connection_id, retained.connection_id)
        self.assertEqual(retained.certificates[0].metadata.subject, "CN=hub")

    async def test_started_non_sc_client_has_explicit_error(self):
        async with BACnetClient("127.0.0.1", 0) as client:
            with self.assertRaisesRegex(RuntimeError, "requires an SC client"):
                await client.sc_peer_info()
