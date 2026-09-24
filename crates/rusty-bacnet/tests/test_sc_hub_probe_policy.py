"""Installed Hub operator settings drive real mutual-TLS traffic and counters."""
import asyncio

from rusty_bacnet import ScHub
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer


class HubProbePolicyTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer
    send = peer.PeerUuidTests.send
    binary = peer.PeerUuidTests.binary

    async def close_peer(self, endpoint):
        endpoint[1].close()
        try:
            await asyncio.wait_for(endpoint[1].wait_closed(), 3)
        except ConnectionResetError:
            pass

    async def start(self, **settings):
        hub = ScHub("127.0.0.1:0", self.path("hub.pem"), self.path("hub.key"),
                    mtls.HUB_VMAC, ca_cert=self.path("site.pem"),
                    device_uuid=mtls.HUB_UUID, **settings)
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        return hub

    async def register(self, hub, ident):
        endpoint = await self.open_peer(await hub.address())
        self.addAsyncCleanup(self.close_peer, endpoint)
        await self.send(endpoint[1], b"\x06\0\x22\x33" + bytes([ident]) * 22 + b"\x05\xc4\x05\xc4")
        self.assertEqual((await self.binary(endpoint[0]))[:4], b"\x07\0\x22\x33")
        return endpoint

    async def test_configured_probe_emits_ack_refreshes_then_wrong_ack_expires(self):
        hub = await self.start(probe_scan_interval_ms=20, probe_idle_age_ms=50,
                               probe_ack_age_ms=80, probe_send_budget_ms=40,
                               relay_send_budget_ms=30)
        endpoint = await self.register(hub, 0x42)
        # These observed probes arrive within one second; the default first idle
        # probe would require over 60s. Exact ms boundaries are paused Rust tests.
        first = await asyncio.wait_for(self.binary(endpoint[0]), 1)
        self.assertEqual(first[:2], b"\x0a\0")
        await self.send(endpoint[1], b"\x0b\0" + first[2:4])
        second = await asyncio.wait_for(self.binary(endpoint[0]), 1)
        self.assertEqual(second[:2], b"\x0a\0")
        self.assertNotEqual(second[2:4], first[2:4])
        wrong = ((int.from_bytes(second[2:4], "big") + 1) % 65536).to_bytes(2, "big")
        await self.send(endpoint[1], b"\x0b\0" + wrong)
        header = await asyncio.wait_for(endpoint[0].readexactly(2), 1)
        self.assertEqual(header[0], 0x88)
        self.assertEqual((await hub.status())["client_count"], 0)
        await self.stop_hub(hub)

    async def test_configured_sender_and_global_buckets_drop_and_count_wire_requests(self):
        for limited in ("sender", "global"):
            with self.subTest(limited=limited):
                settings = dict(broadcast_sender_burst=1000, broadcast_sender_per_second=1000,
                                broadcast_global_burst=1000, broadcast_global_per_second=1000)
                settings[f"broadcast_{limited}_burst"] = 1
                settings[f"broadcast_{limited}_per_second"] = 1
                hub = await self.start(**settings)
                sender = await self.register(hub, 0x42)
                receiver = await self.register(hub, 0x43)
                async def burst():
                    for ident in range(10):
                        await self.send(sender[1], bytes([1, 4, 0, ident]) + bytes([255]) * 6 + b"\x01\0")
                    await self.send(sender[1], b"\x0a\0\x77\x88")
                    self.assertEqual(await self.binary(sender[0]), b"\x0b\0\x77\x88")
                await asyncio.wait_for(burst(), 3)
                status = await hub.status()
                drops = status[f"broadcast_{limited}_exhausted"]
                self.assertGreater(drops, 0)
                other = "global" if limited == "sender" else "sender"
                self.assertEqual(status[f"broadcast_{other}_exhausted"], 0)
                for _ in range(10 - drops):
                    frame = await self.binary(receiver[0])
                    self.assertEqual(frame[:2], b"\x01\x0c")
                    self.assertEqual(frame[4:10], b"\x42" * 6)
                    self.assertEqual(frame[10:16], b"\xff" * 6)
                    self.assertEqual(frame[16:], b"\x01\0")
                # Ordered reply proves the counters match actual delivered
                # frames: no extra broadcast can hide behind the expected set.
                await self.send(receiver[1], b"\x0a\0\x66\x77")
                self.assertEqual(await self.binary(receiver[0]), b"\x0b\0\x66\x77")
                await self.stop_hub(hub)

    async def test_configured_relay_budget_serves_all_transit_families(self):
        hub = await self.start(relay_send_budget_ms=37)
        source = await self.register(hub, 0x42)
        target = await self.register(hub, 0x43)
        for function, destination, body in (
            (1, b"\x43" * 6, b"\x01\0"),
            (13, b"\x43" * 6, b"\x01\0"),
            (1, b"\xff" * 6, b"\x01\0"),
            (13, b"\xff" * 6, b"\x01\0"),
            (0, b"\x43" * 6, b"\x02\0"),
        ):
            with self.subTest(function=function, broadcast=destination[0] == 255):
                await self.send(source[1], bytes([function, 4, 0, 23]) + destination + body)
                broadcast = destination[0] == 255
                expected = bytes([function, 12 if broadcast else 8, 0, 23]) + b"\x42" * 6
                if broadcast:
                    expected += destination
                self.assertEqual(await self.binary(target[0]), expected + body)
        await self.send(source[1], b"\x0a\0\x66\x77")
        self.assertEqual(await self.binary(source[0]), b"\x0b\0\x66\x77")
        self.assertTrue(all(value == 0 for value in (await hub.status())["outcomes"].values()))
