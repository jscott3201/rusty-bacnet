"""Rejected native startup retains owned, explicitly untrusted TLS evidence."""
import asyncio
import errno
import contextlib
import io
from pathlib import Path
import runpy
import ssl

from rusty_bacnet import BacnetError, BacnetTransportError
from sc_client_fixture import ClientScFixture

EXAMPLE = runpy.run_path(str(Path(__file__).resolve().parents[3] / "examples/python/sc_peer_certificates.py"))


class ScUntrustedPeerTests(ClientScFixture):
    async def test_rejected_startup_keeps_exact_untrusted_certificate(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await hub.start()
        relay = await self.relay(await hub.url())
        client = self.client(relay.url, sc_ca_cert=self.path("foreign.pem"))
        with self.assertRaises(BacnetTransportError) as caught:
            await asyncio.wait_for(client.__aenter__(), 5)
        error = caught.exception
        self.assertIsInstance(error, BacnetError)
        self.assertIsInstance(error, OSError)
        self.assertIsNone(error.errno)  # InvalidData has no platform errno mapping.
        self.assertIsNone(error.strerror)
        self.assertIn("UnknownIssuer", str(error))
        info = getattr(error, "sc_untrusted_peer", None)
        self.assertIsNotNone(info, "rejected startup must retain its observed peer chain")
        from rusty_bacnet import ScUntrustedPeerInfo, ScPeerCertificate
        self.assertIsInstance(info, ScUntrustedPeerInfo)
        self.assertFalse(hasattr(info, "connection_id"))
        self.assertEqual(info.server_name, "localhost")
        self.assertIsInstance(info.certificates, tuple)
        self.assertEqual(len(info.certificates), 1)
        cert = info.certificates[0]
        self.assertIsInstance(cert, ScPeerCertificate)
        der = ssl.PEM_cert_to_DER_cert((self.root / "hub.pem").read_text())
        self.assertEqual(cert.der, der)
        self.assertEqual(cert.metadata.subject, "CN=hub")
        self.assertEqual(cert.metadata.issuer, "CN=site")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertIs(EXAMPLE["show_rejected"](error), info)
        self.assertIn("UNTRUSTED CN=hub CN=site", output.getvalue())
        for value, attribute, replacement in [
            (info, "certificates", ()), (info, "server_name", "changed"),
            (cert, "der", b"changed"), (cert.metadata, "issuer", "changed"),
        ]:
            with self.assertRaises(AttributeError):
                setattr(value, attribute, replacement)
        for value in [info, cert, cert.metadata, error]:
            self.assertNotIn("CN=hub", repr(value))
            self.assertNotIn(der.hex(), str(value))
        self.assertEqual(len(relay.connections), 1)
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await client.sc_peer_info()
        await client.stop()
        del caught, error, client
        self.assertEqual(info.certificates[0].der, der)

    async def test_failure_before_certificate_has_none(self):
        # A closed local listener gives a real TCP failure before observation.
        listener = await asyncio.start_server(lambda r, w: w.close(), "127.0.0.1", 0)
        port = listener.sockets[0].getsockname()[1]
        listener.close()
        await listener.wait_closed()
        client = self.client(f"wss://localhost:{port}")
        with self.assertRaises(BacnetTransportError) as caught:
            await asyncio.wait_for(client.__aenter__(), 5)
        self.assertEqual(caught.exception.errno, errno.ECONNREFUSED)
        self.assertIsNone(caught.exception.sc_untrusted_peer)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertIsNone(EXAMPLE["show_rejected"](caught.exception))

    async def test_accepted_example_runs_against_actual_hub(self):
        hub, _ = await self.serving_hub()
        client = self.client(await hub.url())
        async with client:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                peer = await EXAMPLE["show_accepted"](client)
            self.assertEqual(peer.certificates[0].metadata.subject, "CN=hub")
            self.assertIn(peer.leaf_sha256.hex(), output.getvalue())
            self.assertIn("CN=hub CN=site", output.getvalue())

