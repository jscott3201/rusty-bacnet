"""Installed-native Python hub/node authentication; no system trust or persistent keys.

OpenSSL CLI creates a temporary site CA and distinct operational certificates.
TLS negatives use independent stdlib TLS peers, not SC reconnect timeouts.
"""
import asyncio
import base64
import hashlib
import inspect
import itertools
import os
import socket
import ssl
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetError, ObjectIdentifier, ObjectType,
    PropertyIdentifier, ScHub,
)

# Deterministic test provisioning only, not defaults for deployed devices.
SERVER_UUID = bytes.fromhex("8e62ac46d7084226913776a32b619315")
CLIENT_UUID = bytes.fromhex("95dfe4ef97f6490d9a2cf2b4b0c0e682")


class MtlsFixture(unittest.IsolatedAsyncioTestCase):
    """Shared fixture only: no inherited test methods or duplicate discovery."""
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
                  read_property=False, serve_ready=None, serve_done=None):
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
                if read_property or serve_ready is not None:
                    self.exchange_read_property(tls, serve_ready, serve_done)
                return tls.version()

    def exchange_read_property(self, tls, serve_ready=None, serve_done=None):
        # Literal SC Connect-Request/Accept, modeled on sc_frame's independent
        # connect_test_support vectors. This peer has its own provisioned UUID.
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
        send(b"\x06\x00\x22\x33" + vmac + CLIENT_UUID + b"\x05\xc4\x05\xc4")
        try:
            accepted = receive()
        except TimeoutError as error:
            raise AssertionError("timed out waiting for SC ConnectAccept") from error
        self.assertEqual(accepted[:4], b"\x07\x00\x22\x33")
        if serve_ready is not None:
            assert serve_done is not None
            serve_ready.set()
            # Independently respond to a native client's ReadProperty AI0/PV.
            for _ in range(8):
                request = receive()
                if request.endswith(b"\x0c\x0c\0\0\0\0\x19\x55"):
                    self.assertEqual(request[:2], b"\x01\x08")
                    self.assertEqual(request[4:10], b"\x02\0\0\0\0\2")
                    self.assertEqual(request[10:13], b"\x01\x04\x02")
                    invoke = request[14:15]
                    send(b"\x01\x04\x22\x35" + b"\x02\0\0\0\0\2" + b"\x01\x00" +
                         b"\x30" + invoke + b"\x0c\x0c\0\0\0\0\x19\x55\x3e\x44\x42\x91\0\0\x3f")
                    self.assertTrue(serve_done.wait(3), "native client did not finish reading")
                    return
            self.fail("no ReadProperty request within bounded incoming frames")
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

    async def stop_hub(self, hub):
        await asyncio.wait_for(hub.stop(), 5)

    async def stop_server(self, server):
        await asyncio.wait_for(server.stop(), 5)


class HubMtlsTests(MtlsFixture):
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
                              sc_device_uuid=SERVER_UUID,
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

class NodeCredentialTests(unittest.TestCase):
    def test_sc_constructor_requires_each_credential(self):
        # Genuine baseline RED: both existing APIs accepted missing credentials.
        # This does not claim a bypass of the now-mandatory hub verification.
        fields = ("sc_ca_cert", "sc_client_cert", "sc_client_key")
        omitted = object()
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            for values in itertools.product((omitted, None, "", "placeholder.pem"), repeat=3):
                kwargs = {name: value for name, value in zip(fields, values) if value is not omitted}
                with self.subTest(api=api.__name__, credentials=kwargs):
                    if all(value == "placeholder.pem" for value in values):
                        api(*args, transport="sc", sc_device_uuid=SERVER_UUID, **kwargs)
                    else:
                        with self.assertRaisesRegex(ValueError, "sc_.*(cert|key)"):
                            api(*args, transport="sc", **kwargs)

    def test_non_sc_credentials_remain_optional_and_unread(self):
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            for transport in ("bip", "ipv6", "mstp"):
                for value in (None, "", "not-a-real-file.pem"):
                    with self.subTest(api=api.__name__, transport=transport, value=value):
                        api(*args, transport=transport, sc_ca_cert=value,
                            sc_client_cert=value, sc_client_key=value)

    def test_sc_positional_defaults_and_following_slots(self):
        for api in (BACnetClient, BACnetServer):
            params = inspect.signature(api).parameters
            for name in ("sc_ca_cert", "sc_client_cert", "sc_client_key",
                         "sc_heartbeat_interval_ms", "sc_heartbeat_timeout_ms", "ipv6_interface"):
                self.assertIsNone(params[name].default)
                self.assertEqual(params[name].kind, inspect.Parameter.POSITIONAL_OR_KEYWORD)
        # Nonexistent paths are allowed until startup, including positional SC use.
        BACnetClient("0.0.0.0", 0, "255.255.255.255", 6000, "sc", "wss://localhost:1",
                     b"\x02\0\0\0\0\2", "ca.pem", "cert.pem", "key.pem", 30000, 60000, "::1",
                     sc_device_uuid=CLIENT_UUID)
        BACnetServer(3000, "SC", "0.0.0.0", 0, "255.255.255.255", "sc", "wss://localhost:1",
                     b"\x02\0\0\0\0\2", "ca.pem", "cert.pem", "key.pem", 30000, 60000, "::1",
                     "dcc-password", "reinit-password", sc_device_uuid=SERVER_UUID)


