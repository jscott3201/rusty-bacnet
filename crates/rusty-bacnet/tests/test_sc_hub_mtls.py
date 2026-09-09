"""Installed-native Python hub authentication; no system trust or persistent keys.

OpenSSL CLI creates a temporary site CA and distinct operational certificates.
TLS negatives use the independent stdlib TLS client, not SC reconnect timeouts.
"""
import asyncio
import inspect
import os
import socket
import ssl
import subprocess
import tempfile
import unittest
from pathlib import Path

from rusty_bacnet import BACnetServer, BacnetError, ScHub


class HubMtlsTests(unittest.IsolatedAsyncioTestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="bacnet-hub-mtls-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)

        def run(*args):
            subprocess.run(["openssl", *args], cwd=cls.root, check=True,
                           capture_output=True, timeout=10)

        # Explicit CA constraints; leaves are never substituted for trust anchors.
        for ca in ("site", "foreign"):
            run("req", "-x509", "-newkey", "ec", "-pkeyopt",
                "ec_paramgen_curve:prime256v1", "-nodes", "-days", "2",
                "-subj", f"/CN={ca}", "-keyout", f"{ca}.key", "-out", f"{ca}.pem",
                "-addext", "basicConstraints=critical,CA:TRUE",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign")
        (cls.root / "index").write_text("")
        (cls.root / "serial").write_text("1000\n")
        (cls.root / "ca.cnf").write_text(
            "[ca]\ndefault_ca=issuer\n[issuer]\ndatabase=index\n"
            "serial=serial\nnew_certs_dir=.\ncertificate=site.pem\n"
            "private_key=site.key\ndefault_md=sha256\npolicy=policy\n"
            "unique_subject=no\n[policy]\ncommonName=supplied\n"
            "[leaf]\nbasicConstraints=critical,CA:FALSE\n"
            "keyUsage=critical,digitalSignature\n"
            "extendedKeyUsage=serverAuth,clientAuth\n"
            "subjectAltName=DNS:localhost,IP:127.0.0.1\n")
        for name in ("hub", "server", "client", "wrong", "expired", "future"):
            run("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1",
                "-nodes", "-subj", f"/CN={name}", "-keyout", f"{name}.key",
                "-out", f"{name}.csr")
            if name in ("expired", "future"):
                start, end = (("20000101000000Z", "20010101000000Z") if name == "expired"
                              else ("20990101000000Z", "21000101000000Z"))
                run("ca", "-batch", "-config", "ca.cnf", "-extensions", "leaf",
                    "-in", f"{name}.csr", "-out", f"{name}.pem", "-notext",
                    "-startdate", start, "-enddate", end)
            else:
                ca = "foreign" if name == "wrong" else "site"
                run("x509", "-req", "-in", f"{name}.csr", "-CA", f"{ca}.pem",
                    "-CAkey", f"{ca}.key", "-CAcreateserial", "-days", "1",
                    "-extfile", "ca.cnf", "-extensions", "leaf", "-out", f"{name}.pem")

    def path(self, name):
        return str(self.root / name)

    def hub(self, **overrides):
        return ScHub(listen=overrides.get("listen", "127.0.0.1:0"),
                     cert=overrides.get("cert", self.path("hub.pem")),
                     key=overrides.get("key", self.path("hub.key")),
                     vmac=b"\x02\0\0\0\0\1",
                     ca_cert=overrides.get("ca_cert", self.path("site.pem")))

    def websocket(self, address, peer=None, ca="site", version=ssl.TLSVersion.TLSv1_3,
                  read_property=False):
        """Bounded independent TLS + WebSocket handshake, including TLS 1.3 alerts.

        The post-handshake read matters: TLS 1.3 may report missing-client-cert
        rejection only after wrap_socket has returned on the initiating side.
        This synchronous call is always joined, not abandoned on asyncio timeout.
        """
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.minimum_version = ctx.maximum_version = version
        ctx.load_verify_locations(self.path(f"{ca}.pem"))
        if peer:
            ctx.load_cert_chain(self.path(f"{peer}.pem"), self.path(f"{peer}.key"))
        port = int(address.rsplit(":", 1)[1])
        with socket.create_connection(("127.0.0.1", port), timeout=3) as tcp:
            with ctx.wrap_socket(tcp, server_hostname="localhost") as tls:
                tls.sendall(
                    b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
                    b"Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
                    b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                    b"Sec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
                response = tls.recv(4096)
                self.assertTrue(response.startswith(b"HTTP/1.1 101"), response)
                if read_property:
                    self.exchange_read_property(tls)
                return tls.version()

    def exchange_read_property(self, tls):
        # Literal SC Connect-Request/Accept, modeled on sc_frame's independent
        # connect_test_support vectors. A nonzero UUID avoids the existing
        # all-zero default shared by Python BACnetClient/BACnetServer nodes.
        def send(payload):
            self.assertLess(len(payload), 126)
            mask = os.urandom(4)
            tls.sendall(bytes([0x82, 0x80 | len(payload)]) + mask +
                        bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload)))

        def exact(size):
            data = b""
            while len(data) < size:
                part = tls.recv(size - len(data))
                self.assertTrue(part, "peer closed during frame")
                data += part
            return data

        def receive():
            header = exact(2)
            self.assertEqual(header[0], 0x82)
            self.assertFalse(header[1] & 0x80)
            length = header[1] & 0x7f
            if length == 126:
                length = int.from_bytes(exact(2), "big")
            self.assertLess(length, 4096)
            return exact(length)

        vmac = b"\x02\0\0\0\0\3"
        send(b"\x06\x00\x22\x33" + vmac + b"\x01" * 16 + b"\x05\xc4\x05\xc4")
        try:
            accepted = receive()
        except TimeoutError as error:
            raise AssertionError("timed out waiting for SC ConnectAccept") from error
        self.assertEqual(accepted[:4], b"\x07\x00\x22\x33")
        self.invoke = getattr(self, "invoke", 0) + 1
        invoke = bytes([self.invoke])
        # Encapsulated-NPDU to destination; hub supplies the originating VMAC.
        send(b"\x01\x04\x22\x34" + b"\x02\0\0\0\0\2" +
             b"\x01\x04\x00\x03" + invoke + b"\x0c\x0c\0\0\0\0\x19\x55")
        # Ignore a bounded number of unsolicited I-Am packets; the reply is
        # independently asserted through exact APDU bytes (REAL 72.5).
        seen = []
        for _ in range(8):
            try:
                reply = receive()
            except TimeoutError as error:
                raise AssertionError(f"timed out waiting for ReadProperty ACK; frames={seen}") from error
            seen.append(reply.hex())
            if b"\x30" + invoke + b"\x0c" in reply:
                self.assertTrue(reply.endswith(
                    b"\x30" + invoke + b"\x0c\x0c\0\0\0\0\x19\x55\x3e\x44\x42\x91\0\0\x3f"), reply)
                return
        self.fail("no ReadProperty ACK within bounded incoming frames")

    async def test_absent_ca_cannot_admit_certificate_less_peer(self):
        # Baseline behavioral RED: the old constructor starts, then permits an
        # unauthenticated TLS/WebSocket upgrade. New early validation is valid.
        try:
            hub = self.hub(ca_cert=None)
        except ValueError as error:
            self.assertIn("ca_cert", str(error))
            return
        try:
            await asyncio.wait_for(hub.start(), 3)
            with self.assertRaises(ssl.SSLError):
                await asyncio.to_thread(self.websocket, await hub.address())
        finally:
            await asyncio.wait_for(hub.stop(), 3)

    async def test_constructor_signature_and_required_ca(self):
        parameters = inspect.signature(ScHub).parameters
        self.assertEqual(list(parameters), ["listen", "cert", "key", "vmac", "ca_cert"])
        self.assertIsNone(parameters["ca_cert"].default)
        self.assertEqual(parameters["ca_cert"].kind, inspect.Parameter.POSITIONAL_OR_KEYWORD)
        args = ("127.0.0.1:0", self.path("hub.pem"), self.path("hub.key"), b"\x02\0\0\0\0\1")
        for extra in [(), (None,), ("",)]:
            with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, "ca_cert"):
                ScHub(*args, *extra)
        hub = ScHub(*args, self.path("site.pem"))
        self.assertIsNone(await hub.address())
        try:
            await asyncio.wait_for(hub.start(), 3)
            self.assertEqual(await asyncio.to_thread(self.websocket, await hub.address(), "client"),
                             "TLSv1.3")
        finally:
            await asyncio.wait_for(hub.stop(), 3)

    async def test_invalid_files_fail_before_bind(self):
        for name, content in [("empty.pem", ""), ("garbage.pem", "not a certificate"),
                              ("invalid.pem", "-----BEGIN CERTIFICATE-----\nYQ==\n-----END CERTIFICATE-----\n")]:
            (self.root / name).write_text(content)
        cases = [({"ca_cert": self.path(name)}, message) for name, message in [
            ("empty.pem", "no CA certificates"), ("garbage.pem", "no CA certificates"),
            ("invalid.pem", "failed to add CA cert"), ("missing.pem", "failed to read CA cert")]]
        cases += [({"ca_cert": str(self.root)}, "failed to read CA cert"),
                  ({"cert": self.path("empty.pem")}, "no server certificates"),
                  ({"cert": self.path("missing.pem")}, "failed to read server cert"),
                  ({"key": self.path("garbage.pem")}, "failed to parse server key"),
                  ({"key": self.path("missing.pem")}, "failed to read server key"),
                  ({"key": self.path("client.key")}, "TLS server config error")]
        for overrides, message in cases:
            with self.subTest(overrides=overrides):
                # Holding an already-bound port proves TLS validation precedes
                # bind: otherwise AddressInUse would replace the expected error.
                with socket.socket() as reservation:
                    reservation.bind(("127.0.0.1", 0))
                    reservation.listen()
                    address = f"127.0.0.1:{reservation.getsockname()[1]}"
                    hub = self.hub(listen=address, **overrides)
                    try:
                        with self.assertRaisesRegex(BacnetError, message):
                            await asyncio.wait_for(hub.start(), 3)
                        self.assertIsNone(await hub.address())
                    finally:
                        await asyncio.wait_for(hub.stop(), 3)
                with socket.socket() as probe:
                    probe.bind(("127.0.0.1", int(address.rsplit(":", 1)[1])))

    async def test_rejected_peers_leave_trusted_read_property_usable(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        url, address = await hub.url(), await hub.address()
        server = BACnetServer(3000, "MTLS server", transport="sc", sc_hub=url,
                              sc_vmac=b"\x02\0\0\0\0\2", sc_ca_cert=self.path("site.pem"),
                              sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        async def barrier():
            self.assertEqual(await asyncio.to_thread(
                self.websocket, address, "client", read_property=True), "TLSv1.3")

        await barrier()
        for peer, ca, version in [(None, "site", ssl.TLSVersion.TLSv1_3),
                                  ("wrong", "site", ssl.TLSVersion.TLSv1_3),
                                  ("expired", "site", ssl.TLSVersion.TLSv1_3),
                                  ("future", "site", ssl.TLSVersion.TLSv1_3),
                                  ("client", "foreign", ssl.TLSVersion.TLSv1_3),
                                  ("client", "site", ssl.TLSVersion.TLSv1_2)]:
            with self.subTest(peer=peer, ca=ca, version=version):
                # Timeout, refused TCP, EOF, or a WebSocket error is NOT a TLS
                # rejection oracle. Only a completed SSL protocol error passes.
                with self.assertRaises(ssl.SSLError) as rejected:
                    await asyncio.to_thread(self.websocket, address, peer, ca, version)
                self.assertNotIsInstance(rejected.exception, ssl.SSLEOFError)
                self.assertRegex(rejected.exception.reason, "ALERT|CERTIFICATE_VERIFY_FAILED")
                await barrier()
        self.assertEqual(await asyncio.to_thread(self.websocket, address, "client"), "TLSv1.3")
        await barrier()

    async def stop_hub(self, hub):
        await asyncio.wait_for(hub.stop(), 5)

    async def stop_server(self, server):
        await asyncio.wait_for(server.stop(), 5)
