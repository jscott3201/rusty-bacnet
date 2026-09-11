"""Public installed ScHub: unsolicited peer responses are silent, not NAKs."""
import asyncio

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer


class HubResponseSilenceTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer
    send = peer.PeerUuidTests.send
    binary = peer.PeerUuidTests.binary

    async def test_unsolicited_responses_preserve_native_hub_and_registration(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        address = await hub.address()
        server = BACnetServer(
            3000, "Response survivor", transport="sc", sc_hub=await hub.url(),
            sc_device_uuid=mtls.SERVER_UUID, sc_vmac=b"\x02\0\0\0\0\2",
            sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        reader, writer = await self.open_peer(address)
        try:
            for registered in (False, True):
                for function in (7, 9):
                    # Accept spoofs a live UUID with different VMAC/limits.
                    payload = (b"\x02\0\0\0\0\5" + mtls.SERVER_UUID + b"\x02\0\0\x80"
                               if function == 7 else b"")
                    for flags, fields in ((0, b""), (2, b"\x1e"), (2, b"\x5e"),
                                          (8, b"\x44" * 6), (1, b"\x1e"), (0, b"\x42")):
                        for message_id in (0, 0x2233, 0xffff):
                            with self.subTest(registered=registered, function=function,
                                              flags=flags, fields=fields, message_id=message_id):
                                await self.send(writer, bytes([function, flags]) +
                                                message_id.to_bytes(2, "big") + fields + payload)
                                # Ordered invalid-Request barrier, not a silence timeout.
                                await self.send(writer, b"\x08\0\x33\x44\x42")
                                self.assertEqual(await self.binary(reader),
                                                 b"\0\0\x33\x44\x08\x01\0\0\x07\0\x07")
                if not registered:
                    # Distinct from the ReadProperty fixture's CLIENT_UUID;
                    # a genuine duplicate UUID would intentionally replace us.
                    await self.send(writer, b"\x06\0\x22\x33" + b"\x02\0\0\0\0\5" +
                                    bytes(15) + b"\x01\x05\xc4\x05\xc4")
                    self.assertEqual((await self.binary(reader))[:4], b"\x07\0\x22\x33")
                await self.send(writer, b"\x0a\0\x66\x77")
                self.assertEqual(await self.binary(reader), b"\x0b\0\x66\x77")
                self.assertEqual(await asyncio.to_thread(
                    self.websocket, address, "client", read_property=True), "TLSv1.3")
            await self.send(writer, b"\x08\0\x77\x88")
            self.assertEqual(await self.binary(reader), b"\x09\0\x77\x88")
            self.assertEqual(await asyncio.wait_for(reader.readexactly(2), 3), b"\x88\0")
        finally:
            writer.close()
            try:
                await asyncio.wait_for(writer.wait_closed(), 3)
            except ConnectionResetError:
                # The hub may drop TCP after its asserted ACK + WebSocket Close.
                # This only tolerates terminal teardown, not a protocol read failure.
                pass
