"""Installed-native three-path NAK wire/healthy-operation and fresh-start smoke.

Rust gates prove held-write expiry and automatic fresh-only recovery. Python has
no public short heartbeat/reconnect knob: this test does not force its 60s budget
or claim native OS backpressure. Each lifecycle explicitly starts a fresh node.
"""
import asyncio
import base64
import hashlib
import ssl

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls


class RejectionDeadlineTests(mtls.MtlsFixture):
    frame = mtls.NodeIdentityMtlsTests.frame
    send_frame = mtls.NodeIdentityMtlsTests.send_frame
    node = mtls.NodeIdentityMtlsTests.node

    async def binary(self, reader):
        # A native server may announce I-Am after start. Do not discard arbitrary
        # NPDUs here: an erroneous response to the rejected request must fail.
        for _ in range(8):
            opcode, wire = await self.frame(reader, True)
            self.assertEqual(opcode, 2)
            if wire[:2] == b"\x01\x04" and wire[10:14] == b"\x01\0\x10\0":
                continue
            return wire
        self.fail("no expected frame within the bounded I-Am drain")

    async def test_three_rejection_naks_then_read_property_on_each_fresh_native_socket(self):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(self.path("hub.pem"), self.path("hub.key"))
        context.load_verify_locations(self.path("site.pem"))
        context.verify_mode = ssl.CERT_REQUIRED
        tasks, sessions = [], []
        completed = asyncio.Queue()
        release = asyncio.Event()
        source = b"\x22" * 6
        vmac = b"\x02\0\0\0\0\x04"

        async def peer(reader, writer):
            tasks.append(asyncio.current_task())
            try:
                tls = writer.get_extra_info("ssl_object")
                self.assertEqual(tls.version(), "TLSv1.3")
                self.assertTrue(tls.getpeercert(binary_form=True))
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
                connect = await self.binary(reader)
                self.assertEqual(connect, b"\x06\0\0\1" + vmac + mtls.SERVER_UUID + b"\x16\x49\x05\xc4")
                sessions.append(writer.get_extra_info("peername"))
                await self.send_frame(writer, b"\x07\0\0\1" + mtls.HUB_VMAC + mtls.HUB_UUID + b"\x05\xc4\x05\xc4")
                # Independent RP AI0/PV bytes. Neither rejected NPDU may dispatch.
                rp = b"\x01\x04\x00\x03\x11\x0c\x0c\0\0\0\0\x19\x55"
                cases = (
                    (b"\x0a\0\x22\x33\x42", b"\0\0\x22\x33\x0a\1\0\0\7\0\7"),
                    (b"\x01\0\x22\x33" + rp, b"\0\0\x22\x33\1\1\0\0\7\0\x50"),
                    (b"\x01\x0a\x22\x33" + source + b"\xe2\0\0\x1f" + rp,
                     b"\0\4\x22\x33" + source + b"\1\1\xe2\0\7\0\x92"),
                )
                for rejected, nak in cases:
                    with self.subTest(function=rejected[0], flags=rejected[1]):
                        await self.send_frame(writer, rejected)
                        self.assertEqual(await self.binary(reader), nak)
                await self.send_frame(writer, b"\x01\x08\x44\x55" + source + rp)
                response = await self.binary(reader)
                self.assertEqual(response[:2], b"\x01\x04")
                self.assertEqual(response[4:10], source)
                self.assertTrue(response.endswith(
                    b"\x30\x11\x0c\x0c\0\0\0\0\x19\x55\x3e\x44\x42\x91\0\0\x3f"), response)
                await self.send_frame(writer, b"\x0a\0\x66\x77")
                self.assertEqual(await self.binary(reader), b"\x0b\0\x66\x77")
                completed.put_nowait(None)
                await asyncio.wait_for(release.wait(), 3)
                async def drain():
                    while await reader.read(4096):
                        pass
                await asyncio.wait_for(drain(), 3)
            except Exception as error:
                completed.put_nowait(error)
                raise
            finally:
                writer.close()
                await asyncio.wait_for(writer.wait_closed(), 3)

        listener = await asyncio.start_server(peer, "127.0.0.1", 0, ssl=context)
        url = f"wss://localhost:{listener.sockets[0].getsockname()[1]}"
        node = None
        try:
            for lifecycle in range(2):
                with self.subTest(lifecycle=lifecycle):
                    release.clear()
                    node = self.node(BACnetServer, url, mtls.SERVER_UUID, vmac)
                    await asyncio.wait_for(node.start(), 5)
                    result = await asyncio.wait_for(completed.get(), 3)
                    if result is not None:
                        raise result
                    release.set()
                    await self.stop_server(node)
                    await asyncio.wait_for(asyncio.gather(*tasks), 3)
            self.assertEqual(len(sessions), 2)
            self.assertNotEqual(sessions[0], sessions[1], "fresh native start must open a new TCP socket")
        finally:
            release.set()
            if node is not None:
                await self.stop_server(node)
            listener.close()
            await listener.wait_closed()
            for task in tasks:
                if not task.done():
                    task.cancel()
            results = await asyncio.wait_for(asyncio.gather(*tasks, return_exceptions=True), 3)
            for result in results:
                if isinstance(result, Exception):
                    raise result
