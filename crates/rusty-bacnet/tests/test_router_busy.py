"""A router that says it is busy (#2724).

A raw B/IP peer stands in for a router that refuses every routed request
as busy: its reject gives reason 2 and names the request's network. The request
keeps its retries and then times out as a typed call's timeout does, a
BacnetAbortError with reason TSM_TIMEOUT, whose ``busy_router`` and
``busy_network`` name that router and network.
"""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll

AI_0 = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 0)
PV = rb.PropertyIdentifier.PRESENT_VALUE


def busy_router(datagram, source):
    """Reason 2 for the network a routed NPDU names; nothing for others."""
    if datagram[:2] != b"\x81\x0a":
        return None
    npdu = datagram[4:]
    if npdu[0] != 0x01 or not npdu[1] & 0x20:
        return None
    network = npdu[2:4]
    return [bvll(0x0A, b"\x01\x80\x03\x02" + network)]


class RouterBusyTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.client = rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=200)
        await self.client.__aenter__()
        self.addAsyncCleanup(self.stop)
        self.router = RawPeer(busy_router)
        self.addCleanup(self.router.close)

    async def stop(self):
        # The native stop's future needs the running loop.
        await asyncio.wait_for(self.client.stop(), 10)

    async def test_a_busy_router_then_silence_is_a_timeout_naming_it(self):
        target = rb.RoutedTarget(9, "10", router=self.router.address)
        with self.assertRaises(rb.BacnetAbortError) as caught:
            await asyncio.wait_for(self.client.read_property(target, AI_0, PV), 10)
        error = caught.exception
        self.assertEqual(error.reason, rb.AbortReason.TSM_TIMEOUT)
        self.assertEqual((error.busy_router, error.busy_network), (self.router.address, 9))
        self.assertIn("busy for network 9 (router-busy, reason 2)", str(error))
        # Every attempt went: the busy router changed no retry.
        self.assertGreater(len(self.router.received), 1)

    def test_the_attributes_are_none_on_the_classes(self):
        for cls in (rb.BacnetAbortError, rb.BacnetTimeoutError):
            self.assertIsNone(cls.busy_router)
            self.assertIsNone(cls.busy_network)
        self.assertIsNone(rb.BacnetNetworkRejectError.reached_device)


if __name__ == "__main__":
    unittest.main()
