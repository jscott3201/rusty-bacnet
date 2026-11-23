"""Isolated router ports (#2716): `BipRouterPort(..., isolated=True)`.

One router, three B/IP ports on 127.0.0.1: network 1 is ordinary, networks 5
and 7 are isolated. A real client sits on network 5. Every send is unicast,
so no BBMD is needed.
"""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer

AI_0 = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 0)
PV = rb.PropertyIdentifier.PRESENT_VALUE


def port(network, **kw):
    return rb.BipRouterPort(network, interface="127.0.0.1", port=0,
                            broadcast_address="127.0.0.1", **kw)


class IsolatedPortSurfaceTests(unittest.TestCase):
    def test_the_flag_is_keyword_only_false_by_default_and_read_only(self):
        self.assertIs(port(1).isolated, False)
        self.assertIs(port(1, isolated=False).isolated, False)
        isolated = port(5, isolated=True)
        self.assertIs(isolated.isolated, True)
        with self.assertRaises(AttributeError):
            isolated.isolated = False
        with self.assertRaises(TypeError):
            rb.BipRouterPort(5, "127.0.0.1", 0, "127.0.0.1", None, None, True)
        self.assertEqual(repr(isolated),
                         "BipRouterPort(network_number=5, interface=\"127.0.0.1\", "
                         "port=0, mode=\"normal\", isolated=True)")
        self.assertTrue(repr(port(1)).endswith("isolated=False)"))
        # Every port may be isolated, and so may one alone.
        rb.BACnetRouter([port(1, isolated=True), port(5, isolated=True)])
        rb.BACnetRouter([port(1, isolated=True)])


class IsolatedPortRoutingTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    async def cleanup(method):
        await asyncio.wait_for(method(), 10)

    async def unanswered(self, target):
        """A read that nothing answers ends as the client's own abort."""
        with self.assertRaises(rb.BacnetAbortError) as caught:
            await asyncio.wait_for(self.client.read_property(target, AI_0, PV), 20)
        self.assertEqual(caught.exception.reason, rb.AbortReason.TSM_TIMEOUT)

    async def start(self, isolated):
        """The router, a server on network 1, a raw node on network 7 and a
        client on network 5 that follows its network reports."""
        self.router = rb.BACnetRouter([port(1), port(5, isolated=isolated),
                                       port(7, isolated=isolated)])
        await self.router.start()
        self.addAsyncCleanup(self.cleanup, self.router.close)
        self.ports = await self.router.ports()

        self.server = rb.BACnetServer(271601, interface="127.0.0.1", port=0)
        self.server.add_analog_input(0, "Network 1 temperature", 62, 21.5)
        await self.server.start()
        self.addAsyncCleanup(self.cleanup, self.server.stop)

        self.node = RawPeer(lambda data, source: ())
        self.addCleanup(self.node.close)

        self.client = rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=250)
        await self.client.__aenter__()
        self.addAsyncCleanup(self.cleanup, self.client.stop)
        self.reports = await self.client.network_reports()

    def through_port_5(self, network, address):
        return rb.RoutedTarget(network, address, router=self.ports[1].address)

    async def reject(self):
        async def first():
            async for report in self.reports:
                if report.kind == "reject_message_to_network":
                    return report
        return await asyncio.wait_for(first(), 10)

    async def test_a_request_between_isolated_ports_is_rejected_and_one_to_an_ordinary_port_routes(self):
        await self.start(isolated=True)
        # Isolated network 5 to ordinary network 1 and back: routed.
        ordinary = self.through_port_5(1, await self.server.local_address())
        value = await asyncio.wait_for(self.client.read_property(ordinary, AI_0, PV), 10)
        self.assertAlmostEqual(value.value, 21.5)

        # Isolated network 5 to isolated network 7: the router answers with
        # Reject-Message-To-Network, reason 1, naming network 7, and the node
        # on network 7 hears nothing. The read fails on that reject, with no
        # retry and no timeout (#2724), naming the router as RoutedTarget
        # takes it.
        blocked = self.through_port_5(7, self.node.address)
        with self.assertRaises(rb.BacnetNetworkRejectError) as caught:
            await asyncio.wait_for(self.client.read_property(blocked, AI_0, PV), 10)
        error = caught.exception
        self.assertEqual((error.reason, error.network), (1, 7))
        self.assertEqual(error.router, self.ports[1].address)
        # One frame went, and the router refused it: the node never got it.
        self.assertIs(error.reached_device, False)
        self.assertIn("no route to network 7 (not-directly-connected, reason 1)", str(error))
        self.assertNotIsInstance(error, rb.BacnetTimeoutError)
        self.assertIsNone(rb.BacnetNetworkRejectError.router)
        report = await self.reject()
        self.assertEqual((report.reason, report.network), (1, 7))
        self.assertEqual(report.source_mac, self.ports[1].mac)
        self.assertEqual(self.node.received, [])

        # The ordinary port still routes afterwards.
        value = await asyncio.wait_for(self.client.read_property(ordinary, AI_0, PV), 10)
        self.assertAlmostEqual(value.value, 21.5)

    async def test_without_the_flag_the_same_request_reaches_the_other_port(self):
        await self.start(isolated=False)
        target = self.through_port_5(7, self.node.address)
        await self.unanswered(target)  # the raw node never answers
        self.assertTrue(self.node.received)
        datagram, source = self.node.received[0]
        # From the router's port on network 7, with the client's network 5
        # and its B/IP address as the source.
        self.assertEqual("%s:%d" % source, self.ports[2].address)
        self.assertEqual(datagram[4:6], bytes([0x01, 0x0C]))
        self.assertEqual(datagram[6:13], bytes([0x00, 0x05, 0x06, 127, 0, 0, 1]))


if __name__ == "__main__":
    unittest.main()
