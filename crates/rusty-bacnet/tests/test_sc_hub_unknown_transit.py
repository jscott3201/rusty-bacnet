"""Raw mTLS peers -> public installed native hub -> real native node -> raw peer.

This is not a fake hub or a replacement node codec. Rust owns held-lock, clock,
retirement and deadline evidence; no native 60s/OS-backpressure claim.
"""
import asyncio

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer


class HubUnknownTransitTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer

    async def send(self, writer, payload):
        self.assertLessEqual(len(payload), 8192)
        mask = b"\x12\x34\x56\x78"
        length = (bytes([0x80 | len(payload)]) if len(payload) < 126
                  else b"\xfe" + len(payload).to_bytes(2, "big"))
        writer.write(b"\x82" + length + mask +
                     bytes(value ^ mask[i % 4] for i, value in enumerate(payload)))
        await asyncio.wait_for(writer.drain(), 3)

    async def binary(self, reader):
        for _ in range(8):
            header = await asyncio.wait_for(reader.readexactly(2), 3)
            self.assertEqual(header[0], 0x82)
            self.assertFalse(header[1] & 0x80)
            size = header[1]
            if size == 126:
                size = int.from_bytes(await asyncio.wait_for(reader.readexactly(2), 3), "big")
            else:
                self.assertLess(size, 126)
            self.assertLessEqual(size, 8192)
            wire = await asyncio.wait_for(reader.readexactly(size), 3)
            # Only the native server's unsolicited I-Am may be drained.
            if wire[:2] == b"\x01\x0c" and wire[10:16] == b"\xff" * 6 and wire[16:20] == b"\x01\0\x10\0":
                continue
            return wire
        self.fail("expected response missing after bounded I-Am drain")

    async def barrier(self, endpoint):
        reader, writer = endpoint
        await self.send(writer, b"\x08\0\x55\x66\x42")
        self.assertEqual(await self.binary(reader), b"\0\0\x55\x66\x08\1\0\0\7\0\7")

    async def register(self, endpoint, ident, max_bvlc=6000, max_npdu=1497):
        reader, writer = endpoint
        await self.send(writer, b"\x06\0\x22\x33" + bytes([ident]) * 6 +
                        bytes(15) + bytes([ident]) + max_bvlc.to_bytes(2, "big") + max_npdu.to_bytes(2, "big"))
        self.assertEqual((await self.binary(reader))[:4], b"\x07\0\x22\x33")

    async def read_property(self, endpoint, invoke):
        reader, writer = endpoint
        node = b"\x02\0\0\0\0\2"
        await self.send(writer, b"\x01\4\x44\x55" + node +
                        b"\x01\x04\x00\x03" + bytes([invoke]) + b"\x0c\x0c\0\0\0\0\x19\x55")
        response = await self.binary(reader)
        self.assertEqual(response[:2], b"\x01\x08")
        self.assertEqual(response[4:10], node)
        self.assertEqual(response[10:], b"\x01\0\x30" + bytes([invoke]) +
                         b"\x0c\x0c\0\0\0\0\x19\x55\x3e\x44\x42\x91\0\0\x3f")

    async def close_peer(self, endpoint):
        _, writer = endpoint
        writer.close()
        try:
            await asyncio.wait_for(writer.wait_closed(), 3)
        except ConnectionResetError:
            pass  # terminal cleanup only, never a response oracle

    async def test_native_hub_unknown_roundtrip_raw_fanout_caps_and_healthy_node(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        server = BACnetServer(
            3000, "Unknown transit survivor", transport="sc", sc_hub=await hub.url(),
            sc_device_uuid=mtls.SERVER_UUID, sc_vmac=b"\x02\0\0\0\0\2",
            sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        endpoints = []
        for _ in range(3):
            endpoint = await self.open_peer(await hub.address())
            self.addAsyncCleanup(self.close_peer, endpoint)
            endpoints.append(endpoint)
        a, b, c = endpoints
        av, bv, cv = (bytes([ident]) * 6 for ident in (0x42, 0x43, 0x44))
        node, broadcast = b"\x02\0\0\0\0\2", b"\xff" * 6
        await self.register(b, 0x43, 1600, 1)
        await self.register(c, 0x44, max_npdu=1)
        # Pre-registration addressed Unknown is a same-socket local diagnostic,
        # including origin metadata, never a route to the already registered B.
        await self.send(a[1], b"\x42\x0c\0\0" + bv + bv)
        self.assertEqual(await self.binary(a[0]), b"\0\4\0\0" + bv + b"\x42\1\0\0\7\0\x8f")
        await self.send(a[1], b"\x42\4\0\0" + broadcast)
        for endpoint in endpoints:
            await self.barrier(endpoint)
        await self.register(a, 0x42)
        await self.read_property(a, 0x11)
        for function in (0x0D, 0x42, 0xFF):
            for message_id in (b"\0\0", b"\xff\xff"):
                for flags, body in ((4, b""), (7, b"\xe2\0\0\x1f\xfe\0\1\xbb\x1f\xff")):
                    with self.subTest(function=function, id=message_id, flags=flags):
                        await self.send(a[1], bytes([function, flags]) + message_id + node + body)
                        # Origin B (native node), not absent (hub-local fallback).
                        self.assertEqual(await self.binary(a[0]), b"\0\x08" + message_id + node +
                                         bytes([function, 1, 0, 0, 7, 0, 143]))
                        for endpoint in endpoints:
                            await self.barrier(endpoint)
        # Exact opaque unicast and broadcast to two independent raw recipients.
        for dest in (bv, broadcast):
            body = b"\xe2\0\0\x1f\xfe\0\1\xbb\x1f\xff"
            await self.send(a[1], b"\xff\7\x22\x33" + dest + body)
            expected = b"\xff" + (b"\x0f" if dest == broadcast else b"\x0b") + b"\x22\x33" + av
            expected += (broadcast if dest == broadcast else b"") + body
            self.assertEqual(await self.binary(b[0]), expected)
            if dest == broadcast:
                self.assertEqual(await self.binary(c[0]), expected)
                # Ordered node-side sentinel proves native broadcast silence. A
                # broadcast NAK would arrive before this unicast NAK on A.
                await self.send(a[1], b"\x0d\4\x33\x44" + node)
                self.assertEqual(await self.binary(a[0]), b"\0\x08\x33\x44" + node + b"\x0d\1\0\0\7\0\x8f")
            for endpoint in endpoints:
                await self.barrier(endpoint)
        # Explicit self/missing/zero targets and forged transit origins are silent.
        for fields in (av, bytes(6), b"\x77" * 6):
            await self.send(a[1], b"\x42\4\0\0" + fields)
        await self.send(a[1], b"\x42\x0c\0\0" + bv + cv)
        for endpoint in endpoints:
            await self.barrier(endpoint)
        # A peer-local NAK is observably different from the node-returned NAK.
        await self.send(a[1], b"\x42\0\x22\x33")
        self.assertEqual(await self.binary(a[0]), b"\0\0\x22\x33\x42\1\0\0\7\0\x8f")
        # Raw Result ACK and detailed NAK preserve markers, IDs and body bytes.
        for result in (b"\x42\0", b"\xff\1\xfe\0\7\0\x8fdetail-\xc3\xa9"):
            body = b"\xfe\0\0\x1f" + result
            await self.send(b[1], b"\0\6\xff\xff" + av + body)
            self.assertEqual(await self.binary(a[0]), b"\0\x0a\xff\xff" + bv + body)
        # Public peer handshake limits only: Max-NPDU=1 does not cap opaque data.
        for dest in (bv, broadcast):
            for extra in (0, 1):
                with self.subTest(cap_dest=dest, extra=extra):
                    body = b"x" * (1600 - (16 if dest == broadcast else 10) + extra)
                    await self.send(a[1], b"\x42\4\x22\x33" + dest + body)
                    expected = b"\x42" + (b"\x0c" if dest == broadcast else b"\x08") + b"\x22\x33" + av
                    expected += (broadcast if dest == broadcast else b"") + body
                    if not extra:
                        self.assertEqual(await self.binary(b[0]), expected)
                    if dest == broadcast:
                        self.assertEqual(await self.binary(c[0]), expected)
                        await self.send(a[1], b"\x0d\4\x33\x44" + node)
                        self.assertEqual(await self.binary(a[0]), b"\0\x08\x33\x44" + node + b"\x0d\1\0\0\7\0\x8f")
                    for endpoint in endpoints:
                        await self.barrier(endpoint)
        await self.read_property(a, 0x12)
        for endpoint in endpoints:
            await self.barrier(endpoint)
