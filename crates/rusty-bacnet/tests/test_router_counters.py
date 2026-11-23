"""The router's counters and options (#2725): `BACnetRouter.counters()` and
the keyword-only `remote_table_updates`, `next_hop_rejects` and
`control_policy`.

Every router port is B/IP on 127.0.0.1 with its own UDP port, and raw peers
send the network messages by unicast, so no BBMD is needed.
"""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll
from test_bip_modes import eventually, stub_fields

ORIGINAL_UNICAST = 0x0A


def port(network, **kw):
    return rb.BipRouterPort(network, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1", **kw)


def i_am_router(*networks):
    """I-Am-Router-To-Network for `networks`."""
    return bytes([0x01, 0x80, 0x01]) + b"".join(n.to_bytes(2, "big") for n in networks)


def table_update_from(snet, network, port_id):
    """Initialize-Routing-Table with one entry, from station 1 on `snet`."""
    return bytes([0x01, 0x88]) + snet.to_bytes(2, "big") + bytes([1, 1, 0x06, 1]) \
        + network.to_bytes(2, "big") + bytes([port_id, 0])


def reject(network, reason=1):
    """Reject-Message-To-Network for `network`, addressed to the router."""
    return bytes([0x01, 0x80, 0x03, reason]) + network.to_bytes(2, "big")


class RouterOptionSurfaceTests(unittest.TestCase):
    def test_options_are_keyword_only_checked_and_read_back(self):
        plain = rb.BACnetRouter([port(1)])
        self.assertEqual(
            (plain.remote_table_updates, plain.next_hop_rejects, plain.control_policy),
            (False, False, "permissive"))
        set_ = rb.BACnetRouter([port(1)], remote_table_updates=True, next_hop_rejects=True)
        self.assertEqual(
            (set_.remote_table_updates, set_.next_hop_rejects, set_.control_policy),
            (True, True, "permissive"))
        hardened = rb.BACnetRouter([port(1)], control_policy="hardened")
        self.assertEqual(hardened.control_policy, "hardened")
        for name in ("remote_table_updates", "next_hop_rejects", "control_policy"):
            with self.assertRaises(AttributeError):
                setattr(set_, name, None)
        with self.assertRaisesRegex(ValueError, "control_policy must be 'permissive' or 'hardened'"):
            rb.BACnetRouter([port(1)], control_policy="strict")
        # With no authorizer, "hardened" denies every table update and every
        # reject, so these options beside it could never apply.
        for flag, what in (("remote_table_updates", "routing-table update"),
                           ("next_hop_rejects", "reject")):
            with self.subTest(flag=flag), self.assertRaisesRegex(
                    ValueError, f"{flag}=True does nothing with control_policy='hardened'.*{what}"):
                rb.BACnetRouter([port(1)], control_policy="hardened", **{flag: True})
        for flag in ("remote_table_updates", "next_hop_rejects"):
            with self.subTest(flag=flag), self.assertRaises(TypeError):
                rb.BACnetRouter([port(1)], **{flag: 1})
        with self.assertRaises(TypeError):
            rb.BACnetRouter([port(1)], None, True)


class RouterCounterTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def cleanup(method):
        await asyncio.wait_for(method(), 10)

    async def started(self, *ports, **options):
        router = rb.BACnetRouter(list(ports), **options)
        await router.start()
        self.addAsyncCleanup(self.cleanup, router.close)
        return router, await router.ports()

    def peer(self):
        peer = RawPeer(lambda data, source: ())
        self.addCleanup(peer.close)
        return peer

    @staticmethod
    def send(peer, npdu, to):
        host, udp = to.address.rsplit(":", 1)
        peer.send(bvll(ORIGINAL_UNICAST, npdu), (host, int(udp)))

    async def until(self, router, check, seconds=10):
        async def met():
            counts = await router.counters()
            return counts if check(counts) else None
        return await eventually(met, seconds)

    async def routes(self, router):
        return {route.network_number: route for route in await router.routes()}

    async def test_the_counts_are_the_stubs_names_and_need_a_running_router(self):
        router = rb.BACnetRouter([port(1)])
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await router.counters()
        await router.start()
        counts = await router.counters()
        self.assertEqual(set(counts), stub_fields("RouterCounters"))
        self.assertEqual(set(counts["control"]), stub_fields("RouterControlCounters"))
        for name, totals in counts["control"].items():
            self.assertEqual(set(totals), stub_fields("RouterControlTotals"), name)
            self.assertTrue(all(isinstance(n, int) for n in totals.values()))
        self.assertTrue(all(isinstance(counts[n], int) for n in counts if n != "control"))
        await router.close()
        with self.assertRaisesRegex(RuntimeError, "not started"):
            await router.counters()

    async def test_isolated_ports_count_what_they_stop(self):
        router, ports = await self.started(port(1), port(5, isolated=True),
                                           port(7, isolated=True))
        node = self.peer()
        # From network 5: a Who-Is for station 1 on network 7, and a claim
        # to reach network 7, which is directly on the other isolated port.
        self.send(node, bytes([0x01, 0x20, 0x00, 0x07, 0x01, 0x01, 0xFF, 0x10, 0x08]), ports[1])
        self.send(node, i_am_router(7), ports[1])
        counts = await self.until(
            router, lambda c: c["isolated_drops"] and c["isolated_refusals"])
        self.assertEqual((counts["isolated_drops"], counts["isolated_refusals"]), (1, 1))
        self.assertTrue((await self.routes(router))[7].is_direct)

    async def test_a_hardened_policy_learns_no_route_and_counts_why(self):
        for policy in ("permissive", "hardened"):
            with self.subTest(policy=policy):
                router, ports = await self.started(port(1), port(2), control_policy=policy)
                announcer = self.peer()
                self.send(announcer, i_am_router(9), ports[0])
                counts = await self.until(router, lambda c: sum(c["control"]["i_am"].values()))
                i_am = counts["control"]["i_am"]
                if policy == "permissive":
                    self.assertEqual(i_am, {"allow_total": 1, "deny_total": 0,
                                            "policy_deny_total": 0})
                    self.assertEqual(counts["denied"], 0)
                    self.assertIn(9, await self.routes(router))
                else:
                    self.assertEqual(i_am, {"allow_total": 0, "deny_total": 1,
                                            "policy_deny_total": 1})
                    self.assertEqual(counts["denied"], 1)
                    self.assertNotIn(9, await self.routes(router))
                await router.close()

    async def test_a_table_update_from_another_network_needs_the_option(self):
        for remote in (False, True):
            with self.subTest(remote_table_updates=remote):
                router, ports = await self.started(port(1), port(2),
                                                   remote_table_updates=remote)
                manager = self.peer()
                # Through a router from network 77: network 9 via port ID 1.
                self.send(manager, table_update_from(77, 9, 1), ports[0])
                if remote:
                    async def learned():
                        return 9 in await self.routes(router)
                    await eventually(learned, 10)
                    counts = await router.counters()
                    self.assertEqual(counts["control"]["init_mgmt"]["allow_total"], 1)
                    self.assertEqual(counts["denied"], 0)
                else:
                    counts = await self.until(router, lambda c: c["denied"])
                    self.assertEqual(counts["denied"], 1)
                    # Dropped before the policy hears of it.
                    self.assertEqual(sum(counts["control"]["init_mgmt"].values()), 0)
                    self.assertNotIn(9, await self.routes(router))
                await router.close()

    async def test_only_the_next_router_changes_a_route_with_a_reject(self):
        for next_hop_rejects in (False, True):
            with self.subTest(next_hop_rejects=next_hop_rejects):
                router, ports = await self.started(port(1), port(2),
                                                   next_hop_rejects=next_hop_rejects)
                next_router, other = self.peer(), self.peer()
                self.send(next_router, i_am_router(9), ports[0])

                async def learned():
                    return 9 in await self.routes(router)
                await eventually(learned, 10)
                self.send(other, reject(9), ports[0])
                counts = await self.until(router, lambda c: c["control"]["reject"]["allow_total"])
                if next_hop_rejects:
                    counts = await self.until(router, lambda c: c["not_next_hop"])
                    self.assertEqual(counts["not_next_hop"], 1)
                    self.assertEqual((await self.routes(router))[9].reachability, "reachable")
                else:
                    self.assertEqual(counts["not_next_hop"], 0)

                    async def marked():
                        return (await self.routes(router))[9].reachability == "unreachable"
                    await eventually(marked, 10)
                await router.close()


if __name__ == "__main__":
    unittest.main()
