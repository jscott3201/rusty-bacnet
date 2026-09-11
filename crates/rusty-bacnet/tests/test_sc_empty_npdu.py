"""Installed DEV mTLS evidence at two independent seams; no 60s timing claim.

Raw clients attack the public hub before forwarding, while a raw fake hub sends
directly toward a native server's NODE. Rust proves non-delivery and liveness;
native wire checks and healthy ReadProperty prove the installed path is current.
"""
import asyncio

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer
import test_sc_rejection_deadline as deadline


class EmptyNodeTests(mtls.MtlsFixture):
    frame = deadline.RejectionDeadlineTests.frame
    send_frame = deadline.RejectionDeadlineTests.send_frame
    node = deadline.RejectionDeadlineTests.node
    binary = deadline.RejectionDeadlineTests.binary
    exercise_rejections = deadline.RejectionDeadlineTests.exercise_rejections

    async def test_empty_npdu_fake_hub_to_native_node_and_fresh_read_property(self):
        source = b"\x22" * 6
        nak = b"\0\4\x22\x33" + source + b"\1\1\0\0\7\0\x95"
        await self.exercise_rejections((
            (b"\1\x08\x22\x33" + source, nak),
            (b"\1\x0b\x22\x33" + source + b"\xa2\0\0\x1f\x7e\0\1\xbb", nak),
            (b"\1\x0c\x22\x33" + source + b"\xff" * 6, None),
            (b"\1\x0c\x22\x33" + source + b"\x02\0\0\0\0\x04", None),
            (b"\1\0\x22\x33", b"\0\0\x22\x33\1\1\0\0\7\0\x50"),
            (b"\1\x0a\x22\x33" + source + b"\xe2\0\0\x1f",
             b"\0\4\x22\x33" + source + b"\1\1\xe2\0\7\0\x92"),
            # Single-byte SC admission remains compatible, though this is not a
            # valid network PDU. The Rust receiver test observes actual delivery.
            (b"\1\x08\x22\x33" + source + b"\x42", None),
        ))


class EmptyHubTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer
    send = peer.PeerUuidTests.send

    async def binary(self, reader):
        for _ in range(8):
            wire = await peer.PeerUuidTests.binary(self, reader)
            if wire[:2] == b"\1\x0c" and wire[16:20] == b"\1\0\x10\0":
                continue  # only an independently recognized server I-Am
            return wire
        self.fail("expected response missing after bounded I-Am drain")

    async def barrier(self, reader, writer):
        await self.send(writer, b"\x08\0\x44\x55\x42")
        self.assertEqual(await self.binary(reader), b"\0\0\x44\x55\x08\1\0\0\7\0\7")

    async def test_empty_npdu_public_hub_no_fanout_then_native_read_property(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        address = await hub.address()
        server_vmac = b"\x02\0\0\0\0\2"
        server = BACnetServer(
            3000, "Empty NPDU survivor", transport="sc", sc_hub=await hub.url(),
            sc_device_uuid=mtls.SERVER_UUID, sc_vmac=server_vmac,
            sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        connections = []
        try:
            for identity in (0x22, 0x43, 0x44):
                reader, writer = await self.open_peer(address)
                connections.append((reader, writer))
                await self.send(writer, b"\x06\0\x22\x33" + bytes([identity]) * 22 + b"\x05\xc4\x05\xc4")
                self.assertEqual((await self.binary(reader))[:4], b"\x07\0\x22\x33")
            reader, writer = connections[0]
            for destination in (b"\x43" * 6, b"\x44" * 6, server_vmac, b"\xff" * 6):
                for flags, options in ((4, b""), (7, b"\xe2\0\0\x1f\x7e\0\1\xbb")):
                    with self.subTest(destination=destination, flags=flags):
                        await self.send(writer, bytes([1, flags, 0x22, 0x33]) + destination + options)
                        if destination != b"\xff" * 6:
                            # Must come from the hub itself, not a destination
                            # node NAK relayed back with an Originating VMAC.
                            self.assertEqual(await self.binary(reader), b"\0\0\x22\x33\1\1\0\0\7\0\x95")
                        await self.barrier(reader, writer)
                        for target_reader, target_writer in connections[1:]:
                            await self.barrier(target_reader, target_writer)
            # Both raw recipients observe exact positive payload/option relay.
            for payload in (b"\x42", b"\1\0\x10\x08"):
                body = b"\xe2\0\0\x1f\x7e\0\1\xbb" + payload
                await self.send(writer, b"\1\7\x22\x33" + b"\xff" * 6 + body)
                expected = b"\1\x0f\x22\x33" + b"\x22" * 6 + b"\xff" * 6 + body
                for target_reader, _ in connections[1:]:
                    self.assertEqual(await self.binary(target_reader), expected)
                await self.barrier(reader, writer)
            self.assertEqual(await asyncio.to_thread(
                self.websocket, address, "client", read_property=True), "TLSv1.3")
            for reader, writer in connections:
                await self.send(writer, b"\x0a\0\x66\x77")
                self.assertEqual(await self.binary(reader), b"\x0b\0\x66\x77")
                await self.send(writer, b"\x08\0\x77\x88")
                self.assertEqual(await self.binary(reader), b"\x09\0\x77\x88")
                self.assertEqual(await asyncio.wait_for(reader.readexactly(2), 3), b"\x88\0")
        finally:
            for _, writer in connections:
                writer.close()
            for _, writer in connections:
                try:
                    await asyncio.wait_for(writer.wait_closed(), 3)
                except ConnectionResetError:
                    pass  # terminal teardown only, never a protocol-read failure