class NodeMtlsTests(MtlsFixture):
    def node(self, api, url, **overrides):
        kwargs = dict(transport="sc", sc_hub=url, sc_vmac=b"\x02\0\0\0\0\2",
                      sc_device_uuid=SERVER_UUID,
                      sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
                      sc_client_key=self.path("server.key"))
        kwargs.update(overrides)
        return api(*((3000,) if api is BACnetServer else ()), **kwargs)

    async def start_node(self, node):
        if isinstance(node, BACnetServer):
            await asyncio.wait_for(node.start(), 5)
        else:
            self.assertIs(await asyncio.wait_for(node.__aenter__(), 5), node)

    async def test_invalid_local_files_do_not_dial_or_drain(self):
        cases = []
        for field, label, read_error in [
            ("sc_ca_cert", "CERTIFICATE", "CA cert"),
            ("sc_client_cert", "CERTIFICATE", "client cert"),
            ("sc_client_key", "PRIVATE KEY", "client key"),
        ]:
            for kind, contents in [("empty", ""), ("garbage", "not PEM"),
                                   ("bad-base64", f"-----BEGIN {label}-----\n%%%\n-----END {label}-----\n"),
                                   ("bad-der", f"-----BEGIN {label}-----\nYQ==\n-----END {label}-----\n")]:
                name = f"{field}-{kind}.pem"
                (self.root / name).write_text(contents)
                cases.append(({field: self.path(name)}, "TLS config error:"))
            cases.extend([({field: self.path("does-not-exist.pem")}, f"failed to read {read_error}"),
                          ({field: str(self.root)}, f"failed to read {read_error}")])
        cases.append(({"sc_client_key": self.path("client.key")}, "TLS client auth error"))
        for api in (BACnetClient, BACnetServer):
            for overrides, message in cases:
                with self.subTest(api=api.__name__, overrides=overrides), socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    listener.listen()
                    listener.setblocking(False)
                    address = listener.getsockname()
                    node = self.node(api, f"wss://127.0.0.1:{address[1]}", **overrides)
                    if isinstance(node, BACnetServer):
                        node.add_analog_input(0, "Pending AI", 64, 72.5)
                    try:
                        with self.assertRaisesRegex(RuntimeError, message) as rejected:
                            await self.start_node(node)
                        self.assertTrue(str(rejected.exception).startswith("TLS config error:"))
                        # A completed TCP dial would be queued even if immediately
                        # closed. Check the live accept queue, not a dial timeout.
                        with self.assertRaises(BlockingIOError):
                            listener.accept()
                        if isinstance(node, BACnetServer):
                            self.assertEqual(getattr(node, "_pending_registration_count")(), 1)
                    finally:
                        await self.stop_server(node)
                    # Qualify that this exact listener observes an actual dial.
                    listener.settimeout(3)
                    with socket.create_connection(address, timeout=3):
                        accepted, _ = listener.accept()
                        accepted.close()

    async def test_server_repairs_files_and_serves_preserved_registration(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        paths = {name: self.path(f"retry-{name}.pem") for name in
                 ("sc_ca_cert", "sc_client_cert", "sc_client_key")}
        node = self.node(BACnetServer, await hub.url(), **paths)
        self.addAsyncCleanup(self.stop_server, node)
        pending_count = getattr(node, "_pending_registration_count")
        node.add_analog_input(0, "Preserved AI", 64, 72.5)
        # Construction did not read these still-missing files. Repeated failures
        # cannot consume registrations; each start reloads repaired credentials.
        for field, source in [("sc_ca_cert", "site.pem"), ("sc_client_cert", "server.pem"),
                              ("sc_client_key", "client.key")]:
            with self.assertRaisesRegex(RuntimeError, "TLS config error:"):
                await self.start_node(node)
            self.assertEqual(pending_count(), 1)
            Path(paths[field]).write_bytes((self.root / source).read_bytes())
        with self.assertRaisesRegex(RuntimeError, "TLS client auth error"):
            await self.start_node(node)
        self.assertEqual(pending_count(), 1)
        node.add_binary_input(1, "Added after failure")
        Path(paths["sc_client_key"]).write_bytes((self.root / "server.key").read_bytes())
        await self.start_node(node)
        self.assertEqual(pending_count(), 0)
        self.assertEqual(await asyncio.to_thread(self.websocket, await hub.address(), "client",
                                                read_property=True), "TLSv1.3")

    async def test_client_repairs_files_and_reads_through_trusted_hub(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        key = self.root / "retry-client.key"
        key.write_text("not a key")
        node = self.node(BACnetClient, await hub.url(), sc_client_key=str(key))
        self.addAsyncCleanup(self.stop_server, node)
        with self.assertRaisesRegex(RuntimeError, "TLS config error:"):
            await self.start_node(node)
        key.write_bytes((self.root / "server.key").read_bytes())
        await self.start_node(node)
        ready = threading.Event()
        done = threading.Event()
        peer = asyncio.create_task(asyncio.to_thread(
            self.websocket, await hub.address(), "client", serve_ready=ready, serve_done=done))
        try:
            self.assertTrue(await asyncio.to_thread(ready.wait, 3), "raw peer did not connect")
            value = await asyncio.wait_for(node.read_property(
                "02:00:00:00:00:03", ObjectIdentifier(ObjectType.ANALOG_INPUT, 0),
                PropertyIdentifier.PRESENT_VALUE), 3)
            self.assertEqual(value.value, 72.5)
        finally:
            # Socket operations have their own bounds; always join the worker.
            done.set()
            self.assertEqual(await peer, "TLSv1.3")

    def rejected_tls_peer(self, listener, cert, trust, version):
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.minimum_version = ctx.maximum_version = version
        ctx.load_cert_chain(self.path(f"{cert}.pem"), self.path(f"{cert}.key"))
        ctx.load_verify_locations(self.path(f"{trust}.pem"))
        ctx.verify_mode = ssl.CERT_REQUIRED
        listener.settimeout(3)
        tcp, _ = listener.accept()
        with tcp:
            tcp.settimeout(3)
            with self.assertRaises(ssl.SSLError) as rejected:
                with ctx.wrap_socket(tcp, server_side=True) as tls:
                    # TLS 1.3 rejection may arrive after handshake completion.
                    tls.recv(4096)
            self.assertNotIsInstance(rejected.exception, ssl.SSLEOFError)
            self.assertRegex(rejected.exception.reason, "ALERT|CERTIFICATE_VERIFY_FAILED|UNSUPPORTED_PROTOCOL")

    async def test_nodes_reject_untrusted_or_inactive_peers_and_tls12(self):
        # Independent TLS acceptor proves actual alerts, not SC timeout behavior.
        cases = [(cert, "site", "server", ssl.TLSVersion.TLSv1_3)
                 for cert in ("wrong", "expired", "future")]
        cases += [("hub", "site", cert, ssl.TLSVersion.TLSv1_3)
                  for cert in ("wrong", "expired", "future")]
        cases += [("hub", "foreign", "server", ssl.TLSVersion.TLSv1_3),
                  ("hub", "site", "server", ssl.TLSVersion.TLSv1_2)]
        for api in (BACnetClient, BACnetServer):
            for cert, trust, local, version in cases:
                with self.subTest(api=api.__name__, cert=cert, trust=trust, local=local, version=version):
                    with socket.socket() as listener:
                        listener.bind(("127.0.0.1", 0))
                        listener.listen()
                        node = self.node(api, f"wss://localhost:{listener.getsockname()[1]}",
                                         sc_client_cert=self.path(f"{local}.pem"),
                                         sc_client_key=self.path(f"{local}.key"))
                        peer = asyncio.create_task(asyncio.to_thread(
                            self.rejected_tls_peer, listener, cert, trust, version))
                        try:
                            with self.assertRaises(BacnetError):
                                await self.start_node(node)
                        finally:
                            try:
                                await peer
                            finally:
                                await self.stop_server(node)


class NodeIdentityMtlsTests(MtlsFixture):
    """Real native nodes, independent wire bytes, and intended hub replacement."""
    def node(self, api, url, uuid, vmac=None):
        server = api is BACnetServer
        node = api(*((3000,) if server else ()), transport="sc", sc_hub=url,
                   sc_vmac=vmac or b"\x02\0\0\0\0" + bytes([4 if server else 2]),
                   sc_device_uuid=uuid, sc_ca_cert=self.path("site.pem"),
                   sc_client_cert=self.path("server.pem" if server else "client.pem"),
                   sc_client_key=self.path("server.key" if server else "client.key"))
        self.addAsyncCleanup(self.stop_server, node)
        if server:
            node.add_analog_input(0, "Identity AI", 64, 72.5)
        return node

    async def start_node(self, node):
        await asyncio.wait_for(node.start() if isinstance(node, BACnetServer)
                               else node.__aenter__(), 5)

    async def read_value(self, client):
        value = await asyncio.wait_for(client.read_property(
            "02:00:00:00:00:04", ObjectIdentifier(ObjectType.ANALOG_INPUT, 0),
            PropertyIdentifier.PRESENT_VALUE), 3)
        self.assertEqual(value.value, 72.5)

    async def frame(self, reader, masked):
        header = await asyncio.wait_for(reader.readexactly(2), 3)
        self.assertTrue(header[0] & 0x80, "expected final frame")
        self.assertEqual(bool(header[1] & 0x80), masked)
        size = header[1] & 0x7f
        if size == 126:
            size = int.from_bytes(await asyncio.wait_for(reader.readexactly(2), 3), "big")
        self.assertLess(size, 4096)
        mask = await asyncio.wait_for(reader.readexactly(4), 3) if masked else None
        payload = await asyncio.wait_for(reader.readexactly(size), 3)
        if mask:
            payload = bytes(value ^ mask[i % 4] for i, value in enumerate(payload))
        return header[0] & 0x0f, payload

    async def send_frame(self, writer, payload, masked=False, opcode=2):
        self.assertLess(len(payload), 126)
        mask = b"\x12\x34\x56\x78" if masked else b""
        wire = bytes(value ^ mask[i % 4] for i, value in enumerate(payload)) if mask else payload
        writer.write(bytes([0x80 | opcode, len(payload) | (0x80 if masked else 0)]) + mask + wire)
        await asyncio.wait_for(writer.drain(), 3)

    async def test_uuid_owned_wire_bytes_across_stop_start_and_recreation(self):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(self.path("hub.pem"), self.path("hub.key"))
        context.load_verify_locations(self.path("site.pem"))
        context.verify_mode = ssl.CERT_REQUIRED
        for api, uuid, vmac in [(BACnetClient, CLIENT_UUID, b"\x02\0\0\0\0\2"),
                                (BACnetServer, SERVER_UUID, b"\x02\0\0\0\0\4")]:
            tasks, observed = [], []

            async def peer(reader, writer):
                tasks.append(asyncio.current_task())
                try:
                    request = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 3)
                    headers = dict(line.split(b":", 1) for line in request.split(b"\r\n")[1:] if b":" in line)
                    key = next(value.strip() for name, value in headers.items()
                               if name.lower() == b"sec-websocket-key")
                    accept = base64.b64encode(hashlib.sha1(
                        key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
                    writer.write(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                                 b"Connection: Upgrade\r\nSec-WebSocket-Accept: " + accept +
                                 b"\r\nSec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
                    await asyncio.wait_for(writer.drain(), 3)
                    opcode, connect = await self.frame(reader, True)
                    # Independent base-2020 AB.2.10 offsets, not a product decoder.
                    self.assertEqual(opcode, 2)
                    self.assertEqual(len(connect), 30)
                    self.assertEqual(connect[:4], b"\x06\0\0\1")
                    self.assertEqual(connect[4:10], vmac)
                    self.assertEqual(connect[10:26], uuid)
                    self.assertEqual(connect[26:], b"\x16\x49\x05\xc4")
                    observed.append(connect[10:26])
                    await self.send_frame(writer, b"\x07\0\0\1" + b"\x02\0\0\0\0\x09" +
                                          bytes.fromhex("4a015cf2ec394d58ac2d11e1761ce86d") + b"\x05\xc4\x05\xc4")
                    # Identity is established above. Drain until owned shutdown;
                    # server stop/drop does not promise a graceful SC exchange.
                    async def drain_until_closed():
                        while await reader.read(4096):
                            pass
                    await asyncio.wait_for(drain_until_closed(), 3)
                finally:
                    writer.close()
                    await asyncio.wait_for(writer.wait_closed(), 3)

            listener = await asyncio.start_server(peer, "127.0.0.1", 0, ssl=context)
            url = f"wss://localhost:{listener.sockets[0].getsockname()[1]}"
            source = bytearray(uuid)
            node = self.node(api, url, source)
            source[:] = bytes(16)  # Cannot change retained configuration.
            try:
                for lifecycle in range(3):
                    if lifecycle == 2:
                        node = self.node(api, url, uuid)  # Fresh application object, same stored UUID.
                    await self.start_node(node)
                    await self.stop_server(node)
                self.assertEqual(observed, [uuid] * 3)
            finally:
                await self.stop_server(node)
                listener.close()
                await listener.wait_closed()
                # Bound every socket operation and join all owned peer tasks.
                await asyncio.gather(*tasks)

    async def incumbent(self, address, uuid):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_verify_locations(self.path("site.pem"))
        context.load_cert_chain(self.path("client.pem"), self.path("client.key"))
        reader, writer = await asyncio.wait_for(asyncio.open_connection(
            "127.0.0.1", int(address.rsplit(":", 1)[1]), ssl=context, server_hostname="localhost"), 3)
        try:
            writer.write(b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
                         b"Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
                         b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                         b"Sec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
            await asyncio.wait_for(writer.drain(), 3)
            response = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 3)
            self.assertTrue(response.startswith(b"HTTP/1.1 101"), response)
            await self.send_frame(writer, b"\x06\0\x22\x33" + b"\x02\0\0\0\0\3" + uuid +
                                  b"\x05\xc4\x05\xc4", masked=True)
            opcode, accepted = await self.frame(reader, False)
            self.assertEqual(opcode, 2)
            self.assertEqual(accepted[:4], b"\x07\0\x22\x33")
            return reader, writer
        except BaseException:
            writer.close()
            await asyncio.wait_for(writer.wait_closed(), 3)
            raise

    async def test_distinct_nodes_and_same_uuid_replacement_leave_other_node_usable(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        url, address = await hub.url(), await hub.address()
        for api, uuid, other_api, other_uuid in [
                (BACnetClient, CLIENT_UUID, BACnetServer, SERVER_UUID),
                (BACnetServer, SERVER_UUID, BACnetClient, CLIENT_UUID)]:
            other = self.node(other_api, url, other_uuid)
            await self.start_node(other)
            node = self.node(api, url, uuid)
            try:
                await self.start_node(node)
                await self.read_value(node if api is BACnetClient else other)
                await self.stop_server(node)
                for lifecycle in range(2):
                    reader, writer = await self.incumbent(address, uuid)
                    try:
                        # A fresh client resets its invoke IDs. Give it a fresh
                        # VMAC so the surviving server's exact-request duplicate
                        # cache is not this identity test's accidental subject.
                        vmac = b"\x02\0\0\0\0" + bytes([5 + lifecycle]) if api is BACnetClient else None
                        node = self.node(api, url, uuid, vmac)
                        await self.start_node(node)
                        # Same UUID with a different VMAC replaces the incumbent;
                        # observe actual WebSocket Close, not a timeout/failed read.
                        for _ in range(8):
                            opcode, data = await self.frame(reader, False)
                            if opcode == 8:
                                self.assertEqual(data, b"")  # Hub's replacement Close(None).
                                break
                            self.assertEqual(opcode, 2)
                            self.assertEqual(data[0], 1)
                        else:
                            self.fail("same-identity incumbent did not close")
                        await self.read_value(node if api is BACnetClient else other)
                    finally:
                        writer.close()
                        await asyncio.wait_for(writer.wait_closed(), 3)
                        await self.stop_server(node)
            finally:
                await self.stop_server(node)
                await self.stop_server(other)
