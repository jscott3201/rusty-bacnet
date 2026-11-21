"""A shared endpoint queries a remote router and releases the query on cancellation."""
import asyncio
import unittest

import rusty_bacnet as rb
from bvll_peer import RawPeer, bvll, mac


class RoutedRoutingTableTests(unittest.IsolatedAsyncioTestCase):
    async def test_target_forms_and_missing_next_hop_diagnostic(self):
        async with rb.BipEndpoint(2612, interface="127.0.0.1", port=0,
                                  broadcast_address="127.255.255.255") as endpoint:
            client = await endpoint.client()
            with self.assertRaisesRegex(TypeError, "str, bytes, or RoutedTarget"):
                client.routing_table(object())
            with self.assertRaisesRegex(rb.BacnetError, "explicit next hop") as caught:
                await client.routing_table(rb.RoutedTarget(9, b"\x03"), wait=0)
            self.assertIs(type(caught.exception), rb.BacnetError)

    async def test_endpoint_correlates_routed_acks_and_reuses_a_cancelled_query(self):
        arrivals = asyncio.Queue()
        loop = asyncio.get_running_loop()

        def receive(data, source):
            loop.call_soon_threadsafe(arrivals.put_nowait, (data, source))

        hop = RawPeer(receive)
        self.addCleanup(hop.close)
        target = rb.RoutedTarget(9, b"\x03", router=mac(hop.address))
        async with rb.BipEndpoint(2612, interface="127.0.0.1", port=0,
                                  broadcast_address="127.255.255.255") as endpoint:
            client = await endpoint.client()
            address = await endpoint.local_address()
            # Original-Unicast-NPDU, DER, DNET 9, DADR 3, hop count 255,
            # Initialize-Routing-Table with zero entries.
            query = bvll(0x0A, bytes.fromhex("01 a4 0009 01 03 ff 06 00"))
            async with asyncio.timeout(10):
                first = asyncio.ensure_future(client.routing_table(target, wait=60, total=60))
                data, source = await arrivals.get()
                self.assertEqual(data, query)
                self.assertEqual("%s:%d" % source, address)
                first.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await first
                second = asyncio.ensure_future(client.routing_table(target, quiet=0.05))
                self.assertEqual(await arrivals.get(), (query, source))
                # A table from the immediate hop, and one from another
                # station through that hop, are visible but cannot answer.
                hop.send(bvll(0x0A, bytes.fromhex("01 80 07 01 0063 02 00")), source)
                hop.send(bvll(0x0A, bytes.fromhex("01 88 0009 01 04 07 01 0063 02 00")), source)
                hop.send(bvll(0x0A, bytes.fromhex("01 88 0009 01 03 07 01 002a 01 00")), source)
                table = await second
                self.assertEqual((table.acks, table.truncated), (1, False))
                self.assertEqual([(entry.network, entry.port_id) for entry in table.entries],
                                 [(42, 1)])
                self.assertEqual(await endpoint.local_address(), address)
