"""Installed static conflict policy uses the native locked registration decision."""
import asyncio

from rusty_bacnet import ScHub
import test_sc_hub_mtls as mtls
import test_sc_peer_uuid as peer
from test_sc_hub_lifecycle import OUTCOME_KEYS


class HubConflictAdmissionTests(mtls.MtlsFixture):
    open_peer = peer.PeerUuidTests.open_peer
    send = peer.PeerUuidTests.send
    binary = peer.PeerUuidTests.binary

    async def close_peer(self, endpoint):
        _, writer = endpoint
        writer.close()
        try:
            await asyncio.wait_for(writer.wait_closed(), 3)
        except ConnectionResetError:
            pass  # Terminal cleanup only, never an admission-response oracle.

    async def connect(self, endpoint, vmac, uuid):
        reader, writer = endpoint
        await self.send(writer, b"\x06\0\x22\x33" + vmac + uuid + b"\x05\xc4\x05\xc4")
        return await self.binary(reader)

    async def test_static_refusal_preserves_incumbent_while_default_replaces(self):
        for policy in ("allow_all", "deny_uuid_replacement"):
            for moved in (False, True):
                with self.subTest(policy=policy, moved=moved):
                    hub = ScHub(
                        "127.0.0.1:0", self.path("hub.pem"), self.path("hub.key"),
                        mtls.HUB_VMAC, ca_cert=self.path("site.pem"), device_uuid=mtls.HUB_UUID,
                        max_clients=1, admission_policy=policy,
                    )
                    self.addAsyncCleanup(self.stop_hub, hub)
                    await asyncio.wait_for(hub.start(), 3)
                    old = await self.open_peer(await hub.address())
                    self.addAsyncCleanup(self.close_peer, old)
                    old_vmac, uuid = b"\x21" * 6, b"\x11" * 16
                    self.assertEqual((await self.connect(old, old_vmac, uuid))[:4], b"\x07\0\x22\x33")
                    incoming = await self.open_peer(await hub.address())
                    self.addAsyncCleanup(self.close_peer, incoming)
                    new_vmac = b"\x22" * 6 if moved else old_vmac
                    response = await self.connect(incoming, new_vmac, uuid)
                    if policy == "allow_all":
                        self.assertEqual(response[:4], b"\x07\0\x22\x33")
                        # Default Annex AB accepts known UUID, then closes its
                        # incumbent; capacity1 does not prevent replacement.
                        header = await asyncio.wait_for(old[0].readexactly(2), 3)
                        self.assertEqual(header[0], 0x88)
                        live, live_vmac, denied = incoming, new_vmac, 0
                    else:
                        self.assertEqual(response, b"\0\0\x22\x33\x06\1\0\0\3\0\0")
                        live, live_vmac, denied = old, old_vmac, 1
                    # A different UUID colliding with the live VMAC retains the
                    # standard duplicate-VMAC result, not an admin-deny count.
                    collision = await self.open_peer(await hub.address())
                    self.addAsyncCleanup(self.close_peer, collision)
                    self.assertEqual(await self.connect(collision, live_vmac, b"\x44" * 16),
                                     b"\0\0\x22\x33\x06\1\0\0\7\0\x97")
                    await self.send(live[1], b"\x0a\0\x66\x77")
                    self.assertEqual(await self.binary(live[0]), b"\x0b\0\x66\x77")
                    status = await hub.status()
                    self.assertEqual(status["client_count"], 1)
                    self.assertEqual(status["admin_denied"], denied)
                    self.assertEqual(set(status), {
                        "listening", "max_clients", "max_handshakes", "client_count",
                        "handshake_count", "admin_denied", "broadcast_sender_exhausted",
                        "broadcast_global_exhausted", "outcomes",
                    })
                    expected = dict.fromkeys(OUTCOME_KEYS, 0)
                    expected["uuid_replacements"] = int(policy == "allow_all")
                    expected["vmac_collision_rejections"] = 1
                    self.assertEqual(status["outcomes"], expected)
                    self.assertTrue(all(type(value) is int for value in status["outcomes"].values()))
                    await self.stop_hub(hub)

    async def test_unicast_outcomes_count_actual_missing_and_oversized_destinations(self):
        hub = ScHub("127.0.0.1:0", self.path("hub.pem"), self.path("hub.key"),
                    mtls.HUB_VMAC, ca_cert=self.path("site.pem"), device_uuid=mtls.HUB_UUID)
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 3)
        source = await self.open_peer(await hub.address())
        target = await self.open_peer(await hub.address())
        self.addAsyncCleanup(self.close_peer, source)
        self.addAsyncCleanup(self.close_peer, target)
        for endpoint, identity in ((source, 0x21), (target, 0x22)):
            self.assertEqual((await self.connect(endpoint, bytes([identity]) * 6,
                                                 bytes([identity]) * 16))[:4], b"\x07\0\x22\x33")
        for function in (1, 13):
            await self.send(source[1], bytes([function, 4, 0, 1]) + b"\x23" * 6 + b"\x01\0")
            await self.send(source[1], bytes([function, 4, 0, 2]) + b"\x22" * 6 + b"\x01" * 1490)
        # Ordered response proves all four earlier requests were dispatched.
        await self.send(source[1], b"\x0a\0\x66\x77")
        self.assertEqual(await self.binary(source[0]), b"\x0b\0\x66\x77")
        expected = dict.fromkeys(OUTCOME_KEYS, 0)
        expected.update(unicast_no_target=2, unicast_target_limit=2)
        self.assertEqual((await hub.status())["outcomes"], expected)
