"""Installed native node: MU wire rejection and healthy ReadProperty recovery.

The short Rust transport tests prove liveness accounting. This public-API smoke
does not wait for default native heartbeat timers or claim blocked-write safety.
"""
import asyncio

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer


class MuLivenessTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer
    send = peer.PeerUuidTests.send
    binary = peer.PeerUuidTests.binary

    async def test_mu_rejection_then_healthy_native_read_property(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        server_vmac = b"\x02\0\0\0\0\2"
        server = BACnetServer(
            3000, "MU survivor", transport="sc", sc_hub=await hub.url(),
            sc_device_uuid=mtls.SERVER_UUID, sc_vmac=server_vmac,
            sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        reader, writer = await self.open_peer(await hub.address())
        try:
            await self.send(writer, b"\x06\0\x22\x33" + b"\x02\0\0\0\0\5" +
                            bytes(15) + b"\x01\x05\xc4\x05\xc4")
            self.assertEqual((await self.binary(reader))[:4], b"\x07\0\x22\x33")
            # ReadProperty AI0/PV is independently encoded, not native-generated.
            request = b"\x01\x04\x00\x03\x11\x0c\x0c\0\0\0\0\x19\x55"
            options = ((b"\x42", 0x42), (b"\xc2\x1f", 0xc2),
                       (b"\x62\0\0", 0x62), (b"\xe2\0\0\x5f", 0xe2),
                       (b"\xe2\0\2\xaa\xbb\x1f", 0xe2),
                       (b"\x9e\x7f\0\1\xcc", 0x7f))
            for option, marker in options:
                for broadcast in (False, True):
                    with self.subTest(option=option, broadcast=broadcast):
                        destination = b"\xff" * 6 if broadcast else server_vmac
                        await self.send(writer, b"\x01\x06\x22\x44" + destination + option + request)
                        expected = (b"\0\x08\x22\x44" + server_vmac +
                                    bytes([1, 1, marker, 0, 7, 0, 0x92]))
                        if not broadcast:
                            self.assertEqual(await self.binary(reader), expected)
                        # An ordered unicast MU rejection from the SAME node
                        # follows the silent broadcast. No valid clock sentinel.
                        await self.send(writer, b"\x01\x06\x33\x55" + server_vmac + b"\x42" + request)
                        self.assertEqual(await self.binary(reader),
                                         b"\0\x08\x33\x55" + server_vmac + b"\x01\x01\x42\0\x07\0\x92")
            # MU-clear Destination Options and MU Data Options still allow a
            # real native ReadProperty response after the rejected burst.
            for flags, option in ((6, b"\x22\0\0"), (7, b"\x22\0\0\x62\0\1\xaa")):
                with self.subTest(accepted_flags=flags):
                    # Distinct invoke IDs avoid the server's intentional exact-
                    # request duplicate suppression between these controls.
                    invoke = bytes([flags])
                    accepted_request = request[:4] + invoke + request[5:]
                    await self.send(writer, bytes([1, flags, 0x44, 0x66]) + server_vmac + option + accepted_request)
                    response = await self.binary(reader)
                    self.assertEqual(response[0], 1)
                    self.assertEqual(response[4:10], server_vmac)
                    self.assertTrue(response.endswith(
                        b"\x30" + invoke + b"\x0c\x0c\0\0\0\0\x19\x55\x3e\x44\x42\x91\0\0\x3f"), response)
            await self.send(writer, b"\x0a\0\x66\x77")
            self.assertEqual(await self.binary(reader), b"\x0b\0\x66\x77")
        finally:
            writer.close()
            await asyncio.wait_for(writer.wait_closed(), 3)
