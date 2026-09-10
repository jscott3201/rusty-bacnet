"""Installed native hub rejects nil requests after authenticated TLS/WS, not at dial."""
import asyncio
import ssl

from rusty_bacnet import BACnetServer
from test_sc_hub_mtls import MtlsFixture, SERVER_UUID


class PeerUuidTests(MtlsFixture):
    async def open_peer(self, address):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_verify_locations(self.path("site.pem"))
        context.load_cert_chain(self.path("client.pem"), self.path("client.key"))
        reader, writer = await asyncio.wait_for(asyncio.open_connection(
            "127.0.0.1", int(address.rsplit(":", 1)[1]), ssl=context,
            server_hostname="localhost"), 3)
        try:
            writer.write(b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
                         b"Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n"
                         b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                         b"Sec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
            await asyncio.wait_for(writer.drain(), 3)
            response = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 3)
            self.assertTrue(response.startswith(b"HTTP/1.1 101"), response)
            return reader, writer
        except BaseException:
            writer.close()
            await asyncio.wait_for(writer.wait_closed(), 3)
            raise

    async def send(self, writer, payload):
        self.assertLess(len(payload), 126)
        mask = b"\x12\x34\x56\x78"
        writer.write(bytes([0x82, 0x80 | len(payload)]) + mask +
                     bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))
        await asyncio.wait_for(writer.drain(), 3)

    async def binary(self, reader):
        header = await asyncio.wait_for(reader.readexactly(2), 3)
        self.assertEqual(header[0], 0x82)
        self.assertLess(header[1], 126)  # short, unmasked server frame
        return await asyncio.wait_for(reader.readexactly(header[1]), 3)

    async def test_nil_request_nak_close_repeat_and_surviving_native_read(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        address = await hub.address()
        server = BACnetServer(3000, "Nil peer survivor", transport="sc", sc_hub=await hub.url(),
                              sc_device_uuid=SERVER_UUID, sc_vmac=b"\x02\0\0\0\0\2",
                              sc_ca_cert=self.path("site.pem"),
                              sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        nil_payload = b"\x02\0\0\0\0\2" + bytes(16) + b"\x05\xc4\x05\xc4"
        nak = b"\0\0\x22\x33\x06\x01\0\0\x07\0\x50"
        # The proposed VMAC collides with the live server: range, not duplicate-VMAC.
        for fields, reply in [(b"\0", nak),
                              (b"\x08" + b"\x44" * 6,
                               b"\0\x04\x22\x33" + b"\x44" * 6 + nak[4:]),
                              (b"\x04" + b"\xff" * 6, None),
                              (b"\x08" + bytes(6), None)]:
            reader, writer = await self.open_peer(address)
            try:
                await self.send(writer, b"\x06" + fields[:1] + b"\x22\x33" + fields[1:] + nil_payload)
                if reply is not None:
                    self.assertEqual(await self.binary(reader), reply)
                self.assertEqual(await asyncio.wait_for(reader.read(1), 3), b"")
            finally:
                writer.close()
                await asyncio.wait_for(writer.wait_closed(), 3)
            self.assertEqual(await asyncio.to_thread(
                self.websocket, address, "client", read_property=True), "TLSv1.3")

        reader, writer = await self.open_peer(address)
        try:
            # Sparse, non-RFC-shaped UUID is accepted, with its own distinct VMAC.
            await self.send(writer, b"\x06\0\x22\x33" + b"\x02\0\0\0\0\5" +
                            bytes(15) + b"\1\x05\xc4\x05\xc4")
            self.assertEqual((await self.binary(reader))[:4], b"\x07\0\x22\x33")
            for _ in range(10):
                await self.send(writer, b"\x06\0\x22\x33" + nil_payload)
                self.assertEqual(await self.binary(reader), nak)
            await self.send(writer, b"\x0a\0\x66\x77")
            self.assertEqual(await self.binary(reader), b"\x0b\0\x66\x77")
            self.assertEqual(await asyncio.to_thread(
                self.websocket, address, "client", read_property=True), "TLSv1.3")
        finally:
            writer.close()
            await asyncio.wait_for(writer.wait_closed(), 3)
