"""Public native HUB transit, with raw mTLS peers owning AR endpoint semantics.

The native NODE does not implement Address-Resolution responses. It is a third
peer serving real ReadProperty before/after, never an invented URI responder.
No URI parsing/dialing, fake hub, native expiry or OS-backpressure claim.
"""
import asyncio

from rusty_bacnet import BACnetServer
import test_sc_hub_mtls as mtls
import test_sc_hub_unknown_transit as wire
import test_sc_peer_uuid as peer


class HubResolutionTransitTests(mtls.MtlsFixture):
    # Reuse existing bounded wire/TLS fixtures without inheriting their tests.
    open_peer = peer.PeerUuidTests.open_peer
    send = wire.HubUnknownTransitTests.send
    binary = wire.HubUnknownTransitTests.binary
    barrier = wire.HubUnknownTransitTests.barrier
    register = wire.HubUnknownTransitTests.register
    read_property = wire.HubUnknownTransitTests.read_property
    close_peer = wire.HubUnknownTransitTests.close_peer

    async def test_native_hub_resolution_request_ack_result_and_healthy_node(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        server = BACnetServer(
            3000, "Resolution transit survivor", transport="sc", sc_hub=await hub.url(),
            sc_device_uuid=mtls.SERVER_UUID, sc_vmac=b"\x02\0\0\0\0\2",
            sc_ca_cert=self.path("site.pem"), sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "AI-0", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        endpoints = []
        for ident in (0x42, 0x43):
            endpoint = await self.open_peer(await hub.address())
            self.addAsyncCleanup(self.close_peer, endpoint)
            # Only B advertises Max-NPDU=1: A must receive the real RP NPDU.
            await self.register(endpoint, ident, 1600 if ident == 0x43 else 6000,
                                1 if ident == 0x43 else 1497)
            endpoints.append(endpoint)
        a, b = endpoints
        av, bv, broadcast = b"\x42" * 6, b"\x43" * 6, b"\xff" * 6
        await self.read_property(a, 0x11)
        for message_id in (b"\0\0", b"\xff\xff"):
            for uri_list in (b"", b"wss://one.example/sc", b"wss://one.example/sc wss://two.example:8443/sc"):
                with self.subTest(id=message_id, uri_list=uri_list):
                    await self.send(a[1], b"\2\4" + message_id + bv)
                    self.assertEqual(await self.binary(b[0]), b"\2\x08" + message_id + av)
                    # Independent raw responder B, not native NODE AR behavior.
                    await self.send(b[1], b"\3\4" + message_id + av + uri_list)
                    self.assertEqual(await self.binary(a[0]), b"\3\x08" + message_id + bv + uri_list)
                    for endpoint in endpoints:
                        await self.barrier(endpoint)
            for result in (b"\2\0", b"\2\1\xfe\0\7\0\x96detail-\xc3\xa9"):
                body = b"\xfe\0\0\x1f" + result
                await self.send(b[1], b"\0\6" + message_id + av + body)
                self.assertEqual(await self.binary(a[0]), b"\0\x0a" + message_id + bv + body)
        for function in (2, 3):
            # Raw MU/MoreOptions/empty HeaderData/DataOptions/non-UTF8 preserved.
            # These are opaque transit vectors, not valid endpoint AR formats.
            body = b"\xe2\0\0\x1f\xfe\0\1\xbb\x1f\xff"
            await self.send(a[1], bytes([function, 7, 0, 0]) + bv + body)
            self.assertEqual(await self.binary(b[0]), bytes([function, 11, 0, 0]) + av + body)
            for dest in (av, bytes(6), b"\x77" * 6, broadcast):
                await self.send(a[1], bytes([function, 4, 0, 0]) + dest)
            await self.send(a[1], bytes([function, 12, 0, 0]) + bv + bv)
            await self.send(a[1], bytes([function, 0, 0, 0]))
            if function == 2:
                self.assertEqual(await self.binary(a[0]), b"\0\0\0\0\2\1\0\0\7\0\x96")
            for endpoint in endpoints:
                await self.barrier(endpoint)
            for extra in (0, 1):
                with self.subTest(function=function, recipient_cap_extra=extra):
                    body = b"wss://peer.example/" + b"x" * (1590 + extra - 19)
                    self.assertEqual(len(body), 1590 + extra)
                    await self.send(a[1], bytes([function, 4, 0, 0]) + bv + body)
                    if not extra:
                        self.assertEqual(await self.binary(b[0]), bytes([function, 8, 0, 0]) + av + body)
                    for endpoint in endpoints:
                        await self.barrier(endpoint)
            body = b"x" * (5705 - 10)
            await self.send(b[1], bytes([function, 4, 0, 0]) + av + body)
            self.assertEqual(await self.binary(a[0]), bytes([function, 8, 0, 0]) + bv + body)
            for endpoint in reversed(endpoints):
                await self.barrier(endpoint)
        for extra in (0, 1):
            body = b"\2\1\0\0\7\0\x96" + b"x" * (1590 + extra - 7)
            await self.send(a[1], b"\0\4\xff\xff" + bv + body)
            if not extra:
                self.assertEqual(await self.binary(b[0]), b"\0\x08\xff\xff" + av + body)
            for endpoint in endpoints:
                await self.barrier(endpoint)
        # ResultForACK remains silent, as do malformed and DataOptions Results.
        for result in (b"\3\0", b"\3\1\0\0\7\0\x96", b"\2\0\0", b"\2\1\0\0\7\0\x96\xff"):
            await self.send(b[1], b"\0\4\0\0" + av + result)
        await self.send(b[1], b"\0\5\0\0" + av + b"\x1e\2\0")
        for endpoint in reversed(endpoints):
            await self.barrier(endpoint)
        # The public WebSocket adapter closes at ingress cap+1 before handler
        # dispatch. Use a fresh sacrificial registered peer for each function,
        # not a "silent but healthy" assertion against this existing close path.
        for function in (2, 3):
            oversized = await self.open_peer(await hub.address())
            try:
                await self.register(oversized, 0x44)
                await self.send(oversized[1], bytes([function, 4, 0, 0]) + av + b"x" * (5706 - 10))
                header = await asyncio.wait_for(oversized[0].readexactly(2), 3)
                self.assertEqual(header[0], 0x88)  # Close, never a BVLC Result
                self.assertLessEqual(header[1], 125)
                await asyncio.wait_for(oversized[0].readexactly(header[1]), 3)
            finally:
                await self.close_peer(oversized)
            for endpoint in endpoints:
                await self.barrier(endpoint)
        await self.read_property(a, 0x12)

    async def test_native_hub_resolution_prereg_local_request_vs_silent_ack(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        observer = await self.open_peer(await hub.address())
        self.addAsyncCleanup(self.close_peer, observer)
        await self.register(observer, 0x43)
        bv, broadcast = b"\x43" * 6, b"\xff" * 6
        cases = 0
        for origin in (b"", bv, bytes(6), broadcast):
            for dest in (b"", bv, bytes(6), broadcast):
                # Observer-first, fresh preregistration peer per TWO functions.
                source = await self.open_peer(await hub.address())
                try:
                    for function in (2, 3):
                        with self.subTest(function=function, origin=origin, dest=dest):
                            flags = (8 if origin else 0) | (4 if dest else 0)
                            await self.send(source[1], bytes([function, flags, 0xff, 0xff]) + origin + dest)
                            if function == 2 and dest != broadcast and origin not in (bytes(6), broadcast):
                                self.assertEqual(await self.binary(source[0]),
                                                 bytes([0, 4 if origin else 0, 0xff, 0xff]) + origin + b"\2\1\0\0\7\0\x96")
                            await self.barrier(source)
                            await self.barrier(observer)
                            cases += 1
                    await self.register(source, 0x42)
                    await self.barrier(source)
                finally:
                    await self.close_peer(source)
        self.assertEqual(cases, 32)
